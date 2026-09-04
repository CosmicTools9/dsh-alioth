//! SCIM Group CRUD 端点（`/scim/v2/Groups`）。
//! 组映射到 `isahl_auth.ngac_user_attribute` / `ngac_user_rr_attribute`。

use actix_web::{web, HttpRequest, HttpResponse};
use sqlx::{PgPool, Row};

use super::{default_policy_class_id, require_scim_token};
use crate::scim::models::*;

// ── Group CRUD（映射到 ngac_user_attribute / ngac_user_rr_attribute） ──────────

/// GET /scim/v2/Groups —— list。
pub async fn list_groups(req: HttpRequest, pool: web::Data<PgPool>) -> HttpResponse {
    if let Err(resp) = require_scim_token(&req) {
        return resp;
    }
    let rows = sqlx::query("SELECT id, o_name FROM isahl_auth.ngac_user_attribute WHERE deleted_at IS NULL ORDER BY id")
        .fetch_all(pool.get_ref())
        .await;
    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            log::error!("scim list_groups error: {}", e);
            return error_response(
                actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Database error",
            );
        }
    };
    let resources: Vec<ScimGroup> = rows
        .iter()
        .map(|r| ScimGroup {
            schemas: Some(vec![
                "urn:ietf:params:scim:schemas:core:2.0:Group".to_string()
            ]),
            id: Some(r.get::<i64, _>("id").to_string()),
            display_name: r.get::<Option<String>, _>("o_name"),
            members: None,
            meta: Some(ScimMeta {
                resource_type: Some("Group".to_string()),
                location: Some(format!("/scim/v2/Groups/{}", r.get::<i64, _>("id"))),
                ..Default::default()
            }),
        })
        .collect();

    HttpResponse::Ok().json(ListResponse {
        schemas: vec!["urn:ietf:params:scim:api:messages:2.0:ListResponse".to_string()],
        totalResults: resources.len(),
        startIndex: 1,
        itemsPerPage: resources.len(),
        Resources: resources,
    })
}

pub async fn create_group(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<ScimGroup>,
) -> HttpResponse {
    if let Err(resp) = require_scim_token(&req) {
        return resp;
    }
    let display_name = match body.display_name.as_ref().filter(|s| !s.is_empty()) {
        Some(n) => n.clone(),
        None => {
            return error_response(
                actix_web::http::StatusCode::BAD_REQUEST,
                "displayName is required",
            )
        }
    };

    // 解析默认 policy class（o_name='default'，id 为动态 zuid，不可硬编码）
    let pc_id = match default_policy_class_id(pool.get_ref()).await {
        Ok(id) => id,
        Err(e) => {
            log::error!("scim create_group policy class error: {}", e);
            return error_response(
                actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                "NGAC policy class not configured",
            );
        }
    };

    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => {
            log::error!("scim create_group tx error: {}", e);
            return error_response(
                actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Database error",
            );
        }
    };

    let (attr_id,): (i64,) = match sqlx::query_as(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at) \
         VALUES ($1, $2, NOW(), NOW()) RETURNING id",
    )
    .bind(&display_name)
    .bind(pc_id)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            log::error!("scim create_group insert error: {}", e);
            return error_response(
                actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to create group",
            );
        }
    };

    for m in body.members.iter().flatten() {
        if let Some(uid) = m.value.as_ref().and_then(|v| v.parse::<i64>().ok()) {
            let _ = sqlx::query(
                "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at) \
                 VALUES ($1, $2, NOW(), NOW()) \
                 ON CONFLICT (fk_user, fk_user_attribute) DO NOTHING",
            )
            .bind(uid)
            .bind(attr_id)
            .execute(&mut *tx)
            .await;
        }
    }

    if let Err(e) = tx.commit().await {
        log::error!("scim create_group commit error: {}", e);
        return error_response(
            actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to commit group",
        );
    }

    match fetch_group(pool.get_ref(), attr_id).await {
        Some(g) => HttpResponse::Created().json(g),
        None => error_response(
            actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to read created group",
        ),
    }
}

pub async fn get_group(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<String>,
) -> HttpResponse {
    if let Err(resp) = require_scim_token(&req) {
        return resp;
    }
    let id: i64 = match path.into_inner().parse() {
        Ok(i) => i,
        Err(_) => return error_response(actix_web::http::StatusCode::BAD_REQUEST, "Invalid id"),
    };
    get_group_inner(pool.get_ref(), id).await
}

pub async fn replace_group(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<String>,
    body: web::Json<ScimGroup>,
) -> HttpResponse {
    if let Err(resp) = require_scim_token(&req) {
        return resp;
    }
    let id: i64 = match path.into_inner().parse() {
        Ok(i) => i,
        Err(_) => return error_response(actix_web::http::StatusCode::BAD_REQUEST, "Invalid id"),
    };

    let exists: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);
    if exists.is_none() {
        return error_response(actix_web::http::StatusCode::NOT_FOUND, "Group not found");
    }

    let display_name = body.display_name.clone().unwrap_or_default();

    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(_) => {
            return error_response(
                actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Database error",
            )
        }
    };

    let _ = sqlx::query(
        "UPDATE isahl_auth.ngac_user_attribute SET o_name = $2, updated_at = NOW() WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(&display_name)
    .execute(&mut *tx)
    .await;

    // 全量替换 members：先删除现有，再插入请求中的。
    let _ = sqlx::query(
        "UPDATE isahl_auth.ngac_user_rr_attribute SET deleted_at = NOW() WHERE fk_user_attribute = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await;

    for m in body.members.iter().flatten() {
        if let Some(uid) = m.value.as_ref().and_then(|v| v.parse::<i64>().ok()) {
            let _ = sqlx::query(
                "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at) \
                 VALUES ($1, $2, NOW(), NOW()) ON CONFLICT (fk_user, fk_user_attribute) DO NOTHING",
            )
            .bind(uid)
            .bind(id)
            .execute(&mut *tx)
            .await;
        }
    }

    if let Err(e) = tx.commit().await {
        log::error!("scim replace_group commit error: {}", e);
        return error_response(
            actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to commit group",
        );
    }

    match fetch_group(pool.get_ref(), id).await {
        Some(g) => HttpResponse::Created().json(g),
        None => error_response(actix_web::http::StatusCode::NOT_FOUND, "Group not found"),
    }
}

pub async fn delete_group(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<String>,
) -> HttpResponse {
    if let Err(resp) = require_scim_token(&req) {
        return resp;
    }
    let id: i64 = match path.into_inner().parse() {
        Ok(i) => i,
        Err(_) => return error_response(actix_web::http::StatusCode::BAD_REQUEST, "Invalid id"),
    };
    let result = sqlx::query(
        "UPDATE isahl_auth.ngac_user_attribute SET deleted_at = NOW(), updated_at = NOW() \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .execute(pool.get_ref())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => HttpResponse::NoContent().finish(),
        Ok(_) => error_response(actix_web::http::StatusCode::NOT_FOUND, "Group not found"),
        Err(e) => {
            log::error!("scim delete_group error: {}", e);
            error_response(
                actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Database error",
            )
        }
    }
}

async fn fetch_group(pool: &PgPool, id: i64) -> Option<ScimGroup> {
    let row = sqlx::query("SELECT id, o_name FROM isahl_auth.ngac_user_attribute WHERE id = $1 AND deleted_at IS NULL")
        .bind(id)
        .fetch_optional(pool)
        .await
        .unwrap_or(None)?;

    let attr_id: i64 = row.get("id");
    let o_name: Option<String> = row.get("o_name");

    let member_rows = sqlx::query(
        "SELECT fk_user FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user_attribute = $1 AND deleted_at IS NULL",
    )
    .bind(attr_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let members: Vec<ScimMemberRef> = member_rows
        .iter()
        .map(|r| {
            let uid: i64 = r.get("fk_user");
            ScimMemberRef {
                member_type: Some("User".to_string()),
                ref_: Some("/scim/v2/Users/".to_string()),
                value: Some(uid.to_string()),
                display: None,
            }
        })
        .collect();

    Some(ScimGroup {
        schemas: Some(vec![
            "urn:ietf:params:scim:schemas:core:2.0:Group".to_string()
        ]),
        id: Some(attr_id.to_string()),
        display_name: o_name,
        members: if members.is_empty() {
            None
        } else {
            Some(members)
        },
        meta: Some(ScimMeta {
            resource_type: Some("Group".to_string()),
            location: Some(format!("/scim/v2/Groups/{}", attr_id)),
            ..Default::default()
        }),
    })
}

async fn get_group_inner(pool: &PgPool, id: i64) -> HttpResponse {
    match fetch_group(pool, id).await {
        Some(g) => HttpResponse::Ok().json(g),
        None => error_response(actix_web::http::StatusCode::NOT_FOUND, "Group not found"),
    }
}
