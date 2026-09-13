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
    // 令牌提取走共享实现（scoped cookie → Bearer → 环境 cookie；见 jwt::extract_token）
    let access_token = match crate::auth::jwt::extract_token(&req) {
        Some(t) => t,
        None => {
            return HttpResponse::Unauthorized().json(AuthError {
                error: "No authentication token".to_string(),
            })
        }
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

    // App 路径判定（add-app-visibility-ngac-isolation D4）：
    // /apps/{code}/... → App OA association decide；无 OA/无关联 → 拒绝（fail-closed）。
    // admin 豁免与 PDP list §6.2 同语义；判定通过后置 fall-through 至既有策略。
    if let Some(code) = path
        .strip_prefix("/apps/")
        .and_then(|rest| rest.split('/').next())
        .filter(|c| !c.is_empty())
    {
        if !attrs.iter().any(|a| a == "admin") {
            let oa_id: Option<i64> = sqlx::query_scalar(
                r#"SELECT fk_resource FROM isahl_auth.ngac_object_attribute
                   WHERE resource_type = 'app' AND resource_identifier = $1 AND deleted_at IS NULL
                   LIMIT 1"#,
            )
            .bind(code)
            .fetch_optional(pool.get_ref())
            .await
            .unwrap_or(None);
            let permitted = match oa_id {
                Some(oa) => {
                    crate::ngac::pdp::decide_access(
                        pool.get_ref(),
                        user_id,
                        &format!("app:{oa}"),
                        "read",
                    )
                    .await
                        == crate::ngac::pdp::Decision::Permit
                }
                None => false, // App OA 未配置 → fail-closed
            };
            if !permitted {
                return HttpResponse::Ok().json(CheckAccessResponse {
                    allowed: false,
                    redirect: Some("/".to_string()),
                    reason: Some("APP_NOT_VISIBLE".to_string()),
                });
            }
            // App 关联判定通过即放行（不再落 shop 时代 storefront 二元策略——
            // 外部角色的门户 App 路径会被策略 1 误杀）
            return HttpResponse::Ok().json(CheckAccessResponse {
                allowed: true,
                redirect: None,
                reason: None,
            });
        }
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
