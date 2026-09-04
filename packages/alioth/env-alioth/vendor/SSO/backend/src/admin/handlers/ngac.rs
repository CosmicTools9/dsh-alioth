//! Admin NGAC 配置 handlers
//!
//! NGAC 用户属性（UA）/ 对象属性（OA）、association（访问策略）与 prohibition（禁止规则）
//! 的管理面端点。写路径统一经 `ngac::integrity::with_validated_write` 做完整性校验。

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};

use crate::ngac::pip::PostgresPip;

/// User attribute response
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct UserAttributeResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    pub description: Option<String>,
    /// 父属性 id（规范边）。children 由前端从 ancestor_ids 映射派生。
    #[serde(with = "common::serde_zuid::seq")]
    pub ancestor_ids: Vec<i64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Object attribute response
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ObjectAttributeResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    pub resource_type: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_resource: Option<i64>,
    /// 业务可读标识（notice → code → 回退编号；NGAC_SPEC §2.2，add-ngac-oa-readable-identifier）。
    pub resource_identifier: Option<String>,
    /// 展示名（NGAC_SPEC §2.2 解析链：实例级 identifier → meta_collections.name
    /// → 内置映射 → o_name；add-ngac-oa-display-name）。
    #[sqlx(skip)]
    pub display_name: String,
    /// 模块归属（add-ngac-oa-module-observability）：主要使用模块中文名；系统域为 null。
    #[sqlx(skip)]
    pub module_name: Option<String>,
    /// 模块归属：Gateway 模块路由前缀（如 /outgo-wz），前端跳转用；系统域为 null。
    #[sqlx(skip)]
    pub module_route: Option<String>,
    /// 模块归属：namespace（如 WZ）；系统域 null。
    #[sqlx(skip)]
    pub namespace: Option<String>,
    /// 页面预览（add-ngac-oa-preview，dev-only）：采集器截图 + 高亮 rect；未采集/未启用 null。
    #[sqlx(skip)]
    pub preview: Option<crate::ngac::display::OaPreviewInfo>,
    /// 父属性 id（规范边）
    #[serde(with = "common::serde_zuid::seq")]
    pub ancestor_ids: Vec<i64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Create user attribute request
#[derive(Debug, Deserialize)]
pub struct CreateUserAttributeRequest {
    pub o_name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    pub description: Option<String>,
    /// 父属性 id（属性层级，继承其授权）。规范边为 ancestor_ids，children 由服务端派生。
    #[serde(default)]
    #[serde(with = "common::serde_zuid::seq")]
    pub ancestor_ids: Vec<i64>,
}

/// Update user attribute request
#[derive(Debug, Deserialize)]
pub struct UpdateUserAttributeRequest {
    pub o_name: Option<String>,
    pub description: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt_seq")]
    pub ancestor_ids: Option<Vec<i64>>,
}

/// Create object attribute request
#[derive(Debug, Deserialize)]
pub struct CreateObjectAttributeRequest {
    pub o_name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    pub resource_type: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_resource: Option<i64>,
    /// 业务可读标识（NGAC_SPEC §2.2，add-ngac-oa-readable-identifier）；缺省 null。
    pub resource_identifier: Option<String>,
    /// 父属性 id（属性层级）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::seq")]
    pub ancestor_ids: Vec<i64>,
}

/// Update object attribute request
#[derive(Debug, Deserialize)]
pub struct UpdateObjectAttributeRequest {
    pub o_name: Option<String>,
    pub resource_type: Option<String>,
    /// 业务可读标识（NGAC_SPEC §2.2，add-ngac-oa-readable-identifier）；None 不更新。
    pub resource_identifier: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt_seq")]
    pub ancestor_ids: Option<Vec<i64>>,
}

/// Bind user attribute request
#[derive(Debug, Deserialize)]
pub struct BindUserAttributeRequest {
    #[serde(with = "common::serde_zuid")]
    pub fk_user_attribute: i64,
}

use super::{require_admin, require_admin_claims};
use crate::auth::AuthState;

// ============================================================================
// User Attributes (NGAC roles)
// ============================================================================

/// GET /api/admin/users/{id}/attributes
/// List NGAC user attributes assigned to a user.
pub async fn list_user_attributes(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let user_id_param = path.into_inner();

    let attrs = sqlx::query_as::<_, UserAttributeResponse>(
        r#"
        SELECT ua.id, ua.o_name, ua.fk_policy_class, ua.property->>'description' AS description, ua.created_at
        FROM isahl_auth.ngac_user_attribute ua
        JOIN isahl_auth.ngac_user_rr_attribute ur ON ua.id = ur.fk_user_attribute
        WHERE ur.fk_user = $1
          AND ur.deleted_at IS NULL
          AND ua.deleted_at IS NULL
        ORDER BY ua.o_name
        "#,
    )
    .bind(user_id_param)
    .fetch_all(pool.get_ref())
    .await;

    match attrs {
        Ok(rows) => HttpResponse::Ok().json(serde_json::json!({"attributes": rows})),
        Err(e) => {
            log::error!("list_user_attributes DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to list user attributes"}))
        }
    }
}

/// GET /api/admin/user-attributes
/// List all defined NGAC user attributes (role catalog).
pub async fn list_all_user_attributes(
    req: HttpRequest,
    query: web::Query<NgacListParams>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let limit = query.limit.unwrap_or(100).clamp(1, 5000);
    let offset = query.offset.unwrap_or(0).max(0);
    let q = query.q.as_deref().unwrap_or("").trim();
    let like = format!("%{}%", q);

    let attrs = sqlx::query_as::<_, UserAttributeResponse>(
        r#"
        SELECT id, o_name, fk_policy_class, property->>'description' AS description,
               COALESCE(ancestor_ids, '{}') AS ancestor_ids,
               created_at
        FROM isahl_auth.ngac_user_attribute
        WHERE deleted_at IS NULL
          AND ($1 = '' OR o_name ILIKE $2)
        ORDER BY o_name
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(q)
    .bind(&like)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool.get_ref())
    .await;

    match attrs {
        Ok(rows) => HttpResponse::Ok().json(serde_json::json!({"attributes": rows})),
        Err(e) => {
            log::error!("list_all_user_attributes DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to list user attributes"}))
        }
    }
}

/// POST /api/admin/user-attributes
/// Create a new NGAC user attribute.
pub async fn create_user_attribute(
    req: HttpRequest,
    body: web::Json<CreateUserAttributeRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let (admin_id, claims) = match require_admin_claims(&req, pool.get_ref(), state.get_ref()).await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };

    let o_name = body.o_name.clone();
    let fk_policy_class = body.fk_policy_class;
    let description = body.description.clone();
    let ancestor_ids = body.ancestor_ids.clone();
    let ancestor_ids_w = ancestor_ids.clone();
    let ip = crate::ngac::audit_writer::client_ip(&req);
    let result = crate::ngac::integrity::with_validated_write(
        pool.get_ref(),
        "user_attribute",
        None,
        &ancestor_ids,
        fk_policy_class,
        |tx| Box::pin(async move {
            let id: (i64,) = sqlx::query_as(
                r#"
                INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, property, ancestor_ids, children_ids, created_by_id)
                VALUES ($1, COALESCE($2, (SELECT MIN(id) FROM isahl_auth.ngac_policy_class WHERE is_active)), CASE WHEN $3 IS NULL THEN '{}'::jsonb ELSE jsonb_build_object('description', $3) END, COALESCE($4, '{}'::bigint[]), '{}'::bigint[], $5)
                RETURNING id
                "#,
            )
            .bind(&o_name)
            .bind(fk_policy_class)
            .bind(&description)
            .bind(&ancestor_ids_w)
            .bind(admin_id)
            .fetch_one(&mut *tx)
            .await?;
            // 同事务审计（change add-ngac-audit-trail-view D1）
            let new_values =
                crate::ngac::audit_writer::row_mirror_tx(tx, "ngac_user_attribute", id.0).await?;
            crate::ngac::audit_writer::write_audit_tx(tx, &crate::ngac::audit_writer::AuditRecord {
                action: "insert",
                entity_type: "user_attribute",
                entity_id: id.0,
                old_values: None,
                new_values,
                actor: admin_id,
                session_id,
                ip_address: ip,
            })
            .await?;
            Ok(id)
        }),
    )
    .await;

    match result {
        Ok((id,)) => HttpResponse::Created().json(serde_json::json!({
            // zuid 量级超 JS 安全整数——字符串化（对齐列表响应 serde_zuid 约定，
            // 同 create_prohibition 修复）
            "id": id.to_string(),
            "o_name": body.o_name,
            "fk_policy_class": body.fk_policy_class,
            "ancestor_ids": body.ancestor_ids,
        })),
        Err(e) => {
            log::error!("create_user_attribute failed: {}", e);
            HttpResponse::BadRequest().json(serde_json::json!({"error": e}))
        }
    }
}

/// PUT /api/admin/user-attributes/{id}
/// Update a user attribute definition.
pub async fn update_user_attribute(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateUserAttributeRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let (admin_id, claims) = match require_admin_claims(&req, pool.get_ref(), state.get_ref()).await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };

    let attr_id = path.into_inner();

    // 恒定 SET（COALESCE 语义）：占位符恒为 $1..$5，避免条件拼接导致 bind 数错位
    let sql = r#"
        UPDATE isahl_auth.ngac_user_attribute SET
            o_name = COALESCE($2, o_name),
            property = CASE WHEN $3 IS NULL THEN property
                            ELSE jsonb_set(COALESCE(property, '{}'), '{description}', to_jsonb($3)) END,
            ancestor_ids = COALESCE($5, '{}'::bigint[]),
            updated_at = NOW(),
            updated_by_id = $4
        WHERE id = $1 AND deleted_at IS NULL
    "#;

    let o_name = body.o_name.clone();
    let description = body.description.clone();
    let ancestor_ids = body.ancestor_ids.clone();
    let ancestor_ids_w = ancestor_ids.clone();
    let fk_policy_class = body.fk_policy_class;
    let ip = crate::ngac::audit_writer::client_ip(&req);
    // 层级变更时在同一事务内先校验（父存在/同策略类/环）再写入
    let result = crate::ngac::integrity::with_validated_write(
        pool.get_ref(),
        "user_attribute",
        Some(attr_id),
        ancestor_ids.as_deref().unwrap_or(&[]),
        fk_policy_class,
        |tx| {
            Box::pin(async move {
                // 同事务审计（D1）：变更前行镜像 → UPDATE → 变更后行镜像
                let old_values =
                    crate::ngac::audit_writer::row_mirror_tx(tx, "ngac_user_attribute", attr_id)
                        .await?;
                let r = sqlx::query(sql)
                    .bind(attr_id)
                    .bind(&o_name)
                    .bind(&description)
                    .bind(admin_id)
                    .bind(&ancestor_ids_w)
                    .execute(&mut *tx)
                    .await?;
                if r.rows_affected() > 0 {
                    let new_values = crate::ngac::audit_writer::row_mirror_tx(
                        tx,
                        "ngac_user_attribute",
                        attr_id,
                    )
                    .await?;
                    crate::ngac::audit_writer::write_audit_tx(
                        tx,
                        &crate::ngac::audit_writer::AuditRecord {
                            action: "update",
                            entity_type: "user_attribute",
                            entity_id: attr_id,
                            old_values,
                            new_values,
                            actor: admin_id,
                            session_id,
                            ip_address: ip,
                        },
                    )
                    .await?;
                }
                Ok(r)
            })
        },
    )
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            HttpResponse::Ok().json(serde_json::json!({"status": "updated", "id": attr_id}))
        }
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "Attribute not found"})),
        Err(e) => {
            // 校验拒绝（环/父不存在/跨策略类）或事务失败
            log::error!("update_user_attribute failed: {}", e);
            HttpResponse::BadRequest().json(serde_json::json!({"error": e}))
        }
    }
}

/// DELETE /api/admin/user-attributes/{id}
/// Soft-delete a user attribute.
pub async fn delete_user_attribute(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let (admin_id, claims) = match require_admin_claims(&req, pool.get_ref(), state.get_ref()).await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };

    let attr_id = path.into_inner();
    let ip = crate::ngac::audit_writer::client_ip(&req);
    // 同事务审计（D1）：软删 + 审计行同生共死
    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("delete_user_attribute tx begin error: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to delete user attribute"}));
        }
    };
    let result: sqlx::Result<sqlx::postgres::PgQueryResult> = async {
        let old_values =
            crate::ngac::audit_writer::row_mirror_tx(&mut tx, "ngac_user_attribute", attr_id)
                .await?;
        let r = sqlx::query(
            "UPDATE isahl_auth.ngac_user_attribute SET deleted_at = NOW(), updated_by_id = $2 WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(attr_id)
        .bind(admin_id)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() > 0 {
            crate::ngac::audit_writer::write_audit_tx(&mut tx, &crate::ngac::audit_writer::AuditRecord {
                action: "delete",
                entity_type: "user_attribute",
                entity_id: attr_id,
                old_values,
                new_values: None,
                actor: admin_id,
                session_id,
                ip_address: ip,
            })
            .await?;
        }
        sqlx::Result::Ok(r)
    }
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            if let Err(e) = tx.commit().await {
                log::error!("delete_user_attribute commit error: {}", e);
                return HttpResponse::InternalServerError()
                    .json(serde_json::json!({"error": "Failed to delete user attribute"}));
            }
            HttpResponse::Ok().json(serde_json::json!({"status": "deleted", "id": attr_id}))
        }
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "Attribute not found"})),
        Err(e) => {
            log::error!("delete_user_attribute DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to delete user attribute"}))
        }
    }
}

/// POST /api/admin/users/{id}/attributes/bind
/// Bind a user to a user attribute.
pub async fn bind_user_attribute(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<BindUserAttributeRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let (admin_id, claims) = match require_admin_claims(&req, pool.get_ref(), state.get_ref()).await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };

    let user_id_param = path.into_inner();
    let ip = crate::ngac::audit_writer::client_ip(&req);
    // 同事务审计（D1）：ON CONFLICT 无变更（rows_affected=0）不留痕
    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("bind_user_attribute tx begin error: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to bind user attribute"}));
        }
    };
    let result: sqlx::Result<Option<i64>> = async {
        let r = sqlx::query_as::<_, (i64,)>(
            r#"
            INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_by_id)
            VALUES ($1, $2, $3)
            ON CONFLICT (fk_user, fk_user_attribute) DO NOTHING
            RETURNING id
            "#,
        )
        .bind(user_id_param)
        .bind(body.fk_user_attribute)
        .bind(admin_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((binding_id,)) = r {
            let new_values =
                crate::ngac::audit_writer::user_rr_mirror_tx(&mut tx, user_id_param, body.fk_user_attribute)
                    .await?;
            crate::ngac::audit_writer::write_audit_tx(&mut tx, &crate::ngac::audit_writer::AuditRecord {
                action: "insert",
                entity_type: "user_assignment",
                entity_id: binding_id,
                old_values: None,
                new_values,
                actor: admin_id,
                session_id,
                ip_address: ip,
            })
            .await?;
            tx.commit().await?;
            return sqlx::Result::Ok(Some(binding_id));
        }
        sqlx::Result::Ok(None)
    }
    .await;

    match result {
        Ok(Some(_)) => HttpResponse::Ok().json(serde_json::json!({
            "status": "bound",
            "user_id": user_id_param,
            "fk_user_attribute": body.fk_user_attribute,
        })),
        Ok(None) => HttpResponse::Ok().json(serde_json::json!({
            "status": "already_bound",
            "user_id": user_id_param,
            "fk_user_attribute": body.fk_user_attribute,
        })),
        Err(e) => {
            log::error!("bind_user_attribute DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to bind user attribute"}))
        }
    }
}

/// DELETE /api/admin/users/{id}/attributes/{ua_id}
/// Unbind a user attribute from a user (soft-delete the binding).
pub async fn unbind_user_attribute(
    req: HttpRequest,
    path: web::Path<(i64, i64)>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let (admin_id, claims) = match require_admin_claims(&req, pool.get_ref(), state.get_ref()).await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };

    let (user_id, ua_id) = path.into_inner();
    let ip = crate::ngac::audit_writer::client_ip(&req);
    // 同事务审计（D1）：软删绑定 + 审计行同生共死
    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("unbind_user_attribute tx begin error: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to unbind user attribute"}));
        }
    };
    let result: sqlx::Result<u64> = async {
        // 绑定行 id（审计 entity_id 锚点）+ 变更前镜像
        let binding: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM isahl_auth.ngac_user_rr_attribute \
             WHERE fk_user = $1 AND fk_user_attribute = $2 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(ua_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((binding_id,)) = binding else {
            return sqlx::Result::Ok(0_u64);
        };
        let old_values =
            crate::ngac::audit_writer::user_rr_mirror_tx(&mut tx, user_id, ua_id).await?;
        let r = sqlx::query(
            r#"
            UPDATE isahl_auth.ngac_user_rr_attribute
            SET deleted_at = NOW(), updated_at = NOW()
            WHERE fk_user = $1 AND fk_user_attribute = $2 AND deleted_at IS NULL
            "#,
        )
        .bind(user_id)
        .bind(ua_id)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() > 0 {
            crate::ngac::audit_writer::write_audit_tx(
                &mut tx,
                &crate::ngac::audit_writer::AuditRecord {
                    action: "delete",
                    entity_type: "user_assignment",
                    entity_id: binding_id,
                    old_values,
                    new_values: None,
                    actor: admin_id,
                    session_id,
                    ip_address: ip,
                },
            )
            .await?;
        }
        sqlx::Result::Ok(r.rows_affected())
    }
    .await;

    match result {
        Ok(n) if n > 0 => {
            if let Err(e) = tx.commit().await {
                log::error!("unbind_user_attribute commit error: {}", e);
                return HttpResponse::InternalServerError()
                    .json(serde_json::json!({"error": "Failed to unbind user attribute"}));
            }
            HttpResponse::Ok().json(serde_json::json!({
                "status": "unbound",
                "user_id": user_id,
                "fk_user_attribute": ua_id,
            }))
        }
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "Binding not found"})),
        Err(e) => {
            log::error!("unbind_user_attribute DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to unbind user attribute"}))
        }
    }
}

// ============================================================================
// Object Attributes (NGAC resource-side labels)
// ============================================================================

/// 管理面列表查询参数（分页 + 名称搜索；全部可选）
#[derive(Debug, Deserialize)]
pub struct NgacListParams {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub q: Option<String>,
}

/// GET /api/admin/object-attributes
/// List NGAC object attributes.
pub async fn list_object_attributes(
    req: HttpRequest,
    query: web::Query<NgacListParams>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let limit = query.limit.unwrap_or(100).clamp(1, 5000);
    let offset = query.offset.unwrap_or(0).max(0);
    let q = query.q.as_deref().unwrap_or("").trim();
    let like = format!("%{}%", q);
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let attrs = sqlx::query_as::<_, ObjectAttributeResponse>(
        r#"
        SELECT id, o_name, fk_policy_class, resource_type, fk_resource,
               resource_identifier,
               COALESCE(ancestor_ids, '{}') AS ancestor_ids,
               created_at
        FROM isahl_auth.ngac_object_attribute
        WHERE deleted_at IS NULL
          AND ($1 = '' OR o_name ILIKE $2)
        ORDER BY o_name
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(q)
    .bind(&like)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool.get_ref())
    .await;
    match attrs {
        Ok(mut rows) => {
            // 展示名批量解析（一次 meta_collections 查询；NGAC_SPEC §2.2）
            let types: std::collections::HashSet<String> = rows
                .iter()
                .filter_map(|r| r.resource_type.clone())
                .filter(|t| !t.is_empty())
                .collect();
            let meta_names = crate::ngac::display::meta_display_names(pool.get_ref(), &types).await;
            for row in &mut rows {
                let rt = row.resource_type.as_deref().unwrap_or("");
                row.display_name = crate::ngac::display::resolve_display_name(
                    row.fk_resource,
                    row.resource_identifier.as_deref(),
                    rt,
                    &meta_names,
                    &row.o_name,
                );
                let (module_name, module_route, namespace) =
                    crate::ngac::display::module_fields(rt);
                row.module_name = module_name.map(String::from);
                row.module_route = module_route.map(String::from);
                row.namespace = namespace.map(String::from);
            }
            // 页面预览合并（NGAC_PREVIEW_DIR 未设 → 空 map → 全部 null）
            let preview_dir = state
                .ngac_preview_dir
                .as_deref()
                .map(String::from)
                .unwrap_or_default();
            if !preview_dir.is_empty() {
                let manifest = crate::ngac::display::load_preview_manifest(&preview_dir);
                for row in &mut rows {
                    let rt = row.resource_type.as_deref().unwrap_or("");
                    if let Some(info) = manifest.get(&rt.replace('-', "_")) {
                        let mut info = info.clone();
                        info.png_url =
                            format!("/api/admin/ngac/previews/{}.png", rt.replace('-', "_"));
                        row.preview = Some(info);
                    }
                }
            }
            HttpResponse::Ok().json(serde_json::json!({"attributes": rows}))
        }
        Err(e) => {
            log::error!("list_object_attributes DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to list object attributes"}))
        }
    }
}

/// POST /api/admin/object-attributes
/// Create a new NGAC object attribute.
pub async fn create_object_attribute(
    req: HttpRequest,
    body: web::Json<CreateObjectAttributeRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let (admin_id, claims) = match require_admin_claims(&req, pool.get_ref(), state.get_ref()).await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };

    let o_name = body.o_name.clone();
    let fk_policy_class = body.fk_policy_class;
    let resource_type = body.resource_type.clone();
    let fk_resource = body.fk_resource;
    let resource_identifier = body.resource_identifier.clone();
    let ancestor_ids = body.ancestor_ids.clone();
    let ancestor_ids_w = ancestor_ids.clone();
    let ip = crate::ngac::audit_writer::client_ip(&req);
    let result = crate::ngac::integrity::with_validated_write(
        pool.get_ref(),
        "object_attribute",
        None,
        &ancestor_ids,
        fk_policy_class,
        |tx| Box::pin(async move {
            let id: (i64,) = sqlx::query_as(
                r#"
                INSERT INTO isahl_auth.ngac_object_attribute
                    (o_name, fk_policy_class, resource_type, fk_resource, resource_identifier, ancestor_ids, children_ids, created_by_id)
                VALUES ($1, COALESCE($2, (SELECT MIN(id) FROM isahl_auth.ngac_policy_class WHERE is_active)), $3, $4, $5, COALESCE($6, '{}'::bigint[]), '{}'::bigint[], $7)
                RETURNING id
                "#,
            )
            .bind(&o_name)
            .bind(fk_policy_class)
            .bind(&resource_type)
            .bind(fk_resource)
            .bind(&resource_identifier)
            .bind(&ancestor_ids_w)
            .bind(admin_id)
            .fetch_one(&mut *tx)
            .await?;
            // 同事务审计（D1）
            let new_values =
                crate::ngac::audit_writer::row_mirror_tx(tx, "ngac_object_attribute", id.0).await?;
            crate::ngac::audit_writer::write_audit_tx(tx, &crate::ngac::audit_writer::AuditRecord {
                action: "insert",
                entity_type: "object_attribute",
                entity_id: id.0,
                old_values: None,
                new_values,
                actor: admin_id,
                session_id,
                ip_address: ip,
            })
            .await?;
            Ok(id)
        }),
    )
    .await;

    match result {
        Ok((id,)) => HttpResponse::Created().json(serde_json::json!({
            "id": id,
            "o_name": body.o_name,
            "fk_policy_class": body.fk_policy_class,
            "resource_type": body.resource_type,
            "fk_resource": body.fk_resource,
            "resource_identifier": body.resource_identifier,
            "ancestor_ids": body.ancestor_ids,
        })),
        Err(e) => {
            log::error!("create_object_attribute failed: {}", e);
            HttpResponse::BadRequest().json(serde_json::json!({"error": e}))
        }
    }
}

/// PUT /api/admin/object-attributes/{id}
/// Update an object attribute definition.
pub async fn update_object_attribute(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateObjectAttributeRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let (admin_id, claims) = match require_admin_claims(&req, pool.get_ref(), state.get_ref()).await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };

    let attr_id = path.into_inner();

    // 恒定 SET（COALESCE 语义）：占位符恒为 $1..$7，避免条件拼接导致 bind 数错位
    let sql = r#"
        UPDATE isahl_auth.ngac_object_attribute SET
            o_name = COALESCE($2, o_name),
            resource_type = COALESCE($3, resource_type),
            fk_policy_class = COALESCE($5, fk_policy_class),
            resource_identifier = COALESCE($7, resource_identifier),
            ancestor_ids = COALESCE($6, '{}'::bigint[]),
            updated_at = NOW(),
            updated_by_id = $4
        WHERE id = $1 AND deleted_at IS NULL
    "#;

    let o_name = body.o_name.clone();
    let resource_type = body.resource_type.clone();
    let resource_identifier = body.resource_identifier.clone();
    let fk_policy_class = body.fk_policy_class;
    let ancestor_ids = body.ancestor_ids.clone();
    let ancestor_ids_w = ancestor_ids.clone();
    let ip = crate::ngac::audit_writer::client_ip(&req);
    // 层级变更时在同一事务内先校验（父存在/同策略类/环）再写入
    let result = crate::ngac::integrity::with_validated_write(
        pool.get_ref(),
        "object_attribute",
        Some(attr_id),
        ancestor_ids.as_deref().unwrap_or(&[]),
        fk_policy_class,
        |tx| {
            Box::pin(async move {
                // 同事务审计（D1）：变更前行镜像 → UPDATE → 变更后行镜像
                let old_values =
                    crate::ngac::audit_writer::row_mirror_tx(tx, "ngac_object_attribute", attr_id)
                        .await?;
                let r = sqlx::query(sql)
                    .bind(attr_id)
                    .bind(o_name)
                    .bind(&resource_type)
                    .bind(admin_id)
                    .bind(fk_policy_class)
                    .bind(&ancestor_ids_w)
                    .bind(resource_identifier)
                    .execute(&mut *tx)
                    .await?;
                if r.rows_affected() > 0 {
                    let new_values = crate::ngac::audit_writer::row_mirror_tx(
                        tx,
                        "ngac_object_attribute",
                        attr_id,
                    )
                    .await?;
                    crate::ngac::audit_writer::write_audit_tx(
                        tx,
                        &crate::ngac::audit_writer::AuditRecord {
                            action: "update",
                            entity_type: "object_attribute",
                            entity_id: attr_id,
                            old_values,
                            new_values,
                            actor: admin_id,
                            session_id,
                            ip_address: ip,
                        },
                    )
                    .await?;
                }
                Ok(r)
            })
        },
    )
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            HttpResponse::Ok().json(serde_json::json!({"status": "updated", "id": attr_id}))
        }
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "Attribute not found"})),
        Err(e) => {
            log::error!("update_object_attribute DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to update object attribute"}))
        }
    }
}

/// DELETE /api/admin/object-attributes/{id}
/// Soft-delete an NGAC object attribute.
///
/// 语义与 `delete_user_attribute` 一致（NGAC_SPEC：软删后属性定义不再可查询）。
/// 机制：PDP 决策时对象侧经 `get_inherited_object_attributes` 仅包含未删属性，
/// 故引用软删 OA 的 association/prohibition 不再参与匹配（graph 加载本身不过滤）。
pub async fn delete_object_attribute(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let (admin_id, claims) = match require_admin_claims(&req, pool.get_ref(), state.get_ref()).await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };

    let attr_id = path.into_inner();
    let ip = crate::ngac::audit_writer::client_ip(&req);
    // 同事务审计（D1）：软删 + 审计行同生共死
    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("delete_object_attribute tx begin error: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to delete object attribute"}));
        }
    };
    let result: sqlx::Result<sqlx::postgres::PgQueryResult> = async {
        let old_values =
            crate::ngac::audit_writer::row_mirror_tx(&mut tx, "ngac_object_attribute", attr_id)
                .await?;
        let r = sqlx::query(
            r#"
            UPDATE isahl_auth.ngac_object_attribute
            SET deleted_at = NOW(), updated_at = NOW(), updated_by_id = $1
            WHERE id = $2 AND deleted_at IS NULL
            "#,
        )
        .bind(admin_id)
        .bind(attr_id)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() > 0 {
            crate::ngac::audit_writer::write_audit_tx(
                &mut tx,
                &crate::ngac::audit_writer::AuditRecord {
                    action: "delete",
                    entity_type: "object_attribute",
                    entity_id: attr_id,
                    old_values,
                    new_values: None,
                    actor: admin_id,
                    session_id,
                    ip_address: ip,
                },
            )
            .await?;
        }
        sqlx::Result::Ok(r)
    }
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            if let Err(e) = tx.commit().await {
                log::error!("delete_object_attribute commit error: {}", e);
                return HttpResponse::InternalServerError()
                    .json(serde_json::json!({"error": "Failed to delete object attribute"}));
            }
            HttpResponse::Ok().json(serde_json::json!({"status": "deleted"}))
        }
        Ok(_) => HttpResponse::NotFound().json(serde_json::json!({"error": "Attribute not found"})),
        Err(e) => {
            log::error!("delete_object_attribute DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to delete object attribute"}))
        }
    }
}
// ============================================================================
// NGAC Associations (访问策略配置)
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateAssociationRequest {
    pub o_name: Option<String>,
    #[serde(with = "common::serde_zuid")]
    pub fk_user_attribute: i64,
    #[serde(with = "common::serde_zuid")]
    pub fk_object_attribute: i64,
    #[serde(with = "common::serde_zuid::seq")]
    pub ak_access_rights: Vec<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    pub conditions: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAssociationRequest {
    pub o_name: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_user_attribute: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_object_attribute: Option<i64>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_access_rights: Option<Vec<i64>>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    pub conditions: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AccessRightResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    pub description: Option<String>,
    pub applicable_types: Option<Vec<String>>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PolicyClassResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    pub description: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AssociationResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_user_attribute: Option<i64>,
    pub user_attribute: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_object_attribute: Option<i64>,
    pub object_attribute: Option<String>,
    pub resource_type: Option<String>,
    #[serde(with = "common::serde_zuid::seq")]
    pub ak_access_rights: Vec<i64>,
    pub access_rights: Vec<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    pub conditions: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

const ASSOCIATION_SELECT: &str = r#"
    SELECT a.id, a.o_name, a.fk_user_attribute, ua.o_name AS user_attribute,
           a.fk_object_attribute, oa.o_name AS object_attribute, oa.resource_type,
           a.ak_access_rights,
           COALESCE(ARRAY(
               SELECT ar.o_name FROM isahl_auth.ngac_access_right ar
               WHERE ar.id = ANY(a.ak_access_rights)
           ), '{}') AS access_rights,
           a.fk_policy_class, a.conditions, a.created_at
    FROM isahl_auth.ngac_association a
    LEFT JOIN isahl_auth.ngac_user_attribute ua ON ua.id = a.fk_user_attribute
    LEFT JOIN isahl_auth.ngac_object_attribute oa ON oa.id = a.fk_object_attribute
"#;

/// GET /api/admin/ngac/access-rights
/// 操作权限字典（NGAC access right 目录）。
pub async fn list_access_rights(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let rows = sqlx::query_as::<_, AccessRightResponse>(
        r#"
        SELECT id, o_name, description, applicable_types
        FROM isahl_auth.ngac_access_right
        ORDER BY o_name
        "#,
    )
    .fetch_all(pool.get_ref())
    .await;

    match rows {
        Ok(rows) => HttpResponse::Ok().json(serde_json::json!({"access_rights": rows})),
        Err(e) => {
            log::error!("list_access_rights DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to list access rights"}))
        }
    }
}

/// GET /api/admin/ngac/policy-classes
/// NGAC 策略类目录。
pub async fn list_policy_classes(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let rows = sqlx::query_as::<_, PolicyClassResponse>(
        r#"
        SELECT id, o_name, description, is_active
        FROM isahl_auth.ngac_policy_class
        ORDER BY o_name
        "#,
    )
    .fetch_all(pool.get_ref())
    .await;

    match rows {
        Ok(rows) => HttpResponse::Ok().json(serde_json::json!({"policy_classes": rows})),
        Err(e) => {
            log::error!("list_policy_classes DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to list policy classes"}))
        }
    }
}

/// GET /api/admin/ngac/associations
/// 列出全部访问策略（含属性名与操作权限名）。
pub async fn list_associations(
    req: HttpRequest,
    query: web::Query<NgacListParams>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let limit = query.limit.unwrap_or(100).clamp(1, 5000);
    let offset = query.offset.unwrap_or(0).max(0);
    let q = query.q.as_deref().unwrap_or("").trim();
    let like = format!("%{}%", q);
    let sql = format!(
        "{} WHERE a.deleted_at IS NULL AND ($1 = '' OR ua.o_name ILIKE $2 OR oa.o_name ILIKE $2) ORDER BY a.created_at DESC LIMIT $3 OFFSET $4",
        ASSOCIATION_SELECT
    );
    let rows = sqlx::query_as::<_, AssociationResponse>(AssertSqlSafe(sql.as_str()))
        .bind(q)
        .bind(&like)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool.get_ref())
        .await;

    match rows {
        Ok(rows) => HttpResponse::Ok().json(serde_json::json!({"associations": rows})),
        Err(e) => {
            log::error!("list_associations DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to list associations"}))
        }
    }
}

/// POST /api/admin/ngac/associations
/// 创建访问策略（UA ↔ OA + AccessRights）。
pub async fn create_association(
    req: HttpRequest,
    body: web::Json<CreateAssociationRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let (admin_id, claims) = match require_admin_claims(&req, pool.get_ref(), state.get_ref()).await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };
    let ip = crate::ngac::audit_writer::client_ip(&req);

    // 同事务审计（D1）：INSERT + 审计行同生共死
    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("create_association tx begin error: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to create association"}));
        }
    };
    let result: sqlx::Result<i64> = async {
        let (id,): (i64,) = sqlx::query_as(
            r#"
            INSERT INTO isahl_auth.ngac_association
                (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights,
                 fk_policy_class, conditions, created_by_id)
            VALUES ($1, $2, $3, $4, COALESCE($5, (SELECT MIN(id) FROM isahl_auth.ngac_policy_class WHERE is_active)), $6, $7)
            RETURNING id
            "#,
        )
        .bind(&body.o_name)
        .bind(body.fk_user_attribute)
        .bind(body.fk_object_attribute)
        .bind(&body.ak_access_rights)
        .bind(body.fk_policy_class)
        .bind(&body.conditions)
        .bind(admin_id)
        .fetch_one(&mut *tx)
        .await?;
        let new_values =
            crate::ngac::audit_writer::row_mirror_tx(&mut tx, "ngac_association", id).await?;
        crate::ngac::audit_writer::write_audit_tx(&mut tx, &crate::ngac::audit_writer::AuditRecord {
            action: "insert",
            entity_type: "association",
            entity_id: id,
            old_values: None,
            new_values,
            actor: admin_id,
            session_id,
            ip_address: ip,
        })
        .await?;
        tx.commit().await?;
        sqlx::Result::Ok(id)
    }
    .await;

    match result {
        Ok(id) => HttpResponse::Created().json(serde_json::json!({
            "id": id,
            "fk_user_attribute": body.fk_user_attribute,
            "fk_object_attribute": body.fk_object_attribute,
            "ak_access_rights": body.ak_access_rights,
        })),
        Err(e) => {
            log::error!("create_association DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to create association"}))
        }
    }
}

/// PUT /api/admin/ngac/associations/{id}
/// 更新访问策略（COALESCE 语义：未传字段保持原值）。
pub async fn update_association(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateAssociationRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let (admin_id, claims) = match require_admin_claims(&req, pool.get_ref(), state.get_ref()).await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };
    let ip = crate::ngac::audit_writer::client_ip(&req);

    let assoc_id = path.into_inner();
    // 同事务审计（D1）：变更前镜像 → UPDATE → 变更后镜像
    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("update_association tx begin error: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to update association"}));
        }
    };
    let result: sqlx::Result<sqlx::postgres::PgQueryResult> = async {
        let old_values =
            crate::ngac::audit_writer::row_mirror_tx(&mut tx, "ngac_association", assoc_id).await?;
        let r = sqlx::query(
            r#"
            UPDATE isahl_auth.ngac_association SET
                o_name = COALESCE($2, o_name),
                fk_user_attribute = COALESCE($3, fk_user_attribute),
                fk_object_attribute = COALESCE($4, fk_object_attribute),
                ak_access_rights = COALESCE($5, ak_access_rights),
                fk_policy_class = COALESCE($6, fk_policy_class),
                conditions = COALESCE($7, conditions),
                updated_at = NOW(),
                updated_by_id = $8
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(assoc_id)
        .bind(&body.o_name)
        .bind(body.fk_user_attribute)
        .bind(body.fk_object_attribute)
        .bind(&body.ak_access_rights)
        .bind(body.fk_policy_class)
        .bind(&body.conditions)
        .bind(admin_id)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() > 0 {
            let new_values =
                crate::ngac::audit_writer::row_mirror_tx(&mut tx, "ngac_association", assoc_id)
                    .await?;
            crate::ngac::audit_writer::write_audit_tx(
                &mut tx,
                &crate::ngac::audit_writer::AuditRecord {
                    action: "update",
                    entity_type: "association",
                    entity_id: assoc_id,
                    old_values,
                    new_values,
                    actor: admin_id,
                    session_id,
                    ip_address: ip,
                },
            )
            .await?;
            tx.commit().await?;
        }
        sqlx::Result::Ok(r)
    }
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            HttpResponse::Ok().json(serde_json::json!({"status": "updated", "id": assoc_id}))
        }
        Ok(_) => {
            HttpResponse::NotFound().json(serde_json::json!({"error": "Association not found"}))
        }
        Err(e) => {
            log::error!("update_association DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to update association"}))
        }
    }
}

/// DELETE /api/admin/ngac/associations/{id}
/// 软删除访问策略。
pub async fn delete_association(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let (admin_id, claims) = match require_admin_claims(&req, pool.get_ref(), state.get_ref()).await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };
    let ip = crate::ngac::audit_writer::client_ip(&req);

    let assoc_id = path.into_inner();
    // 同事务审计（D1）：软删 + 审计行同生共死
    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("delete_association tx begin error: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to delete association"}));
        }
    };
    let result: sqlx::Result<sqlx::postgres::PgQueryResult> = async {
        let old_values =
            crate::ngac::audit_writer::row_mirror_tx(&mut tx, "ngac_association", assoc_id).await?;
        let r = sqlx::query(
            r#"
            UPDATE isahl_auth.ngac_association
            SET deleted_at = NOW(), updated_at = NOW(), updated_by_id = $1
            WHERE id = $2 AND deleted_at IS NULL
            "#,
        )
        .bind(admin_id)
        .bind(assoc_id)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() > 0 {
            crate::ngac::audit_writer::write_audit_tx(
                &mut tx,
                &crate::ngac::audit_writer::AuditRecord {
                    action: "delete",
                    entity_type: "association",
                    entity_id: assoc_id,
                    old_values,
                    new_values: None,
                    actor: admin_id,
                    session_id,
                    ip_address: ip,
                },
            )
            .await?;
            tx.commit().await?;
        }
        sqlx::Result::Ok(r)
    }
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            HttpResponse::Ok().json(serde_json::json!({"status": "deleted"}))
        }
        Ok(_) => {
            HttpResponse::NotFound().json(serde_json::json!({"error": "Association not found"}))
        }
        Err(e) => {
            log::error!("delete_association DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to delete association"}))
        }
    }
}

// ============================================================
// NGAC Prohibition（禁止规则）
// ============================================================

/// Prohibition response（含属性名与操作权限名解析）
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ProhibitionResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_user_attribute: Option<i64>,
    pub user_attribute: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_object_attribute: Option<i64>,
    pub object_attribute: Option<String>,
    pub resource_type: Option<String>,
    #[serde(with = "common::serde_zuid::seq")]
    pub ak_access_rights: Vec<i64>,
    pub access_rights: Vec<String>,
    pub is_active: bool,
    pub conditions: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

const PROHIBITION_SELECT: &str = r#"
    SELECT p.id, p.o_name, p.fk_user_attribute, ua.o_name AS user_attribute,
           p.fk_object_attribute, oa.o_name AS object_attribute, oa.resource_type,
           p.ak_access_rights,
           COALESCE(ARRAY(
               SELECT ar.o_name FROM isahl_auth.ngac_access_right ar
               WHERE ar.id = ANY(p.ak_access_rights)
           ), '{}') AS access_rights,
           p.is_active, p.conditions, p.created_at
    FROM isahl_auth.ngac_prohibition p
    LEFT JOIN isahl_auth.ngac_user_attribute ua ON ua.id = p.fk_user_attribute
    LEFT JOIN isahl_auth.ngac_object_attribute oa ON oa.id = p.fk_object_attribute
"#;

/// Create prohibition request
#[derive(Debug, Deserialize)]
pub struct CreateProhibitionRequest {
    pub o_name: Option<String>,
    #[serde(with = "common::serde_zuid")]
    pub fk_user_attribute: i64,
    #[serde(with = "common::serde_zuid")]
    pub fk_object_attribute: i64,
    #[serde(with = "common::serde_zuid::seq")]
    pub ak_access_rights: Vec<i64>,
    #[serde(default = "default_true_bool")]
    pub is_active: bool,
    pub conditions: Option<serde_json::Value>,
}

/// Update prohibition request（COALESCE 语义）
#[derive(Debug, Deserialize)]
pub struct UpdateProhibitionRequest {
    pub o_name: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_user_attribute: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_object_attribute: Option<i64>,
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub ak_access_rights: Option<Vec<i64>>,
    pub is_active: Option<bool>,
    pub conditions: Option<serde_json::Value>,
}

fn default_true_bool() -> bool {
    true
}

/// GET /api/admin/ngac/prohibitions
pub async fn list_prohibitions(
    req: HttpRequest,
    query: web::Query<NgacListParams>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let limit = query.limit.unwrap_or(100).clamp(1, 5000);
    let offset = query.offset.unwrap_or(0).max(0);
    let q = query.q.as_deref().unwrap_or("").trim();
    let like = format!("%{}%", q);
    let sql = format!(
        "{} WHERE p.deleted_at IS NULL AND ($1 = '' OR ua.o_name ILIKE $2 OR oa.o_name ILIKE $2) ORDER BY p.created_at DESC LIMIT $3 OFFSET $4",
        PROHIBITION_SELECT
    );
    let rows = sqlx::query_as::<_, ProhibitionResponse>(AssertSqlSafe(sql.as_str()))
        .bind(q)
        .bind(&like)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool.get_ref())
        .await;

    match rows {
        Ok(rows) => HttpResponse::Ok().json(serde_json::json!({"prohibitions": rows})),
        Err(e) => {
            log::error!("list_prohibitions DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to list prohibitions"}))
        }
    }
}

/// POST /api/admin/ngac/prohibitions
pub async fn create_prohibition(
    req: HttpRequest,
    body: web::Json<CreateProhibitionRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let (admin_id, claims) = match require_admin_claims(&req, pool.get_ref(), state.get_ref()).await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };
    let ip = crate::ngac::audit_writer::client_ip(&req);

    // 同事务审计（D1）：INSERT + 审计行同生共死
    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("create_prohibition tx begin error: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to create prohibition"}));
        }
    };
    let result: sqlx::Result<i64> = async {
        let (id,): (i64,) = sqlx::query_as(
            r#"
            INSERT INTO isahl_auth.ngac_prohibition
                (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights,
                 is_active, conditions, created_by_id)
            VALUES ($1, $2, $3, $4, COALESCE($5, TRUE), $6, $7)
            RETURNING id
            "#,
        )
        .bind(&body.o_name)
        .bind(body.fk_user_attribute)
        .bind(body.fk_object_attribute)
        .bind(&body.ak_access_rights)
        .bind(body.is_active)
        .bind(&body.conditions)
        .bind(admin_id)
        .fetch_one(&mut *tx)
        .await?;
        let new_values =
            crate::ngac::audit_writer::row_mirror_tx(&mut tx, "ngac_prohibition", id).await?;
        crate::ngac::audit_writer::write_audit_tx(
            &mut tx,
            &crate::ngac::audit_writer::AuditRecord {
                action: "insert",
                entity_type: "prohibition",
                entity_id: id,
                old_values: None,
                new_values,
                actor: admin_id,
                session_id,
                ip_address: ip,
            },
        )
        .await?;
        tx.commit().await?;
        sqlx::Result::Ok(id)
    }
    .await;

    match result {
        Ok(id) => HttpResponse::Created().json(serde_json::json!({
            // zuid 量级超 JS 安全整数——字符串化（对齐列表响应 serde_zuid 约定；
            // 修复 create/列表 id 类型不一致，ngac_explain_test 比较恒假）
            "id": id.to_string(),
            "fk_user_attribute": body.fk_user_attribute.to_string(),
            "fk_object_attribute": body.fk_object_attribute.to_string(),
            "ak_access_rights": body.ak_access_rights,
            "is_active": body.is_active,
        })),
        Err(e) => {
            log::error!("create_prohibition DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to create prohibition"}))
        }
    }
}

/// PUT /api/admin/ngac/prohibitions/{id}
pub async fn update_prohibition(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateProhibitionRequest>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let (admin_id, claims) = match require_admin_claims(&req, pool.get_ref(), state.get_ref()).await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };
    let ip = crate::ngac::audit_writer::client_ip(&req);

    let prohibition_id = path.into_inner();
    // 同事务审计（D1）：变更前镜像 → UPDATE → 变更后镜像
    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("update_prohibition tx begin error: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to update prohibition"}));
        }
    };
    let result: sqlx::Result<sqlx::postgres::PgQueryResult> = async {
        let old_values =
            crate::ngac::audit_writer::row_mirror_tx(&mut tx, "ngac_prohibition", prohibition_id)
                .await?;
        let r = sqlx::query(
            r#"
            UPDATE isahl_auth.ngac_prohibition SET
                o_name = COALESCE($2, o_name),
                fk_user_attribute = COALESCE($3, fk_user_attribute),
                fk_object_attribute = COALESCE($4, fk_object_attribute),
                ak_access_rights = COALESCE($5, ak_access_rights),
                is_active = COALESCE($6, is_active),
                conditions = COALESCE($7, conditions),
                updated_at = NOW(),
                updated_by_id = $8
            WHERE id = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(prohibition_id)
        .bind(&body.o_name)
        .bind(body.fk_user_attribute)
        .bind(body.fk_object_attribute)
        .bind(&body.ak_access_rights)
        .bind(body.is_active)
        .bind(&body.conditions)
        .bind(admin_id)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() > 0 {
            let new_values = crate::ngac::audit_writer::row_mirror_tx(
                &mut tx,
                "ngac_prohibition",
                prohibition_id,
            )
            .await?;
            crate::ngac::audit_writer::write_audit_tx(
                &mut tx,
                &crate::ngac::audit_writer::AuditRecord {
                    action: "update",
                    entity_type: "prohibition",
                    entity_id: prohibition_id,
                    old_values,
                    new_values,
                    actor: admin_id,
                    session_id,
                    ip_address: ip,
                },
            )
            .await?;
            tx.commit().await?;
        }
        sqlx::Result::Ok(r)
    }
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            HttpResponse::Ok().json(serde_json::json!({"status": "updated", "id": prohibition_id}))
        }
        Ok(_) => {
            HttpResponse::NotFound().json(serde_json::json!({"error": "Prohibition not found"}))
        }
        Err(e) => {
            log::error!("update_prohibition DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to update prohibition"}))
        }
    }
}

/// DELETE /api/admin/ngac/prohibitions/{id}
pub async fn delete_prohibition(
    req: HttpRequest,
    path: web::Path<i64>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let (admin_id, claims) = match require_admin_claims(&req, pool.get_ref(), state.get_ref()).await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };
    let ip = crate::ngac::audit_writer::client_ip(&req);

    let prohibition_id = path.into_inner();
    // 同事务审计（D1）：软删 + 审计行同生共死
    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("delete_prohibition tx begin error: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to delete prohibition"}));
        }
    };
    let result: sqlx::Result<sqlx::postgres::PgQueryResult> = async {
        let old_values =
            crate::ngac::audit_writer::row_mirror_tx(&mut tx, "ngac_prohibition", prohibition_id)
                .await?;
        let r = sqlx::query(
            r#"
            UPDATE isahl_auth.ngac_prohibition
            SET deleted_at = NOW(), updated_at = NOW(), updated_by_id = $1
            WHERE id = $2 AND deleted_at IS NULL
            "#,
        )
        .bind(admin_id)
        .bind(prohibition_id)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() > 0 {
            crate::ngac::audit_writer::write_audit_tx(
                &mut tx,
                &crate::ngac::audit_writer::AuditRecord {
                    action: "delete",
                    entity_type: "prohibition",
                    entity_id: prohibition_id,
                    old_values,
                    new_values: None,
                    actor: admin_id,
                    session_id,
                    ip_address: ip,
                },
            )
            .await?;
            tx.commit().await?;
        }
        sqlx::Result::Ok(r)
    }
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            HttpResponse::Ok().json(serde_json::json!({"status": "deleted"}))
        }
        Ok(_) => {
            HttpResponse::NotFound().json(serde_json::json!({"error": "Prohibition not found"}))
        }
        Err(e) => {
            log::error!("delete_prohibition DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to delete prohibition"}))
        }
    }
}

// ============================================================
// NGAC 策略矩阵（矩阵投影，只读）
// ============================================================

/// GET /api/admin/ngac/matrix 查询参数：`policy_class` 必填（ZUID）。
#[derive(Debug, Deserialize)]
pub struct MatrixQueryParams {
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    pub policy_class: Option<i64>,
}

/// GET /api/admin/ngac/matrix
/// PC 作用域的策略矩阵投影：UA 行（含继承与成员数）× OA 列（按 resource_type
/// 分组、实例折叠）× 单元格（direct / effective / denied 三态权限集）。
///
/// `effective` / `denied` 由 PDP 同源遍历计算（与 `/api/ngac/decide/explain`
/// 同一 `evaluate_pair` 语义路径），前端不本地推导授权语义。
pub async fn get_policy_matrix(
    req: HttpRequest,
    query: web::Query<MatrixQueryParams>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let pc_id = match query.policy_class {
        Some(id) => id,
        None => {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"error": "policy_class query parameter is required"}));
        }
    };

    let pip = PostgresPip::new(pool.get_ref().clone());
    let pdp = crate::ngac::pdp::Pdp::global();
    match pdp.policy_matrix(&pip, pc_id).await {
        Ok(mut matrix) => {
            // 页面预览（add-ngac-oa-preview，dev-only）：矩阵列头模块徽章
            // 点击预览（2026-08-21 从跳转链接改为不出戏的截图+高亮预览）。
            // 在 handler 层填充（文件系统数据不进 PDP 矩阵缓存）。
            let preview_dir = state.ngac_preview_dir.as_deref().unwrap_or("");
            if !preview_dir.is_empty() {
                let manifest = crate::ngac::display::load_preview_manifest(preview_dir);
                for g in matrix.object_groups.iter_mut() {
                    let rt = g.resource_type.replace('-', "_");
                    if let Some(src) = manifest.get(&g.resource_type).or_else(|| manifest.get(&rt))
                    {
                        let mut info = src.clone();
                        info.png_url = format!("/api/admin/ngac/previews/{}.png", rt);
                        g.preview = Some(info);
                    }
                }
            }
            HttpResponse::Ok().json(matrix)
        }
        Err(crate::ngac::pdp::MatrixError::PolicyClassNotFound(id)) => {
            log::warn!("get_policy_matrix: policy class {} not found", id);
            HttpResponse::NotFound()
                .json(serde_json::json!({"error": format!("Policy class {} not found", id)}))
        }
        Err(e) => {
            log::error!("get_policy_matrix DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to load policy matrix"}))
        }
    }
}

// ============================================================
// NGAC 访问审查（主体/资源双中心，只读）
// ============================================================

/// GET /api/admin/ngac/review/user/{id}
/// 主体中心访问审查：用户 UA 指派链 + 按 resource_type 分组的有效权限
/// （allowed / denied，PDP `evaluate_pair` 同源，与 explain 逐 action 一致）。
pub async fn get_user_access_review(
    req: HttpRequest,
    path: web::Path<String>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let user_id: i64 = match path.into_inner().parse() {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"error": "Invalid user id"}));
        }
    };

    let pip = PostgresPip::new(pool.get_ref().clone());
    let pdp = crate::ngac::pdp::Pdp::global();
    match pdp.user_access_review(&pip, user_id).await {
        Ok(review) => HttpResponse::Ok().json(review),
        Err(crate::ngac::pdp::ReviewError::UserNotFound(id)) => {
            log::warn!("get_user_access_review: user {} not found", id);
            HttpResponse::NotFound()
                .json(serde_json::json!({"error": format!("User {} not found", id)}))
        }
        Err(e) => {
            log::error!("get_user_access_review DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to load user access review"}))
        }
    }
}

/// GET /api/admin/ngac/review/resource 查询参数：`resource_type` 必填，
/// `fk_resource` 缺省 0（集合级）。
#[derive(Debug, Deserialize)]
pub struct ResourceReviewQueryParams {
    pub resource_type: Option<String>,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_resource: Option<i64>,
}

/// GET /api/admin/ngac/review/resource
/// 资源中心访问审查：持有者的有效权限（UA + allowed/denied + 成员用户）。
/// 与 `/api/ngac/decide/explain` 同源；holders 稀疏（仅非空者）。
pub async fn get_resource_access_review(
    req: HttpRequest,
    query: web::Query<ResourceReviewQueryParams>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let resource_type = match query.resource_type.as_deref() {
        Some(rt) if !rt.is_empty() => rt.to_string(),
        _ => {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"error": "resource_type query parameter is required"}));
        }
    };
    let fk_resource = query.fk_resource.unwrap_or(0);

    let pip = PostgresPip::new(pool.get_ref().clone());
    let pdp = crate::ngac::pdp::Pdp::global();
    match pdp
        .resource_access_review(&pip, &resource_type, fk_resource)
        .await
    {
        Ok(review) => HttpResponse::Ok().json(review),
        Err(e) => {
            log::error!("get_resource_access_review DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to load resource access review"}))
        }
    }
}

// ============================================================
// NGAC 策略变更审计轨迹 + 删除影响预览（change add-ngac-audit-trail-view）
// ============================================================

/// GET /api/admin/ngac/audit-log 查询参数。
#[derive(Debug, Deserialize)]
pub struct AuditLogQueryParams {
    pub entity_type: Option<String>,
    pub action: Option<String>,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    pub actor: Option<i64>,
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// GET /api/admin/ngac/audit-log
/// 策略变更审计轨迹（分页 + entity_type/action/actor/时间窗过滤，倒序）。
/// 数据分级：old/new 为策略行完整镜像——仅超级管理员可见（SECURITY_SPEC
/// §10.1/§10.3「完整镜像仅审计管理员」，require_admin 满足）。无缓存（实时性）。
pub async fn get_audit_log(
    req: HttpRequest,
    query: web::Query<AuditLogQueryParams>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let offset = query.offset.unwrap_or(0).max(0);

    // 恒定占位符（$1..$6 恒在，未传过滤条件时参数恒 TRUE 通过）
    let where_clause = r#"
        WHERE ($1::text IS NULL OR l.entity_type = $1)
          AND ($2::text IS NULL OR l.action = $2)
          AND ($3::bigint IS NULL OR l.fk_user = $3)
          AND ($4::timestamptz IS NULL OR l.created_at >= $4)
          AND ($5::timestamptz IS NULL OR l.created_at < $5)
    "#;

    // where_clause 为上方硬编码字面量（占位符恒 $1..$5），无用户输入拼接
    let total: Result<i64, _> = sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM isahl_auth.ngac_policy_audit_log l {}",
        where_clause
    )))
    .bind(&query.entity_type)
    .bind(&query.action)
    .bind(query.actor)
    .bind(query.from)
    .bind(query.to)
    .fetch_one(pool.get_ref())
    .await;

    let total = match total {
        Ok(t) => t,
        Err(e) => {
            log::error!("get_audit_log count error: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to load audit log"}));
        }
    };

    #[derive(Serialize, sqlx::FromRow)]
    struct AuditLogRow {
        #[serde(with = "common::serde_zuid")]
        id: i64,
        action: String,
        entity_type: String,
        #[serde(with = "common::serde_zuid")]
        entity_id: i64,
        old_values: Option<serde_json::Value>,
        new_values: Option<serde_json::Value>,
        #[serde(with = "common::serde_zuid::opt", default)]
        fk_user: Option<i64>,
        actor_username: Option<String>,
        session_id: Option<String>,
        ip_address: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
    }

    let rows = sqlx::query_as::<_, AuditLogRow>(AssertSqlSafe(format!(
        r#"
        SELECT l.id, l.action, l.entity_type, l.entity_id, l.old_values, l.new_values,
               l.fk_user, u.username AS actor_username, l.session_id, l.ip_address::text AS ip_address, l.created_at
        FROM isahl_auth.ngac_policy_audit_log l
        LEFT JOIN isahl_auth.auth_users u ON u.id = l.fk_user
        {}
        ORDER BY l.created_at DESC, l.id DESC
        LIMIT $6 OFFSET $7
        "#,
        where_clause
    )))
    .bind(&query.entity_type)
    .bind(&query.action)
    .bind(query.actor)
    .bind(query.from)
    .bind(query.to)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool.get_ref())
    .await;

    match rows {
        Ok(rows) => HttpResponse::Ok().json(serde_json::json!({
            "rows": rows,
            "total": total,
            "limit": limit,
            "offset": offset,
        })),
        Err(e) => {
            log::error!("get_audit_log DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to load audit log"}))
        }
    }
}

/// GET /api/admin/ngac/impact-preview 查询参数。
#[derive(Debug, Deserialize)]
pub struct ImpactPreviewQueryParams {
    pub entity_type: Option<String>,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    pub id: Option<i64>,
}

/// GET /api/admin/ngac/impact-preview
/// 删除影响预览：模拟删除目标实体（association/prohibition/UA/OA），返回
/// before/after 双图比对的有效权限差异（受影响 UA × resource_type ×
/// lost_allow/lost_deny + 受影响用户）。PDP `evaluate_pair_in` 同源。
/// 已知盲区：按当前时刻求值——删除尚未到 not_before 生效窗的边，其未来
/// 授权不体现在 lost_allow 中。
pub async fn get_impact_preview(
    req: HttpRequest,
    query: web::Query<ImpactPreviewQueryParams>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let entity_type = match query.entity_type.as_deref() {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"error": "entity_type query parameter is required"}));
        }
    };
    let entity_id = match query.id {
        Some(id) => id,
        None => {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"error": "id query parameter is required"}));
        }
    };

    let pip = PostgresPip::new(pool.get_ref().clone());
    let pdp = crate::ngac::pdp::Pdp::global();
    match pdp.impact_preview(&pip, &entity_type, entity_id).await {
        Ok(preview) => HttpResponse::Ok().json(preview),
        Err(crate::ngac::pdp::ImpactError::InvalidEntityType(t)) => {
            HttpResponse::BadRequest().json(serde_json::json!({
                "error": format!("Invalid entity_type: {} (expected association|prohibition|user_attribute|object_attribute)", t)
            }))
        }
        Err(crate::ngac::pdp::ImpactError::NotFound(t, id)) => {
            HttpResponse::NotFound()
                .json(serde_json::json!({"error": format!("{} {} not found", t, id)}))
        }
        Err(e) => {
            log::error!("get_impact_preview error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to compute impact preview"}))
        }
    }
}

/// GET /api/admin/ngac/previews/{filename}
/// OA 页面预览截图静态服务（add-ngac-oa-preview，dev-only）：
/// 从 `NGAC_PREVIEW_DIR` 目录读取；文件名白名单（仅 `{resource_type}.png`，
/// 防路径穿越）；目录未配置 → 404。
pub async fn get_ngac_preview_file(
    req: HttpRequest,
    path: web::Path<String>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(
        &req,
        req.app_data::<web::Data<PgPool>>().unwrap().get_ref(),
        state.get_ref(),
    )
    .await
    {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let dir = match state.ngac_preview_dir.as_deref() {
        Some(d) if !d.is_empty() => d,
        _ => return HttpResponse::NotFound().finish(),
    };

    let filename = path.into_inner();
    // 白名单：仅允许 `[a-z0-9_]+\.png`（采集器产物命名）
    if !filename.ends_with(".png")
        || !filename
            .trim_end_matches(".png")
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return HttpResponse::NotFound().finish();
    }
    let file_path = std::path::Path::new(dir).join(&filename);
    match tokio::fs::read(&file_path).await {
        Ok(bytes) => HttpResponse::Ok().content_type("image/png").body(bytes),
        Err(_) => HttpResponse::NotFound().finish(),
    }
}

// ============================================================
// NGAC 图快照（refactor-ngac-admin-nl-graph，只读聚合）
// ============================================================

/// GET /api/admin/ngac/graph
/// 策略图全量快照（版本/策略类/UA+持有者/OA+展示名/边/access rights）。
/// 聚合实现在 `ngac::graph::graph_snapshot`（唯一实现，Gateway nl-assist
/// 进程内共用——design D2）；本 handler 仅做 admin 门控与序列化。
pub async fn get_ngac_graph(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let _admin_id = match require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    match crate::ngac::graph::graph_snapshot(pool.get_ref()).await {
        Ok(mut snapshot) => {
            // 页面预览合并（add-ngac-oa-preview 同语：NGAC_PREVIEW_DIR 未设 → 空 map → 全部 null）
            let preview_dir = state
                .ngac_preview_dir
                .as_deref()
                .map(String::from)
                .unwrap_or_default();
            if !preview_dir.is_empty() {
                let manifest = crate::ngac::display::load_preview_manifest(&preview_dir);
                for oa in &mut snapshot.object_attributes {
                    let rt = oa.resource_type.as_deref().unwrap_or("");
                    if let Some(info) = manifest.get(&rt.replace('-', "_")) {
                        let mut info = info.clone();
                        info.png_url =
                            format!("/api/admin/ngac/previews/{}.png", rt.replace('-', "_"));
                        oa.preview = Some(info);
                    }
                }
            }
            HttpResponse::Ok().json(snapshot)
        }
        Err(e) => {
            log::error!("get_ngac_graph DB error: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "Failed to build NGAC graph snapshot"}))
        }
    }
}
