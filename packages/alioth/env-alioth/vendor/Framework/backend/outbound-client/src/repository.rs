//! 出向调用方注册表与凭据数据访问（`isahl_auth.outbound_client`）。
//!
//! 列集为 **provider 中立**形态（`outbound-provider-abstraction`）：`code`（行键）、
//! `provider`（派发键）、`auth_type`、`base_url`、`config`（jsonb，provider 私有配置）、
//! `app_secret_enc`（密文）。单 provider 的字段 MUST 落在 `config` 内——本模块是唯一
//! 读写该表的实现面。

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

    /// 列出出向调用方配置（不返回密文，仅 `hasSecret` 标记）
    pub async fn list_clients(&self) -> Result<Vec<Value>, ApiError> {
        let rows = sqlx::query_as::<
            _,
            (
                i64,
                String,
                String,
                String,
                Option<String>,
                Value,
                bool,
                bool,
                i32,
            ),
        >(
            "SELECT id, code, provider, auth_type, base_url, config, \
                    (app_secret_enc IS NOT NULL AND app_secret_enc <> '') AS has_secret, \
                    enabled, version \
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
                    auth_type,
                    base_url,
                    config,
                    has_secret,
                    enabled,
                    version,
                )| {
                    json!({
                        "id": id.to_string(),
                        "code": code,
                        "provider": provider,
                        "authType": auth_type,
                        "baseUrl": base_url,
                        "config": config,
                        "hasSecret": has_secret,
                        "enabled": enabled,
                        "version": version,
                    })
                },
            )
            .collect())
    }

    /// 取某 provider 的首个 enabled 行 code（无 ⇒ `None`）。
    ///
    /// 供消费方在只知 provider 代号（如 `wechat-pay`）时定位注册行——消费方 MUST NOT
    /// 自行写注册表 SQL（凭据/注册表读写唯一实现 = 本模块）。
    pub async fn first_enabled_code_for_provider(
        &self,
        provider_id: &str,
    ) -> Result<Option<String>, ApiError> {
        sqlx::query_scalar(
            "SELECT code FROM isahl_auth.outbound_client \
             WHERE provider = $1 AND enabled = TRUE AND deleted_at IS NULL \
             ORDER BY code LIMIT 1",
        )
        .bind(provider_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(ApiError::from_sqlx)
    }

    /// 取某注册行的 `(provider, auth_type)`（行不存在/已软删 ⇒ `None`）。
    ///
    /// 供管理面**保存期成员校验**读取「载荷未覆盖的维度」：部分更新时生效值 = 载荷值 ?? 本行值。
    /// 消费方 MUST NOT 自行写注册表 SQL（注册表读写唯一实现 = 本模块）。
    pub async fn provider_auth_for_id(
        &self,
        id: i64,
    ) -> Result<Option<(String, String)>, ApiError> {
        sqlx::query_as(
            "SELECT provider, auth_type FROM isahl_auth.outbound_client \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(ApiError::from_sqlx)
    }

    /// 创建出向调用方（凭据密文可空——`auth_type='none'` 的 provider 无凭据）；
    /// `code` 已存在（含软删行）⇒ 409。
    pub async fn create_client(
        &self,
        code: &str,
        provider: &str,
        auth_type: &str,
        base_url: &str,
        config: &Value,
        app_secret_enc: Option<&str>,
    ) -> Result<i64, ApiError> {
        let id: Option<i64> = sqlx::query_scalar(
            "INSERT INTO isahl_auth.outbound_client \
             (code, provider, auth_type, base_url, config, app_secret_enc, enabled, version) \
             VALUES ($1, $2, $3, $4, $5, $6, TRUE, 1) \
             ON CONFLICT (code) DO NOTHING \
             RETURNING id",
        )
        .bind(code)
        .bind(provider)
        .bind(auth_type)
        .bind(base_url)
        .bind(config)
        .bind(app_secret_enc)
        .fetch_optional(&self.pool)
        .await
        .map_err(ApiError::from_sqlx)?;
        id.ok_or_else(|| ApiError::Conflict(format!("出向调用方 code 已存在: {code}")))
    }

    /// 更新出向调用方（部分更新；提供密文时 `version+1` 表示凭据轮换）。
    ///
    /// 行不存在/已软删 ⇒ 404。
    #[allow(clippy::too_many_arguments)] // 部分更新字段为注册表固定维度
    pub async fn update_client(
        &self,
        id: i64,
        provider: Option<&str>,
        auth_type: Option<&str>,
        base_url: Option<&str>,
        config: Option<&Value>,
        app_secret_enc: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<(), ApiError> {
        let rotated = app_secret_enc.is_some();
        let affected = sqlx::query(
            "UPDATE isahl_auth.outbound_client SET \
                provider = COALESCE($2, provider), \
                auth_type = COALESCE($3, auth_type), \
                base_url = COALESCE($4, base_url), \
                config = COALESCE($5, config), \
                app_secret_enc = COALESCE($6, app_secret_enc), \
                enabled = COALESCE($7, enabled), \
                version = version + CASE WHEN $8 THEN 1 ELSE 0 END, \
                updated_at = NOW() \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .bind(provider)
        .bind(auth_type)
        .bind(base_url)
        .bind(config)
        .bind(app_secret_enc)
        .bind(enabled)
        .bind(rotated)
        .execute(&self.pool)
        .await
        .map_err(ApiError::from_sqlx)?
        .rows_affected();
        if affected == 0 {
            return Err(ApiError::NotFound(format!("出向调用方不存在: {id}")));
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
