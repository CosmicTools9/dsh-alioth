//! 主体关联桥 Handler（strengthen-identity-org）
//!
//! 覆盖 `zc_id_subjects_rr_place` / `_rr_storage` / `_rr_container` 三桥
//! （ref_left=主体 id, ref_right=目标实体 id），统一语义：
//! - `GET    /subjects/{id}/places|storages|containers`           — 桥列表
//! - `POST   /subjects/{id}/places|storages|containers`           — 添加关联（幂等）
//! - `DELETE /subjects/{id}/places|storages|containers/{relId}`   — 软删关联
//!
//! 模式复制 subjects.rs accounts 桥（同 lifecycle_rr_non_self 桥族）。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::Deserialize;
use sqlx::{AssertSqlSafe, PgPool};

use crate::handlers::subjects::ensure_subject_exists;

#[derive(Debug, Deserialize)]
pub struct AddSubjectRefRequest {
    /// 目标实体 id（ref_right——place/storage/container 继承链行）
    #[serde(with = "common::serde_zuid")]
    pub target_id: i64,
    /// 期间标量引用（qk_period，可空）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub period_id: Option<i64>,
}

/// 桥配置（白名单字面量，无注入面）
struct BridgeSpec {
    table: &'static str,
    label: &'static str,
}

fn bridge_spec(kind: &str) -> Option<BridgeSpec> {
    match kind {
        "places" => Some(BridgeSpec {
            table: "zc_id_subjects_rr_place",
            label: "场所",
        }),
        "storages" => Some(BridgeSpec {
            table: "zc_id_subjects_rr_storage",
            label: "仓储",
        }),
        "containers" => Some(BridgeSpec {
            table: "zc_id_subjects_rr_container",
            label: "容器",
        }),
        _ => None,
    }
}

/// 桥表行（id/ref_right/qk_period/comments/target_notice/target_code）
type BridgeRow = (
    i64,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
);

async fn list_bridge(
    pool: &PgPool,
    subject_id: i64,
    spec: &BridgeSpec,
) -> Result<Vec<serde_json::Value>, ApiError> {
    // 表名为白名单字面量（bridge_spec），AssertSqlSafe 仅放行这三张桥表
    let sql = format!(
        "SELECT b.id, b.ref_right, b.qk_period, b.comments, t.notice AS target_notice, t.code AS target_code \
         FROM \"isahl\".\"{}\" b \
         LEFT JOIN \"isahl\".\"zc_id_lifecycle\" t ON t.id = b.ref_right AND t.deleted_at IS NULL \
         WHERE b.ref_left = $1 AND b.deleted_at IS NULL ORDER BY b.id",
        spec.table
    );
    let rows: Vec<BridgeRow> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
        .bind(subject_id)
        .fetch_all(pool)
        .await
        .map_err(ApiError::from_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|(id, ref_right, qk_period, comments, tnotice, tcode)| {
            serde_json::json!({
                "id": id.to_string(),
                "target_id": ref_right.to_string(),
                "period_id": qk_period.map(|v| v.to_string()),
                "comments": comments,
                "target_notice": tnotice,
                "target_code": tcode,
            })
        })
        .collect())
}

async fn add_bridge(
    pool: &PgPool,
    user_id: i64,
    subject_id: i64,
    body: &AddSubjectRefRequest,
    spec: &BridgeSpec,
) -> Result<HttpResponse, ApiError> {
    ensure_subject_exists(pool, subject_id).await?;
    // 目标实体存在性（lifecycle 继承链统一可见）
    let target_exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM \"isahl\".\"zc_id_lifecycle\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(body.target_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !target_exists {
        return Err(ApiError::NotFound(format!(
            "{}目标实体不存在: {}",
            spec.label, body.target_id
        )));
    }

    // 幂等：已存在 → 返回现有关联
    let check_sql = format!(
        "SELECT id FROM \"isahl\".\"{}\" WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL LIMIT 1",
        spec.table
    );
    let existing: Option<i64> = sqlx::query_scalar(AssertSqlSafe(check_sql.as_str()))
        .bind(subject_id)
        .bind(body.target_id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::from_sqlx)?;
    if let Some(rel_id) = existing {
        return Ok(
            HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                "id": rel_id.to_string(),
            }))),
        );
    }

    let insert_sql = format!(
        "INSERT INTO \"isahl\".\"{}\" (notice, ref_left, ref_right, qk_period, created_by_id, updated_by_id) \
         VALUES ($1, $2, $3, $4, $5, $5) RETURNING id",
        spec.table
    );
    let rel_id: i64 = sqlx::query_scalar(AssertSqlSafe(insert_sql.as_str()))
        .bind(format!("subject-{} {}", subject_id, spec.label))
        .bind(subject_id)
        .bind(body.target_id)
        .bind(body.period_id)
        .bind(user_id)
        .fetch_one(pool)
        .await
        .map_err(ApiError::from_sqlx)?;

    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": rel_id.to_string(),
        }))),
    )
}

async fn delete_bridge(
    pool: &PgPool,
    user_id: i64,
    subject_id: i64,
    rel_id: i64,
    spec: &BridgeSpec,
) -> Result<HttpResponse, ApiError> {
    let sql = format!(
        "UPDATE \"isahl\".\"{}\" SET deleted_at = NOW(), deleted_by_id = $3 \
         WHERE id = $1 AND ref_left = $2 AND deleted_at IS NULL",
        spec.table
    );
    let rows = sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(rel_id)
        .bind(subject_id)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(ApiError::from_sqlx)?;
    if rows.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!(
            "{}关联不存在: rel={} subject={}",
            spec.label, rel_id, subject_id
        )));
    }
    Ok(HttpResponse::NoContent().finish())
}

macro_rules! bridge_handlers {
    ($list_fn:ident, $add_fn:ident, $del_fn:ident, $kind:literal) => {
        pub async fn $list_fn(
            req: HttpRequest,
            pool: web::Data<PgPool>,
            path: web::Path<i64>,
        ) -> Result<HttpResponse, ApiError> {
            let user_id = require_auth(&req)?;
            let subject_id = path.into_inner();
            require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "read")
                .await?;
            let spec = bridge_spec($kind).expect("bridge kind whitelisted");
            let items = list_bridge(pool.get_ref(), subject_id, &spec).await?;
            Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
        }

        pub async fn $add_fn(
            req: HttpRequest,
            pool: web::Data<PgPool>,
            path: web::Path<i64>,
            body: web::Json<AddSubjectRefRequest>,
        ) -> Result<HttpResponse, ApiError> {
            let user_id = require_auth(&req)?;
            let subject_id = path.into_inner();
            require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "update")
                .await?;
            let spec = bridge_spec($kind).expect("bridge kind whitelisted");
            add_bridge(pool.get_ref(), user_id, subject_id, &body, &spec).await
        }

        pub async fn $del_fn(
            req: HttpRequest,
            pool: web::Data<PgPool>,
            path: web::Path<(i64, i64)>,
        ) -> Result<HttpResponse, ApiError> {
            let user_id = require_auth(&req)?;
            let (subject_id, rel_id) = path.into_inner();
            require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "update")
                .await?;
            let spec = bridge_spec($kind).expect("bridge kind whitelisted");
            delete_bridge(pool.get_ref(), user_id, subject_id, rel_id, &spec).await
        }
    };
}

bridge_handlers!(list_places, add_place, delete_place, "places");
bridge_handlers!(list_storages, add_storage, delete_storage, "storages");
bridge_handlers!(
    list_containers,
    add_container,
    delete_container,
    "containers"
);

/// 注册主体关联桥路由（strengthen-identity-org）
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/subjects/{id}/places")
            .route(web::get().to(list_places))
            .route(web::post().to(add_place)),
    )
    .service(web::resource("/subjects/{id}/places/{relId}").route(web::delete().to(delete_place)))
    .service(
        web::resource("/subjects/{id}/storages")
            .route(web::get().to(list_storages))
            .route(web::post().to(add_storage)),
    )
    .service(
        web::resource("/subjects/{id}/storages/{relId}").route(web::delete().to(delete_storage)),
    )
    .service(
        web::resource("/subjects/{id}/containers")
            .route(web::get().to(list_containers))
            .route(web::post().to(add_container)),
    )
    .service(
        web::resource("/subjects/{id}/containers/{relId}")
            .route(web::delete().to(delete_container)),
    );
}
