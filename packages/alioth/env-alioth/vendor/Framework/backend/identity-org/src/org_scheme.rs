//! D-2 组织方案发布（org-scheme publish）—— 事务编排，无独立表
//!
//! D-2 概念设计中的 draft/版本/快照行留后续（comments/新表成本高）；本片裁决：
//! **首版方案 = 请求体 JSON 校验后直接 publish（publish 动作即方案）**。方案资产
//! 的 JSONB 快照形态 `{title, version, department_snapshot?, position_templates,
//! policy_class_codes}` 由请求体携带并在响应中原样回显（`scheme` 段），不落库。
//!
//! `POST /org-scheme/publish` 编排（两段）：
//! 1. **预检段（写前 fail-fast）**：全部 `policy_class_codes` 必须命中
//!    `isahl_auth.org_policy_class` 且 `state='active'`（同 [`common::ngac_policy`]
//!    派生器只认 active 规范）；任一缺失即整体 400，不产生任何写。
//! 2. **模板落岗段（单事务）**：每个 position_template 条目落岗为真实岗位行
//!    （`_f_ IS NULL`）：
//!    - `template_id` 引用：校验范例行在册（`_f_='设计' AND _t_='范例'`、
//!      tpl_id NULL、未删）→ 实例化（复用 D-2a instantiate 语义：ck_category
//!      继承、notice=范例名，同范例已有在册实例追加 `-{序号}` 消歧）；
//!    - 内联（`category_code` 必填）：校验 `zc_id_category` **基表行**
//!      （tableoid 过滤，B-1 align-cognition-ua-category 同源——子族字典不派生）
//!      → 先建范例行（comments 承载 `{"max_heads","note"}` 编制文档，D-2a
//!      文档化契约）→ 随即实例化落岗。
//!    实例行每次发布新增（可重复发布形成多批次岗位）；派生态幂等见第 3 段。
//! 3. **类派生段**：逐 code 调 [`common::ngac_policy::derive_from_class`]——
//!    该函数自带单事务（pool 级 begin/commit，重复调用零新增），故派生不在
//!    模板事务内；幂等 upsert 保证重发安全。
//!
//! 端点挂载：各 isahl-db 服务壳 `register_service_routes` 内
//! `.configure(identity_org::org_scheme::register)`（仿 org_tree）。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

// ═══════════════════════════════════════════════════════════
// DTO — 方案发布请求（camelCase；快照形态即方案资产）
// ═══════════════════════════════════════════════════════════

/// 方案内单个岗位编制条目：`template_id` 引用（范例行）与内联
/// `{name, category_code, max_heads}` 二选一（同时给出 → 400）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemePositionTemplate {
    /// 引用既有岗位编制范例行（D-2a 设计态行）→ 仅实例化落岗。
    #[serde(default)]
    pub template_id: Option<i64>,
    /// 内联建范例名（缺省回退 category_code）。
    #[serde(default)]
    pub name: Option<String>,
    /// 内联必填：岗位类别 code（zc_id_category 基表行，子族字典不派生）。
    #[serde(default)]
    pub category_code: Option<String>,
    /// 编制上限（落内联范例行 comments 文档；template_id 引用时忽略）。
    #[serde(default)]
    pub max_heads: Option<i64>,
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

/// POST /org-scheme/publish — 方案事务编排（见模块文档）。
/// 权限同岗位写端点：`positions` 资源 `create`。
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
    cfg.service(
        web::resource("/org-scheme/publish").route(web::post().to(publish_scheme_handler)),
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
                            "内联条目 category_code 必填：岗位必须绑定 zc_id_category 基表行".into(),
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
                // 编制元数据 comments JSON 文档（D-2a 文档化契约：
                // {"max_heads","note"}，全缺 → "{}"）
                let mut doc = serde_json::Map::new();
                if let Some(mh) = item.max_heads {
                    doc.insert("max_heads".to_string(), serde_json::json!(mh));
                }
                let comments = serde_json::to_string(&serde_json::Value::Object(doc))
                    .unwrap_or_else(|_| "{}".to_string());
                let id: i64 = sqlx::query_scalar(
                    r#"INSERT INTO isahl."zc_id_subj-position"
                         (notice, code, comments, ck_category, tpl_id, _f_, _t_)
                       VALUES ($1, NULL, $2, $3, NULL, '设计', '范例')
                       RETURNING id"#,
                )
                .bind(&name)
                .bind(&comments)
                .bind(category_id)
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
            r#"INSERT INTO isahl."zc_id_subj-position" (notice, comments, ck_category, tpl_id)
               VALUES ($1, '', $2, $3)
               RETURNING id"#,
        )
        .bind(&notice)
        .bind(category_id)
        .bind(tpl_id)
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
