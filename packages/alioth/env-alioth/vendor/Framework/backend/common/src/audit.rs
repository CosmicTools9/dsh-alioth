//! 审计事件记录 — 下沉自 Gateway EPP（event_handler.rs）
//!
//! 供 Framework crate（approval 等）与 Gateway 共用。系统操作（SLA 自动驳回、
//! 流程推进等）经此记录操作级审计，受审计框架监管。
//!
//! 写入表：`isahl_audit.audit_events`。插入非阻塞，错误仅记录不传播。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// 简化的决策类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Decision {
    Permit,
    Deny,
    NotApplicable,
}

/// 审计事件
#[derive(Debug, Serialize)]
pub struct AuditEvent {
    #[serde(with = "crate::serde_zuid")]
    pub user_id: i64,
    pub user_email: String,
    pub object_path: String,
    pub operation: String,
    pub decision: String,
    pub created_at: chrono::DateTime<Utc>,
}

/// Audit error types
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
    #[error("Validation error: {0}")]
    ValidationError(String),
}

/// 记录审计事件 - 完整数据库插入
///
/// 注意：审计插入是非阻塞的，错误只记录不传播
pub async fn record_audit_event(
    pool: &PgPool,
    user_id: i64,
    user_email: &str,
    object_path: &str,
    operation: &str,
    decision: &Decision,
) -> Result<(), AuditError> {
    // 验证输入
    if user_id <= 0 {
        return Err(AuditError::ValidationError(
            "user_id must be positive".to_string(),
        ));
    }
    if user_email.is_empty() {
        return Err(AuditError::ValidationError(
            "user_email cannot be empty".to_string(),
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
    .bind(user_email)
    .bind(object_path)
    .bind(operation)
    .bind(decision_str)
    .bind(Vec::<String>::new()) // obligations_triggered (varchar[])
    .bind(Option::<String>::None) // ip_address (可选)
    .bind(Option::<String>::None) // user_agent (可选)
    .bind(serde_json::json!({})) // metadata
    .bind(Utc::now())
    .execute(pool)
    .await
    {
        Ok(_) => {
            crate::telemetry::info!(
                "Audit event recorded: user={} decision={} resource={}",
                user_email,
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
