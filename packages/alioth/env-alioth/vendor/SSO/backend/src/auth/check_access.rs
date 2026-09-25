//! 门户路径访问权限检查（PEP → PDP）
//!
//! 根据 GATEWAY_DESIGN_SPEC.md §3.3.7：
//! 前端路由守卫作为 PEP，向 PDP 查询路径级访问权限。
//!
//! POST /auth/check-access
//! { "path": "/apps/wz-external/home" }
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

/// 路径级门户判定的纯判定面（GATEWAY_DESIGN_SPEC §3.3.2 策略 1/2/3）。
/// 与 DB 无关；`/apps/{code}` 的 App 关联判定需 DB，以 `PathAccess::AppRoute` 回传
/// 由 handler 委派 `pdp::decide_access`。
#[derive(Debug, PartialEq)]
enum PathAccess {
    Allow,
    Deny { redirect: String, reason: String },
    AppRoute(String),
}

/// 判定顺序：公开路径 → App 路径（回传委派）→ storefront-only 白名单 → 默认放行
/// （workbench 归属含规约默认策略：无 portal-scope 属性 → workbench）。
fn decide_path_access(
    portal_scope: &[String],
    path: &str,
    storefront_prefix: &str,
    landing: &str,
) -> PathAccess {
    if path == "/auth/login" || path == "/auth/register" || path == "/auth/reset-password" {
        return PathAccess::Allow;
    }
    if let Some(code) = path
        .strip_prefix("/apps/")
        .and_then(|rest| rest.split('/').next())
        .filter(|c| !c.is_empty())
    {
        return PathAccess::AppRoute(code.to_string());
    }
    let has_workbench = portal_scope.iter().any(|s| s == "workbench");
    let has_storefront = portal_scope.iter().any(|s| s == "storefront");
    // 策略 1：storefront-only 用户 → 仅允许 storefront 白名单路径
    if has_storefront && !has_workbench {
        // 落地路径自身必须可达——否则 storefront-only 用户在 landing 上被 deny →
        // 前端守卫重定向回 landing → 同 URL 自跳循环
        if path == landing || (!storefront_prefix.is_empty() && path.starts_with(storefront_prefix))
        {
            return PathAccess::Allow;
        }
        return PathAccess::Deny {
            redirect: landing.to_string(),
            reason: "STORE_ONLY_ACCESS".to_string(),
        };
    }
    // 策略 2：workbench 用户（含默认归属）→ 允许所有路径
    PathAccess::Allow
}

/// storefront 路径白名单前缀（部署配置 `STOREFRONT_PATH_PREFIX`，缺省 = 无白名单——
/// storefront-only 用户仅放行公开路径与 `/apps/{code}`（App OA 判定），其余 fail-closed）。
/// MUST NOT 硬编码模块路径字面量（shop 为休眠模块，全 ns 不存在）。
fn storefront_path_prefix() -> String {
    std::env::var("STOREFRONT_PATH_PREFIX")
        .unwrap_or_default()
        .trim()
        .to_string()
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

    // 查询用户的 NGAC 属性；DB 错误 → fail-closed 500
    // （MUST NOT 落空属性集进兜底分支——否则 DB 抖动即「全路径放行」）
    let attrs: Vec<String> = match sqlx::query_scalar(
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
    {
        Ok(attrs) => attrs,
        Err(e) => {
            log::error!(
                "check-access: NGAC attrs query failed for user {}: {}",
                user_id,
                e
            );
            return HttpResponse::InternalServerError().json(AuthError {
                error: "Internal error".to_string(),
            });
        }
    };

    // 门户归属推导走单一实现（portal.rs::derive_portal_scope）——扁平业务角色名不参与门户判定
    let portal_scope = crate::auth::portal::derive_portal_scope(&attrs);
    let path = &body.path;

    match decide_path_access(
        &portal_scope,
        path,
        &storefront_path_prefix(),
        &crate::auth::portal::storefront_landing_path(),
    ) {
        PathAccess::Allow => HttpResponse::Ok().json(CheckAccessResponse {
            allowed: true,
            redirect: None,
            reason: None,
        }),
        PathAccess::Deny { redirect, reason } => HttpResponse::Ok().json(CheckAccessResponse {
            allowed: false,
            redirect: Some(redirect),
            reason: Some(reason),
        }),
        // App 路径判定（add-app-visibility-ngac-isolation D4）：
        // /apps/{code}/... → App OA association decide；无 OA/无关联 → 拒绝（fail-closed）。
        // admin 豁免与 PDP list §6.2 同语义。
        PathAccess::AppRoute(code) => {
            if attrs.iter().any(|a| a == "admin") {
                return HttpResponse::Ok().json(CheckAccessResponse {
                    allowed: true,
                    redirect: None,
                    reason: None,
                });
            }
            let oa_id: Option<i64> = match sqlx::query_scalar(
                r#"SELECT fk_resource FROM isahl_auth.ngac_object_attribute
                   WHERE resource_type = 'app' AND resource_identifier = $1 AND deleted_at IS NULL
                   LIMIT 1"#,
            )
            .bind(&code)
            .fetch_optional(pool.get_ref())
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    // fail-closed：App OA 查询失败 MUST NOT 按「无 OA」拒绝或放行
                    log::error!(
                        "check-access: App OA lookup failed for code {}: {}",
                        code,
                        e
                    );
                    return HttpResponse::InternalServerError().json(AuthError {
                        error: "Internal error".to_string(),
                    });
                }
            };
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
            HttpResponse::Ok().json(CheckAccessResponse {
                allowed: permitted,
                redirect: if permitted {
                    None
                } else {
                    Some("/".to_string())
                },
                reason: if permitted {
                    None
                } else {
                    Some("APP_NOT_VISIBLE".to_string())
                },
            })
        }
    }
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/auth/check-access", web::post().to(check_access));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }
    const LANDING: &str = "/";
    const PREFIX: &str = "/modules/portal-x/";

    #[test]
    fn public_paths_allow_regardless() {
        for p in ["/auth/login", "/auth/register", "/auth/reset-password"] {
            assert_eq!(
                decide_path_access(&scope(&[]), p, PREFIX, LANDING),
                PathAccess::Allow
            );
        }
    }

    #[test]
    fn app_paths_delegate_to_oa_branch() {
        assert_eq!(
            decide_path_access(&scope(&[]), "/apps/wz-external/home", PREFIX, LANDING),
            PathAccess::AppRoute("wz-external".to_string())
        );
    }

    #[test]
    fn storefront_only_whitelisted_paths_allow() {
        assert_eq!(
            decide_path_access(
                &scope(&["storefront"]),
                "/modules/portal-x/home",
                PREFIX,
                LANDING,
            ),
            PathAccess::Allow
        );
    }

    #[test]
    fn storefront_only_other_paths_deny_with_landing_redirect() {
        assert_eq!(
            decide_path_access(
                &scope(&["storefront"]),
                "/modules/transport-wz/x",
                PREFIX,
                LANDING,
            ),
            PathAccess::Deny {
                redirect: "/".to_string(),
                reason: "STORE_ONLY_ACCESS".to_string(),
            }
        );
    }

    #[test]
    fn storefront_only_without_prefix_config_is_fail_closed() {
        assert_eq!(
            decide_path_access(&scope(&["storefront"]), "/anything", "", LANDING),
            PathAccess::Deny {
                redirect: "/".to_string(),
                reason: "STORE_ONLY_ACCESS".to_string(),
            }
        );
    }

    #[test]
    fn workbench_and_dual_scope_allow_all_paths() {
        for s in [
            scope(&["workbench"]),
            scope(&["workbench", "storefront"]),
            scope(&[]), // 默认归属 = workbench（规约策略 3）
        ] {
            assert_eq!(
                decide_path_access(&s, "/modules/transport-wz/x", PREFIX, LANDING),
                PathAccess::Allow
            );
        }
    }

    #[test]
    fn storefront_only_landing_path_itself_is_allowed() {
        // 回归：landing 被 deny 会让前端守卫自跳循环（重定向回 landing 再 deny）
        assert_eq!(
            decide_path_access(&scope(&["storefront"]), "/", "", "/"),
            PathAccess::Allow
        );
    }
}
