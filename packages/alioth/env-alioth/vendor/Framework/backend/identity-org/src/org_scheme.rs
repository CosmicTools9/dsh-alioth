//! D-2 组织方案（org-scheme）—— 方案资产 draft/active 状态机；publish-direct 已弃用
//!
//! D-2 概念设计的 draft/版本/快照行首版以「publish 动作即方案」直接发布
//! （响应 `scheme` 段回显，不落库）；B-1 起补 `isahl_auth.org_design_scheme`
//! 方案资产表（draft→active→superseded 状态机，见文件尾部「方案资产」节）。
//! B-2（本文件）：activate 已接入差异派生（模板落岗差额 + 策略类幂等重放）、
//! max_heads 编制上限与激活派生限流；publish-direct 端点保留为 @deprecated
//! 兼容（草稿即发），新方案一律走 draft→activate。
//! 1. **预检段（写前 fail-fast）**：全部 `policy_class_codes` 必须命中
//!    派生器只认 active 规范）；任一缺失即整体 400，不产生任何写。
//! 2. **模板落岗段**：每个 position_template 条目落岗为真实岗位行（`_f_ IS NULL`）：
//!    - `template_id` 引用：校验范例行在册（`_f_='设计' AND _t_='范例'`、
//!      tpl_id NULL、未删）→ 实例化（复用 D-2a instantiate 语义：ck_category
//!      继承、notice=范例名，同范例已有在册实例追加 `-{序号}` 消歧）；
//!      引用形态仅携带 template_id——与内联字段（name/categoryCode/maxHeads/
//!      departmentId）互斥 400（activate 预检 fail-fast）；
//!    - 内联（`category_code` 必填）：校验 `zc_id_category` **基表行**
//!      （tableoid 过滤，B-1 align-cognition-ua-category 同源——子族字典不派生）
//!      → 先建范例行（编制上限落 `projection` 文本载荷，comments 回归自由
//!      文本）→ 随即实例化落岗。
//!    B-2 activate 差异派生幂等：内联按 (category_code, name) 去重（命中复用既有
//!    设计范例行）；链上零在册实例才补建首实例——同类别多部门条目（同
//!    (category, name)、不同 departmentId）共享链上单实例（各自经
//!    org_rr_position 桥挂接，不各自建实例），重复 activate 零新增（差额补齐）。
//!    publish-direct（@deprecated）每次发布新增实例（可重复形成多批次岗位）；
//!    派生态幂等见第 3 段。
//! 3. **类派生段**：逐 code 调 [`common::ngac_policy::derive_from_class`]——
//!    该函数自带单事务（pool 级 begin/commit，重复调用零新增），故派生不在
//!    模板事务内；幂等 upsert 保证重发安全。
//!
//! 端点挂载：各 isahl-db 服务壳 `register_service_routes` 内
//! `.configure(identity_org::org_scheme::register)`（仿 org_tree）。

use actix_web::{web, HttpRequest, HttpResponse};
use common::audit::{record_audit_event, Decision};
use common::context::{extract_user_email, require_auth};
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

// ═══════════════════════════════════════════════════════════
// 方案资产表 ensure（B-1：isahl_auth.org_design_scheme）
// ═══════════════════════════════════════════════════════════

/// 方案资产表惰性 ensure（运行时幂等自愈，同 handlers::subjects ensure 先例；
/// isahl_auth 为 NGAC agent 可建 schema——无该 schema 的部署自愈建 schema 再建表）。
/// AtomicBool 仅作免重复标记——DDL 幂等，并发重入无害。
static ORG_DESIGN_SCHEME_ENSURED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 建表 + 部分唯一/状态索引；失败返回 Err（写端点 500，下次调用重试）。
pub(crate) async fn ensure_org_design_scheme(pool: &PgPool) -> Result<(), ApiError> {
    use std::sync::atomic::Ordering;
    if ORG_DESIGN_SCHEME_ENSURED.load(Ordering::Relaxed) {
        return Ok(());
    }
    let result = async {
        sqlx::query("CREATE SCHEMA IF NOT EXISTS isahl_auth")
            .execute(pool)
            .await?;
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS isahl_auth.org_design_scheme (
                id BIGINT PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
                code VARCHAR(64) NOT NULL,
                title TEXT NOT NULL,
                scope JSONB NOT NULL DEFAULT '{}',
                content JSONB NOT NULL DEFAULT '{}',
                state TEXT NOT NULL DEFAULT 'draft'
                    CHECK (state IN ('draft', 'active', 'superseded')),
                version INT NOT NULL DEFAULT 1,
                created_by_id BIGINT,
                updated_by_id BIGINT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                deleted_at TIMESTAMPTZ,
                deleted_by_id BIGINT
            )"#,
        )
        .execute(pool)
        .await?;
        // 活行 code 唯一（软删行不占位，允许多版本历史留痕）+ state 查询索引
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS uq_org_design_scheme_code_live \
             ON isahl_auth.org_design_scheme (code) WHERE deleted_at IS NULL",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_org_design_scheme_state \
             ON isahl_auth.org_design_scheme (state) WHERE deleted_at IS NULL",
        )
        .execute(pool)
        .await
    }
    .await;
    match result {
        Ok(_) => {}
        // 并发首次 ensure 的 pg_class 唯一索引竞态（23505）——已建成，视为成功
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23505") => {}
        Err(e) => return Err(ApiError::from_sqlx(e)),
    }
    ORG_DESIGN_SCHEME_ENSURED.store(true, Ordering::Relaxed);
    Ok(())
}

// ═══════════════════════════════════════════════════════════
// DTO — 方案发布请求（camelCase；快照形态即方案资产）
// ═══════════════════════════════════════════════════════════

/// 方案内单个岗位编制条目（B-2 起兼作方案资产 content.templates 条目形态）：
/// `template_id` 引用（范例行，仅实例化）与内联形态二选一——**引用仅携带
/// template_id**，内联形态 `{name, category_code, max_heads, department_id?}`；
/// 引用 + 任一内联字段（name/categoryCode/maxHeads/departmentId）同时给出 → 400
/// （activate 预检互斥；draft 写入不校验，activate 时 fail-fast）。
///
/// 激活差异派生幂等键（与 realize_scheme_template 实现同源）：
/// - 设计范例行去重键 (category_code, name)：内联命中复用、未命中新建；
/// - 实例键 = 链在册实例数：链上零在册实例才补建首实例——**同类别多部门条目
///   共享链上单实例**（各自经 org_rr_position 桥挂接）；
/// - 部门编制键 (department_id, 链)：链上已挂该组织即视为已实现。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemePositionTemplate {
    /// 引用既有岗位编制范例行（D-2a 设计态行）→ 仅实例化落岗（与内联字段互斥）。
    #[serde(default)]
    pub template_id: Option<i64>,
    /// 内联建范例名（缺省回退 category_code）。
    #[serde(default)]
    pub name: Option<String>,
    /// 内联必填：岗位类别 code（zc_id_category 基表行，子族字典不派生）。
    #[serde(default)]
    pub category_code: Option<String>,
    /// 编制上限（仅内联形态；template_id 引用互斥 → 400）：落内联范例行
    /// `projection` 文本载荷（comments 回归自由文本）供 org_tree 实例化端点读回执行；
    /// 激活派生实例化步同样判定（step-scope：已达上限仅跳过本次实例化，
    /// 不 400/计失败）。缺省 = 不设限。
    #[serde(default)]
    pub max_heads: Option<i64>,
    /// 编制部门（可选，仅内联形态；template_id 引用互斥 → 400）：激活派生时
    /// 该 (category_code, name) 链的在册实例经 org_rr_position 桥挂到该组织
    /// （同类别多部门共享单实例，各自挂接）。
    #[serde(default)]
    pub department_id: Option<i64>,
}

/// POST /org-scheme/publish 请求体（= 方案资产 JSONB 快照形态）
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishSchemeRequest {
    /// 方案名（必填，非空）
    pub title: String,
    /// 方案版本（缺省 v1）
    #[serde(default = "default_version")]
    pub version: String,
    /// 部门快照占位（首版不落部门行，仅随方案回显）
    #[serde(default)]
    pub department_snapshot: Option<serde_json::Value>,
    /// 岗位编制条目（可为空 = 仅派生策略类）
    #[serde(default)]
    pub position_templates: Vec<SchemePositionTemplate>,
    /// 策略类 code 列表（state=active 才派生；可为空 = 仅落岗）
    #[serde(default)]
    pub policy_class_codes: Vec<String>,
}

fn default_version() -> String {
    "v1".to_string()
}

/// 已落岗实例摘要
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceOut {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    #[serde(with = "common::serde_zuid::opt")]
    pub template_id: Option<i64>,
}

/// 单策略类派生摘要（镜像 common::ngac_policy::DeriveStats）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassDeriveOut {
    pub code: String,
    pub ua_name: String,
    pub ua_created: i64,
    pub oa_created: i64,
    pub associations_created: i64,
    pub rules_processed: usize,
}

/// 发布统计（响应 body data）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemePublishStats {
    /// 方案资产快照回显（title/version/department_snapshot/原始条目）
    pub scheme: serde_json::Value,
    /// 本次发布新建范例行数（仅内联条目）
    pub templates_created: usize,
    /// 本次发布落岗实例
    pub instances: Vec<InstanceOut>,
    /// 逐策略类派生结果
    pub classes: Vec<ClassDeriveOut>,
    /// 派生态合计
    pub ua_created_total: i64,
    pub oa_created_total: i64,
    pub associations_created_total: i64,
}

// ═══════════════════════════════════════════════════════════
// 发布编排
// ═══════════════════════════════════════════════════════════

/// @deprecated — POST /org-scheme/publish（草稿即发兼容端点，D-2 直落语义）。
/// 保留运行兼容但不再演进：重复发布同方案会形成多批次岗位且无编制上限校验。
/// 新方案一律走 draft→activate 资产链：POST /org-scheme → PATCH /org-scheme/{id}
/// → POST /org-scheme/{id}/activate（幂等差异派生 + max_heads 上限 + 条目限流）。
/// 方案事务编排（见模块文档）；权限同岗位写端点：`positions` 资源 `create`。
pub async fn publish_scheme_handler(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<PublishSchemeRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "positions", 0, "create").await?;
    let stats = publish_scheme(pool.get_ref(), body.into_inner()).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(stats)))
}

/// 路由注册（服务壳 `register_service_routes` configure 挂载）
pub fn register(cfg: &mut actix_web::web::ServiceConfig) {
    cfg.service(web::resource("/org-scheme/publish").route(web::post().to(publish_scheme_handler)))
        .service(
            web::resource("/org-scheme/active").route(web::get().to(get_active_scheme_handler)),
        )
        .service(web::resource("/org-scheme").route(web::post().to(create_scheme_handler)))
        .service(
            web::resource("/org-scheme/{id}")
                .route(web::get().to(get_scheme_handler))
                .route(web::patch().to(update_scheme_handler)),
        )
        .service(
            web::resource("/org-scheme/{id}/activate")
                .route(web::post().to(activate_scheme_handler)),
        );
}

/// 方案发布核心（可脱离 HTTP 复用）
pub async fn publish_scheme(
    pool: &PgPool,
    req: PublishSchemeRequest,
) -> Result<SchemePublishStats, ApiError> {
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err(ApiError::BadRequest("title 不能为空".into()));
    }
    let version = if req.version.trim().is_empty() {
        default_version()
    } else {
        req.version.trim().to_string()
    };

    // ── 段 1：策略类预检（写前 fail-fast；state=active 才可派生） ──
    let mut class_codes: Vec<String> = Vec::new();
    for code in req.policy_class_codes {
        let code = code.trim().to_string();
        if code.is_empty() || class_codes.contains(&code) {
            continue;
        }
        let active: bool = sqlx::query_scalar(
            "SELECT COUNT(*) > 0 FROM isahl_auth.org_policy_class \
             WHERE code = $1 AND state = 'active' AND deleted_at IS NULL",
        )
        .bind(&code)
        .fetch_one(pool)
        .await
        .map_err(ApiError::from_sqlx)?;
        if !active {
            return Err(ApiError::BadRequest(format!(
                "策略类 code '{}' 不存在或未激活（state='active' 才可派生）",
                code
            )));
        }
        class_codes.push(code);
    }

    // ── 段 2：模板落岗（单事务：内联建范例 + 全条目实例化） ──
    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve_conn(&mut *tx, ("TX", "FJA", "↓_GG"))
            .await
            .map_err(ApiError::from_sqlx)?;
    let mut templates_created = 0usize;
    let mut instances: Vec<InstanceOut> = Vec::new();
    for item in &req.position_templates {
        // 确定范例来源：(template_id 引用) XOR (内联建范例行)
        let tpl: (i64, String, i64) = match item.template_id {
            Some(tid) => {
                if item.category_code.is_some() || item.name.is_some() {
                    return Err(ApiError::BadRequest(
                        "templateId 引用与内联字段（name/categoryCode）互斥".into(),
                    ));
                }
                let row: Option<(i64, String, Option<i64>)> = sqlx::query_as(
                    r#"SELECT id, notice::text, ck_category
                       FROM isahl."zc_id_subj-position"
                       WHERE id = $1 AND deleted_at IS NULL
                         AND _f_ = '设计' AND _t_ = '范例' AND tpl_id IS NULL"#,
                )
                .bind(tid)
                .fetch_optional(&mut *tx)
                .await
                .map_err(ApiError::from_sqlx)?;
                let Some((id, notice, category_id)) = row else {
                    return Err(ApiError::NotFound(format!(
                        "Position template not found: {}",
                        tid
                    )));
                };
                if category_id.is_none() {
                    // 建范例强制类别非空；NULL 仅脏数据可达——fail-closed 拒绝落岗
                    return Err(ApiError::BadRequest(format!(
                        "Position template {} 缺类别（ck_category NULL），不可实例化",
                        tid
                    )));
                }
                (id, notice, category_id.unwrap())
            }
            None => {
                let category = item
                    .category_code
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ApiError::BadRequest(
                            "内联条目 category_code 必填：岗位必须绑定 zc_id_category 基表行"
                                .into(),
                        )
                    })?;
                let category_id: i64 = sqlx::query_scalar(
                    r#"SELECT c.id FROM isahl.zc_id_category c
                       WHERE c.code = $1 AND c.deleted_at IS NULL
                         AND c.tableoid = 'isahl.zc_id_category'::regclass"#,
                )
                .bind(category)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::from_sqlx)?;
                let name = item
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(category)
                    .to_string();
                // 编制上限落既有文本载荷列 projection（comments 回归自由文本；
                // 零 DDL——本表无 qk_* 标量槽位/数值列，映射契约见
                // handlers::org_tree::headcount_carriers）
                let (projection, _) =
                    crate::handlers::org_tree::headcount_carriers(item.max_heads, None);
                let id: i64 = sqlx::query_scalar(
                    r#"INSERT INTO isahl."zc_id_subj-position"
                         (notice, code, comments, projection, ck_category, tpl_id, _f_, _t_, dk_scene, dk_factor, dk_function)
                       VALUES ($1, NULL, NULL, $2, $3, NULL, '设计', '范例', $4, $5, $6)
                       RETURNING id"#,
                )
                .bind(&name)
                .bind(&projection)
                .bind(category_id)
                .bind(dk_scene)
                .bind(dk_factor)
                .bind(dk_function)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::from_sqlx)?;
                templates_created += 1;
                (id, name, category_id)
            }
        };
        let (tpl_id, tpl_name, category_id) = tpl;
        // notice = 范例名；同范例已有在册实例 → 追加序号消歧（首实例不带序号）
        let live: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM isahl.\"zc_id_subj-position\" WHERE tpl_id = $1 AND deleted_at IS NULL",
        )
        .bind(tpl_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        let notice = if live == 0 {
            tpl_name.clone()
        } else {
            format!("{}-{}", tpl_name, live + 1)
        };
        let instance_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_subj-position" (notice, comments, ck_category, tpl_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, '', $2, $3, $4, $5, $6)
               RETURNING id"#,
        )
        .bind(&notice)
        .bind(category_id)
        .bind(tpl_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        instances.push(InstanceOut {
            id: instance_id,
            name: notice,
            template_id: Some(tpl_id),
        });
    }
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    // ── 段 3：类派生（逐类自事务幂等，重复发布零新增） ──
    let mut classes: Vec<ClassDeriveOut> = Vec::new();
    let mut totals = (0i64, 0i64, 0i64);
    for code in &class_codes {
        let class_id: i64 = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.org_policy_class \
             WHERE code = $1 AND state = 'active' AND deleted_at IS NULL",
        )
        .bind(code)
        .fetch_one(pool)
        .await
        .map_err(ApiError::from_sqlx)?;
        let stats = common::ngac_policy::derive_from_class(pool, class_id)
            .await
            .map_err(ApiError::from_sqlx)?;
        totals.0 += stats.ua_created;
        totals.1 += stats.oa_created;
        totals.2 += stats.associations_created;
        classes.push(ClassDeriveOut {
            code: code.clone(),
            ua_name: stats.ua_name,
            ua_created: stats.ua_created,
            oa_created: stats.oa_created,
            associations_created: stats.associations_created,
            rules_processed: stats.rules_processed,
        });
    }

    // 方案资产快照回显（department_snapshot 原样携带）
    let scheme = serde_json::json!({
        "title": title,
        "version": version,
        "departmentSnapshot": req.department_snapshot,
        "positionTemplateCount": req.position_templates.len(),
        "policyClassCodes": class_codes,
    });

    Ok(SchemePublishStats {
        scheme,
        templates_created,
        instances,
        classes,
        ua_created_total: totals.0,
        oa_created_total: totals.1,
        associations_created_total: totals.2,
    })
}

// ═══════════════════════════════════════════════════════════
// 方案资产（B-1）— isahl_auth.org_design_scheme draft/active/superseded
// ═══════════════════════════════════════════════════════════

/// POST /org-scheme 请求体：title 必填；code 可空（缺省 = content hash，
/// 同内容草稿按活行 code 唯一拒绝）；scope/content 可空（缺省 {}，可 PATCH 补齐）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSchemeRequest {
    pub title: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub scope: Option<serde_json::Value>,
    #[serde(default)]
    pub content: Option<serde_json::Value>,
}

/// PATCH /org-scheme/{id} 请求体：任一字段提供即更新（缺省保持原值）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSchemeRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub scope: Option<serde_json::Value>,
    #[serde(default)]
    pub content: Option<serde_json::Value>,
}

/// 方案资产行响应（id/actor id 走 zuid 字符串序列化）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemeOut {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: String,
    pub title: String,
    pub scope: serde_json::Value,
    pub content: serde_json::Value,
    pub state: String,
    pub version: i32,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub created_by_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub updated_by_id: Option<i64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
struct SchemeRow {
    id: i64,
    code: String,
    title: String,
    scope: serde_json::Value,
    content: serde_json::Value,
    state: String,
    version: i32,
    created_by_id: Option<i64>,
    updated_by_id: Option<i64>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<SchemeRow> for SchemeOut {
    fn from(r: SchemeRow) -> Self {
        SchemeOut {
            id: r.id,
            code: r.code,
            title: r.title,
            scope: r.scope,
            content: r.content,
            state: r.state,
            version: r.version,
            created_by_id: r.created_by_id,
            updated_by_id: r.updated_by_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// 单行读取（活行；NotFound → 404）。
async fn fetch_scheme(pool: &PgPool, id: i64) -> Result<SchemeOut, ApiError> {
    let row: Option<SchemeRow> = sqlx::query_as(
        "SELECT id, code, title, scope, content, state, version, created_by_id, \
                updated_by_id, created_at, updated_at \
         FROM isahl_auth.org_design_scheme \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    row.map(SchemeOut::from)
        .ok_or_else(|| ApiError::NotFound(format!("org scheme not found: {}", id)))
}

/// 操作级审计（actor email；record_audit_event 非阻塞语义——失败仅 warn）。
async fn audit_scheme(pool: &PgPool, user_id: i64, email: &str, id: i64, operation: &str) {
    if let Err(e) = record_audit_event(
        pool,
        user_id,
        email,
        &format!("org_scheme:{}", id),
        operation,
        &Decision::Permit,
    )
    .await
    {
        common::telemetry::warn!(
            "org_scheme 审计 {} 失败（scheme {}，actor {}）: {}",
            operation,
            id,
            user_id,
            e
        );
    }
}

/// POST /org-scheme — 建 draft 资产行（活行 code 唯一；冲突 → 400）。
pub async fn create_scheme(
    pool: &PgPool,
    user_id: i64,
    email: &str,
    req: CreateSchemeRequest,
) -> Result<SchemeOut, ApiError> {
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err(ApiError::BadRequest("title 不能为空".into()));
    }
    let scope = req.scope.unwrap_or_else(|| serde_json::json!({}));
    let content = req.content.unwrap_or_else(|| serde_json::json!({}));
    // code 缺省 = content hash（jsonb 规范化文本 md5，'os-' 前缀）
    let code: String = match req.code.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(c) => c.to_string(),
        None => {
            let h: String = sqlx::query_scalar("SELECT md5($1::text)")
                .bind(&content)
                .fetch_one(pool)
                .await
                .map_err(ApiError::from_sqlx)?;
            format!("os-{}", h)
        }
    };
    if code.len() > 64 {
        return Err(ApiError::BadRequest("code 超长（≤64）".into()));
    }

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;
    let dup: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.org_design_scheme \
         WHERE code = $1 AND deleted_at IS NULL)",
    )
    .bind(&code)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    if dup {
        return Err(ApiError::BadRequest(format!(
            "方案 code '{}' 已存在活行（活行 code 唯一；同内容草稿请 PATCH 既有 draft）",
            code
        )));
    }
    let insert: Result<i64, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO isahl_auth.org_design_scheme \
         (code, title, scope, content, created_by_id) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(&code)
    .bind(&title)
    .bind(&scope)
    .bind(&content)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await;
    let id = match insert {
        Ok(id) => id,
        // 并发竞态下部分唯一索引兜底（23505）→ 同样按活行冲突 400 语义
        Err(sqlx::Error::Database(dbe)) if dbe.code().as_deref() == Some("23505") => {
            return Err(ApiError::BadRequest(format!(
                "方案 code '{}' 已存在活行（活行 code 唯一）",
                code
            )))
        }
        Err(e) => return Err(ApiError::from_sqlx(e)),
    };
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    audit_scheme(pool, user_id, email, id, "scheme.create").await;
    fetch_scheme(pool, id).await
}

/// GET /org-scheme/active — 当前生效方案（state=active 最新；激活迁移保证单活行，
/// 按 id 倒序取顶兜底）。无 active → 404。
pub async fn get_active_scheme(pool: &PgPool) -> Result<SchemeOut, ApiError> {
    let row: Option<SchemeRow> = sqlx::query_as(
        "SELECT id, code, title, scope, content, state, version, created_by_id, \
                updated_by_id, created_at, updated_at \
         FROM isahl_auth.org_design_scheme \
         WHERE state = 'active' AND deleted_at IS NULL \
         ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    row.map(SchemeOut::from).ok_or_else(|| {
        ApiError::NotFound("当前无 active 方案（先 POST /org-scheme 建 draft 再激活）".into())
    })
}

/// PATCH /org-scheme/{id} — 仅 draft 可编辑；任一字段差量更新；version+1。
pub async fn update_scheme(
    pool: &PgPool,
    user_id: i64,
    email: &str,
    id: i64,
    req: UpdateSchemeRequest,
) -> Result<SchemeOut, ApiError> {
    let title = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if title.is_none() && req.scope.is_none() && req.content.is_none() {
        return Err(ApiError::BadRequest(
            "至少提供 title/scope/content 之一".into(),
        ));
    }
    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;
    let state: Option<String> = sqlx::query_scalar(
        "SELECT state FROM isahl_auth.org_design_scheme \
         WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    let Some(state) = state else {
        return Err(ApiError::NotFound(format!("org scheme not found: {}", id)));
    };
    if state != "draft" {
        return Err(ApiError::BadRequest(format!(
            "仅 draft 方案可编辑（当前 state='{}'）",
            state
        )));
    }
    sqlx::query(
        "UPDATE isahl_auth.org_design_scheme SET \
            title = COALESCE($2, title), \
            scope = COALESCE($3, scope), \
            content = COALESCE($4, content), \
            version = version + 1, \
            updated_at = NOW(), \
            updated_by_id = $5 \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(title)
    .bind(&req.scope)
    .bind(&req.content)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    audit_scheme(pool, user_id, email, id, "scheme.update").await;
    fetch_scheme(pool, id).await
}

/// POST /org-scheme/{id}/activate — draft→active（B-2：状态迁移 + 差异派生）。
///
/// 编排（预检 fail-fast 语义对齐 publish-direct）：
/// 1. **预检段（写前 fail-fast，零写）**：draft 行锁读取（非 draft → 400）→
///    content 按资产快照形态解析（`templates` 数组 + `policyClassCodes`；
///    缺失键容错为空）→ 派生条目限流（templates + 去重类码 ≤ 20，超出 400
///    提示分批/先建子方案）→ 逐条目语义校验：templateId 引用范例行在册且
///    类别非空、且**引用仅携带 templateId**（name/categoryCode/maxHeads/
///    departmentId 互斥 → 400）；内联 categoryCode 必填且命中 `zc_id_category`
///    基表行、maxHeads>0；departmentId 命中在册组织（department ∪
///    non-banking-legal）→ 全部 policyClassCodes 必须 state='active'。
/// 2. **状态迁移段（单事务）**：旧 active → superseded（版本留痕），
///    目标 draft → active。
/// 3. **差异派生段（事务外，幂等；单条目失败 warn 不阻断——状态已迁移，
///    差额留痕由下次 activate 幂等补齐，文档化边界）**：逐模板条目幂等落岗
///    ——内联按 (category, name) 去重复用/新建设计范例行，链上零在册实例才
///    补建首实例（同类别多部门条目共享该单实例，各自经 (department_id, 链)
///    键桥挂接；引用形态同按链在册实例数补建）；逐策略类调 derive_from_class
///    幂等重放。汇总统计落日志。
pub async fn activate_scheme(
    pool: &PgPool,
    user_id: i64,
    email: &str,
    id: i64,
) -> Result<SchemeOut, ApiError> {
    // ── 段 1+2：行锁读 draft + 预检（同事务读，写前 fail-fast） ──
    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;
    let row: Option<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT state, content FROM isahl_auth.org_design_scheme \
         WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    let Some((state, content)) = row else {
        return Err(ApiError::NotFound(format!("org scheme not found: {}", id)));
    };
    if state != "draft" {
        return Err(ApiError::BadRequest(format!(
            "仅 draft 方案可激活（当前 state='{}'）",
            state
        )));
    }
    let plan = validate_activation_content(pool, content).await?;

    // ── 段 2：状态迁移（旧 active → superseded；目标 draft → active） ──
    sqlx::query(
        "UPDATE isahl_auth.org_design_scheme SET state = 'superseded', \
            updated_at = NOW(), updated_by_id = $1 \
         WHERE state = 'active' AND deleted_at IS NULL AND id <> $2",
    )
    .bind(user_id)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    sqlx::query(
        "UPDATE isahl_auth.org_design_scheme SET state = 'active', \
            updated_at = NOW(), updated_by_id = $1 \
         WHERE id = $2 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    // ── 段 3：差异派生（事务外；单条目失败 warn，不阻断状态迁移） ──
    let stats = derive_activation_diff(pool, &plan, user_id).await;

    audit_scheme(pool, user_id, email, id, "scheme.activate").await;
    // 派生统计落日志留痕（activate 审计事件 comments 槽固定无 detail）
    let summary = format!(
        "org_scheme {} activate diff-derive: templates_created={}, instances_created={}, \
         dept_wired={}, classes_derived={}, ua_created_total={}, oa_created_total={}, \
         associations_created_total={}, template_failures={}, class_failures={}",
        id,
        stats.templates_created,
        stats.instances_created,
        stats.dept_wired,
        stats.classes_derived,
        stats.ua_created_total,
        stats.oa_created_total,
        stats.associations_created_total,
        stats.template_failures,
        stats.class_failures,
    );
    if stats.template_failures + stats.class_failures > 0 {
        common::telemetry::warn!("{}（差异留痕：下次 activate 幂等补齐）", summary);
    } else {
        common::telemetry::info!("{}", summary);
    }
    fetch_scheme(pool, id).await
}

// ═══════════════════════════════════════════════════════════
// B-2 激活差异派生 — content 解析/预检 + 幂等落岗/类派生
// ═══════════════════════════════════════════════════════════

/// 方案资产 content JSONB 快照（B-2 契约；键 camelCase，snake 别名容错）：
/// `{"templates":[<SchemePositionTemplate + departmentId>], "policyClassCodes":[…]}`。
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SchemeContentSnapshot {
    #[serde(default)]
    templates: Vec<SchemePositionTemplate>,
    #[serde(default, alias = "policy_class_codes")]
    policy_class_codes: Vec<String>,
}

/// 激活派生条目上限（设计 §风险：单 activate ≤ 20，超出 400 分批/先建子方案）。
const ACTIVATE_ENTRY_LIMIT: usize = 20;

/// 激活派生计划（预检段产物：语义已全量校验，派生段照单执行）。
struct ActivationPlan {
    templates: Vec<SchemePositionTemplate>,
    /// trim + 去重后的策略类 code（全部 state='active' 已验）。
    class_codes: Vec<String>,
}

/// 激活派生统计（响应不入 body——scheme 资产读径不变；汇总落日志）。
#[derive(Default)]
struct SchemeDeriveStats {
    templates_created: usize,
    instances_created: usize,
    dept_wired: usize,
    classes_derived: usize,
    ua_created_total: i64,
    oa_created_total: i64,
    associations_created_total: i64,
    template_failures: usize,
    class_failures: usize,
}

/// content 解析 + 派生预检（写前 fail-fast，全部 400 发生在状态迁移前）：
/// 结构/限流/类别基表行/范例引用/组织在册/策略类 active。
async fn validate_activation_content(
    pool: &PgPool,
    content: serde_json::Value,
) -> Result<ActivationPlan, ApiError> {
    let snapshot: SchemeContentSnapshot = match serde_json::from_value(content) {
        Ok(s) => s,
        Err(e) => {
            return Err(ApiError::BadRequest(format!(
                "方案 content 结构错误（须为 {{templates:[…], policyClassCodes:[…]}}）: {}",
                e
            )))
        }
    };
    // 策略类 code：trim + 去重（对齐 publish-direct 预检段语义）
    let mut class_codes: Vec<String> = Vec::new();
    for code in snapshot.policy_class_codes {
        let code = code.trim().to_string();
        if code.is_empty() || class_codes.contains(&code) {
            continue;
        }
        class_codes.push(code);
    }
    // 限流：templates + 类码 > 20 → 400（提示分批或先建子方案收窄 scope）
    let entries = snapshot.templates.len() + class_codes.len();
    if entries > ACTIVATE_ENTRY_LIMIT {
        return Err(ApiError::BadRequest(format!(
            "激活派生条目超限（{} > {}）：请分批激活，或先建子方案（scope 收窄）再激活",
            entries, ACTIVATE_ENTRY_LIMIT
        )));
    }
    for code in &class_codes {
        let active: bool = sqlx::query_scalar(
            "SELECT COUNT(*) > 0 FROM isahl_auth.org_policy_class \
             WHERE code = $1 AND state = 'active' AND deleted_at IS NULL",
        )
        .bind(code)
        .fetch_one(pool)
        .await
        .map_err(ApiError::from_sqlx)?;
        if !active {
            return Err(ApiError::BadRequest(format!(
                "策略类 code '{}' 不存在或未激活（state='active' 才可派生）",
                code
            )));
        }
    }
    for item in &snapshot.templates {
        match item.template_id {
            Some(tid) => {
                // 引用形态仅携带 templateId——与全部内联字段互斥（P3a 补全
                // maxHeads/departmentId：引用链编制上限以范例行 projection
                // 载荷为准、部门编制经内联 (category, name) 复用表达，拒绝混载）。
                if item.category_code.is_some()
                    || item.name.is_some()
                    || item.max_heads.is_some()
                    || item.department_id.is_some()
                {
                    return Err(ApiError::BadRequest(
                        "templateId 引用与内联字段互斥（引用仅携带 templateId：\
                         name/categoryCode/maxHeads/departmentId 均拒绝）"
                            .into(),
                    ));
                }
                let row: Option<(i64, Option<i64>)> = sqlx::query_as(
                    r#"SELECT id, ck_category
                       FROM isahl."zc_id_subj-position"
                       WHERE id = $1 AND deleted_at IS NULL
                         AND _f_ = '设计' AND _t_ = '范例' AND tpl_id IS NULL"#,
                )
                .bind(tid)
                .fetch_optional(pool)
                .await
                .map_err(ApiError::from_sqlx)?;
                let Some((_, ck_category)) = row else {
                    return Err(ApiError::NotFound(format!(
                        "Position template not found: {}",
                        tid
                    )));
                };
                if ck_category.is_none() {
                    return Err(ApiError::BadRequest(format!(
                        "Position template {} 缺类别（ck_category NULL），不可实例化",
                        tid
                    )));
                }
            }
            None => {
                let category = item
                    .category_code
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ApiError::BadRequest(
                            "内联条目 category_code 必填：岗位必须绑定 zc_id_category 基表行"
                                .into(),
                        )
                    })?;
                let found: Option<i64> = sqlx::query_scalar(
                    r#"SELECT c.id FROM isahl.zc_id_category c
                       WHERE c.code = $1 AND c.deleted_at IS NULL
                         AND c.tableoid = 'isahl.zc_id_category'::regclass"#,
                )
                .bind(category)
                .fetch_optional(pool)
                .await
                .map_err(ApiError::from_sqlx)?;
                if found.is_none() {
                    return Err(ApiError::BadRequest(format!(
                        "未知岗位类别 code: '{}'（须为 zc_id_category 基表行，子族字典不派生）",
                        category
                    )));
                }
                if let Some(mh) = item.max_heads {
                    if mh <= 0 {
                        return Err(ApiError::BadRequest(
                            "maxHeads 须 > 0（编制上限；缺省 = 不设限）".into(),
                        ));
                    }
                }
            }
        }
        if let Some(dept) = item.department_id {
            let ok: bool = sqlx::query_scalar(
                r#"SELECT EXISTS (SELECT 1 FROM isahl."zc_id_orga-department"
                       WHERE id = $1 AND deleted_at IS NULL)
                    OR EXISTS (SELECT 1 FROM isahl."zc_id_orga-non-banking-legal"
                       WHERE id = $1 AND deleted_at IS NULL)"#,
            )
            .bind(dept)
            .fetch_one(pool)
            .await
            .map_err(ApiError::from_sqlx)?;
            if !ok {
                return Err(ApiError::BadRequest(format!(
                    "departmentId {} 不存在或已删除（编制部门须为在册组织）",
                    dept
                )));
            }
        }
    }
    Ok(ActivationPlan {
        templates: snapshot.templates,
        class_codes,
    })
}

/// 差异派生主循环（事务外）：逐模板条目幂等落岗 + 逐策略类幂等重放。
/// 单条目失败仅 warn（统计留痕），绝不阻断——状态已迁移，差额下次 activate 补齐。
async fn derive_activation_diff(
    pool: &PgPool,
    plan: &ActivationPlan,
    user_id: i64,
) -> SchemeDeriveStats {
    let mut stats = SchemeDeriveStats::default();
    for (idx, item) in plan.templates.iter().enumerate() {
        if let Err(e) = realize_scheme_template(pool, item, user_id, &mut stats).await {
            stats.template_failures += 1;
            common::telemetry::warn!(
                "org_scheme activate 模板条目 #{} 落岗失败（差异留痕，下次 activate 补齐）: {}",
                idx + 1,
                e
            );
        }
    }
    for code in &plan.class_codes {
        let class_id: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.org_policy_class \
             WHERE code = $1 AND state = 'active' AND deleted_at IS NULL",
        )
        .bind(code)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        let Some(class_id) = class_id else {
            stats.class_failures += 1;
            common::telemetry::warn!(
                "org_scheme activate 策略类 '{}' 派生跳过（类缺失/未激活，下次 activate 补齐）",
                code
            );
            continue;
        };
        match common::ngac_policy::derive_from_class(pool, class_id).await {
            Ok(d) => {
                stats.classes_derived += 1;
                stats.ua_created_total += d.ua_created;
                stats.oa_created_total += d.oa_created;
                stats.associations_created_total += d.associations_created;
            }
            Err(e) => {
                stats.class_failures += 1;
                common::telemetry::warn!(
                    "org_scheme activate 策略类 '{}' 派生失败（差异留痕，下次 activate 补齐）: {}",
                    code,
                    e
                );
            }
        }
    }
    stats
}

/// 激活差异派生：单模板条目幂等落岗（每条目自事务，失败独立回滚由调用方 warn）。
///
/// 幂等键（B-2 设计 §风险；与 SchemePositionTemplate doc 同源）：
/// - 设计范例行去重键 (category_id, name)：内联按此 find-or-create（tpl_id NULL、
///   `_f_='设计' AND _t_='范例'`）→ 命中复用既有同款行，未命中新建（编制上限
///   载荷落 `projection`）；
/// - 实例键 = 该范例链在册实例数：**链上零在册实例才补建首实例**——同一
///   (category, name) 链至多单实例，重复 activate 零新增；同类别多部门条目
///   （同 (category, name)、不同 departmentId）共享该单实例，各自经桥挂接
///   （不各自建实例）；
/// - template_id 引用形态：范例行直取（引用仅携带 templateId，内联字段预检
///   互斥）→ 同按链在册实例数幂等补建/跳过；
/// - 部门编制键 (department_id, 链)：链上已挂该组织即视为已实现（复活软删桥行
///   + INSERT ON CONFLICT DO NOTHING，幂等）——首挂触发 B-2 heal_position_scope
///   （事务外 heal，失败仅 warn）。
///
/// 并发串行化（与 org_tree instantiate_position_template 同锁纪律）：
/// - 引用/复用链：`SELECT … FOR UPDATE` 锁被引（复用）范例行；
/// - 新链：插入后立即行锁自身——计数与插入全程持模板行锁；
/// - 内联建链前先锁 `zc_id_category` 基表行：同类别建链串行（find-or-create
///   无唯一约束可依，非锁定读会双 activate 双建范例/双插）。
/// live COUNT 与 INSERT 同事务锁内，无并发窗口；跨方案/与实例化端点并发
/// activate+instantiate 在范例行锁上排队，后到者计数可见先行者已落实例。
///
/// R-D3 编制上限（step-scope，P2①）：maxHeads>0 上限检查收窄到实例化步——
/// 仅当将补建首实例时判定，已达上限仅跳过该次实例化（不 400/不计失败），
/// 部门挂接照常执行；链上已有在册实例即视为编制已实现（幂等键，实例不重复
/// 建即不超编），差额由下次 activate 补齐。
async fn realize_scheme_template(
    pool: &PgPool,
    item: &SchemePositionTemplate,
    user_id: i64,
    stats: &mut SchemeDeriveStats,
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve_conn(&mut *tx, ("TX", "FJA", "↓_GG"))
            .await
            .map_err(ApiError::from_sqlx)?;

    // 1. 范例行确定 + 行锁：templateId 引用直取（FOR UPDATE 锁被引范例行——
    //    与 org_tree instantiate_position_template 同锁串行，并发 activate/
    //    instantiate 在范例行锁上排队，杜绝双插）；内联按 (category_id, name)
    //    find-or-create（先锁类别基表行串行化建链，见下）。
    let (tpl_id, tpl_name, category_id): (i64, String, i64) = match item.template_id {
        Some(tid) => {
            let row: Option<(i64, String, Option<i64>)> = sqlx::query_as(
                r#"SELECT id, notice::text, ck_category
                   FROM isahl."zc_id_subj-position"
                   WHERE id = $1 AND deleted_at IS NULL
                     AND _f_ = '设计' AND _t_ = '范例' AND tpl_id IS NULL
                   FOR UPDATE"#,
            )
            .bind(tid)
            .fetch_optional(&mut *tx)
            .await
            .map_err(ApiError::from_sqlx)?;
            let Some((id, notice, ck)) = row else {
                return Err(ApiError::NotFound(format!(
                    "Position template not found: {}",
                    tid
                )));
            };
            let Some(ck) = ck else {
                return Err(ApiError::BadRequest(format!(
                    "Position template {} 缺类别（ck_category NULL），不可实例化",
                    tid
                )));
            };
            (id, notice, ck)
        }
        None => {
            let category = item
                .category_code
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    ApiError::BadRequest(
                        "内联条目 category_code 必填：岗位必须绑定 zc_id_category 基表行".into(),
                    )
                })?;
            // 锁类别基表行（FOR UPDATE）：串行化同类别内联建链——find-or-create
            // 无唯一约束可依，非锁定读会双 activate 双建范例/双插（跨方案并发
            // 共享 (category, name) 链时尤为关键）；后到者重查可见先行者已建行。
            let category_id: i64 = sqlx::query_scalar(
                r#"SELECT c.id FROM isahl.zc_id_category c
                   WHERE c.code = $1 AND c.deleted_at IS NULL
                     AND c.tableoid = 'isahl.zc_id_category'::regclass
                   FOR UPDATE"#,
            )
            .bind(category)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::from_sqlx)?;
            let name = item
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(category)
                .to_string();
            // find-or-create：同款（类别+名）设计范例行优先复用——重复激活不重复建
            // 范例；FOR UPDATE 命中即锁既有范例行（与引用形态/org_tree instantiate
            // 同锁串行——复用链的计数与插入同样无并发窗口）。
            let existing: Option<i64> = sqlx::query_scalar(
                r#"SELECT id FROM isahl."zc_id_subj-position"
                   WHERE ck_category = $1 AND notice = $2 AND tpl_id IS NULL
                     AND _f_ = '设计' AND _t_ = '范例' AND deleted_at IS NULL
                   ORDER BY id LIMIT 1
                   FOR UPDATE"#,
            )
            .bind(category_id)
            .bind(&name)
            .fetch_optional(&mut *tx)
            .await
            .map_err(ApiError::from_sqlx)?;
            if let Some(eid) = existing {
                (eid, name, category_id)
            } else {
                // 编制上限落既有文本载荷列 projection（comments 回归自由文本；
                // 映射契约见 handlers::org_tree::headcount_carriers）
                let (projection, _) =
                    crate::handlers::org_tree::headcount_carriers(item.max_heads, None);
                let id: i64 = sqlx::query_scalar(
                    r#"INSERT INTO isahl."zc_id_subj-position"
                         (notice, code, comments, projection, ck_category, tpl_id, _f_, _t_, dk_scene, dk_factor, dk_function)
                       VALUES ($1, NULL, NULL, $2, $3, NULL, '设计', '范例', $4, $5, $6)
                       RETURNING id"#,
                )
                .bind(&name)
                .bind(&projection)
                .bind(category_id)
                .bind(dk_scene)
                .bind(dk_factor)
                .bind(dk_function)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::from_sqlx)?;
                stats.templates_created += 1;
                // 新链行锁自身（持锁至提交）：与既有链 FOR UPDATE 同纪律——
                // 实例化计数/插入全程持模板行锁（类别锁已串行建链，此处补齐
                // 模板行锁维度，使后续实例化判定与 org_tree instantiate 同锁串行）。
                sqlx::query_scalar::<_, i64>(
                    r#"SELECT id FROM isahl."zc_id_subj-position"
                       WHERE id = $1 AND deleted_at IS NULL
                       FOR UPDATE"#,
                )
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::from_sqlx)?;
                (id, name, category_id)
            }
        }
    };

    // 2. 实例化差额（幂等键：该范例链在册实例数；重复 activate 零新增）。
    //    模板行锁已持（引用/复用行 FOR UPDATE、新链行锁自身）——计数与插入
    //    同事务锁内，无并发窗口（与 org_tree instantiate_position_template
    //    同锁串行：并发 activate+instantiate/双 activate 不双插）。
    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl.\"zc_id_subj-position\" WHERE tpl_id = $1 AND deleted_at IS NULL",
    )
    .bind(tpl_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    // R-D3 编制上限（step-scope，P2①）：maxHeads>0 上限检查收窄到实例化步——
    // 已达上限（live >= cap）→ 仅跳过本次实例化（不整体 Err/400/计失败），
    // 部门挂接照常执行；链上已有在册实例即编制已实现（实例不重复建即不超编），
    // 差额由下次 activate 幂等补齐。（注：cap>0 时 live==0 恒未达上限，此判定
    // 兜底非校验路径的 cap<=0 等脏值——fail-closed 不落超编实例。）
    let cap_reached = item
        .max_heads
        .filter(|c| *c > 0)
        .is_some_and(|cap| live >= cap);
    // 实例化步：仅链上零在册实例（且未达上限）时补建首实例（范例名直接作 notice）。
    let instance_id: Option<i64> = if live == 0 && !cap_reached {
        let iid: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_subj-position" (notice, comments, ck_category, tpl_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, '', $2, $3, $4, $5, $6)
               RETURNING id"#,
        )
        .bind(&tpl_name)
        .bind(category_id)
        .bind(tpl_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        stats.instances_created += 1;
        Some(iid)
    } else {
        None
    };

    // 3. 部门挂接（departmentId 可空；幂等：链上已有该组织挂接即视为已实现）
    let mut wired_pos: Option<i64> = None;
    if let Some(dept_id) = item.department_id {
        let already: bool = sqlx::query_scalar(
            r#"SELECT EXISTS (
                 SELECT 1 FROM isahl."zc_id_subj-org_rr_position" b
                 JOIN isahl."zc_id_subj-position" p ON p.id = b.ref_right AND p.deleted_at IS NULL
                 WHERE b.ref_left = $1 AND b.deleted_at IS NULL AND p.tpl_id = $2
               )"#,
        )
        .bind(dept_id)
        .bind(tpl_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        if !already {
            // 挂该范例链最早在册实例（新建实例优先）
            let pos_id: i64 = match instance_id {
                Some(iid) => iid,
                None => sqlx::query_scalar(
                    r#"SELECT id FROM isahl."zc_id_subj-position"
                       WHERE tpl_id = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1"#,
                )
                .bind(tpl_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::from_sqlx)?,
            };
            // 复活同键软删桥行（唯一约束含表达式无法 ON CONFLICT 推断）→ 幂等插
            sqlx::query(
                r#"UPDATE isahl."zc_id_subj-org_rr_position"
                   SET deleted_at = NULL, deleted_by_id = NULL
                   WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NOT NULL"#,
            )
            .bind(dept_id)
            .bind(pos_id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::from_sqlx)?;
            let inserted = sqlx::query(
                r#"INSERT INTO isahl."zc_id_subj-org_rr_position"
                     (ref_left, ref_right, created_by_id)
                   VALUES ($1, $2, $3) ON CONFLICT DO NOTHING"#,
            )
            .bind(dept_id)
            .bind(pos_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::from_sqlx)?
            .rows_affected();
            if inserted > 0 {
                wired_pos = Some(pos_id);
            }
        }
    }
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    // 首挂成功 → B-2 heal：岗位 OA/部门闭包收敛（事务外幂等 heal，失败仅 warn）
    if let Some(pos_id) = wired_pos {
        stats.dept_wired += 1;
        crate::ngac_org_ensure::heal_position_scope(pool, pos_id).await;
    }
    Ok(())
}

/// POST /org-scheme — 建 draft。权限对齐 publish：`positions` 资源 `create`。
pub async fn create_scheme_handler(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<CreateSchemeRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "positions", 0, "create").await?;
    ensure_org_design_scheme(pool.get_ref()).await?;
    let email = extract_user_email(&req).unwrap_or_default();
    let out = create_scheme(pool.get_ref(), user_id, &email, body.into_inner()).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(out)))
}

/// GET /org-scheme/active — 当前生效方案。
pub async fn get_active_scheme_handler(
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "positions", 0, "read").await?;
    ensure_org_design_scheme(pool.get_ref()).await?;
    let out = get_active_scheme(pool.get_ref()).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(out)))
}

/// GET /org-scheme/{id} — 读指定资产行。
pub async fn get_scheme_handler(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "positions", 0, "read").await?;
    ensure_org_design_scheme(pool.get_ref()).await?;
    let out = fetch_scheme(pool.get_ref(), path.into_inner()).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(out)))
}

/// PATCH /org-scheme/{id} — 仅 draft；version+1；title/scope/content 差量更新。
pub async fn update_scheme_handler(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<UpdateSchemeRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "positions", 0, "create").await?;
    ensure_org_design_scheme(pool.get_ref()).await?;
    let email = extract_user_email(&req).unwrap_or_default();
    let out = update_scheme(
        pool.get_ref(),
        user_id,
        &email,
        path.into_inner(),
        body.into_inner(),
    )
    .await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(out)))
}

/// POST /org-scheme/{id}/activate — draft→active（旧 active 自动 superseded）。
pub async fn activate_scheme_handler(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "positions", 0, "create").await?;
    ensure_org_design_scheme(pool.get_ref()).await?;
    let email = extract_user_email(&req).unwrap_or_default();
    let out = activate_scheme(pool.get_ref(), user_id, &email, path.into_inner()).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(out)))
}
