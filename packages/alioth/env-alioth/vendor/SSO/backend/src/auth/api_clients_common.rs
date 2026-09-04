//! OpenAPI 调用方注册表共享逻辑（admin 管理面 + 用户自助面共用）
//!
//! 从 `admin/api_clients.rs` 提取的 leaf 函数与事务编排，两个消费面使用同一套
//! 密钥生成 / hash / 服务用户 / 默认订阅规则，避免双轨漂移（REUSE_FIRST_SPEC）。
//!
//! 对齐 `openspec/changes/add-openapi-external-access/` 与
//! `openspec/changes/openapi-self-service-portal/`：
//! 每个 client 创建时同步创建服务用户（`isahl_auth.auth_users`，user_type='service'），
//! 服务令牌经 Gateway PEP 解析为 `svc_user_id` 走 NGAC PDP 决策。

use base64::Engine;
use chrono::{DateTime, Utc};
use sqlx::Postgres;
use uuid::Uuid;

use super::client_secret::{hash_client_secret_async, ClientSecretError};
use super::service_user;

// ── 密钥生成（create 与 rotate-secret 共用同一规则）──────────────────────────────

/// 生成 API Key 明文：`ak_<base64url(32 bytes)>`（与旧 api_keys 格式一致）。
pub fn generate_api_key() -> String {
    let bytes: [u8; 32] = rand::random();
    format!(
        "ak_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

/// 生成 client_secret 明文（UUID v4，与旧 oidc_clients 一致）。
pub fn generate_client_secret() -> String {
    Uuid::new_v4().to_string()
}

/// 按 client_type 生成 client 凭证明文：
/// - apikey → `ak_<base64url(32 bytes)>`（client_id 即密钥明文）
/// - oauth2 → UUID v4
pub fn generate_secret_for(client_type: &str) -> String {
    match client_type {
        "apikey" => generate_api_key(),
        _ => generate_client_secret(),
    }
}

// ── 默认订阅（free 档位幂等补种）─────────────────────────────────────────────────

/// 创建默认订阅（free 套餐）。幂等：client 已有订阅则跳过。
pub async fn ensure_default_subscription<'e, A>(exec: A, client_id: i64) -> Result<(), sqlx::Error>
where
    A: sqlx::Acquire<'e, Database = Postgres>,
{
    let mut conn = exec.acquire().await?;
    let has_sub: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.api_subscriptions \
         WHERE fk_client = $1 AND deleted_at IS NULL)",
    )
    .bind(client_id)
    .fetch_one(&mut *conn)
    .await
    .unwrap_or(false);
    if has_sub {
        return Ok(());
    }
    // free 档位缺省时幂等补种（否则首个 client 创建必败）；
    // id 走 isahl.gen_next_zuid()（isahl_auth 链规则），其余列用表默认值（quota=0 不限）。
    let plan_id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO isahl_auth.api_plans (id, code) \
         SELECT isahl.gen_next_zuid(), 'free' \
         WHERE NOT EXISTS (SELECT 1 FROM isahl_auth.api_plans WHERE code = 'free' AND deleted_at IS NULL) \
         RETURNING id",
    )
    .fetch_optional(&mut *conn)
    .await?;
    let plan_id: i64 = match plan_id {
        Some(id) => id,
        None => {
            sqlx::query_scalar(
                "SELECT id FROM isahl_auth.api_plans WHERE code = 'free' AND enabled AND deleted_at IS NULL",
            )
            .fetch_one(&mut *conn)
            .await?
        }
    };
    sqlx::query(
        "INSERT INTO isahl_auth.api_subscriptions (id, fk_client, fk_plan, status) \
         VALUES (isahl.gen_next_zuid(), $1, $2, 'active')",
    )
    .bind(client_id)
    .bind(plan_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

// ── 创建（事务：唯一性预检 → hash → 服务用户 → insert → 默认订阅）──────────────

#[derive(Debug)]
pub struct CreatedApiClient {
    pub id: i64,
    pub fk_service_user: i64,
}

#[derive(Debug)]
pub enum CreateClientError {
    /// client_id 已被占用（未软删）
    ClientIdTaken,
    /// argon2id hash 失败
    Hash(ClientSecretError),
    /// 开启事务失败
    Begin(sqlx::Error),
    /// 服务用户创建失败
    ServiceUser(sqlx::Error),
    /// client insert 失败（并发唯一冲突等）
    Insert(sqlx::Error),
    /// 默认订阅创建失败
    Subscription(sqlx::Error),
    /// 事务提交失败
    Commit(sqlx::Error),
}

/// 创建 OpenAPI 调用方：同步创建服务用户（NGAC 主体）+ 默认 free 订阅，
/// 全部在单个事务内完成（失败回滚，不留孤儿服务用户）。
///
/// `secret` 为明文凭据，argon2id 哈希后落库；明文由调用方在响应中仅返回一次。
pub async fn create_api_client(
    pool: &sqlx::PgPool,
    client_id: String,
    client_type: &str,
    client_name: &str,
    secret: String,
    scopes: Vec<String>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<CreatedApiClient, CreateClientError> {
    // 唯一性预检（INSERT 前先给友好 409；并发冲突由 Insert 分支兜底）
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.api_clients \
         WHERE client_id = $1 AND deleted_at IS NULL)",
    )
    .bind(&client_id)
    .fetch_one(pool)
    .await
    .map_err(CreateClientError::Begin)?;
    if exists {
        return Err(CreateClientError::ClientIdTaken);
    }

    let secret_hash = hash_client_secret_async(secret)
        .await
        .map_err(CreateClientError::Hash)?;

    let mut tx = pool.begin().await.map_err(CreateClientError::Begin)?;

    let svc_user_id = service_user::ensure_service_user(&mut *tx, &client_id, client_name)
        .await
        .map_err(CreateClientError::ServiceUser)?;

    let row: (i64,) = sqlx::query_as(
        r#"
        INSERT INTO isahl_auth.api_clients
            (id, client_id, client_type, client_name, secret_hash, scopes, fk_service_user, enabled, expires_at)
        VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5::TEXT[], $6, TRUE, $7)
        RETURNING id
        "#,
    )
    .bind(&client_id)
    .bind(client_type)
    .bind(client_name)
    .bind(&secret_hash)
    .bind(&scopes)
    .bind(svc_user_id)
    .bind(expires_at)
    .fetch_one(&mut *tx)
    .await
    .map_err(CreateClientError::Insert)?;

    ensure_default_subscription(&mut *tx, row.0)
        .await
        .map_err(CreateClientError::Subscription)?;

    tx.commit().await.map_err(CreateClientError::Commit)?;

    Ok(CreatedApiClient {
        id: row.0,
        fk_service_user: svc_user_id,
    })
}

// ── 轮换 / 吊销 ─────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct RotatedSecret {
    pub client_id: String,
    pub secret: String,
}

#[derive(Debug)]
pub enum ClientOpError {
    /// argon2id hash 失败
    Hash(ClientSecretError),
    /// DB 错误
    Db(sqlx::Error),
}

impl From<sqlx::Error> for ClientOpError {
    fn from(e: sqlx::Error) -> Self {
        ClientOpError::Db(e)
    }
}

impl From<ClientSecretError> for ClientOpError {
    fn from(e: ClientSecretError) -> Self {
        ClientOpError::Hash(e)
    }
}

/// 轮换调用方密钥（openapi-client-secret-rotation）：
/// - apikey 型 client_id 即密钥明文（auth 按 `left(client_id, 8)` 前缀索引），
///   轮换时必须同步替换 client_id，否则新密钥无法通过前缀定位；
/// - oauth2 型 client_id 为稳定标识，仅覆盖 secret_hash。
///
/// 明文仅在此返回一次；旧 secret 立即失效（哈希已被覆盖）。
/// 返回 `Ok(None)` 表示 client 不存在或已软删。
pub async fn rotate_client_secret<'e, A>(
    exec: A,
    client_row_id: i64,
) -> Result<Option<RotatedSecret>, ClientOpError>
where
    A: sqlx::Acquire<'e, Database = Postgres>,
{
    let mut conn = exec.acquire().await?;

    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT client_type, client_id FROM isahl_auth.api_clients \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(client_row_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((client_type, old_client_id)) = row else {
        return Ok(None);
    };

    let secret = generate_secret_for(&client_type);
    let secret_hash = hash_client_secret_async(secret.clone()).await?;

    let new_client_id = if client_type == "apikey" {
        secret.clone()
    } else {
        old_client_id
    };
    let updated = if client_type == "apikey" {
        sqlx::query(
            "UPDATE isahl_auth.api_clients SET client_id = $1, secret_hash = $2 \
             WHERE id = $3 AND deleted_at IS NULL",
        )
        .bind(&new_client_id)
        .bind(&secret_hash)
        .bind(client_row_id)
        .execute(&mut *conn)
        .await
    } else {
        sqlx::query(
            "UPDATE isahl_auth.api_clients SET secret_hash = $1 \
             WHERE id = $2 AND deleted_at IS NULL",
        )
        .bind(&secret_hash)
        .bind(client_row_id)
        .execute(&mut *conn)
        .await
    };
    match updated {
        Ok(r) if r.rows_affected() > 0 => Ok(Some(RotatedSecret {
            client_id: new_client_id,
            secret,
        })),
        Ok(_) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// 吊销（软删除）client，并同步挂起其 active 订阅。
/// 服务用户保留（历史审计可解析），不再可签发令牌。
/// 返回受影响行数（0 = 不存在或已软删）。
pub async fn soft_delete_client<'e, A>(exec: A, client_row_id: i64) -> Result<u64, sqlx::Error>
where
    A: sqlx::Acquire<'e, Database = Postgres>,
{
    let mut conn = exec.acquire().await?;
    let updated = sqlx::query(
        "UPDATE isahl_auth.api_clients SET deleted_at = NOW(), enabled = FALSE \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(client_row_id)
    .execute(&mut *conn)
    .await?;
    if updated.rows_affected() > 0 {
        sqlx::query(
            "UPDATE isahl_auth.api_subscriptions SET status = 'canceled', deleted_at = NOW() \
             WHERE fk_client = $1 AND deleted_at IS NULL",
        )
        .bind(client_row_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(updated.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_format() {
        let k = generate_api_key();
        assert!(k.starts_with("ak_"), "API Key 必须以 ak_ 开头");
        assert_eq!(k.len(), 3 + 43, "ak_ + 32 bytes base64url 无填充 = 43 字符");
    }

    #[test]
    fn client_secret_format() {
        let s = generate_client_secret();
        assert_eq!(s.len(), 36, "UUID v4 格式");
    }

    #[test]
    fn rotate_secret_rules_match_create() {
        let apikey = generate_secret_for("apikey");
        assert!(apikey.starts_with("ak_"), "apikey 明文必须以 ak_ 开头");
        assert_eq!(
            apikey.len(),
            3 + 43,
            "ak_ + 32 bytes base64url 无填充 = 43 字符"
        );
        let oauth2 = generate_secret_for("oauth2");
        assert_eq!(oauth2.len(), 36, "UUID v4 格式");
        assert_eq!(
            oauth2.chars().filter(|c| *c == '-').count(),
            4,
            "UUID 含 4 个连字符"
        );
        assert_ne!(generate_secret_for("apikey"), apikey, "轮换必须生成新密钥");
        assert_ne!(generate_secret_for("oauth2"), oauth2, "轮换必须生成新密钥");
    }
}
