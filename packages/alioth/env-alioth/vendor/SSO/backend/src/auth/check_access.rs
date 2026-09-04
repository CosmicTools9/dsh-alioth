//! 门户路径访问权限检查（PEP → PDP）
//!
//! 根据 GATEWAY_DESIGN_SPEC.md §3.3.7：
//! 前端路由守卫作为 PEP，向 PDP 查询路径级访问权限。
//!
//! POST /auth/check-access
//! { "path": "/modules/shop/admin/products" }
//! → { "allowed": true/false, "redirect": "..." }

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};

use super::jwt::{decode_token_any, Claims};
use super::login::AuthError;

#[derive(Debug, Deserialize)]
pub struct CheckAccessRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct CheckAccessResponse {
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// 判断是否为 storefront-only 路径
fn is_storefront_path(path: &str) -> bool {
    path.starts_with("/modules/shop/store/")
}

/// 根据用户的 portal-scope 判断指定路径是否有访问权限。
/// 简化 PEP 实现——scope 值已由 SSO PDP 预计算。
pub async fn check_access(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    state: web::Data<super::AuthState>,
    body: web::Json<CheckAccessRequest>,
) -> HttpResponse {
    let access_token = match req.cookie("access_token") {
        Some(c) => c.value().to_string(),
        None => match req
            .headers()
            .get(actix_web::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|auth| auth.strip_prefix("Bearer "))
        {
            Some(token) => token.to_string(),
            None => {
                return HttpResponse::Unauthorized().json(AuthError {
                    error: "No authentication token".to_string(),
                })
            }
        },
    };

    let claims: Claims = match decode_token_any(&access_token, &state.verification_keys()) {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid or expired token".to_string(),
            })
        }
    };

    let user_id: i64 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "Invalid user ID in token".to_string(),
            })
        }
    };

    // 查询用户的 portal-scope
    let attrs: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT DISTINCT ua.o_name
        FROM isahl_auth.ngac_user_rr_attribute rel
        JOIN isahl_auth.ngac_user_attribute ua ON ua.id = rel.fk_user_attribute
        WHERE rel.fk_user = $1
          AND (rel.deleted_at IS NULL)
          AND (rel.expires_at IS NULL OR rel.expires_at > NOW())
        "#,
    )
    .bind(user_id)
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_default();

    let has_workbench = attrs.iter().any(|a| a == "admin" || a == "operator");
    let has_storefront = attrs
        .iter()
        .any(|a| a == "user" || a == "customer" || a == "storefront");
    let path = &body.path;

    // 公开路径：任何用户均可访问
    if path == "/auth/login" || path == "/auth/register" || path == "/auth/reset-password" {
        return HttpResponse::Ok().json(CheckAccessResponse {
            allowed: true,
            redirect: None,
            reason: None,
        });
    }

    // 策略 1：storefront-only 用户 → 仅允许 storefront 路径
    if has_storefront && !has_workbench {
        if is_storefront_path(path) {
            return HttpResponse::Ok().json(CheckAccessResponse {
                allowed: true,
                redirect: None,
                reason: None,
            });
        }
        return HttpResponse::Ok().json(CheckAccessResponse {
            allowed: false,
            redirect: Some("/modules/shop/store/products".to_string()),
            reason: Some("STORE_ONLY_ACCESS".to_string()),
        });
    }

    // 策略 2：workbench 用户 → 允许所有路径
    HttpResponse::Ok().json(CheckAccessResponse {
        allowed: true,
        redirect: None,
        reason: None,
    })
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/auth/check-access", web::post().to(check_access));
}
