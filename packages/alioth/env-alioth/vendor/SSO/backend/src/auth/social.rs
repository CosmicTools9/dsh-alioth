//! 社交账号管理 handlers
//!
//! GET    /auth/social/accounts             — 列出当前用户已关联的 OAuth 账号
//! DELETE /auth/social/unlink/{providerId}  — 解绑当前用户某一 identity provider 的账号
//!
//! 数据来源：`isahl_auth.user_oauth_accounts`，经 `isahl_auth.identity_providers` 解析
//! provider 名称与类型。仅返回/操作当前 JWT 用户自身的关联账号（跨用户解绑被拒绝）。

use actix_web::{web, HttpRequest, HttpResponse};
use serde::Serialize;
use sqlx::FromRow;
use sqlx::PgPool;

use crate::auth::{extract_user_id, AuthState};

/// 与前端 `LinkedAccount` 接口对齐（camelCase）。
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LinkedAccount {
    pub provider_id: String,
    pub provider_name: String,
    pub provider_type: String,
    pub external_email: Option<String>,
    pub linked_at: String,
    pub icon: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, FromRow)]
struct OAuthRow {
    provider_id: i64,
    email: Option<String>,
    display_name: Option<String>,
    linked_at: chrono::DateTime<chrono::Utc>,
    provider_name: String,
}

fn to_linked_account(
    provider_id: i64,
    provider_name: &str,
    provider_type: &str,
    email: Option<String>,
    display_name: Option<String>,
    linked_at: chrono::DateTime<chrono::Utc>,
) -> LinkedAccount {
    LinkedAccount {
        provider_id: provider_id.to_string(),
        provider_name: provider_name.to_string(),
        provider_type: provider_type.to_string(),
        external_email: email.or(display_name),
        linked_at: linked_at.to_rfc3339(),
        icon: None,
        color: None,
    }
}

/// GET /auth/social/accounts
pub async fn list_social_accounts(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };

    let oauth_rows = sqlx::query_as::<_, OAuthRow>(
        r#"
        SELECT oa.provider_id, oa.email, oa.display_name, oa.created_at AS linked_at,
               ip.name AS provider_name
        FROM isahl_auth.user_oauth_accounts oa
        JOIN isahl_auth.identity_providers ip ON ip.id = oa.provider_id
        WHERE oa.user_id = $1
        ORDER BY oa.created_at DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(pool.get_ref())
    .await;

    let mut accounts: Vec<LinkedAccount> = Vec::new();

    match oauth_rows {
        Ok(rows) => {
            for r in rows {
                accounts.push(to_linked_account(
                    r.provider_id,
                    &r.provider_name,
                    "oauth",
                    r.email,
                    r.display_name,
                    r.linked_at,
                ));
            }
        }
        Err(e) => {
            log::error!("list_social_accounts (oauth): {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": "Failed to load linked accounts" }));
        }
    }

    HttpResponse::Ok().json(serde_json::json!({ "accounts": accounts }))
}

/// DELETE /auth/social/unlink/{providerId}
///
/// `providerId` 为 identity provider 的配置 id（BIGINT 字符串形式）。从
/// `user_oauth_accounts` 中删除属于当前用户且匹配该 provider 的行；禁止跨用户解绑。
pub async fn unlink_social_account(
    req: HttpRequest,
    path: web::Path<String>,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
) -> HttpResponse {
    let user_id = match extract_user_id(&req, &state) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };

    let provider_id: i64 = match path.into_inner().parse() {
        Ok(v) => v,
        Err(_) => {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({ "error": "Invalid provider id" }));
        }
    };

    let oa = sqlx::query(
        "DELETE FROM isahl_auth.user_oauth_accounts WHERE user_id = $1 AND provider_id = $2",
    )
    .bind(user_id)
    .bind(provider_id)
    .execute(pool.get_ref())
    .await;

    match oa {
        Ok(d) => {
            if d.rows_affected() > 0 {
                HttpResponse::NoContent().finish()
            } else {
                HttpResponse::NotFound()
                    .json(serde_json::json!({ "error": "Linked account not found" }))
            }
        }
        Err(e) => {
            log::error!("unlink_social_account: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": "Failed to unlink account" }))
        }
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/social")
            .route("/accounts", web::get().to(list_social_accounts))
            .route(
                "/unlink/{providerId}",
                web::delete().to(unlink_social_account),
            ),
    );
}
