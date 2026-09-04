//! SCIM 2.0 端点实现（RFC 7642-7644）
//!
//! 路由前缀 `/scim/v2`（见 `mod.rs`）。认证由静态 Bearer token（`SCIM_BEARER_TOKEN`
//! 环境变量）保护——在每个 handler 入口经 `require_scim_token` 校验，缺失/错误即 401。
//!
//! M26 结构性拆分：按业务域拆为子模块（纯重构，公开路径 `scim::handlers::*` 不变）：
//! - `users`：用户行映射 + 用户 CRUD
//! - `groups`：组 CRUD + 组内部读取
//! - `config`：静态配置端点（ServiceProviderConfig / Schemas / ResourceTypes）

use actix_web::{HttpRequest, HttpResponse};
use sqlx::PgPool;

use super::models::*;

mod config;
mod groups;
mod users;

// Re-export：保持 `scim::handlers::<name>` 与 `scim/mod.rs` 路由表引用路径稳定。
pub use config::*;
pub use groups::*;
pub use users::*;

#[cfg(test)]
use actix_web::web;

/// 解析默认 NGAC policy class（`o_name = 'default'`）的 id。
/// 不硬编码 id：policy class 主键由 `isahl.gen_next_zuid()` 动态生成，
/// 固定值（如 1）必然 FK 违规。
async fn default_policy_class_id(pool: &PgPool) -> Result<i64, String> {
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("query default policy class: {}", e))?
    .ok_or_else(|| "default NGAC policy class not seeded".to_string())
}

/// 校验 `Authorization: Bearer <SCIM_BEARER_TOKEN>`。失败返回 Err(401 HttpResponse)。
fn require_scim_token(req: &HttpRequest) -> Result<(), HttpResponse> {
    let expected = match std::env::var("SCIM_BEARER_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            // 未配置 token 时拒绝所有 SCIM 请求（fail-closed），避免未授权访问。
            log::error!("SCIM: SCIM_BEARER_TOKEN not configured; rejecting all SCIM requests");
            return Err(error_response(
                actix_web::http::StatusCode::UNAUTHORIZED,
                "SCIM endpoint is not enabled (SCIM_BEARER_TOKEN unset)",
            ));
        }
    };

    let header = req
        .headers()
        .get(actix_web::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    match header {
        Some(h) if h == format!("Bearer {}", expected) => Ok(()),
        _ => Err(error_response(
            actix_web::http::StatusCode::UNAUTHORIZED,
            "Missing or invalid SCIM bearer token",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::http::header::AUTHORIZATION;
    use sqlx::PgPool;
    use std::collections::HashMap;

    const SCIM_TOKEN: &str = "test-scim-token";

    async fn test_pool() -> PgPool {
        let url = std::env::var("DATABASE_URL")
            .or_else(|_| std::env::var("SSO_TEST_DATABASE_URL"))
            .unwrap_or_else(|_| {
                let user = std::env::var("USER").unwrap_or_else(|_| "william.d.zk".to_string());
                format!("postgres://{}@localhost:5432/aliothstudio_test", user)
            });
        PgPool::connect(&url)
            .await
            .expect("无法连接测试库，请先运行 `bash scripts/db/reset-db.sh --test`")
    }

    fn auth_req() -> HttpRequest {
        actix_web::test::TestRequest::default()
            .insert_header((AUTHORIZATION, format!("Bearer {}", SCIM_TOKEN)))
            .to_http_parts()
            .0
    }

    fn noauth_req() -> HttpRequest {
        actix_web::test::TestRequest::default().to_http_parts().0
    }

    async fn body_to_json(resp: HttpResponse) -> serde_json::Value {
        use actix_web::body::to_bytes;
        let bytes = to_bytes(resp.into_body()).await.expect("read body");
        serde_json::from_slice(&bytes).expect("parse json")
    }

    fn unique_email(prefix: &str) -> String {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{}@{}.example.com", prefix, n)
    }

    async fn cleanup_user(pool: &PgPool, id: i64) {
        let _ = sqlx::query(
            "UPDATE isahl_auth.auth_users SET is_active = false, status = 'disabled' WHERE id = $1",
        )
        .bind(id)
        .execute(pool)
        .await;
    }

    async fn cleanup_group(pool: &PgPool, id: i64) {
        let _ = sqlx::query(
            "UPDATE isahl_auth.ngac_user_attribute SET deleted_at = NOW() WHERE id = $1",
        )
        .bind(id)
        .execute(pool)
        .await;
    }

    #[tokio::test]
    async fn invalid_token_is_rejected() {
        std::env::set_var("SCIM_BEARER_TOKEN", SCIM_TOKEN);
        let pool = test_pool().await;
        let resp = list_users(
            noauth_req(),
            web::Data::new(pool),
            web::Query(HashMap::new()),
        )
        .await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn full_user_lifecycle() {
        std::env::set_var("SCIM_BEARER_TOKEN", SCIM_TOKEN);
        let pool = test_pool().await;
        let email = unique_email("scim_life");

        // CREATE
        let create = ScimUser {
            user_name: Some(email.clone()),
            display_name: Some("Alice Life".to_string()),
            active: Some(true),
            external_id: Some("idp-001".to_string()),
            ..Default::default()
        };
        let resp = create_user(auth_req(), web::Data::new(pool.clone()), web::Json(create)).await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::CREATED);
        let json = body_to_json(resp).await;
        assert_eq!(json["userName"], serde_json::json!(email));
        assert_eq!(json["active"], serde_json::json!(true));
        let id = json["id"].as_str().unwrap().to_string();

        // GET
        let resp = get_user(
            auth_req(),
            web::Data::new(pool.clone()),
            web::Path::from(id.clone()),
        )
        .await;
        let json = body_to_json(resp).await;
        assert_eq!(json["displayName"], serde_json::json!("Alice Life"));
        assert_eq!(json["externalId"], serde_json::json!("idp-001"));

        // REPLACE (PUT) — disable
        let replace = ScimUser {
            user_name: Some(email.clone()),
            active: Some(false),
            ..Default::default()
        };
        let resp = replace_user(
            auth_req(),
            web::Data::new(pool.clone()),
            web::Path::from(id.clone()),
            web::Json(replace),
        )
        .await;
        let json = body_to_json(resp).await;
        assert_eq!(json["active"], serde_json::json!(false));

        // PATCH — set displayName
        let patch = ScimPatchRequest {
            schemas: Some(vec![
                "urn:ietf:params:scim:api:messages:2.0:PatchOp".to_string()
            ]),
            Operations: vec![PatchOperation {
                op: "replace".to_string(),
                path: Some("displayName".to_string()),
                value: Some(serde_json::json!("Alice Patched")),
            }],
        };
        let resp = patch_user(
            auth_req(),
            web::Data::new(pool.clone()),
            web::Path::from(id.clone()),
            web::Json(patch),
        )
        .await;
        let json = body_to_json(resp).await;
        assert_eq!(json["displayName"], serde_json::json!("Alice Patched"));

        // DELETE (soft-disable)
        let resp = delete_user(
            auth_req(),
            web::Data::new(pool.clone()),
            web::Path::from(id.clone()),
        )
        .await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::NO_CONTENT);

        // verify disabled in DB
        let status: (Option<String>, Option<bool>) =
            sqlx::query_as("SELECT status, is_active FROM isahl_auth.auth_users WHERE id = $1")
                .bind(id.parse::<i64>().unwrap())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status.0.as_deref(), Some("disabled"));
        assert_eq!(status.1, Some(false));

        cleanup_user(&pool, id.parse().unwrap()).await;
    }

    #[tokio::test]
    async fn group_membership_syncs_to_ngac() {
        std::env::set_var("SCIM_BEARER_TOKEN", SCIM_TOKEN);
        let pool = test_pool().await;
        let email = unique_email("scim_grp");

        // 先创建一个用户
        let create = ScimUser {
            user_name: Some(email.clone()),
            active: Some(true),
            ..Default::default()
        };
        let resp = create_user(auth_req(), web::Data::new(pool.clone()), web::Json(create)).await;
        let json = body_to_json(resp).await;
        let user_id: i64 = json["id"].as_str().unwrap().parse().unwrap();

        // 创建 group 并指派该用户为成员
        let group_name = format!(
            "Engineering-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let group = ScimGroup {
            display_name: Some(group_name.clone()),
            members: Some(vec![ScimMemberRef {
                member_type: Some("User".to_string()),
                ref_: Some("/scim/v2/Users/".to_string()),
                value: Some(user_id.to_string()),
                display: None,
            }]),
            ..Default::default()
        };
        let resp = create_group(auth_req(), web::Data::new(pool.clone()), web::Json(group)).await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::CREATED);
        let json = body_to_json(resp).await;
        let group_id: i64 = json["id"].as_str().unwrap().parse().unwrap();
        assert_eq!(json["displayName"], serde_json::json!(group_name));
        assert_eq!(
            json["members"][0]["value"],
            serde_json::json!(user_id.to_string())
        );

        // 验证 NGAC 关联行存在
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM isahl_auth.ngac_user_rr_attribute \
             WHERE fk_user = $1 AND fk_user_attribute = $2 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(group_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count.0, 1);

        cleanup_group(&pool, group_id).await;
        cleanup_user(&pool, user_id).await;
    }
}
