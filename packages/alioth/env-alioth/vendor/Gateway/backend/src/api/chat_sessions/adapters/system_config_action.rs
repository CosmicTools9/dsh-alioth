//! 系统设置动作（add-system-config-ai-action 2.3）。
//!
//! 通道：HTTP `POST /api/chat-sessions/{sid}/execute-action`（confirmed 来自用户
//! 点击）→ `GatewayActionHandler::run_action("system_config_apply")` → 本模块。
//! 提案来源：ai-agent `system_config_draft` 工具（只出 `needs_confirmation`，零写入）。
//!
//! 门控：`common::permissions::is_admin_user`（与 `require_admin` 同一判据实现）。
//! 写入：`system-config` 服务层（`SystemConfigService<SystemConfigRepo>`——同 crate
//! 仓库实现，凭证按 schema 敏感键加密）；**审计等价复刻**活规约
//! `system-config::config-change-audit`（operation/object_path/metadata 同形、
//! subject 走 `common::audit::resolve_subject`、`created_by_id`/`updated_by_id`
//! 经服务层 actor_id 回填）。逐项执行、失败即停并明示已完成项（不自动回滚）。
//!
//! 凭证：`credentials` 仅专属键承载（parse 层非对象拒绝）；**动作回执/metadata/
//! 日志 MUST NOT 回显凭证值**（回执仅 `{id, category, code, provider}` 概要 +
//! `credentials_set` 布尔）。

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::PgPool;
use system_config::operation::{audit_write, parse_operation, ParsedOperation};
use system_config::service::SystemConfigService;

use crate::system_config_repo::SystemConfigRepo;

use super::apply_protocol::{apply_operations, gate_admin, ApplyCtx, ItemExecutor};

/// 动作入口：admin 门控 + 逐项执行（协议实现单源 = `apply_protocol`）。
pub async fn apply(pool: &PgPool, user_id: i64, params: &Value) -> Result<Value, String> {
    let gate = gate_admin(pool, user_id, "系统设置变更").await;
    let service = SystemConfigService::new(SystemConfigRepo::new(pool.clone()));
    apply_operations(
        pool,
        user_id,
        gate,
        params,
        SystemConfigExecutor { service: &service },
    )
    .await
}

/// 单项执行体（解析 → 服务层 → 审计口径等价复刻；错误文本保留可行动语义）。
struct SystemConfigExecutor<'a> {
    service: &'a SystemConfigService<SystemConfigRepo>,
}

#[async_trait]
impl ItemExecutor for SystemConfigExecutor<'_> {
    async fn apply_one(&self, op: &Value, ctx: &ApplyCtx<'_>) -> Result<Value, String> {
        let service = self.service;
        let pool = ctx.pool;
        let uid = ctx.user_id;
        let subject = ctx.subject().await;
        let subject = subject.as_str();
        let email = ctx.email.as_str();
        let (name, parsed) = parse_operation(op)?;
        let summary = op
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or(&name)
            .to_string();
        match parsed {
            ParsedOperation::Create(input) => {
                let row = service
                    .create(input, Some(uid))
                    .await
                    .map_err(|e| e.to_string())?;
                audit_write(
                pool,
                uid,
                subject,
                "system_config.create",
                &format!(
                    "system_config/{}/{}",
                    row._f_.as_deref().unwrap_or_default(),
                    row.id
                ),
                json!({ "category": row._f_, "code": row.code, "provider": row._t_, "email": email }),
            )
            .await;
                Ok(json!({
                    "op": name, "id": row.id, "category": row._f_, "code": row.code,
                    "provider": row._t_, "credentials_set": row.credentials_set, "summary": summary
                }))
            }
            ParsedOperation::Update { id, input } => {
                let row = service
                    .update(id, input, Some(uid))
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("TARGET_NOT_FOUND: system_config {id} 不存在"))?;
                audit_write(
                pool,
                uid,
                subject,
                "system_config.update",
                &format!(
                    "system_config/{}/{}",
                    row._f_.as_deref().unwrap_or_default(),
                    id
                ),
                json!({ "category": row._f_, "code": row.code, "provider": row._t_, "email": email }),
            )
            .await;
                Ok(json!({
                    "op": name, "id": row.id, "category": row._f_, "code": row.code,
                    "provider": row._t_, "credentials_set": row.credentials_set, "summary": summary
                }))
            }
            ParsedOperation::Delete { id } => {
                let rows = service.delete(id).await.map_err(|e| e.to_string())?;
                if rows == 0 {
                    return Err(format!("TARGET_NOT_FOUND: system_config {id} 不存在"));
                }
                audit_write(
                    pool,
                    uid,
                    subject,
                    "system_config.delete",
                    &format!("system_config/{id}"),
                    json!({ "id": id.to_string(), "email": email }),
                )
                .await;
                Ok(json!({ "op": name, "id": id.to_string(), "summary": summary }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_create_defaults_and_split_channels() {
        let (name, parsed) = parse_operation(&json!({
            "op": "create_config", "category": "llm", "provider": "deepseek",
            "settings": { "model": "deepseek-v4" },
            "credentials": { "api_key": "sk-x" }
        }))
        .unwrap();
        assert_eq!(name, "create_config");
        match parsed {
            ParsedOperation::Create(input) => {
                assert_eq!(input._f_.as_deref(), Some("llm"));
                assert_eq!(input._t_.as_deref(), Some("deepseek"));
                assert!(input.enabled, "缺省 enabled = true");
                assert!(!input.is_default);
                assert!(input.credentials.is_some());
                assert!(input.settings.is_some());
            }
            _ => panic!("op 解析类型不符"),
        }
    }

    #[test]
    fn parse_rejects_non_object_credentials_and_settings() {
        assert!(parse_operation(&json!({
            "op": "update_config", "id": 3, "credentials": "sk-plain"
        }))
        .unwrap_err()
        .contains("credentials MUST 为 JSON 对象"));
        assert!(parse_operation(&json!({
            "op": "create_config", "category": "llm", "provider": "x", "settings": ["a"]
        }))
        .unwrap_err()
        .contains("settings MUST 为 JSON 对象"));
    }

    #[test]
    fn parse_requires_ids_and_category() {
        assert!(parse_operation(&json!({"op": "update_config"}))
            .unwrap_err()
            .contains("id"));
        assert!(parse_operation(&json!({"op": "delete_config"}))
            .unwrap_err()
            .contains("id"));
        assert!(
            parse_operation(&json!({"op": "create_config", "provider": "x"}))
                .unwrap_err()
                .contains("category")
        );
        assert!(parse_operation(&json!({"op": "drop_config"}))
            .unwrap_err()
            .contains("白名单"));
    }

    #[test]
    fn parse_update_partial_fields_stay_none() {
        let (_, parsed) =
            parse_operation(&json!({"op": "update_config", "id": 9, "enabled": false})).unwrap();
        match parsed {
            ParsedOperation::Update { id, input } => {
                assert_eq!(id, 9);
                assert_eq!(input.enabled, Some(false));
                assert!(input.settings.is_none());
                assert!(input.credentials.is_none());
                assert!(input.notice.is_none());
            }
            _ => panic!("op 解析类型不符"),
        }
    }
}
