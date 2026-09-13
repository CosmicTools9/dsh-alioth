//! 审计事件记录 — 下沉自 Gateway EPP（event_handler.rs）
//!
//! 供 Framework crate（approval 等）与 Gateway 共用。系统操作（SLA 自动驳回、
//! 流程推进等）经此记录操作级审计，受审计框架监管。
//!
//! 写入表：`isahl_audit.audit_events`。插入 await（毫秒级）完成；DB 错误仅
//! telemetry 记录、不传播不阻断主链（fix-chat-ai-capability-gaps D2.4 明确口径）。
//!
//! **主体标识口径**（fix-ngac-audit-subject-identity）：`subject` 参数承载审计主体
//! **唯一性标识** —— username 优先，缺失时回落 `user:{user_id}`。email 既非唯一也
//! 可不存（种子/服务账号），**不得**作为唯一性标识。表列名 `user_email` 为历史遗留，
//! 承载的即本标识字符串。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// 审计主体**唯一性标识**（跨审计面唯一实现：NGAC 决策审计 + 血缘审计
/// `data_change_logs`/`audit_outbox`）：username 非空取之；否则回落
/// `user:{user_id}`（保留唯一性，禁止跨主体共享常量占位）。
///
/// email 既非唯一也可为空，**不得**作为唯一性标识（联系方式可经
/// `user_id → isahl_auth.auth_users` 关联取得）。
pub fn resolve_subject(username: &str, user_id: i64) -> String {
    let username = username.trim();
    if username.is_empty() {
        format!("user:{user_id}")
    } else {
        username.to_string()
    }
}

/// 简化的决策类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Decision {
    Permit,
    Deny,
    NotApplicable,
}

/// Audit error types
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
    #[error("Validation error: {0}")]
    ValidationError(String),
}

/// 记录审计事件 - 完整数据库插入（带 metadata）
///
/// 语义（fix-chat-ai-capability-gaps D2.4 明确）：插入 await 毫秒级完成、
/// DB 错误仅 telemetry 记录不传播（返回 Ok），不阻断主链；Err 仅限输入校验。
/// 既有 `record_audit_event` 委托本函数（metadata = {}），调用点零迁移。
///
/// `subject`：审计主体唯一性标识（username 优先，回落 `user:{user_id}`）——
/// 见模块头「主体标识口径」。email 属联系方式，存在时应置于 `metadata`。
pub async fn record_audit_event_with_metadata(
    pool: &PgPool,
    user_id: i64,
    subject: &str,
    object_path: &str,
    operation: &str,
    decision: &Decision,
    metadata: serde_json::Value,
) -> Result<(), AuditError> {
    // 验证输入
    if user_id <= 0 {
        return Err(AuditError::ValidationError(
            "user_id must be positive".to_string(),
        ));
    }
    if subject.is_empty() {
        return Err(AuditError::ValidationError(
            "audit subject cannot be empty".to_string(),
        ));
    }
    if object_path.is_empty() {
        return Err(AuditError::ValidationError(
            "object_path cannot be empty".to_string(),
        ));
    }
    if operation.is_empty() {
        return Err(AuditError::ValidationError(
            "operation cannot be empty".to_string(),
        ));
    }

    let decision_str = match decision {
        Decision::Permit => "permit",
        Decision::Deny => "deny",
        Decision::NotApplicable => "not_applicable",
    };

    // 生成 audit event ID (使用 snowflake 或时间戳)
    let audit_id = Utc::now().timestamp_nanos_opt().unwrap_or(0);

    // 同步插入（审计为监管要求，需确定完成；不再 fire-and-forget）
    match sqlx::query(
        r#"
        INSERT INTO isahl_audit.audit_events 
        (id, user_id, user_email, object_path, operation, decision, 
         subject_attributes, object_attributes, obligations_triggered, 
         ip_address, user_agent, metadata, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, '{}'::ag_catalog.ltree[], '{}'::ag_catalog.ltree[], $7, $8, $9, $10, $11)
        "#,
    )
    .bind(audit_id)
    .bind(user_id)
    .bind(subject)
    .bind(object_path)
    .bind(operation)
    .bind(decision_str)
    .bind(Vec::<String>::new()) // obligations_triggered (varchar[])
    .bind(Option::<String>::None) // ip_address (可选)
    .bind(Option::<String>::None) // user_agent (可选)
    .bind(metadata) // 业务 metadata（jsonb 既有列）
    .bind(Utc::now())
    .execute(pool)
    .await
    {
        Ok(_) => {
            crate::telemetry::info!(
                "Audit event recorded: subject={} decision={} resource={}",
                subject,
                decision_str,
                object_path
            );
        }
        Err(e) => {
            crate::telemetry::error!("Failed to record audit event: {}", e);
        }
    }

    Ok(())
}

/// 记录审计事件 - 完整数据库插入（metadata = {}，委托 with_metadata 变体）
///
/// 注意：审计插入 await 完成，错误只记录不传播。
/// `subject`：审计主体唯一性标识（username 优先，回落 `user:{user_id}`）。
pub async fn record_audit_event(
    pool: &PgPool,
    user_id: i64,
    subject: &str,
    object_path: &str,
    operation: &str,
    decision: &Decision,
) -> Result<(), AuditError> {
    record_audit_event_with_metadata(
        pool,
        user_id,
        subject,
        object_path,
        operation,
        decision,
        serde_json::json!({}),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_prefers_username() {
        assert_eq!(resolve_subject("alice", 42), "alice");
        assert_eq!(resolve_subject("  alice  ", 42), "alice", "两端空白应裁剪");
    }

    #[test]
    fn subject_falls_back_to_unique_user_id() {
        assert_eq!(resolve_subject("", 42), "user:42");
        assert_eq!(resolve_subject("   ", 42), "user:42");
        assert_ne!(
            resolve_subject("", 42),
            resolve_subject("", 43),
            "回落值必须对每个主体唯一（禁止共享常量）"
        );
    }
}
