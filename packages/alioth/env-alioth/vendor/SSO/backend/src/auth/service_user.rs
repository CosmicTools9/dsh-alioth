//! 服务用户（Service User）— OpenAPI 服务令牌的 NGAC 主体
//!
//! 每个 `api_clients` 调用方对应一个 `isahl_auth.auth_users` 服务用户行
//! （`user_type='service'`，不可登录、无密码）。服务令牌（client_credentials /
//! API Key）经 Gateway PEP 解析为 `svc_user_id`，以该服务用户身份走 NGAC PDP
//! 决策（openspec/changes/add-openapi-external-access/）。
//!
//! `ensure_service_user` 幂等：client_id 已存在则返回既有服务用户 id。

/// 服务用户命名：`svc-<client_id>`（auth_users.name 唯一约束保证幂等锚点）。
pub fn service_user_name(client_id: &str) -> String {
    format!("svc-{}", client_id)
}

/// 服务用户唯一用户名：`svc:<client_id>`（auth_users.username 唯一约束）。
pub fn service_user_username(client_id: &str) -> String {
    format!("svc:{}", client_id)
}

/// 确保服务用户存在，返回其 auth_users.id（幂等）。
///
/// 服务用户属性：
/// - `user_type='service'`：与自然人/系统用户区分，禁止交互登录
/// - 无密码、无 email（email 唯一约束下服务用户不宜占用邮箱）
/// - `is_active=true`、`status='active'`
///
/// 泛型 Acquire：调用方既可传 `&PgPool`（独立调用），也可传 `&mut *tx`
/// （纳入外部事务——create_api_client 的服务用户+client+订阅须同生共死）。
pub async fn ensure_service_user<'e, A>(
    exec: A,
    client_id: &str,
    client_name: &str,
) -> Result<i64, sqlx::Error>
where
    A: sqlx::Acquire<'e, Database = sqlx::Postgres>,
{
    let mut conn = exec.acquire().await?;
    // 幂等锚点：按 username 查找既有服务用户
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE username = $1 AND user_type = 'service'",
    )
    .bind(service_user_username(client_id))
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(id) = existing {
        return Ok(id);
    }

    // 创建服务用户（name/username 双唯一约束；email 置 NULL 避免占用邮箱）
    let row: (i64,) = sqlx::query_as(
        r#"INSERT INTO isahl_auth.auth_users
           (name, username, email, password_hash, user_type, is_active, status,
            display_name, created_at, updated_at, failed_login_attempts, notification_preferences)
           VALUES ($1, $2, NULL, NULL, 'service', TRUE, 'active',
                   $3, NOW(), NOW(), 0, '{}'::jsonb)
           RETURNING id"#,
    )
    .bind(service_user_name(client_id))
    .bind(service_user_username(client_id))
    .bind(client_name)
    .fetch_one(&mut *conn)
    .await?;

    Ok(row.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_user_names_unique_shape() {
        assert_eq!(service_user_name("partner-a"), "svc-partner-a");
        assert_eq!(service_user_username("partner-a"), "svc:partner-a");
        // 不同 client_id 不产生同名冲突
        assert_ne!(service_user_name("a"), service_user_name("b"));
    }
}
