//! 业务活动流读原语（per-entity）—— 统一实现，供各 Service 挂载。
//!
//! 数据源 = `isahl_audit.data_change_logs`（ADR D-027 应用层 outbox 转写）。
//! 行级授权：`record_id ∈ X-Visible-Ids`（NGAC_SPEC §6.3 三段绑定顺序：
//! filters → visible_ids → 分页）。响应仅含 `changed_fields`，前后镜像不返回
//! （SECURITY_SPEC §10.3）。
//!
//! 实体消歧：同一物理表可承载多个逻辑实体（如 `zc_id_even-accident` 同时承载
//! NCR 与 RiskItem）。作用域取 `visible_ids` 时天然按行隔离；admin（`None`）时按
//! 实体 `COORDINATE_FILTER` 限缩，避免跨实体/跨 namespace 串档。
//!
//! 端点约定：`GET /{entity}/activity`（实体段 MUST 已在 NGAC ResourceRegistry
//! 注册，否则 PEP 不注入 `X-Visible-Ids`，RLS 失效）；`/{entity}/activity` MUST
//! 先于 CRUD scope 注册（NGAC_SPEC §7.2，actix 无回落语义）。

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use common::AliothError;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::{AssertSqlSafe, PgPool};
use std::collections::HashMap;

use crate::cascade::bare_table_name;
use crate::entity::AliothDbEntity;
use crate::handler::parse_visible_ids;

/// 主状态桥表（`audit_primary_status_tx` 写入；`record_id` = 实体 id、
/// `old/new_values = {ref_right}`）。读取时按 `ref_right` 解析目标状态。
const PRIMARY_STATUS_TABLE: &str = "zc_id_lifecycle_r_primary-status";
/// 分页上限（SERVICE_SPEC §13.1 / API_DESIGN_SPEC §2）。
const MAX_PAGE_SIZE: i64 = 100;
const DEFAULT_PAGE_SIZE: i64 = 20;
/// admin 全量作用域的有界上限（避免无界 id 集）。
const SCOPE_LIMIT: i64 = 5000;

/// 活动流分页查询参数。
#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub page_size: Option<i64>,
}

/// 归一化活动项。
///
/// 字段按 L2 DTO 惯例 camelCase 输出（与模块前端 `ActivityItem` 契约一致）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    /// 目标实体 id（id 语义字段 → 串化输出，ID_JSON_PRECISION）。
    #[serde(with = "common::serde_zuid")]
    pub record_id: i64,
    /// INSERT / UPDATE / DELETE（存储事实；状态变更行为 SELECT 于桥表）。
    pub action: String,
    pub occurred_at: DateTime<Utc>,
    /// 操作者标识（username，SECURITY_SPEC §11.2 口径）。
    pub actor: Option<String>,
    /// 目标标签（实体 `notice`）。
    pub target_label: Option<String>,
    /// 目标编码（实体 `code`）。
    pub target_code: Option<String>,
    /// 是否为主状态迁移行。
    pub status_change: bool,
    /// 状态变更后的目标状态标签（仅 `status_change` 有值）。
    pub status_label: Option<String>,
    /// 变更字段名集合（不含前后镜像值）。
    pub changed_fields: Option<JsonValue>,
}

/// 分页响应体（`list`/`total`/`page`/`page_size`）。
#[derive(Debug, Serialize)]
pub struct ActivityPage {
    pub list: Vec<ActivityItem>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// 数据库行（内部）。
#[derive(Debug, sqlx::FromRow)]
struct AuditRow {
    record_id: i64,
    table_name: String,
    action: String,
    action_timestamp: DateTime<Utc>,
    performed_by_email: Option<String>,
    changed_fields: Option<JsonValue>,
    old_values: Option<JsonValue>,
    new_values: Option<JsonValue>,
    total: i64,
}

fn empty_page(page: i64, page_size: i64) -> ActivityPage {
    ActivityPage {
        list: Vec::new(),
        total: 0,
        page,
        page_size,
    }
}

/// 实体作用域 id 集。
///
/// - `visible_ids = Some(ids)`：RLS 集即作用域（`Some([])` → 空，零行）。
/// - `None`（admin）：按实体 `COORDINATE_FILTER` 取有界全量 id，保证实体语义
///   （同表多逻辑实体不串档）。
async fn entity_scope_ids<E: AliothDbEntity>(
    pool: &PgPool,
    visible_ids: Option<&[i64]>,
) -> Result<Vec<i64>, AliothError> {
    if let Some(ids) = visible_ids {
        return Ok(ids.to_vec());
    }
    let mut sql = format!("SELECT id FROM {} WHERE ", E::table_name());
    sql.push_str(if E::SOFT_DELETE {
        "deleted_at IS NULL"
    } else {
        "TRUE"
    });
    if !E::COORDINATE_FILTER.is_empty() {
        sql.push_str(" AND ");
        sql.push_str(E::COORDINATE_FILTER);
    }
    sql.push_str(&format!(" LIMIT {}", SCOPE_LIMIT));
    sqlx::query_scalar(AssertSqlSafe(sql))
        .fetch_all(pool)
        .await
        .map_err(|e| AliothError::Database(e.to_string()))
}

/// 实体 id → (notice, code) 标签表。
async fn entity_labels<E: AliothDbEntity>(
    pool: &PgPool,
    ids: &[i64],
) -> Result<HashMap<i64, (Option<String>, Option<String>)>, AliothError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let sql = format!(
        "SELECT id, notice, code FROM {} WHERE id = ANY($1)",
        E::table_name()
    );
    let rows: Vec<(i64, Option<String>, Option<String>)> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(ids)
        .fetch_all(pool)
        .await
        .map_err(|e| AliothError::Database(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|(id, notice, code)| (id, (notice, code)))
        .collect())
}

/// 状态 id → notice 标签表。
async fn status_labels(pool: &PgPool, ids: &[i64]) -> Result<HashMap<i64, String>, AliothError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(i64, Option<String>)> =
        sqlx::query_as(r#"SELECT id, notice FROM isahl.zc_id_status WHERE id = ANY($1)"#)
            .bind(ids)
            .fetch_all(pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, notice)| notice.map(|n| (id, n)))
        .collect())
}

/// 从审计行取状态引用 id（`new_values.ref_right` 优先，回落 `old_values.ref_right`）。
fn status_ref_id(row: &AuditRow) -> Option<i64> {
    let pick = |v: &Option<JsonValue>| -> Option<i64> {
        v.as_ref()
            .and_then(|j| j.get("ref_right"))
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<i64>().ok())
    };
    pick(&row.new_values).or_else(|| pick(&row.old_values))
}

/// `changed_fields` 仅保留字段名集合，满足 SECURITY_SPEC §10.3 默认脱敏。
///
/// 两种存储形态兼容：① `enqueue` 统一派生写入的**字段名数组**（现形态，原样透传）；
/// ② 历史/外部写入的**对象快照**（取键）。
fn changed_field_names(v: &Option<JsonValue>) -> Option<JsonValue> {
    match v.as_ref()? {
        JsonValue::Array(_) => v.clone(),
        JsonValue::Object(obj) => Some(JsonValue::Array(
            obj.keys().map(|k| JsonValue::String(k.clone())).collect(),
        )),
        _ => None,
    }
}

/// 通用 per-entity 活动流 handler。
///
/// 挂载：`cfg.route("/{entity}/activity", web::get().to(crud_activity::<E>))`
/// —— MUST 先于该实体的 CRUD scope 注册。
pub async fn crud_activity<E: AliothDbEntity>(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    query: web::Query<ActivityQuery>,
) -> Result<HttpResponse, AliothError> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query
        .page_size
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);

    let visible_ids = parse_visible_ids(&req);
    if let Some(ids) = visible_ids.as_deref() {
        if ids.is_empty() {
            return Ok(
                HttpResponse::Ok().json(common::data::ApiResponse::success(empty_page(
                    page, page_size,
                ))),
            );
        }
    }

    let scope = entity_scope_ids::<E>(pool.get_ref(), visible_ids.as_deref()).await?;
    if scope.is_empty() {
        return Ok(
            HttpResponse::Ok().json(common::data::ApiResponse::success(empty_page(
                page, page_size,
            ))),
        );
    }

    let entity_table = bare_table_name(E::table_name());
    let tables = vec![entity_table, PRIMARY_STATUS_TABLE];
    let offset = (page - 1) * page_size;

    let rows: Vec<AuditRow> = sqlx::query_as(
        r#"SELECT record_id, table_name, action, action_timestamp,
                  performed_by_email, changed_fields, old_values, new_values,
                  COUNT(*) OVER() AS total
           FROM isahl_audit.data_change_logs
           WHERE record_id = ANY($1) AND table_name = ANY($2)
           ORDER BY action_timestamp DESC, id DESC
           LIMIT $3 OFFSET $4"#,
    )
    .bind(&scope)
    .bind(&tables)
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool.get_ref())
    .await
    .map_err(|e| AliothError::Database(e.to_string()))?;

    let total = rows.first().map(|r| r.total).unwrap_or(0);

    // 标签解析：实体标签 + 状态标签各一次批量查询。
    let labels = entity_labels::<E>(pool.get_ref(), &scope).await?;
    let status_ids: Vec<i64> = rows
        .iter()
        .filter(|r| r.table_name == PRIMARY_STATUS_TABLE)
        .filter_map(status_ref_id)
        .collect();
    let statuses = status_labels(pool.get_ref(), &status_ids).await?;

    let list = rows
        .into_iter()
        .map(|r| {
            let is_status = r.table_name == PRIMARY_STATUS_TABLE;
            let (label, code) = labels.get(&r.record_id).cloned().unwrap_or((None, None));
            let status_label = if is_status {
                status_ref_id(&r).and_then(|sid| statuses.get(&sid).cloned())
            } else {
                None
            };
            ActivityItem {
                record_id: r.record_id,
                action: r.action,
                occurred_at: r.action_timestamp,
                actor: r.performed_by_email,
                target_label: label,
                target_code: code,
                status_change: is_status,
                status_label,
                changed_fields: changed_field_names(&r.changed_fields),
            }
        })
        .collect();

    Ok(
        HttpResponse::Ok().json(common::data::ApiResponse::success(ActivityPage {
            list,
            total,
            page,
            page_size,
        })),
    )
}
