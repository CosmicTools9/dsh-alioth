//! 组织管理主体 Handler — WZ org-wz 模块数据源
//!
//! 覆盖：
//! - `GET /subjects` — 主体列表（视角/社会分类/搜索/分页筛选 + 可选 kind 按主体分类过滤 + 状态解析 + 视角标签聚合）
//! - `GET /subjects/{id}/view-tags` / `PUT /subjects/{id}/view-tags` — 交易视角标签读写
//! - `GET /subjects/{id}/accounts` / `POST /subjects/{id}/accounts` / `DELETE /subjects/{id}/accounts/{relId}` — 账户关联读写
//! - `POST /subjects` — 创建主体（notice/code 必填，可选 comments）
//! - `PUT /subjects/{id}` / `DELETE /subjects/{id}` — 更新 / 软删除主体
//!
//! 模型语义（零 DDL）：
//! - 主体统一叶表 `isahl.zc_id_subjects`（社会分类由 tableoid 叶表判定）
//! - 交易视角 = 主体→岗位(`zc_id_subj-post_rr_view`)→`zc_id_relation-post_view_r_tags`→`zc_id_tags-post_view` 字典
//! - 账户关联 = `zc_id_subjects_rr_account`（ref_left=主体 id, ref_right=账户实体 id）
//! - 状态 = `zc_id_lifecycle_r_primary-status` → `zc_id_stus-org`（normal/disabled，黑名单语义）
//! - 状态切换写路径复用 contract-wz `POST /counterparties/{id}/status`，不在此重复实现

use crate::handlers::identities::category_id_by_code;
use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool, Postgres, QueryBuilder};

/// 主体 MDM 编码扩展表 ensure（零 DDL 交付：运行时幂等自愈 → backup-ddl 快照收编）。
/// isahl schema 冻结 + comments 禁嵌 JSON，MDM 编码落 `wz_fssc.subject_mdm`
/// （先例：subject_bank_card / subject_invoice_info）。
/// AtomicBool 仅作免重复标记——DDL 幂等，并发重入无害。
static SUBJECT_MDM_ENSURED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(crate) async fn ensure_subject_mdm(pool: &PgPool) -> Result<(), ApiError> {
    use std::sync::atomic::Ordering;
    if SUBJECT_MDM_ENSURED.load(Ordering::Relaxed) {
        return Ok(());
    }
    // wz_fssc 为 WZ 扩展 schema（共享内核跨 namespace 复用——AVIC-CAASEC 等
    // 库无该 schema）：先自愈建 schema 再建表，两段均幂等。
    let result = async {
        sqlx::query("CREATE SCHEMA IF NOT EXISTS wz_fssc")
            .execute(pool)
            .await?;
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS wz_fssc.subject_mdm (
                subject_id    bigint NOT NULL PRIMARY KEY,
                mdm_code      text NOT NULL,
                created_by_id bigint,
                updated_by_id bigint,
                created_at    timestamptz DEFAULT now() NOT NULL,
                updated_at    timestamptz DEFAULT now() NOT NULL
            )"#,
        )
        .execute(pool)
        .await
    }
    .await;
    match result {
        Ok(_) => {}
        // 并发首次 ensure 的 pg_type/pg_class 唯一索引竞态（23505）——表已由并发请求建成，视为成功
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23505") => {}
        Err(e) => return Err(ApiError::from_sqlx(e)),
    }
    SUBJECT_MDM_ENSURED.store(true, Ordering::Relaxed);
    Ok(())
}

/// tableoid 叶表名 → 社会分类标签（对齐原型 8 类 + DB 既有叶表）
fn social_category(leaf: &str) -> &'static str {
    // tableoid::regclass::text 可能带引号（如 "zc_id_subj-org"）
    let clean = leaf.trim_matches('"');
    match clean {
        "zc_id_orga-legal" => "法人",
        "zc_id_orga-non-banking-legal" => "非银行法人",
        "zc_id_bank-commercial" => "商业银行",
        "zc_id_bank-central" => "中央银行",
        "zc_id_empl-natural" => "自然人",
        "zc_id_empl-agent" => "智能体",
        "zc_id_subj-group" => "组",
        "zc_id_orga-department" => "部门",
        "zc_id_subj-org" => "组织",
        "zc_id_subj-employee" => "雇员",
        "zc_id_subj-country" => "国家",
        "zc_id_subj-supranational" => "超国家",
        _ => "主体",
    }
}

fn leaf_table_for_category(category: &str) -> &'static str {
    match category {
        "法人" => "zc_id_orga-legal",
        "非银行法人" => "zc_id_orga-non-banking-legal",
        "商业银行" => "zc_id_bank-commercial",
        // 开户机构登记表（code=联行号；银行卡 fk_trustee 指向该叶表行）
        "银行机构" => "zc_id_subj-bank",
        "自然人" => "zc_id_empl-natural",
        "智能体" => "zc_id_empl-agent",
        "组" => "zc_id_subj-group",
        "部门" => "zc_id_orga-department",
        "组织" => "zc_id_subj-org",
        "雇员" => "zc_id_subj-employee",
        "国家" => "zc_id_subj-country",
        "超国家" => "zc_id_subj-supranational",
        _ => "zc_id_subjects",
    }
}

#[derive(Debug, Deserialize)]
pub struct SubjectListQuery {
    /// 交易视角 code（如 VIEW-CUST / VIEW-SUPP / VIEW-CARR / VIEW-CPARTY）
    pub view: Option<String>,
    /// 社会分类标签（如 法人 / 自然人 / 商业银行）
    pub category: Option<String>,
    /// 搜索词（匹配 notice / code）
    pub q: Option<String>,
    /// 主体分类简写（customer / carrier / driver，映射 ck_category → SUBJ-* 分类 code 过滤）
    pub kind: Option<String>,
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
    /// 排除系统主体（默认 true——isahl 管理员等 socialCategory='主体' 不进业务列表）
    #[serde(default = "default_true")]
    pub exclude_system: bool,
    /// 排除停用主体（黑名单 disabled；默认 false——管理页显示全部，选择器显式传 true）
    #[serde(default)]
    pub exclude_disabled: bool,
}

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    20
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubjectListItem {
    #[serde(with = "common::serde_zuid")]
    id: i64,
    code: Option<String>,
    notice: Option<String>,
    social_category: String,
    category: Option<String>,
    view_tags: Vec<String>,
    status: String,
    /// 备注（含电话/联系人 JSON；详情页展示依赖，历史缺失致联系电话不可见）
    comments: Option<String>,
    /// MDM 主数据编码（wz_fssc.subject_mdm；无记录为 null）
    mdm_code: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ViewTagItem {
    #[serde(with = "common::serde_zuid")]
    id: i64,
    code: String,
    notice: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountItem {
    #[serde(with = "common::serde_zuid")]
    rel_id: i64,
    #[serde(with = "common::serde_zuid")]
    account_id: i64,
    account_name: Option<String>,
}

/// POST /subjects 请求体（notice/name 必填，code 可选自动生成，subject_type 落 comments.kind）
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSubjectRequest {
    /// 主体名称（必填；前端契约历史用 name，serde alias 兼容两字段名）
    #[serde(alias = "name")]
    pub notice: String,
    /// 主体编码（前端可不传，空则自动生成 SUBJ-<zuid 后 6 位>）
    #[serde(default)]
    pub code: Option<String>,
    /// 主体类型（业务别名或叶表名，如 zc_id_empl-natural/自然人；落 comments.kind）
    /// rename_all=camelCase 主名接受 subjectType，alias 兼容前端契约 subject_type
    #[serde(default, alias = "subject_type")]
    pub subject_type: Option<String>,
    /// 备注（可选；与 kind 合并为 comments JSON）
    #[serde(default)]
    pub comments: Option<String>,
    /// 岗位 id（可选；有值挂到指定岗位，为空且需写视角标签时自动建默认岗位 POST-AUTO-<id> 挂接）
    #[serde(default)]
    pub position_id: Option<i64>,
    /// 交易视角 code 列表（可选；创建事务内直接落标签关系，与 PUT view-tags 同差量语义）
    #[serde(default)]
    pub view_tags: Option<Vec<String>>,
    /// 联系方式（可选；落「实体↔联系人↔联系方式」链，kind 白名单见 add_entity_contact）
    #[serde(default)]
    pub contacts: Option<Vec<CreateSubjectContact>>,
    /// 证照（可选；落独立表 zc_id_identity + zc_id_entity_rr_identity 关联）
    #[serde(default)]
    pub identities: Option<Vec<CreateSubjectIdentity>>,
    /// 雇佣状态 code（可选；EMPL-ACTIVE/EMPL-PROBATION/EMPL-RETIRED，字典 zc_id_stus-employ。
    /// 有值时同事务建 zc_id_subj-employee 雇员行 + r_employ-status 任职状态关系）
    #[serde(default)]
    pub employ_status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSubjectContact {
    /// 号码（手机/固话）
    pub value: String,
    /// 类型（kind：mobile/phone/emergency 等，落 contacts.comments JSON）
    pub kind: Option<String>,
    /// 是否默认联系方式（entity_rr_contacts.default_contact）
    #[serde(default)]
    pub is_default: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSubjectIdentity {
    /// 证照类型 code（zc_id_cate-identity；缺项自动建字典）
    pub category_code: String,
    /// 证照号（identity 列）
    pub cert_no: String,
    /// 证照名称（dname；缺省用 cert_no）
    pub name: Option<String>,
}

/// PUT /subjects/{id} 请求体（None 字段保持不变）
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSubjectRequest {
    /// 主体名称
    pub notice: Option<String>,
    /// 主体编码
    pub code: Option<String>,
    /// 备注
    pub comments: Option<String>,
    /// MDM 主数据编码（Some(非空)=upsert，Some(空)=清除，None=不动）
    pub mdm_code: Option<String>,
}

/// GET /service/isahl-db/subjects — 组织管理主体列表（视角/分类/搜索/分页；可选 kind 按主体分类过滤）
pub async fn list_subjects(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    query: web::Query<SubjectListQuery>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "list").await?;
    ensure_subject_mdm(pool.get_ref()).await?;

    let q = query.q.clone().unwrap_or_default().trim().to_string();
    let view = query.view.clone().unwrap_or_default().trim().to_string();
    let category = query
        .category
        .clone()
        .unwrap_or_default()
        .trim()
        .to_string();
    let kind = query.kind.clone().unwrap_or_default().trim().to_string();
    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;

    let mut builder: QueryBuilder<Postgres> = QueryBuilder::new(
        "SELECT s.id, s.code, s.notice, s.tableoid::regclass::text AS leaf, s.comments, \
           COALESCE((SELECT ss.code FROM \"isahl\".\"zc_id_lifecycle_r_primary-status\" ps \
             JOIN \"isahl\".\"zc_id_stus-org\" ss ON ss.id = ps.ref_right AND ss.deleted_at IS NULL \
             WHERE ps.ref_left = s.id AND ps.deleted_at IS NULL LIMIT 1), 'normal') AS status, \
           c.code AS category_code, \
           COALESCE((SELECT json_agg(DISTINCT vt.code) \
             FROM \"isahl\".\"zc_id_subj-post_rr_view\" e \
             JOIN \"isahl\".\"zc_id_relation-post_view_r_tags\" r ON r.ref_left = e.ref_left AND r.deleted_at IS NULL \
             JOIN \"isahl\".\"zc_id_tags-post_view\" vt ON vt.id = r.ref_right AND vt.deleted_at IS NULL \
             WHERE e.ref_right = s.id AND e.deleted_at IS NULL), '[]') AS view_tags, \
           mdm.mdm_code \
         FROM \"isahl\".\"zc_id_subjects\" s \
         LEFT JOIN \"isahl\".\"zc_id_cate-subject\" c ON c.id = s.ck_category AND c.deleted_at IS NULL \
         LEFT JOIN wz_fssc.subject_mdm mdm ON mdm.subject_id = s.id \
         WHERE s.deleted_at IS NULL",
    );

    // 批注（用户）：停用主体（黑名单 disabled）其他页面不可选——选择器传 exclude_disabled=1
    if query.exclude_disabled {
        builder.push(
            " AND NOT EXISTS (SELECT 1 FROM \"isahl\".\"zc_id_lifecycle_r_primary-status\" ps \
             JOIN \"isahl\".\"zc_id_stus-org\" ss ON ss.id = ps.ref_right AND ss.deleted_at IS NULL \
             WHERE ps.ref_left = s.id AND ps.deleted_at IS NULL AND ss.code = 'disabled')",
        );
    }

    if !q.is_empty() {
        let pat = format!("%{}%", q);
        builder.push(" AND (s.notice ILIKE ");
        builder.push_bind(pat.clone());
        builder.push(" OR s.code ILIKE ");
        builder.push_bind(pat);
        builder.push(")");
    }
    if !category.is_empty() {
        // 叶表 tableoid 过滤（父表行按 comments.kind 回退匹配已随 comments 文本化移除——
        // 父表直插历史行不再参与分类过滤）
        builder.push(" AND btrim(s.tableoid::regclass::text, '\"') = ");
        builder.push_bind(leaf_table_for_category(&category));
    }
    // 排除系统主体（批注 c1960312/95d25697：isahl 管理员等不进业务列表；
    // comments.kind 不可读后父表行无法分类 → 一并排除）
    let exclude_system = query.exclude_system;
    if exclude_system {
        builder.push(
            " AND btrim(s.tableoid::regclass::text, '\"') NOT IN ('zc_id_subj-position', 'zc_id_subjects')",
        );
    }
    if !view.is_empty() {
        builder.push(
            " AND EXISTS (SELECT 1 FROM \"isahl\".\"zc_id_subj-post_rr_view\" e \
             JOIN \"isahl\".\"zc_id_relation-post_view_r_tags\" r ON r.ref_left = e.ref_left AND r.deleted_at IS NULL \
             JOIN \"isahl\".\"zc_id_tags-post_view\" vt ON vt.id = r.ref_right AND vt.deleted_at IS NULL \
             WHERE e.ref_right = s.id AND e.deleted_at IS NULL AND vt.code = ",
        );
        builder.push_bind(&view);
        builder.push(")");
    }
    // kind 简写 → 主体分类 code（客户/承运商/司机；未知值原样比对，缺省返回全部）
    if !kind.is_empty() {
        let code = match kind.as_str() {
            "customer" => "SUBJ-CUSTOMER",
            "carrier" => "SUBJ-CARRIER",
            "driver" => "SUBJ-DRIVER",
            _ => kind.as_str(),
        };
        builder.push(" AND c.code = ");
        builder.push_bind(code);
    }

    builder.push(" ORDER BY s.notice LIMIT ");
    builder.push_bind(page_size);
    builder.push(" OFFSET ");
    builder.push_bind(offset);

    #[allow(clippy::type_complexity)] // sqlx 行类型
    let rows: Vec<(
        i64,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        String,
        Option<String>,
        serde_json::Value,
        Option<String>,
    )> = builder
        .build_query_as()
        .fetch_all(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?;

    // 总数（分页页码列表——批注 c65c4382：只显示当前页码）
    let total: i64 = {
        let mut cb: QueryBuilder<Postgres> = QueryBuilder::new(
            "SELECT COUNT(*) FROM \"isahl\".\"zc_id_subjects\" s \
             LEFT JOIN \"isahl\".\"zc_id_cate-subject\" c ON c.id = s.ck_category AND c.deleted_at IS NULL \
             WHERE s.deleted_at IS NULL",
        );
        if exclude_system {
            cb.push(
                " AND btrim(s.tableoid::regclass::text, '\"') NOT IN ('zc_id_subj-position', 'zc_id_subjects')",
            );
        }

        if !q.is_empty() {
            let pat = format!("%{}%", q);
            cb.push(" AND (s.notice ILIKE ");
            cb.push_bind(pat.clone());
            cb.push(" OR s.code ILIKE ");
            cb.push_bind(pat);
            cb.push(")");
        }
        if !category.is_empty() {
            // 同列表过滤：叶表 tableoid（父表 comments.kind 回退已移除）
            cb.push(" AND btrim(s.tableoid::regclass::text, '\"') = ");
            cb.push_bind(leaf_table_for_category(&category));
        }
        if !view.is_empty() {
            cb.push(
                " AND EXISTS (SELECT 1 FROM \"isahl\".\"zc_id_subj-post_rr_view\" e \
                 JOIN \"isahl\".\"zc_id_relation-post_view_r_tags\" r ON r.ref_left = e.ref_left AND r.deleted_at IS NULL \
                 JOIN \"isahl\".\"zc_id_tags-post_view\" vt ON vt.id = r.ref_right AND vt.deleted_at IS NULL \
                 WHERE e.ref_right = s.id AND e.deleted_at IS NULL AND vt.code = ",
            );
            cb.push_bind(view);
            cb.push(")");
        }
        if !kind.is_empty() {
            let code = match kind.as_str() {
                "customer" => "SUBJ-CUSTOMER",
                "carrier" => "SUBJ-CARRIER",
                "driver" => "SUBJ-DRIVER",
                _ => kind.as_str(),
            };
            cb.push(" AND c.code = ");
            cb.push_bind(code);
        }
        cb.build_query_scalar::<i64>()
            .fetch_one(pool.get_ref())
            .await
            .map_err(ApiError::from_sqlx)?
    };

    let items: Vec<SubjectListItem> = rows
        .into_iter()
        .map(
            |(id, code, notice, leaf, comments, status, category_code, view_tags, mdm_code)| {
                // 父表插入（tableoid=zc_id_subjects）的行：优先读 comments.kind 回退类型标签
                // （create 时 subject_type 落 comments.kind，叶表名或中文别名）
                let base = social_category(&leaf);
                let social = if base == "主体" {
                    comments
                        .as_deref()
                        .and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok())
                        .and_then(|v| {
                            v.get("kind").and_then(|k| k.as_str()).map(|k| {
                                if k.starts_with("zc_id_") {
                                    social_category(k).to_string()
                                } else {
                                    k.to_string()
                                }
                            })
                        })
                        .unwrap_or_else(|| base.to_string())
                } else {
                    base.to_string()
                };
                SubjectListItem {
                    id,
                    code,
                    notice,
                    social_category: social,
                    category: category_code,
                    view_tags: view_tags
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default(),
                    status,
                    comments,
                    mdm_code,
                }
            },
        )
        .collect();

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "items": items,
            "total": total,
        }))),
    )
}

/// GET /service/isahl-db/subjects/{id}/view-tags — 读交易视角标签
pub async fn get_subject_view_tags(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "read").await?;

    ensure_subject_exists(pool.get_ref(), subject_id).await?;

    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT DISTINCT vt.id, vt.code, vt.notice, vt.o_number \
         FROM \"isahl\".\"zc_id_subj-post_rr_view\" e \
         JOIN \"isahl\".\"zc_id_relation-post_view_r_tags\" r ON r.ref_left = e.ref_left AND r.deleted_at IS NULL \
         JOIN \"isahl\".\"zc_id_tags-post_view\" vt ON vt.id = r.ref_right AND vt.deleted_at IS NULL \
         WHERE e.ref_right = $1 AND e.deleted_at IS NULL \
         ORDER BY vt.o_number, vt.id",
    )
    .bind(subject_id)
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    let items: Vec<ViewTagItem> = rows
        .into_iter()
        .map(|(id, code, notice)| ViewTagItem { id, code, notice })
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

#[derive(Debug, Deserialize)]
pub struct PutViewTagsRequest {
    /// 目标视角 code 列表（如 ["VIEW-CUST", "VIEW-CARR"]）
    pub view_tags: Vec<String>,
}

/// PUT /service/isahl-db/subjects/{id}/view-tags — 写交易视角标签（差额同步，幂等）
pub async fn put_subject_view_tags(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<PutViewTagsRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "update").await?;

    ensure_subject_exists(pool.get_ref(), subject_id).await?;

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    // 主体岗位（subj-post_rr_view: ref_left=岗位, ref_right=被视角主体）
    // 无岗位且目标标签为空 → 无操作直接成功（基本信息保存不应被岗位门禁阻断）；
    // 无岗位但要写标签 → 自动建默认岗位挂接（标签载体是岗位，缺则自建，不再 400）
    let post_id: Option<i64> = sqlx::query_scalar(
        "SELECT ref_left FROM \"isahl\".\"zc_id_subj-post_rr_view\" \
         WHERE ref_right = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1",
    )
    .bind(subject_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    let post_id = match post_id {
        Some(post_id) => post_id,
        None => {
            if body.view_tags.is_empty() {
                tx.commit().await.map_err(ApiError::from_sqlx)?;
                return Ok(
                    HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                        "id": subject_id.to_string(),
                        "viewTags": body.view_tags,
                    }))),
                );
            }
            ensure_subject_post(&mut tx, subject_id, None, user_id).await?
        }
    };

    // 差量同步视角标签（幂等；未知字典 code → 400）
    sync_view_tags(&mut tx, post_id, &body.view_tags, user_id).await?;

    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "id": subject_id.to_string(),
            "viewTags": body.view_tags,
        }))),
    )
}

/// GET /service/isahl-db/subjects/{id}/accounts — 读关联账户
pub async fn list_subject_accounts(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "read").await?;

    ensure_subject_exists(pool.get_ref(), subject_id).await?;

    let rows: Vec<(i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT r.id, r.ref_right, a.notice \
         FROM \"isahl\".\"zc_id_subjects_rr_account\" r \
         LEFT JOIN \"isahl\".\"zc_id_subjects\" a ON a.id = r.ref_right AND a.deleted_at IS NULL \
         WHERE r.ref_left = $1 AND r.deleted_at IS NULL \
         ORDER BY r.id",
    )
    .bind(subject_id)
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    let items: Vec<AccountItem> = rows
        .into_iter()
        .map(|(rel_id, account_id, account_name)| AccountItem {
            rel_id,
            account_id,
            account_name,
        })
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

#[derive(Debug, Deserialize)]
pub struct AddAccountRequest {
    /// 账户实体 id（ref_right，如 zc_id_bank-commercial 叶表行）
    #[serde(with = "common::serde_zuid")]
    pub account_id: i64,
}

/// POST /service/isahl-db/subjects/{id}/accounts — 添加账户关联（幂等）
pub async fn add_subject_account(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<AddAccountRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "update").await?;

    ensure_subject_exists(pool.get_ref(), subject_id).await?;
    // 账户实体存在性（subjects 继承链统一可见）
    let account_exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM \"isahl\".\"zc_id_subjects\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(body.account_id)
    .fetch_one(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    if !account_exists {
        return Err(ApiError::NotFound(format!(
            "账户实体不存在: {}",
            body.account_id
        )));
    }

    // 已存在 → 返回现有关联（幂等）
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM \"isahl\".\"zc_id_subjects_rr_account\" \
         WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(subject_id)
    .bind(body.account_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    if let Some(rel_id) = existing {
        return Ok(
            HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                "id": rel_id.to_string(),
            }))),
        );
    }

    let rel_id: i64 = sqlx::query_scalar(
        "INSERT INTO \"isahl\".\"zc_id_subjects_rr_account\" \
         (notice, ref_left, ref_right, created_by_id, updated_by_id) \
         VALUES ($1, $2, $3, $4, $4) RETURNING id",
    )
    .bind(format!("subject-{} account", subject_id))
    .bind(subject_id)
    .bind(body.account_id)
    .bind(user_id)
    .fetch_one(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": rel_id.to_string(),
        }))),
    )
}

/// DELETE /service/isahl-db/subjects/{id}/accounts/{relId} — 删除账户关联（软删除）
pub async fn delete_subject_account(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (subject_id, rel_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "update").await?;

    let rows = sqlx::query(
        "UPDATE \"isahl\".\"zc_id_subjects_rr_account\" SET deleted_at = NOW(), deleted_by_id = $3 \
         WHERE id = $1 AND ref_left = $2 AND deleted_at IS NULL",
    )
    .bind(rel_id)
    .bind(subject_id)
    .bind(user_id)
    .execute(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    if rows.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!(
            "账户关联不存在: rel={} subject={}",
            rel_id, subject_id
        )));
    }

    Ok(HttpResponse::NoContent().finish())
}

/// 确保主体已挂岗位：返回岗位 id。
/// - 指定岗位：校验存在后幂等挂接 `zc_id_subj-post_rr_view`（ref_left=岗位, ref_right=主体）
/// - 未指定：自动建默认岗位 `POST-AUTO-<subject_id>` 并挂接（与存量回填种子 seed-wz-defaults.sql 同模式）
async fn ensure_subject_post(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    subject_id: i64,
    position_id: Option<i64>,
    user_id: i64,
) -> Result<i64, ApiError> {
    if let Some(post_id) = position_id {
        let exists: bool = sqlx::query_scalar(
            "SELECT COUNT(*) > 0 FROM \"isahl\".\"zc_id_subj-position\" WHERE id = $1 AND deleted_at IS NULL AND _f_ IS NULL",
        )
        .bind(post_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        if !exists {
            return Err(ApiError::BadRequest(format!("岗位不存在: {}", post_id)));
        }
        sqlx::query(
            "INSERT INTO \"isahl\".\"zc_id_subj-post_rr_view\" \
             (notice, ref_left, ref_right, created_by_id) \
             SELECT 'auto-link', $1, $2, $3 \
             WHERE NOT EXISTS (SELECT 1 FROM \"isahl\".\"zc_id_subj-post_rr_view\" e \
                               WHERE e.ref_left = $1 AND e.ref_right = $2 AND e.deleted_at IS NULL)",
        )
        .bind(post_id)
        .bind(subject_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        return Ok(post_id);
    }

    // 无指定岗位：自动建默认岗位 POST-AUTO-<subject_id>（幂等，与种子回填一致）
    // id 显式 gen_next_zuid（与 wz 库 zc_id_subj-position 表默认同生成器；测试库无表默认亦可用）
    let inserted: Option<i64> = sqlx::query_scalar(
        "INSERT INTO \"isahl\".\"zc_id_subj-position\" (id, notice, code, created_by_id) \
         SELECT isahl.gen_next_zuid(), '默认岗位', 'POST-AUTO-' || $1::text, $2 \
         WHERE NOT EXISTS (SELECT 1 FROM \"isahl\".\"zc_id_subj-position\" p \
                           WHERE p.code = 'POST-AUTO-' || $1::text AND p.deleted_at IS NULL) \
         RETURNING id",
    )
    .bind(subject_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    let post_id = match inserted {
        Some(post_id) => post_id,
        None => sqlx::query_scalar(
            "SELECT id FROM \"isahl\".\"zc_id_subj-position\" \
             WHERE code = 'POST-AUTO-' || $1::text AND deleted_at IS NULL",
        )
        .bind(subject_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(ApiError::from_sqlx)?,
    };
    sqlx::query(
        "INSERT INTO \"isahl\".\"zc_id_subj-post_rr_view\" \
         (notice, ref_left, ref_right, created_by_id) \
         SELECT 'auto-link', $1, $2, $3 \
         WHERE NOT EXISTS (SELECT 1 FROM \"isahl\".\"zc_id_subj-post_rr_view\" e \
                           WHERE e.ref_left = $1 AND e.ref_right = $2 AND e.deleted_at IS NULL)",
    )
    .bind(post_id)
    .bind(subject_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    Ok(post_id)
}

/// 差量同步视角标签（幂等）：岗位不在目标集合的标签关系软删，目标视角 upsert。
/// 视角字典未知 code → 400。
async fn sync_view_tags(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    post_id: i64,
    target_codes: &[String],
    user_id: i64,
) -> Result<(), ApiError> {
    // 目标视角字典 id（未知 code → 400）
    let mut target_ids: Vec<i64> = Vec::new();
    for code in target_codes {
        let dict_id: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM \"isahl\".\"zc_id_tags-post_view\" WHERE code = $1 AND deleted_at IS NULL",
        )
        .bind(code)
        .fetch_optional(&mut **tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        let dict_id = dict_id.ok_or_else(|| {
            ApiError::BadRequest(format!(
                "未知交易视角 code: '{}'（字典 zc_id_tags-post_view）",
                code
            ))
        })?;
        target_ids.push(dict_id);
    }

    // 删除岗位不在目标集合中的视角标签关联
    sqlx::query(
        "UPDATE \"isahl\".\"zc_id_relation-post_view_r_tags\" SET deleted_at = NOW(), deleted_by_id = $2 \
         WHERE ref_left = $1 AND deleted_at IS NULL \
         AND ref_right != ALL($3)",
    )
    .bind(post_id)
    .bind(user_id)
    .bind(&target_ids)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    // 幂等 upsert 目标关联（已存在则跳过；表无 UNIQUE 约束，用 WHERE NOT EXISTS）
    for dict_id in &target_ids {
        sqlx::query(
            "INSERT INTO \"isahl\".\"zc_id_relation-post_view_r_tags\" \
             (notice, ref_left, ref_right, created_by_id, updated_by_id) \
             SELECT $1, $2, $3, $4, $4 \
             WHERE NOT EXISTS (SELECT 1 FROM \"isahl\".\"zc_id_relation-post_view_r_tags\" r \
                               WHERE r.ref_left = $2 AND r.ref_right = $3 AND r.deleted_at IS NULL)",
        )
        .bind(format!("post-{} view-tag", post_id))
        .bind(post_id)
        .bind(dict_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::from_sqlx)?;
    }
    Ok(())
}

/// POST /service/isahl-db/subjects — 创建主体（notice/code 必填）
pub async fn create_subject(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<CreateSubjectRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "create").await?;

    let notice = body.notice.trim().to_string();
    if notice.is_empty() {
        return Err(ApiError::BadRequest("notice 不能为空".into()));
    }
    // code 可选：空则自动生成 SUBJ-<zuid 后 6 位>（唯一性由 zuid 保证）
    let code = match body.code.as_deref().map(str::trim) {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => {
            let z: i64 = sqlx::query_scalar("SELECT isahl.gen_next_zuid()")
                .fetch_one(pool.get_ref())
                .await
                .map_err(ApiError::from_sqlx)?;
            format!("SUBJ-{:06}", z % 1_000_000)
        }
    };
    // comments 为纯文本语义（remove-comments-json-embedding）：仅存请求备注原文
    // （subject_type 不再并入——kind 寄生通道移除）
    let comments: Option<String> = body
        .comments
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string);

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    // 批注（用户指示）：主体数据一律插入对应叶表（不回退父表）——
    // subject_leaf_table fail-fast（2026-08-29 裁决）：未知/中间层类型 400——
    // 旧行为静默回退 subj-group 造成错分类，已废除；
    // 只插父表则业务侧查子表（natural-persons/司机下拉等）看不到，事后补子表会双行
    let leaf: &'static str = crate::models::subject_leaf_table(
        body.subject_type.as_deref().unwrap_or(""),
    )
    .ok_or_else(|| {
        ApiError::BadRequest(format!(
            "未知或缺失主体类型: {:?}（法人须明确叶表：非银行法人/商业银行）",
            body.subject_type
        ))
    })?;
    // 表名来自白名单常量映射（无注入）——sqlx 动态 SQL 审计需 AssertSqlSafe 包装
    let sql = format!(
        r#"INSERT INTO {} (id, notice, code, comments, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4)
           RETURNING id"#,
        leaf
    );
    let id: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(&notice)
        .bind(&code)
        .bind(&comments)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;

    // 挂岗 + 视角标签（事务内一次落库，避免创建后 PUT view-tags 无岗位 400）
    // view_tags 为空但指定了岗位 → 仍挂岗（基本信息保存完整落库）；
    // 两者皆空 → 不强制建岗（列表 viewTags 为空即可，后续 PUT 会自建）
    let view_tags = body.view_tags.clone().unwrap_or_default();
    if !view_tags.is_empty() {
        let post_id = ensure_subject_post(&mut tx, id, body.position_id, user_id).await?;
        sync_view_tags(&mut tx, post_id, &view_tags, user_id).await?;
    } else if body.position_id.is_some() {
        ensure_subject_post(&mut tx, id, body.position_id, user_id).await?;
    }

    // 联系方式落「实体↔联系人↔联系方式」本体链（元数据实名，remap-subject-bank-invoice-isahl）：
    // kind → typed 叶表（缺省 telephone），值不落 o_number、kind 不落 comments
    if let Some(contacts) = &body.contacts {
        for (ci, c) in contacts.iter().enumerate() {
            let value = c.value.trim().to_string();
            if value.is_empty() {
                continue;
            }
            add_entity_contact(
                &mut tx,
                id,
                &notice,
                None,
                c.kind.as_deref(),
                &value,
                c.is_default.unwrap_or(ci == 0),
                user_id,
            )
            .await?;
        }
    }
    if let Some(identities) = &body.identities {
        for (ii, ident) in identities.iter().enumerate() {
            let cert_no = ident.cert_no.trim().to_string();
            if cert_no.is_empty() {
                continue;
            }
            let category_id = category_id_by_code(pool.get_ref(), &ident.category_code).await?;
            let dname = ident.name.clone().unwrap_or_else(|| cert_no.clone());
            let identity_id: i64 = sqlx::query_scalar(
                r#"INSERT INTO "isahl"."zc_id_identity" (notice, identity, dname, ck_category, created_by_id, updated_by_id)
                   VALUES ($1, $2, $3, $4, $5, $5) RETURNING id"#,
            )
            .bind(format!("{} 证照 {}", notice, ii + 1))
            .bind(&cert_no)
            .bind(&dname)
            .bind(category_id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::from_sqlx)?;
            sqlx::query(
                r#"INSERT INTO "isahl"."zc_id_entity_rr_identity"
                   (notice, ref_left, ref_right, created_by_id, updated_by_id)
                   VALUES ($1, $2, $3, $4, $4)"#,
            )
            .bind(format!("subject-{} identity", id))
            .bind(id)
            .bind(identity_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::from_sqlx)?;
        }
    }

    // 雇佣状态落雇员链（主体-雇员 + 关系-雇员→任职状态；change: align-org-position-employment-chains）
    if let Some(employ_status) = body
        .employ_status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let status_id: i64 = sqlx::query_scalar(
            r#"SELECT id FROM "isahl"."zc_id_stus-employ" WHERE code = $1 AND deleted_at IS NULL"#,
        )
        .bind(employ_status)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "未知雇佣状态 code: '{}'（字典 zc_id_stus-employ）",
                employ_status
            ))
        })?;
        let employee_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_subj-employee" (id, notice, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2) RETURNING id"#,
        )
        .bind(format!("{} 雇员", notice))
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_subj-employee_r_employ-status"
               (ref_left, ref_right, status_date, created_by_id)
               VALUES ($1, $2, now(), $3)"#,
        )
        .bind(employee_id)
        .bind(status_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
    }

    // 批注（修复：此前 edit 误删 commit——事务 drop 自动回滚，INSERT 全丢但返回 201）
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": id.to_string(),
            "code": code,
        }))),
    )
}

/// PUT /service/isahl-db/subjects/{id} — 更新主体（None 字段保持不变）
pub async fn update_subject(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<UpdateSubjectRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", id, "update").await?;
    if let Some(notice) = &body.notice {
        if notice.trim().is_empty() {
            return Err(ApiError::BadRequest("notice 不能为空".into()));
        }
    }
    if let Some(code) = &body.code {
        if code.trim().is_empty() {
            return Err(ApiError::BadRequest("code 不能为空".into()));
        }
    }

    let updated: Option<i64> = sqlx::query_scalar(
        r#"UPDATE "isahl"."zc_id_subjects"
           SET notice = COALESCE($2, notice),
               code = COALESCE($3, code),
               comments = COALESCE($4, comments)
           WHERE id = $1 AND deleted_at IS NULL
           RETURNING id"#,
    )
    .bind(id)
    .bind(&body.notice)
    .bind(&body.code)
    .bind(&body.comments)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    match updated {
        Some(id) => {
            // MDM 主数据编码（wz_fssc.subject_mdm）：Some(非空)=upsert，Some(空)=清除，None=不动
            if let Some(mdm_code) = &body.mdm_code {
                ensure_subject_mdm(pool.get_ref()).await?;
                let trimmed = mdm_code.trim();
                if trimmed.is_empty() {
                    sqlx::query("DELETE FROM wz_fssc.subject_mdm WHERE subject_id = $1")
                        .bind(id)
                        .execute(pool.get_ref())
                        .await
                        .map_err(ApiError::from_sqlx)?;
                } else {
                    sqlx::query(
                        r#"INSERT INTO wz_fssc.subject_mdm (subject_id, mdm_code, created_by_id, updated_by_id)
                           VALUES ($1, $2, $3, $3)
                           ON CONFLICT (subject_id)
                           DO UPDATE SET mdm_code = EXCLUDED.mdm_code,
                                         updated_by_id = EXCLUDED.updated_by_id,
                                         updated_at = now()"#,
                    )
                    .bind(id)
                    .bind(trimmed)
                    .bind(user_id)
                    .execute(pool.get_ref())
                    .await
                    .map_err(ApiError::from_sqlx)?;
                }
            }
            Ok(
                HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                    "id": id.to_string(),
                }))),
            )
        }
        None => Err(ApiError::NotFound(format!("主体不存在: {}", id))),
    }
}

/// DELETE /service/isahl-db/subjects/{id} — 软删除主体
pub async fn delete_subject(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", id, "delete").await?;

    let deleted = sqlx::query(
        r#"UPDATE "isahl"."zc_id_subjects"
           SET deleted_at = NOW()
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .execute(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?
    .rows_affected();

    if deleted == 0 {
        return Err(ApiError::NotFound(format!("主体不存在: {}", id)));
    }
    Ok(HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({ "deleted": true }))))
}

/// 主体存在性校验（subjects 继承链统一可见）
pub(crate) async fn ensure_subject_exists(pool: &PgPool, subject_id: i64) -> Result<(), ApiError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM \"isahl\".\"zc_id_subjects\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !exists {
        return Err(ApiError::NotFound(format!("主体不存在: {}", subject_id)));
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════
// GET /subjects/types — 主体叶表发现（动态）
// ═══════════════════════════════════════════════════════

/// 主体类型项（叶表发现结果）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubjectTypeItem {
    /// 叶表名（create 时作为 subject_type 传入）
    table_name: String,
    /// 叶表名（兼容旧字段）
    code: String,
    /// 是否系统级分类（国家/超国家/银行等，不提供创建入口由前端过滤）
    system: bool,
}

/// GET /service/isahl-db/subjects/types — 动态发现 zc_id_subjects 继承链叶表
///
/// 数据源：pg_inherits 递归（真实 DB 继承关系），非硬编码。
/// 业务名由叶表名派生（Alioth 模型 collection 名在 Meta 侧，WZ 库无元数据）。
pub async fn list_subject_types(
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "list").await?;

    // 真叶表：pg_inherits 递归 descendants 中**无子表**的表。
    // 中间表（zc_id_orga-legal 法人、zc_id_subj-org 组织、zc_id_subj-employee 雇员）
    // 有子表，非叶表——不可直接插入（tableoid 判定分类会落到父表）。
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"WITH RECURSIVE descendants AS (
               SELECT c.oid, c.relname AS leaf
               FROM pg_class c
               JOIN pg_namespace n ON n.oid = c.relnamespace
               WHERE n.nspname = 'isahl' AND c.relname = 'zc_id_subjects'
               UNION ALL
               SELECT child.oid, child.relname
               FROM pg_inherits i
               JOIN pg_class child ON child.oid = i.inhrelid
               JOIN pg_namespace n ON n.oid = child.relnamespace AND n.nspname = 'isahl'
               JOIN descendants ON descendants.oid = i.inhparent
           ),
           has_children AS (
               SELECT DISTINCT i.inhparent AS oid
               FROM pg_inherits i
               JOIN pg_class child ON child.oid = i.inhrelid
               JOIN pg_namespace n ON n.oid = child.relnamespace
               WHERE n.nspname = 'isahl'
           )
           SELECT DISTINCT d.leaf FROM descendants d
           WHERE d.leaf <> 'zc_id_subjects'
             AND d.oid NOT IN (SELECT oid FROM has_children)
           ORDER BY d.leaf"#,
    )
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    // 系统级分类（不提供常规创建入口；央行/国家/超国家/主权/部委/层级/银行账户）。
    // 注意：商业银行（zc_id_bank-commercial）继承法人（zc_id_orga-legal），是企业类叶表，
    // 提供创建入口；中央银行（zc_id_bank-central）继承部委，属系统级。
    const SYSTEM_LEAVES: [&str; 7] = [
        "zc_id_bank-central",
        "zc_id_subj-country",
        "zc_id_subj-supranational",
        "zc_id_subj-sovereign",
        "zc_id_subj-ministry",
        "zc_id_subj-bank",
        "zc_id_subj-hierarchy",
    ];

    let items: Vec<SubjectTypeItem> = rows
        .into_iter()
        .map(|(leaf,)| {
            let table_name = leaf.clone();
            SubjectTypeItem {
                system: SYSTEM_LEAVES.iter().any(|p| leaf.starts_with(p)),
                code: leaf,
                table_name,
            }
        })
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

/// 校验 subject_type 是合法主体类型（防注入与非主体表）
/// - 叶表名（zc_id_ 前缀）→ 校验 ∈ subjects 继承链**且为真叶表**（无子表）
/// - 业务别名（法人/雇员/组 等）→ subject_leaf_table 白名单映射即合法
#[allow(dead_code)] // 主体叶表校验（防注入），创建路径接线待定
async fn ensure_subject_leaf(pool: &PgPool, subject_type: &str) -> Result<(), ApiError> {
    if subject_type.starts_with("zc_id_") {
        // 真叶表：descendants 中无子表的表。中间表（orga-legal 等）不可插入。
        let exists: bool = sqlx::query_scalar(
            r#"WITH RECURSIVE descendants AS (
                   SELECT c.oid, c.relname AS leaf
                   FROM pg_class c
                   JOIN pg_namespace n ON n.oid = c.relnamespace
                   WHERE n.nspname = 'isahl' AND c.relname = 'zc_id_subjects'
                   UNION ALL
                   SELECT child.oid, child.relname
                   FROM pg_inherits i
                   JOIN pg_class child ON child.oid = i.inhrelid
                   JOIN pg_namespace n ON n.oid = child.relnamespace AND n.nspname = 'isahl'
                   JOIN descendants ON descendants.oid = i.inhparent
               ),
               has_children AS (
                   SELECT DISTINCT i.inhparent AS oid
                   FROM pg_inherits i
                   JOIN pg_class child ON child.oid = i.inhrelid
                   JOIN pg_namespace n ON n.oid = child.relnamespace
                   WHERE n.nspname = 'isahl'
               )
               SELECT EXISTS (
                   SELECT 1 FROM descendants d
                   WHERE d.leaf = $1 AND d.oid NOT IN (SELECT oid FROM has_children)
               )"#,
        )
        .bind(subject_type)
        .fetch_one(pool)
        .await
        .map_err(ApiError::from_sqlx)?;
        if !exists {
            return Err(ApiError::BadRequest(format!(
                "主体类型不是 zc_id_subjects 叶表: {}",
                subject_type
            )));
        }
        Ok(())
    } else {
        // 业务别名：subject_leaf_table 白名单覆盖即合法；未知 → 400（fail-fast）
        if crate::models::subject_leaf_table(subject_type).is_none() {
            return Err(ApiError::BadRequest(format!(
                "未知主体类型: {}",
                subject_type
            )));
        }
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════
// POST /subjects — 分类感知主体创建
// ═══════════════════════════════════════════════════════

/// 分类感知创建请求体。
/// 扩展信息通过 Alioth 关联关系承载（零 DDL）：
/// - view_tags    → 主体→岗位(`zc_id_subj-post_rr_view`)→`zc_id_relation-post_view_r_tags`→`zc_id_tags-post_view`
/// - employ_status→ `zc_id_subj-employee_r_employ-status`（雇佣状态，ref_right→stus-employ 字典）
/// - employer_id  → `zc_id_subj-org_rr_employee`（所属组织：ref_left=组织, ref_right=雇员）
/// - position_id  → `zc_id_subj-post_rr_view`（视角锚点：ref_left=岗位, ref_right=被视角主体；任职关系不在此路径，由 entity-binding 经 post_rr_employee 承载）
/// - place_id     → `zc_id_subjects_rr_place`（地址：ref_left=主体, ref_right=place）
/// - member_ids   → `zc_id_subj-group_rr_member`（组成员：ref_left=组, ref_right=成员）
///
/// 注册 org-wz 主体管理路由
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/subjects")
            .route(web::get().to(list_subjects))
            .route(web::post().to(create_subject)),
    )
    .service(
        // 注意：/subjects/types 必须注册在 /subjects/{id} 之前，
        // 否则 "types" 被 {id} 路由匹配（GET 无 → 405 Method Not Allowed）。
        web::resource("/subjects/types").route(web::get().to(list_subject_types)),
    )
    .service(
        web::resource("/subjects/{id}")
            .route(web::put().to(update_subject))
            .route(web::delete().to(delete_subject)),
    )
    .service(
        web::resource("/subjects/{id}/view-tags")
            .route(web::get().to(get_subject_view_tags))
            .route(web::put().to(put_subject_view_tags)),
    )
    .service(
        web::resource("/subjects/{id}/accounts")
            .route(web::get().to(list_subject_accounts))
            .route(web::post().to(add_subject_account)),
    )
    .service(
        web::resource("/subjects/{id}/accounts/{relId}")
            .route(web::delete().to(delete_subject_account)),
    )
    .service(
        // 联系方式链（实体↔联系人↔联系方式）：读/增/删，供综合管理编辑闭环
        web::resource("/subjects/{id}/contacts")
            .route(web::get().to(list_subject_contact_chain))
            .route(web::post().to(add_subject_contact)),
    )
    .service(
        web::resource("/subjects/{id}/contacts/{contactId}")
            .route(web::delete().to(remove_subject_contact)),
    );
}

/// kind → typed 联系方式叶表（闭式白名单；缺省 telephone）
fn contact_info_table(kind: Option<&str>) -> &'static str {
    match kind.map(str::trim).filter(|k| !k.is_empty()) {
        Some("email") => "zc_id_info-email",
        Some("im") => "zc_id_info-im",
        Some("isahl") => "zc_id_info-isahl",
        Some("postal") | Some("address") => "zc_id_info-postal",
        Some("zipcode") | Some("zip") => "zc_id_info-zipcode",
        _ => "zc_id_info-telephone",
    }
}

/// 向实体（主体=实体子表行）追加一条联系方式（实体↔联系人↔联系方式链，同事务）
pub(crate) async fn add_entity_contact(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    entity_id: i64,
    display_name: &str,
    contact_name: Option<&str>,
    kind: Option<&str>,
    value: &str,
    is_default: bool,
    user_id: i64,
) -> Result<i64, ApiError> {
    let contact_row = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO "isahl"."zc_id_contacts" (id, code, notice, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3) RETURNING id"#,
    )
    .bind(format!("CT-ENT-{}", entity_id))
    .bind(contact_name.unwrap_or(&format!("{} 联系方式", display_name)))
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    let info_table = contact_info_table(kind);
    let info_sql = format!(
        r#"INSERT INTO "isahl"."{info_table}" (id, notice, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2) RETURNING id"#
    );
    let info_id: i64 = sqlx::query_scalar(AssertSqlSafe(info_sql.as_str()))
        .bind(value)
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(ApiError::from_sqlx)?;
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_contacts_rr_infos"
           (notice, ref_left, ref_right, default_info, created_by_id)
           VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(format!("{} 联系方式", display_name))
    .bind(contact_row)
    .bind(info_id)
    .bind(is_default)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_entity_rr_contacts"
           (id, code, notice, ref_left, ref_right, default_contact, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6)"#,
    )
    .bind(format!("REL-CT-{}", entity_id))
    .bind(format!("{} 联系方式", display_name))
    .bind(entity_id)
    .bind(contact_row)
    .bind(is_default)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    Ok(contact_row)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SubjectContactItem {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    /// 联系人显示名（zc_id_contacts.notice）
    pub name: String,
    /// 类型（kind：telephone/email/im/postal/zipcode/isahl，由值行 tableoid 派生）
    pub kind: String,
    /// 值（typed 叶表 notice）
    pub value: String,
    /// 是否默认联系方式（rr_infos.default_info）
    pub is_default: bool,
}

/// GET /subjects/{id}/contacts — 联系方式链只读列表
pub async fn list_subject_contact_chain(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "read").await?;
    ensure_subject_exists(pool.get_ref(), subject_id).await?;

    let rows: Vec<SubjectContactItem> = sqlx::query_as(
        r#"SELECT c.id, COALESCE(c.notice, '') AS name,
                  CASE btrim(ri.tableoid::regclass::text, '"')
                       WHEN 'zc_id_info-email' THEN 'email'
                       WHEN 'zc_id_info-im' THEN 'im'
                       WHEN 'zc_id_info-isahl' THEN 'isahl'
                       WHEN 'zc_id_info-postal' THEN 'postal'
                       WHEN 'zc_id_info-zipcode' THEN 'zipcode'
                       ELSE 'telephone' END AS kind,
                  COALESCE(i.notice, '') AS value,
                  COALESCE(ri.default_info, false) AS is_default
           FROM "isahl"."zc_id_entity_rr_contacts" rc
           JOIN "isahl"."zc_id_contacts" c ON c.id = rc.ref_right AND c.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_contacts_rr_infos" ri ON ri.ref_left = c.id AND ri.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_info-telephone" t ON t.id = ri.ref_right AND t.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_info-email" e ON e.id = ri.ref_right AND e.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_info-im" im ON im.id = ri.ref_right AND im.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_info-isahl" isa ON isa.id = ri.ref_right AND isa.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_info-postal" p ON p.id = ri.ref_right AND p.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_info-zipcode" z ON z.id = ri.ref_right AND z.deleted_at IS NULL
           LEFT JOIN LATERAL (
               SELECT x.notice FROM "isahl"."zc_id_contact_infos" x
               WHERE x.id = ri.ref_right AND x.deleted_at IS NULL LIMIT 1
           ) i ON ri.ref_right IS NOT NULL
           WHERE rc.ref_left = $1 AND rc.deleted_at IS NULL
           ORDER BY rc.id"#,
    )
    .bind(subject_id)
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(rows)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddSubjectContactRequest {
    /// 联系人显示名（可空，缺省「{主体名} 联系方式」）
    pub name: Option<String>,
    /// 类型（telephone/email/im/postal/zipcode/isahl；缺省 telephone）
    pub kind: Option<String>,
    /// 值（必填）
    pub value: String,
    #[serde(default)]
    pub is_default: bool,
}

/// POST /subjects/{id}/contacts — 追加一条联系方式（同事务写链）
pub async fn add_subject_contact(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<AddSubjectContactRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "update").await?;
    if body.value.trim().is_empty() {
        return Err(ApiError::BadRequest("联系方式值不能为空".into()));
    }
    ensure_subject_exists(pool.get_ref(), subject_id).await?;

    let display_name: String = sqlx::query_scalar(
        r#"SELECT COALESCE(notice, '') FROM "isahl"."zc_id_subjects" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .fetch_one(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;
    let contact_id = add_entity_contact(
        &mut tx,
        subject_id,
        &display_name,
        body.name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        body.kind.as_deref(),
        body.value.trim(),
        body.is_default,
        user_id,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": contact_id.to_string(),
        }))),
    )
}

/// DELETE /subjects/{id}/contacts/{contactId} — 软删一条联系方式（链三段同事务）
pub async fn remove_subject_contact(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (subject_id, contact_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "update").await?;

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;
    let linked = sqlx::query(
        r#"UPDATE "isahl"."zc_id_entity_rr_contacts" rc
           SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
           WHERE rc.ref_left = $1 AND rc.ref_right = $3 AND rc.deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .bind(user_id)
    .bind(contact_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?
    .rows_affected();
    if linked == 0 {
        return Err(ApiError::NotFound(format!(
            "联系方式不存在: {}",
            contact_id
        )));
    }
    // 值行软删（先取 id——rr_infos 软删前）
    let info_ids: Vec<i64> = sqlx::query_scalar(
        r#"SELECT ref_right FROM "isahl"."zc_id_contacts_rr_infos"
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(contact_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !info_ids.is_empty() {
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_contact_infos" x
               SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
               WHERE x.id = ANY($1) AND x.deleted_at IS NULL"#,
        )
        .bind(&info_ids)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
    }
    sqlx::query(
        r#"UPDATE "isahl"."zc_id_contacts_rr_infos" ri
           SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
           WHERE ri.ref_left = $1 AND ri.deleted_at IS NULL"#,
    )
    .bind(contact_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    sqlx::query(
        r#"UPDATE "isahl"."zc_id_contacts" c
           SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
           WHERE c.id = $1 AND c.deleted_at IS NULL"#,
    )
    .bind(contact_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(HttpResponse::NoContent().finish())
}
