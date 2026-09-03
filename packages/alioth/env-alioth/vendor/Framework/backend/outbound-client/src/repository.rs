//! 出向调用方注册表与计量数据访问（isahl_auth.outbound_*）。

use serde_json::{json, Value};
use sqlx::PgPool;

use common::AliothError as ApiError;

/// 出向调用方数据访问
#[derive(Clone)]
pub struct OutboundRepository {
    pool: PgPool,
}

impl OutboundRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 列出出向调用方配置（不返回 app_secret 明文）
    pub async fn list_clients(&self) -> Result<Vec<Value>, ApiError> {
        let rows = sqlx::query_as::<
            _,
            (
                i64,
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                bool,
                i32,
            ),
        >(
            "SELECT id, code, provider, base_url, app_id, tenant_id, account_id, enabled, version \
             FROM isahl_auth.outbound_client WHERE deleted_at IS NULL \
             ORDER BY code",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(ApiError::from_sqlx)?;
        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    code,
                    provider,
                    base_url,
                    app_id,
                    tenant_id,
                    account_id,
                    enabled,
                    version,
                )| {
                    json!({
                        "id": id.to_string(),
                        "code": code,
                        "provider": provider,
                        "baseUrl": base_url,
                        "appId": app_id,
                        "tenantId": tenant_id,
                        "accountId": account_id,
                        "hasSecret": true,
                        "enabled": enabled,
                        "version": version,
                    })
                },
            )
            .collect())
    }

    /// 创建出向调用方（app_secret 密文入库）；返回新 id
    #[allow(clippy::too_many_arguments)] // 客户端字段为领域固定维度
    pub async fn create_client(
        &self,
        code: &str,
        provider: &str,
        base_url: &str,
        app_id: &str,
        app_secret_enc: &str,
        tenant_id: &str,
        account_id: &str,
    ) -> Result<i64, ApiError> {
        let id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO isahl_auth.outbound_client \
             (code, provider, base_url, app_id, app_secret_enc, tenant_id, account_id, enabled, version) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, TRUE, 1) \
             RETURNING id",
        )
        .bind(code)
        .bind(provider)
        .bind(base_url)
        .bind(app_id)
        .bind(app_secret_enc)
        .bind(tenant_id)
        .bind(account_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from_sqlx)?;
        Ok(id)
    }

    /// 更新出向调用方（凭据轮换 version+1 + 新密文；其余字段可部分更新）
    #[allow(clippy::too_many_arguments)] // 客户端字段为领域固定维度
    pub async fn update_client(
        &self,
        id: i64,
        base_url: Option<&str>,
        app_id: Option<&str>,
        app_secret_enc: Option<&str>,
        tenant_id: Option<&str>,
        account_id: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<(), ApiError> {
        if app_secret_enc.is_some() {
            sqlx::query(
                "UPDATE isahl_auth.outbound_client SET version = version + 1, updated_at = NOW() \
                 WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(ApiError::from_sqlx)?;
        }
        if let Some(v) = base_url {
            sqlx::query("UPDATE isahl_auth.outbound_client SET base_url = $1, updated_at = NOW() WHERE id = $2 AND deleted_at IS NULL")
                .bind(v).bind(id).execute(&self.pool).await.map_err(ApiError::from_sqlx)?;
        }
        if let Some(v) = app_id {
            sqlx::query("UPDATE isahl_auth.outbound_client SET app_id = $1, updated_at = NOW() WHERE id = $2 AND deleted_at IS NULL")
                .bind(v).bind(id).execute(&self.pool).await.map_err(ApiError::from_sqlx)?;
        }
        if let Some(v) = app_secret_enc {
            sqlx::query("UPDATE isahl_auth.outbound_client SET app_secret_enc = $1, updated_at = NOW() WHERE id = $2 AND deleted_at IS NULL")
                .bind(v).bind(id).execute(&self.pool).await.map_err(ApiError::from_sqlx)?;
        }
        if let Some(v) = tenant_id {
            sqlx::query("UPDATE isahl_auth.outbound_client SET tenant_id = $1, updated_at = NOW() WHERE id = $2 AND deleted_at IS NULL")
                .bind(v).bind(id).execute(&self.pool).await.map_err(ApiError::from_sqlx)?;
        }
        if let Some(v) = account_id {
            sqlx::query("UPDATE isahl_auth.outbound_client SET account_id = $1, updated_at = NOW() WHERE id = $2 AND deleted_at IS NULL")
                .bind(v).bind(id).execute(&self.pool).await.map_err(ApiError::from_sqlx)?;
        }
        if let Some(v) = enabled {
            sqlx::query("UPDATE isahl_auth.outbound_client SET enabled = $1, updated_at = NOW() WHERE id = $2 AND deleted_at IS NULL")
                .bind(v).bind(id).execute(&self.pool).await.map_err(ApiError::from_sqlx)?;
        }
        Ok(())
    }

    /// 软删出向调用方
    pub async fn delete_client(&self, id: i64) -> Result<(), ApiError> {
        let affected = sqlx::query(
            "UPDATE isahl_auth.outbound_client SET deleted_at = NOW(), enabled = FALSE \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(ApiError::from_sqlx)?
        .rows_affected();
        if affected == 0 {
            return Err(ApiError::NotFound(format!("出向调用方不存在: {}", id)));
        }
        Ok(())
    }
}
