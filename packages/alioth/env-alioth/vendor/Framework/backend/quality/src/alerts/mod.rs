use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// 告警频率
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AlertFrequency {
    EveryFailure,   // 每次失败
    DailyDigest,    // 每日汇总
    ThresholdBased, // 阈值触发
}

/// 告警配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid::opt")]
    pub rule_id: Option<i64>, // 关联特定规则
    #[serde(with = "common::serde_zuid::opt")]
    pub collection_id: Option<i64>,
    pub name: String,

    // 通知渠道
    pub webhook_url: Option<String>,
    pub email_recipients: Vec<String>,

    // 触发条件
    pub trigger_frequency: AlertFrequency,
    pub threshold_percentage: Option<f64>, // 通过率低于阈值触发
    pub consecutive_failures: Option<i32>, // 连续失败N次触发

    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 告警载荷
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertPayload {
    pub rule_name: String,
    #[serde(with = "common::serde_zuid")]
    pub rule_id: i64,
    pub status: String,
    pub pass_percentage: f64,
    #[serde(with = "common::serde_zuid")]
    pub failed_rows: i64,
    #[serde(with = "common::serde_zuid")]
    pub total_rows: i64,
    pub timestamp: DateTime<Utc>,
    pub details_url: String,
    pub error_message: Option<String>,
}

/// 告警记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertSent {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub config_id: i64,
    #[serde(with = "common::serde_zuid")]
    pub rule_id: i64,
    pub channel: String, // 'webhook', 'email'
    pub payload: AlertPayload,
    pub sent_at: DateTime<Utc>,
    pub status: String, // 'sent', 'failed'
    pub error_message: Option<String>,
}

/// Webhook 通知器
pub struct WebhookNotifier;

impl WebhookNotifier {
    pub async fn send(&self, webhook_url: &str, payload: &AlertPayload) -> Result<(), AlertError> {
        let client = reqwest::Client::new();
        let response = client
            .post(webhook_url)
            .json(payload)
            .send()
            .await
            .map_err(|e| AlertError::SendFailed(format!("HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(AlertError::SendFailed(format!(
                "Webhook returned status: {}",
                response.status()
            )));
        }

        Ok(())
    }
}

/// 邮件通知器 (预留接口)
pub struct EmailNotifier;

impl EmailNotifier {
    pub async fn send(
        &self,
        _recipients: &[String],
        _subject: &str,
        _body: &str,
    ) -> Result<(), AlertError> {
        // 邮件发送逻辑预留 - 可集成第三方服务
        // 例如: AWS SES, SendGrid, SMTP 等
        Ok(())
    }
}

/// 告警服务
pub struct AlertService {
    db_pool: PgPool,
    webhook_notifier: WebhookNotifier,
    email_notifier: EmailNotifier,
}

impl AlertService {
    pub fn new(db_pool: PgPool) -> Self {
        Self {
            db_pool,
            webhook_notifier: WebhookNotifier,
            email_notifier: EmailNotifier,
        }
    }

    /// 处理执行结果并发送告警
    #[allow(clippy::too_many_arguments)]
    pub async fn process_execution_result(
        &self,
        rule_id: i64,
        rule_name: &str,
        status: &str,
        pass_percentage: f64,
        failed_rows: i64,
        total_rows: i64,
        error_message: Option<String>,
    ) -> Result<Vec<AlertSent>, AlertError> {
        let configs = self.get_alert_configs_for_rule(rule_id).await?;
        let mut sent_alerts = Vec::new();

        for config in configs {
            if !config.enabled {
                continue;
            }

            // 检查是否应该触发告警
            let should_alert = match config.trigger_frequency {
                AlertFrequency::EveryFailure => status == "FAILED" || status == "ERRORED",
                AlertFrequency::ThresholdBased => {
                    if let Some(threshold) = config.threshold_percentage {
                        pass_percentage < threshold
                    } else {
                        false
                    }
                }
                AlertFrequency::DailyDigest => {
                    // 每日汇总在单独的任务中处理
                    false
                }
            };

            if should_alert {
                let payload = AlertPayload {
                    rule_name: rule_name.to_string(),
                    rule_id,
                    status: status.to_string(),
                    pass_percentage,
                    failed_rows,
                    total_rows,
                    timestamp: Utc::now(),
                    details_url: format!("/meta/quality/rules/{}", rule_id),
                    error_message: error_message.clone(),
                };

                // 发送 Webhook
                if let Some(ref webhook_url) = config.webhook_url {
                    match self.webhook_notifier.send(webhook_url, &payload).await {
                        Ok(()) => {
                            let alert = self
                                .record_alert_sent(
                                    config.id, rule_id, "webhook", &payload, "sent", None,
                                )
                                .await?;
                            sent_alerts.push(alert);
                        }
                        Err(e) => {
                            let _ = self
                                .record_alert_sent(
                                    config.id,
                                    rule_id,
                                    "webhook",
                                    &payload,
                                    "failed",
                                    Some(e.to_string()),
                                )
                                .await?;
                        }
                    }
                }

                // 发送邮件 (预留)
                if !config.email_recipients.is_empty() {
                    let subject = format!("数据质量告警: {}", rule_name);
                    let body = format!(
                        "规则 {} 执行{}，通过率: {:.2}%",
                        rule_name, status, pass_percentage
                    );
                    let _ = self
                        .email_notifier
                        .send(&config.email_recipients, &subject, &body)
                        .await;
                }
            }
        }

        Ok(sent_alerts)
    }

    /// 发送每日汇总
    pub async fn send_daily_digest(&self) -> Result<(), AlertError> {
        // 获取昨日所有执行结果
        // 按配置分组发送汇总
        // 简化实现
        Ok(())
    }

    /// 获取告警配置
    async fn get_alert_configs_for_rule(
        &self,
        rule_id: i64,
    ) -> Result<Vec<AlertConfig>, AlertError> {
        let sql = r#"
            SELECT id, rule_id, collection_id, name, webhook_url, email_recipients,
                   trigger_frequency, threshold_percentage, consecutive_failures,
                   enabled, created_at, updated_at
            FROM quality_alert_configs
            WHERE (rule_id = $1 OR rule_id IS NULL) AND enabled = true
        "#;

        let rows: Vec<AlertConfigRow> = sqlx::query_as(sql)
            .bind(rule_id)
            .fetch_all(&self.db_pool)
            .await
            .map_err(AlertError::Database)?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// 记录已发送的告警
    async fn generate_id(&self) -> Result<i64, sqlx::Error> {
        let id: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) + 1 FROM quality_alert_sent")
            .fetch_one(&self.db_pool)
            .await?;
        Ok(id)
    }

    async fn record_alert_sent(
        &self,
        config_id: i64,
        rule_id: i64,
        channel: &str,
        payload: &AlertPayload,
        status: &str,
        error_message: Option<String>,
    ) -> Result<AlertSent, AlertError> {
        let id = self.generate_id().await?;
        let now = Utc::now();

        let sql = r#"
            INSERT INTO quality_alert_sent (
                id, config_id, rule_id, channel, payload, sent_at, status, error_message
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id, config_id, rule_id, channel, payload, sent_at, status, error_message
        "#;

        let row: AlertSentRow = sqlx::query_as(sql)
            .bind(id)
            .bind(config_id)
            .bind(rule_id)
            .bind(channel)
            .bind(serde_json::to_value(payload).unwrap_or_default())
            .bind(now)
            .bind(status)
            .bind(error_message)
            .fetch_one(&self.db_pool)
            .await
            .map_err(AlertError::Database)?;

        Ok(row.into())
    }
}

/// 告警错误类型
#[derive(Debug, thiserror::Error)]
pub enum AlertError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Send failed: {0}")]
    SendFailed(String),
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

// 数据库行结构
#[derive(sqlx::FromRow)]
struct AlertConfigRow {
    id: i64,
    rule_id: Option<i64>,
    collection_id: Option<i64>,
    name: String,
    webhook_url: Option<String>,
    email_recipients: serde_json::Value,
    trigger_frequency: String,
    threshold_percentage: Option<f64>,
    consecutive_failures: Option<i32>,
    enabled: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<AlertConfigRow> for AlertConfig {
    fn from(row: AlertConfigRow) -> Self {
        Self {
            id: row.id,
            rule_id: row.rule_id,
            collection_id: row.collection_id,
            name: row.name,
            webhook_url: row.webhook_url,
            email_recipients: serde_json::from_value(row.email_recipients).unwrap_or_default(),
            trigger_frequency: match row.trigger_frequency.as_str() {
                "EVERY_FAILURE" => AlertFrequency::EveryFailure,
                "DAILY_DIGEST" => AlertFrequency::DailyDigest,
                "THRESHOLD_BASED" => AlertFrequency::ThresholdBased,
                _ => AlertFrequency::EveryFailure,
            },
            threshold_percentage: row.threshold_percentage,
            consecutive_failures: row.consecutive_failures,
            enabled: row.enabled,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct AlertSentRow {
    id: i64,
    config_id: i64,
    rule_id: i64,
    channel: String,
    payload: serde_json::Value,
    sent_at: DateTime<Utc>,
    status: String,
    error_message: Option<String>,
}

impl From<AlertSentRow> for AlertSent {
    fn from(row: AlertSentRow) -> Self {
        Self {
            id: row.id,
            config_id: row.config_id,
            rule_id: row.rule_id,
            channel: row.channel,
            payload: serde_json::from_value(row.payload).unwrap(),
            sent_at: row.sent_at,
            status: row.status,
            error_message: row.error_message,
        }
    }
}
