//! 组织规范资产线（Phase D-1 org-policy-assets；规格：`.planning/ngac-org-phase-d1-spec.md`）。
//!
//! 语义：
//! - 资产：`org_policy_class`（权责规范/权限类别声明）→ `org_policy_rule`（职责规则行）+
//!   `org_policy_label`（分级字典）。表由 `ngac/ensure.rs` 运行时幂等自愈。
//! - 状态机：`draft → in_review → active → retired`，仅允许顺序迁移（非法迁移 Err）；
//!   retired 不可回退（新版本链由服务层 `(code, version)` 承接——create 时若存在
//!   未退休同 code 行则拒绝，否则 version = 既有最大 version + 1）。
//! - 编辑门：class 内容更新与 rule/label 挂接仅限 `draft`/`in_review`（发布后冻结）。
//! - 审计：create/submit_review/activate/retire 及 rule/label 的 create/软删写
//!   `isahl_audit.audit_events`（`common::audit::record_audit_event`；
//!   operation = `policy.class.{create,submit_review,activate,retire}` /
//!   `policy.rule.{create,delete}` / `policy.label.{create,delete}`）。
//! - 投影：`project_policy_class` 是 D-2 派生器接口（spec §5）——读 class + active rules，
//!   产出 UA/OA/action/label 投影，供派生器幂等 ensure 落地。

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use common::audit::{record_audit_event, Decision};
use common::AliothError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// org_policy_class 行（投影/管理面通用载体）。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct OrgPolicyClass {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: String,
    pub notice: String,
    pub state: String,
    pub version: i32,
    pub scope: serde_json::Value,
    pub ua_template: serde_json::Value,
    pub label_code: Option<String>,
    pub prohibition_template: Option<serde_json::Value>,
    pub audit_required: bool,
    pub effective_from: Option<DateTime<Utc>>,
    pub effective_until: Option<DateTime<Utc>>,
    pub comments: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub created_by_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub updated_by_id: Option<i64>,
}

/// org_policy_rule 行。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct OrgPolicyRule {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub policy_class_id: i64,
    pub subject_code: String,
    pub resource_type: String,
    pub actions: serde_json::Value,
    pub condition: Option<serde_json::Value>,
    pub obligation: Option<serde_json::Value>,
    pub label_code: Option<String>,
    pub state: String,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub created_by_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub updated_by_id: Option<i64>,
}

/// org_policy_label 行（分级字典；code 为主键）。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct OrgPolicyLabel {
    pub code: String,
    pub rank: i32,
    pub domain: String,
    pub notice: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub created_by_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub updated_by_id: Option<i64>,
}

/// D-2 派生器投影契约（spec §5）——唯一实现收编于 `common::ngac_policy`（消费同源
/// 义务：SSO/Gateway 禁止复制投影 SQL），本处 re-export 保持管理面类型路径不变。
pub use common::ngac_policy::PolicyProjection;

/// 新建 class 请求体。
#[derive(Debug, Deserialize)]
pub struct CreateOrgPolicyClass {
    pub code: String,
    pub notice: String,
    pub scope: Option<serde_json::Value>,
    pub ua_template: Option<serde_json::Value>,
    pub label_code: Option<String>,
    pub prohibition_template: Option<serde_json::Value>,
    #[serde(default = "default_true")]
    pub audit_required: bool,
    pub effective_from: Option<DateTime<Utc>>,
    pub effective_until: Option<DateTime<Utc>>,
    pub comments: Option<String>,
}

fn default_true() -> bool {
    true
}

/// 更新 class 请求体（全可选；缺省字段保持原值；仅 draft/in_review 可编辑）。
#[derive(Debug, Default, Deserialize)]
pub struct UpdateOrgPolicyClass {
    pub notice: Option<String>,
    pub scope: Option<serde_json::Value>,
    pub ua_template: Option<serde_json::Value>,
    pub label_code: Option<String>,
    pub prohibition_template: Option<serde_json::Value>,
    pub audit_required: Option<bool>,
    pub effective_from: Option<DateTime<Utc>>,
    pub effective_until: Option<DateTime<Utc>>,
    pub comments: Option<String>,
}

/// 新建 rule 请求体。
#[derive(Debug, Deserialize)]
pub struct CreateOrgPolicyRule {
    #[serde(with = "common::serde_zuid")]
    pub policy_class_id: i64,
    pub subject_code: String,
    pub resource_type: String,
    pub actions: Vec<String>,
    pub condition: Option<serde_json::Value>,
    pub obligation: Option<serde_json::Value>,
    pub label_code: Option<String>,
}

/// 更新 rule 请求体（全可选；缺省字段保持原值；仅所属 class draft/in_review 可编辑）。
#[derive(Debug, Default, Deserialize)]
pub struct UpdateOrgPolicyRule {
    pub subject_code: Option<String>,
    pub resource_type: Option<String>,
    pub actions: Option<Vec<String>>,
    pub condition: Option<serde_json::Value>,
    pub obligation: Option<serde_json::Value>,
    pub label_code: Option<String>,
}

/// 新建 label 请求体。
#[derive(Debug, Deserialize)]
pub struct CreateOrgPolicyLabel {
    pub code: String,
    pub rank: i32,
    #[serde(default = "default_domain")]
    pub domain: String,
    pub notice: String,
}

/// 更新 label 请求体（全可选；缺省字段保持原值；软删行不可见不可改）。
#[derive(Debug, Default, Deserialize)]
pub struct UpdateOrgPolicyLabel {
    pub rank: Option<i32>,
    pub domain: Option<String>,
    pub notice: Option<String>,
}

fn default_domain() -> String {
    "security".to_string()
}

const CLASS_STATES: [&str; 4] = ["draft", "in_review", "active", "retired"];

const CLASS_COLUMNS: &str = "id, code, notice, state, version, scope, ua_template, label_code, \
     prohibition_template, audit_required, effective_from, effective_until, comments, \
     created_at, updated_at, created_by_id, updated_by_id";

const RULE_COLUMNS: &str = "id, policy_class_id, subject_code, resource_type, actions, condition, \
     obligation, label_code, state, version, created_at, updated_at, created_by_id, updated_by_id";

const LABEL_COLUMNS: &str = "code, rank, domain, notice, is_active, created_at, updated_at, \
     created_by_id, updated_by_id";

// ─────────────────────────── 服务层 ───────────────────────────

/// 新建 class（默认 draft）。同 code 存在未退休行 → 拒绝；仅 retired 历史 → version 递增。
pub async fn create_org_policy_class(
    pool: &PgPool,
    actor_id: i64,
    actor_email: &str,
    input: CreateOrgPolicyClass,
) -> Result<OrgPolicyClass, AliothError> {
    if input.code.trim().is_empty() || input.notice.trim().is_empty() {
        return Err(AliothError::BadRequest("code/notice 必填".to_string()));
    }
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT state FROM isahl_auth.org_policy_class \
         WHERE code = $1 AND deleted_at IS NULL AND state <> 'retired' LIMIT 1",
    )
    .bind(&input.code)
    .fetch_optional(pool)
    .await?;
    if let Some(state) = existing {
        return Err(AliothError::BadRequest(format!(
            "code '{}' 已存在未退休版本（state={}）；退休后方可开新版本链",
            input.code, state
        )));
    }
    let version: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) FROM isahl_auth.org_policy_class WHERE code = $1",
    )
    .bind(&input.code)
    .fetch_one(pool)
    .await?;

    let row = sqlx::query_as::<_, OrgPolicyClass>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO isahl_auth.org_policy_class \
         (code, notice, scope, ua_template, label_code, prohibition_template, audit_required, \
          effective_from, effective_until, comments, version, state, created_by_id, updated_by_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'draft', $12, $12) \
         RETURNING {CLASS_COLUMNS}"
    )))
    .bind(&input.code)
    .bind(&input.notice)
    .bind(input.scope.unwrap_or_else(|| serde_json::json!({})))
    .bind(input.ua_template.unwrap_or_else(|| serde_json::json!({})))
    .bind(input.label_code)
    .bind(input.prohibition_template)
    .bind(input.audit_required)
    .bind(input.effective_from)
    .bind(input.effective_until)
    .bind(input.comments)
    .bind(version + 1)
    .bind(actor_id)
    .fetch_one(pool)
    .await?;

    audit(pool, actor_id, actor_email, "policy.class.create", &row).await;
    Ok(row)
}

/// 通用审计落点（class/rule/label 资产共用；object_path 形如 `org_policy_class/{id}/{code}`）。
async fn audit_event(
    pool: &PgPool,
    actor_id: i64,
    actor_email: &str,
    operation: &str,
    object_path: &str,
) {
    if let Err(e) = record_audit_event(
        pool,
        actor_id,
        actor_email,
        object_path,
        operation,
        &Decision::Permit,
    )
    .await
    {
        log::warn!("org_policy audit({}) 记录失败（不阻断）: {}", operation, e);
    }
}

async fn audit(
    pool: &PgPool,
    actor_id: i64,
    actor_email: &str,
    operation: &str,
    class: &OrgPolicyClass,
) {
    audit_event(
        pool,
        actor_id,
        actor_email,
        operation,
        &format!("org_policy_class/{}/{}", class.id, class.code),
    )
    .await;
}

/// 取单个 class（软删不可见）。
pub async fn get_org_policy_class(pool: &PgPool, class_id: i64) -> Result<OrgPolicyClass, AliothError> {
    let row = sqlx::query_as::<_, OrgPolicyClass>(sqlx::AssertSqlSafe(format!(
        "SELECT {CLASS_COLUMNS} FROM isahl_auth.org_policy_class \
         WHERE id = $1 AND deleted_at IS NULL"
    )))
    .bind(class_id)
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| AliothError::NotFound(format!("org_policy_class {class_id} 不存在")))
}

/// class 列表（可选 state 过滤 + 分页；state 非白名单视为无过滤）。
pub async fn list_org_policy_classes(
    pool: &PgPool,
    state: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<OrgPolicyClass>, AliothError> {
    // 白名单外视为无过滤：SQL 恒三参占位，未过滤时 $1 bind NULL（$1 IS NULL 放行全量），
    // 避免 08P01（语句要求 3 个参数但只 bind 2 个）。
    let filtered = state
        .map(|s| CLASS_STATES.contains(&s))
        .unwrap_or(false);
    let bound_state = if filtered { state } else { None };
    let sql = format!(
        "SELECT {CLASS_COLUMNS} FROM isahl_auth.org_policy_class \
         WHERE deleted_at IS NULL AND ($1 IS NULL OR state = $1) \
         ORDER BY id DESC LIMIT $2 OFFSET $3"
    );
    let rows = sqlx::query_as::<_, OrgPolicyClass>(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(bound_state)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

fn apply_update(base: &OrgPolicyClass, u: &UpdateOrgPolicyClass) -> OrgPolicyClass {
    let mut out = base.clone();
    if let Some(v) = &u.notice {
        out.notice = v.clone();
    }
    if let Some(v) = &u.scope {
        out.scope = v.clone();
    }
    if let Some(v) = &u.ua_template {
        out.ua_template = v.clone();
    }
    if u.label_code.is_some() {
        out.label_code = u.label_code.clone();
    }
    if u.prohibition_template.is_some() {
        out.prohibition_template = u.prohibition_template.clone();
    }
    if let Some(v) = u.audit_required {
        out.audit_required = v;
    }
    if u.effective_from.is_some() {
        out.effective_from = u.effective_from;
    }
    if u.effective_until.is_some() {
        out.effective_until = u.effective_until;
    }
    if u.comments.is_some() {
        out.comments = u.comments.clone();
    }
    out
}

/// 更新 class 内容（仅 draft/in_review；读改写单事务，FOR UPDATE 防并发覆盖）。
pub async fn update_org_policy_class(
    pool: &PgPool,
    actor_id: i64,
    actor_email: &str,
    class_id: i64,
    input: UpdateOrgPolicyClass,
) -> Result<OrgPolicyClass, AliothError> {
    let mut tx = pool.begin().await?;
    let base = sqlx::query_as::<_, OrgPolicyClass>(sqlx::AssertSqlSafe(format!(
        "SELECT {CLASS_COLUMNS} FROM isahl_auth.org_policy_class \
         WHERE id = $1 AND deleted_at IS NULL FOR UPDATE"
    )))
    .bind(class_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AliothError::NotFound(format!("org_policy_class {class_id} 不存在")))?;
    if base.state != "draft" && base.state != "in_review" {
        return Err(AliothError::BadRequest(format!(
            "class {} state={} 已发布/退役，内容冻结（仅 draft/in_review 可编辑）",
            class_id, base.state
        )));
    }
    let merged = apply_update(&base, &input);
    sqlx::query(
        "UPDATE isahl_auth.org_policy_class SET notice=$2, scope=$3, ua_template=$4, \
         label_code=$5, prohibition_template=$6, audit_required=$7, effective_from=$8, \
         effective_until=$9, comments=$10, updated_at=NOW(), updated_by_id=$11 \
         WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(merged.id)
    .bind(&merged.notice)
    .bind(&merged.scope)
    .bind(&merged.ua_template)
    .bind(&merged.label_code)
    .bind(&merged.prohibition_template)
    .bind(merged.audit_required)
    .bind(merged.effective_from)
    .bind(merged.effective_until)
    .bind(&merged.comments)
    .bind(actor_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    audit(pool, actor_id, actor_email, "policy.class.update", &merged).await;
    Ok(merged)
}

async fn transition_class_state(
    pool: &PgPool,
    actor_id: i64,
    actor_email: &str,
    class_id: i64,
    expected: &str,
    next: &str,
    operation: &str,
) -> Result<OrgPolicyClass, AliothError> {
    let row = sqlx::query_as::<_, OrgPolicyClass>(sqlx::AssertSqlSafe(format!(
        "UPDATE isahl_auth.org_policy_class SET state=$2, updated_at=NOW(), updated_by_id=$3 \
         WHERE id=$1 AND deleted_at IS NULL AND state=$4 \
         RETURNING {CLASS_COLUMNS}"
    )))
    .bind(class_id)
    .bind(next)
    .bind(actor_id)
    .bind(expected)
    .fetch_optional(pool)
    .await?;
    let row = match row {
        Some(r) => r,
        None => {
            // 状态不匹配或不存在：区分 NotFound 与非法迁移（原子 UPDATE 兜底竞态）。
            let current: Option<String> = sqlx::query_scalar(
                "SELECT state FROM isahl_auth.org_policy_class WHERE id=$1 AND deleted_at IS NULL",
            )
            .bind(class_id)
            .fetch_optional(pool)
            .await?;
            return match current {
                None => Err(AliothError::NotFound(format!(
                    "org_policy_class {class_id} 不存在"
                ))),
                Some(cur) => Err(AliothError::BadRequest(format!(
                    "非法状态迁移：{cur} → {next}（仅允许 {expected} → {next}）"
                ))),
            };
        }
    };
    audit(pool, actor_id, actor_email, operation, &row).await;
    Ok(row)
}

/// draft → in_review（提交评审）。
pub async fn submit_review_org_policy_class(
    pool: &PgPool,
    actor_id: i64,
    actor_email: &str,
    class_id: i64,
) -> Result<OrgPolicyClass, AliothError> {
    transition_class_state(pool, actor_id, actor_email, class_id, "draft", "in_review", "policy.class.submit_review").await
}

/// in_review → active。
pub async fn activate_org_policy_class(
    pool: &PgPool,
    actor_id: i64,
    actor_email: &str,
    class_id: i64,
) -> Result<OrgPolicyClass, AliothError> {
    transition_class_state(pool, actor_id, actor_email, class_id, "in_review", "active", "policy.class.activate").await
}

/// active → retired。
pub async fn retire_org_policy_class(
    pool: &PgPool,
    actor_id: i64,
    actor_email: &str,
    class_id: i64,
) -> Result<OrgPolicyClass, AliothError> {
    transition_class_state(pool, actor_id, actor_email, class_id, "active", "retired", "policy.class.retire").await
}

/// 新建 rule（挂接 class 须在 draft/in_review 且未软删）。
pub async fn create_org_policy_rule(
    pool: &PgPool,
    actor_id: i64,
    actor_email: &str,
    input: CreateOrgPolicyRule,
) -> Result<OrgPolicyRule, AliothError> {
    let class_state: Option<String> = sqlx::query_scalar(
        "SELECT state FROM isahl_auth.org_policy_class \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(input.policy_class_id)
    .fetch_optional(pool)
    .await?;
    match class_state.as_deref() {
        Some("draft") | Some("in_review") => {}
        Some(s) => {
            return Err(AliothError::BadRequest(format!(
                "class {} state={} 冻结，不可挂接新 rule",
                input.policy_class_id, s
            )))
        }
        None => {
            return Err(AliothError::NotFound(format!(
                "org_policy_class {} 不存在",
                input.policy_class_id
            )))
        }
    }
    let row = sqlx::query_as::<_, OrgPolicyRule>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO isahl_auth.org_policy_rule \
         (policy_class_id, subject_code, resource_type, actions, condition, obligation, \
          label_code, state, version, created_by_id, updated_by_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'active', 1, $8, $8) \
         RETURNING {RULE_COLUMNS}"
    )))
    .bind(input.policy_class_id)
    .bind(input.subject_code)
    .bind(input.resource_type)
    .bind(serde_json::Value::Array(
        input.actions.into_iter().map(serde_json::Value::String).collect(),
    ))
    .bind(input.condition)
    .bind(input.obligation)
    .bind(input.label_code)
    .bind(actor_id)
    .fetch_one(pool)
    .await?;
    audit_event(
        pool,
        actor_id,
        actor_email,
        "policy.rule.create",
        &format!("org_policy_rule/{}/{}", row.id, row.subject_code),
    )
    .await;
    Ok(row)
}

/// 所属 class 编辑门（draft/in_review 放行；其余状态冻结 / 缺失 NotFound）。
/// 读改写单事务内与行锁配合（FOR UPDATE），防 class 并发激活后 rule 变更落库。
async fn ensure_rule_class_editable(
    pool: &mut sqlx::PgConnection,
    policy_class_id: i64,
) -> Result<(), AliothError> {
    let class_state: Option<String> = sqlx::query_scalar(
        "SELECT state FROM isahl_auth.org_policy_class \
         WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(policy_class_id)
    .fetch_optional(&mut *pool)
    .await?;
    match class_state.as_deref() {
        Some("draft") | Some("in_review") => Ok(()),
        Some(s) => Err(AliothError::BadRequest(format!(
            "class {} state={} 冻结，不可变更其 rule",
            policy_class_id, s
        ))),
        None => Err(AliothError::NotFound(format!(
            "org_policy_class {} 不存在",
            policy_class_id
        ))),
    }
}

fn apply_rule_update(base: &OrgPolicyRule, u: &UpdateOrgPolicyRule) -> OrgPolicyRule {
    let mut out = base.clone();
    if let Some(v) = &u.subject_code {
        out.subject_code = v.clone();
    }
    if let Some(v) = &u.resource_type {
        out.resource_type = v.clone();
    }
    if let Some(v) = &u.actions {
        out.actions = serde_json::Value::Array(
            v.iter().cloned().map(serde_json::Value::String).collect(),
        );
    }
    if u.condition.is_some() {
        out.condition = u.condition.clone();
    }
    if u.obligation.is_some() {
        out.obligation = u.obligation.clone();
    }
    if u.label_code.is_some() {
        out.label_code = u.label_code.clone();
    }
    out
}

/// 更新 rule 内容（仅所属 class draft/in_review；读改写单事务，FOR UPDATE 防并发覆盖）。
pub async fn update_org_policy_rule(
    pool: &PgPool,
    actor_id: i64,
    actor_email: &str,
    rule_id: i64,
    input: UpdateOrgPolicyRule,
) -> Result<OrgPolicyRule, AliothError> {
    let mut tx = pool.begin().await?;
    let base = sqlx::query_as::<_, OrgPolicyRule>(sqlx::AssertSqlSafe(format!(
        "SELECT {RULE_COLUMNS} FROM isahl_auth.org_policy_rule \
         WHERE id = $1 AND deleted_at IS NULL FOR UPDATE"
    )))
    .bind(rule_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AliothError::NotFound(format!("org_policy_rule {rule_id} 不存在")))?;
    ensure_rule_class_editable(&mut tx, base.policy_class_id).await?;
    let merged = apply_rule_update(&base, &input);
    sqlx::query(
        "UPDATE isahl_auth.org_policy_rule SET subject_code=$2, resource_type=$3, actions=$4, \
         condition=$5, obligation=$6, label_code=$7, updated_at=NOW(), updated_by_id=$8 \
         WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(merged.id)
    .bind(&merged.subject_code)
    .bind(&merged.resource_type)
    .bind(&merged.actions)
    .bind(&merged.condition)
    .bind(&merged.obligation)
    .bind(&merged.label_code)
    .bind(actor_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    audit_event(
        pool,
        actor_id,
        actor_email,
        "policy.rule.update",
        &format!("org_policy_rule/{}/{}", merged.id, merged.subject_code),
    )
    .await;
    Ok(merged)
}

/// 软删 rule（仅所属 class draft/in_review；事务 + FOR UPDATE 保证门控一致）。
pub async fn delete_org_policy_rule(
    pool: &PgPool,
    actor_id: i64,
    actor_email: &str,
    rule_id: i64,
) -> Result<OrgPolicyRule, AliothError> {
    let mut tx = pool.begin().await?;
    let base = sqlx::query_as::<_, OrgPolicyRule>(sqlx::AssertSqlSafe(format!(
        "SELECT {RULE_COLUMNS} FROM isahl_auth.org_policy_rule \
         WHERE id = $1 AND deleted_at IS NULL FOR UPDATE"
    )))
    .bind(rule_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AliothError::NotFound(format!("org_policy_rule {rule_id} 不存在")))?;
    ensure_rule_class_editable(&mut tx, base.policy_class_id).await?;
    let row = sqlx::query_as::<_, OrgPolicyRule>(sqlx::AssertSqlSafe(format!(
        "UPDATE isahl_auth.org_policy_rule SET deleted_at=NOW(), deleted_by_id=$2 \
         WHERE id=$1 AND deleted_at IS NULL RETURNING {RULE_COLUMNS}"
    )))
    .bind(rule_id)
    .bind(actor_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    audit_event(
        pool,
        actor_id,
        actor_email,
        "policy.rule.delete",
        &format!("org_policy_rule/{}/{}", row.id, row.subject_code),
    )
    .await;
    Ok(row)
}

/// class 的 rule 列表（含软删过滤；可选 state 过滤）。
pub async fn list_org_policy_rules(
    pool: &PgPool,
    class_id: i64,
) -> Result<Vec<OrgPolicyRule>, AliothError> {
    let rows = sqlx::query_as::<_, OrgPolicyRule>(sqlx::AssertSqlSafe(format!(
        "SELECT {RULE_COLUMNS} FROM isahl_auth.org_policy_rule \
         WHERE policy_class_id = $1 AND deleted_at IS NULL ORDER BY id"
    )))
    .bind(class_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 新建分级 label（code 主键冲突 → BadRequest）。
pub async fn create_org_policy_label(
    pool: &PgPool,
    actor_id: i64,
    actor_email: &str,
    input: CreateOrgPolicyLabel,
) -> Result<OrgPolicyLabel, AliothError> {
    let row = sqlx::query_as::<_, OrgPolicyLabel>(sqlx::AssertSqlSafe(format!(
        "INSERT INTO isahl_auth.org_policy_label (code, rank, domain, notice, created_by_id, updated_by_id) \
         VALUES ($1, $2, $3, $4, $5, $5) \
         RETURNING {LABEL_COLUMNS}"
    )))
    .bind(&input.code)
    .bind(input.rank)
    .bind(input.domain)
    .bind(input.notice)
    .bind(actor_id)
    .fetch_one(pool)
    .await?;
    audit_event(
        pool,
        actor_id,
        actor_email,
        "policy.label.create",
        &format!("org_policy_label/{}", row.code),
    )
    .await;
    Ok(row)
}

fn apply_label_update(base: &OrgPolicyLabel, u: &UpdateOrgPolicyLabel) -> OrgPolicyLabel {
    let mut out = base.clone();
    if let Some(v) = u.rank {
        out.rank = v;
    }
    if let Some(v) = &u.domain {
        out.domain = v.clone();
    }
    if let Some(v) = &u.notice {
        out.notice = v.clone();
    }
    out
}

/// 更新 label 内容（软删行不可见；读改写单事务，FOR UPDATE 防并发覆盖）。
pub async fn update_org_policy_label(
    pool: &PgPool,
    actor_id: i64,
    actor_email: &str,
    code: &str,
    input: UpdateOrgPolicyLabel,
) -> Result<OrgPolicyLabel, AliothError> {
    let mut tx = pool.begin().await?;
    let base = sqlx::query_as::<_, OrgPolicyLabel>(sqlx::AssertSqlSafe(format!(
        "SELECT {LABEL_COLUMNS} FROM isahl_auth.org_policy_label \
         WHERE code = $1 AND deleted_at IS NULL FOR UPDATE"
    )))
    .bind(code)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AliothError::NotFound(format!("org_policy_label '{code}' 不存在")))?;
    let merged = apply_label_update(&base, &input);
    sqlx::query(
        "UPDATE isahl_auth.org_policy_label SET rank=$2, domain=$3, notice=$4, \
         updated_at=NOW(), updated_by_id=$5 WHERE code=$1 AND deleted_at IS NULL",
    )
    .bind(&merged.code)
    .bind(merged.rank)
    .bind(&merged.domain)
    .bind(&merged.notice)
    .bind(actor_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    audit_event(
        pool,
        actor_id,
        actor_email,
        "policy.label.update",
        &format!("org_policy_label/{}", merged.code),
    )
    .await;
    Ok(merged)
}

/// 软删 label（字典项下线；事务 + FOR UPDATE）。
pub async fn delete_org_policy_label(
    pool: &PgPool,
    actor_id: i64,
    actor_email: &str,
    code: &str,
) -> Result<OrgPolicyLabel, AliothError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query_as::<_, OrgPolicyLabel>(sqlx::AssertSqlSafe(format!(
        "UPDATE isahl_auth.org_policy_label SET deleted_at=NOW(), deleted_by_id=$2 \
         WHERE code=$1 AND deleted_at IS NULL RETURNING {LABEL_COLUMNS}"
    )))
    .bind(code)
    .bind(actor_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AliothError::NotFound(format!("org_policy_label '{code}' 不存在")))?;
    tx.commit().await?;
    audit_event(
        pool,
        actor_id,
        actor_email,
        "policy.label.delete",
        &format!("org_policy_label/{}", row.code),
    )
    .await;
    Ok(row)
}

/// label 字典列表（活动优先；分页）。
pub async fn list_org_policy_labels(
    pool: &PgPool,
    limit: i64,
    offset: i64,
) -> Result<Vec<OrgPolicyLabel>, AliothError> {
    let rows = sqlx::query_as::<_, OrgPolicyLabel>(sqlx::AssertSqlSafe(format!(
        "SELECT {LABEL_COLUMNS} FROM isahl_auth.org_policy_label \
         WHERE deleted_at IS NULL ORDER BY rank, code LIMIT $1 OFFSET $2"
    )))
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// D-2 派生器投影（spec §5）——薄转发 `common::ngac_policy`（投影唯一实现，禁止
/// 本地复制 SQL）。语义：仅 `state='active'` class 可投影；不存在/未激活/软删 →
/// NotFound（派生只认 active 规范；草稿预览语义随 D-2 派生契约收敛）。
pub async fn project_policy_class(
    pool: &PgPool,
    class_id: i64,
) -> Result<PolicyProjection, AliothError> {
    common::ngac_policy::project_policy_class(pool, class_id)
        .await
        .map_err(AliothError::from)
}

// ─────────────────────────── HTTP handlers（/api/admin/ngac/org-policy*） ───────────────────────────

fn err_resp(e: &AliothError) -> HttpResponse {
    let (code, status) = match e {
        AliothError::NotFound(_) => ("not_found", actix_web::http::StatusCode::NOT_FOUND),
        AliothError::BadRequest(_) => ("bad_request", actix_web::http::StatusCode::BAD_REQUEST),
        AliothError::Unauthorized(_) => ("unauthorized", actix_web::http::StatusCode::UNAUTHORIZED),
        AliothError::Forbidden(_) => ("forbidden", actix_web::http::StatusCode::FORBIDDEN),
        _ => ("internal_error", actix_web::http::StatusCode::INTERNAL_SERVER_ERROR),
    };
    HttpResponse::build(status).json(serde_json::json!({
        "code": code,
        "error": format!("{e}"),
    }))
}

/// require_admin 的 (actor_id, email) 变体（JWT claims 提供审计 email）。
async fn actor_claims(
    req: &HttpRequest,
    pool: &PgPool,
    auth: &crate::auth::AuthState,
) -> Result<(i64, String), HttpResponse> {
    let (id, claims) = crate::admin::handlers::require_admin_claims(req, pool, auth).await?;
    let email = if claims.email.is_empty() {
        "admin@aliothstudio.local"
    } else {
        &claims.email
    };
    Ok((id, email.to_string()))
}

#[derive(Debug, Deserialize)]
pub struct ClassQuery {
    pub state: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list_classes(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    query: web::Query<ClassQuery>,
) -> HttpResponse {
    if let Err(resp) = require_admin(&req, pool.get_ref(), state.get_ref()).await {
        return resp;
    }
    match list_org_policy_classes(
        pool.get_ref(),
        query.state.as_deref(),
        query.limit.unwrap_or(50).clamp(1, 200),
        query.offset.unwrap_or(0).max(0),
    )
    .await
    {
        Ok(rows) => HttpResponse::Ok().json(rows),
        Err(e) => err_resp(&e),
    }
}

pub async fn get_class(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
) -> HttpResponse {
    if let Err(resp) = require_admin(&req, pool.get_ref(), state.get_ref()).await {
        return resp;
    }
    match get_org_policy_class(pool.get_ref(), path.into_inner()).await {
        Ok(row) => HttpResponse::Ok().json(row),
        Err(e) => err_resp(&e),
    }
}

pub async fn create_class(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    auth: web::Data<crate::auth::AuthState>,
    body: web::Json<CreateOrgPolicyClass>,
) -> HttpResponse {
    let (actor_id, actor_email) = match actor_claims(&req, pool.get_ref(), auth.get_ref()).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match create_org_policy_class(pool.get_ref(), actor_id, &actor_email, body.into_inner()).await {
        Ok(row) => HttpResponse::Created().json(row),
        Err(e) => err_resp(&e),
    }
}

pub async fn update_class(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    auth: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
    body: web::Json<UpdateOrgPolicyClass>,
) -> HttpResponse {
    let (actor_id, actor_email) = match actor_claims(&req, pool.get_ref(), auth.get_ref()).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match update_org_policy_class(
        pool.get_ref(),
        actor_id,
        &actor_email,
        path.into_inner(),
        body.into_inner(),
    )
    .await
    {
        Ok(row) => HttpResponse::Ok().json(row),
        Err(e) => err_resp(&e),
    }
}

async fn class_transition_handler(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    auth: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
    kind: &'static str,
) -> HttpResponse {
    let (actor_id, actor_email) = match actor_claims(&req, pool.get_ref(), auth.get_ref()).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let class_id = path.into_inner();
    let result = match kind {
        "submit_review" => {
            submit_review_org_policy_class(pool.get_ref(), actor_id, &actor_email, class_id).await
        }
        "activate" => {
            activate_org_policy_class(pool.get_ref(), actor_id, &actor_email, class_id).await
        }
        _ => retire_org_policy_class(pool.get_ref(), actor_id, &actor_email, class_id).await,
    };
    match result {
        Ok(row) => HttpResponse::Ok().json(row),
        Err(e) => err_resp(&e),
    }
}

pub async fn submit_review_class(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    auth: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
) -> HttpResponse {
    class_transition_handler(req, pool, auth, path, "submit_review").await
}

pub async fn activate_class(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    auth: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
) -> HttpResponse {
    class_transition_handler(req, pool, auth, path, "activate").await
}

pub async fn retire_class(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    auth: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
) -> HttpResponse {
    class_transition_handler(req, pool, auth, path, "retire").await
}

pub async fn class_projection(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
) -> HttpResponse {
    if let Err(resp) = require_admin(&req, pool.get_ref(), state.get_ref()).await {
        return resp;
    }
    match project_policy_class(pool.get_ref(), path.into_inner()).await {
        Ok(p) => HttpResponse::Ok().json(p),
        Err(e) => err_resp(&e),
    }
}

pub async fn list_rules(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
) -> HttpResponse {
    if let Err(resp) = require_admin(&req, pool.get_ref(), state.get_ref()).await {
        return resp;
    }
    match list_org_policy_rules(pool.get_ref(), path.into_inner()).await {
        Ok(rows) => HttpResponse::Ok().json(rows),
        Err(e) => err_resp(&e),
    }
}

pub async fn create_rule(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    auth: web::Data<crate::auth::AuthState>,
    body: web::Json<CreateOrgPolicyRule>,
) -> HttpResponse {
    let (actor_id, actor_email) = match actor_claims(&req, pool.get_ref(), auth.get_ref()).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match create_org_policy_rule(pool.get_ref(), actor_id, &actor_email, body.into_inner()).await {
        Ok(row) => HttpResponse::Created().json(row),
        Err(e) => err_resp(&e),
    }
}

pub async fn update_rule(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    auth: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
    body: web::Json<UpdateOrgPolicyRule>,
) -> HttpResponse {
    let (actor_id, actor_email) = match actor_claims(&req, pool.get_ref(), auth.get_ref()).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match update_org_policy_rule(
        pool.get_ref(),
        actor_id,
        &actor_email,
        path.into_inner(),
        body.into_inner(),
    )
    .await
    {
        Ok(row) => HttpResponse::Ok().json(row),
        Err(e) => err_resp(&e),
    }
}

pub async fn delete_rule(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    auth: web::Data<crate::auth::AuthState>,
    path: web::Path<i64>,
) -> HttpResponse {
    let (actor_id, actor_email) = match actor_claims(&req, pool.get_ref(), auth.get_ref()).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match delete_org_policy_rule(pool.get_ref(), actor_id, &actor_email, path.into_inner()).await {
        Ok(row) => HttpResponse::Ok().json(row),
        Err(e) => err_resp(&e),
    }
}

pub async fn list_labels(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    query: web::Query<PageQuery>,
) -> HttpResponse {
    if let Err(resp) = require_admin(&req, pool.get_ref(), state.get_ref()).await {
        return resp;
    }
    match list_org_policy_labels(
        pool.get_ref(),
        query.limit.unwrap_or(50).clamp(1, 200),
        query.offset.unwrap_or(0).max(0),
    )
    .await
    {
        Ok(rows) => HttpResponse::Ok().json(rows),
        Err(e) => err_resp(&e),
    }
}

pub async fn create_label(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    auth: web::Data<crate::auth::AuthState>,
    body: web::Json<CreateOrgPolicyLabel>,
) -> HttpResponse {
    let (actor_id, actor_email) = match actor_claims(&req, pool.get_ref(), auth.get_ref()).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match create_org_policy_label(pool.get_ref(), actor_id, &actor_email, body.into_inner()).await {
        Ok(row) => HttpResponse::Created().json(row),
        Err(e) => err_resp(&e),
    }
}

pub async fn update_label(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    auth: web::Data<crate::auth::AuthState>,
    path: web::Path<String>,
    body: web::Json<UpdateOrgPolicyLabel>,
) -> HttpResponse {
    let (actor_id, actor_email) = match actor_claims(&req, pool.get_ref(), auth.get_ref()).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match update_org_policy_label(
        pool.get_ref(),
        actor_id,
        &actor_email,
        &path.into_inner(),
        body.into_inner(),
    )
    .await
    {
        Ok(row) => HttpResponse::Ok().json(row),
        Err(e) => err_resp(&e),
    }
}

pub async fn delete_label(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    auth: web::Data<crate::auth::AuthState>,
    path: web::Path<String>,
) -> HttpResponse {
    let (actor_id, actor_email) = match actor_claims(&req, pool.get_ref(), auth.get_ref()).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match delete_org_policy_label(pool.get_ref(), actor_id, &actor_email, &path.into_inner()).await {
        Ok(row) => HttpResponse::Ok().json(row),
        Err(e) => err_resp(&e),
    }
}

async fn require_admin(
    req: &HttpRequest,
    pool: &PgPool,
    auth: &crate::auth::AuthState,
) -> Result<i64, HttpResponse> {
    crate::admin::handlers::require_admin(req, pool, auth).await
}

/// 注册管理面路由（挂载于 `/api/admin` 作用域，见 admin/mod.rs；前缀 `/ngac/org-policy`）。
pub fn configure_admin_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/ngac/org-policy/classes", web::get().to(list_classes))
        .route("/ngac/org-policy/classes", web::post().to(create_class))
        .route("/ngac/org-policy/classes/{id}", web::get().to(get_class))
        .route(
            "/ngac/org-policy/classes/{id}",
            web::patch().to(update_class),
        )
        .route(
            "/ngac/org-policy/classes/{id}/submit-review",
            web::post().to(submit_review_class),
        )
        .route(
            "/ngac/org-policy/classes/{id}/activate",
            web::post().to(activate_class),
        )
        .route(
            "/ngac/org-policy/classes/{id}/retire",
            web::post().to(retire_class),
        )
        .route(
            "/ngac/org-policy/classes/{id}/projection",
            web::get().to(class_projection),
        )
        .route(
            "/ngac/org-policy/classes/{id}/rules",
            web::get().to(list_rules),
        )
        .route("/ngac/org-policy/rules", web::post().to(create_rule))
        .route("/ngac/org-policy/rules/{id}", web::patch().to(update_rule))
        .route("/ngac/org-policy/rules/{id}", web::delete().to(delete_rule))
        .route("/ngac/org-policy/labels", web::get().to(list_labels))
        .route("/ngac/org-policy/labels", web::post().to(create_label))
        .route(
            "/ngac/org-policy/labels/{code}",
            web::patch().to(update_label),
        )
        .route(
            "/ngac/org-policy/labels/{code}",
            web::delete().to(delete_label),
        );
}
