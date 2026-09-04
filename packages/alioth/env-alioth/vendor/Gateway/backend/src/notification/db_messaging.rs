//! 数据库驱动的 MessagingService 实现
//!
//! 将系统通知直接写入 `zc_id_message` 表，前端通过 `/api/global/overview` 轮询获取。
//! 无需外部消息中间件，适合开发环境和中小规模部署。
//!
//! 当 Gateway 配置了外部消息服务（ZChat/MQTT）时，替换为对应的 Adapter 实现。
//!
//! 投递真实性保证（fix-gateway-infra-gaps）：
//! - 站内信通道（send_direct / send_group / broadcast / send_system_notification）真实落库；
//! - 设备通道（send_device_command / broadcast_device_command / send_raw）与告警
//!   （send_alert，无投递目标参数）返回 `AliothError::NotImplemented`（HTTP 501），
//!   禁止静默 `Ok(())` 空转——调用方必须能感知投递失败。

use async_trait::async_trait;
use common::error::AliothError;
use common::messaging::{AlertLevel, DeviceCommand, MessageDeliveryInfo, MessagingService};
use sqlx::PgPool;

/// 数据库驱动的消息服务
///
/// 将消息持久化到 `zc_id_message` / `zc_id_message_rr_readers` 表，
/// 通过前端轮询实现非实时站内信功能。
#[derive(Clone)]
pub struct DbMessagingService {
    pool: PgPool,
}

impl DbMessagingService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 落库一条站内信并记录投递日志。
    ///
    /// `notice` 作标题、`comments` 作正文（与 global_overview 的
    /// `MessageItem.title ← notice / content ← comments` 映射一致）。
    async fn insert_message(
        &self,
        notice: &str,
        comments: &str,
        created_by_id: i64,
        benefit_users: &[i64],
        thread_id: Option<i64>,
    ) -> Result<(), AliothError> {
        let result = sqlx::query(
            r#"
            INSERT INTO isahl."zc_id_msgs-system" (notice, comments, created_by_id, ak_benefit_user, fk_thread, deleted_at)
            VALUES ($1, $2, $3, $4, $5, NULL)
            "#,
        )
        .bind(notice)
        .bind(comments)
        .bind(created_by_id)
        .bind(benefit_users)
        .bind(thread_id)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => {
                common::telemetry::info!(
                    "DbMessaging: message stored (notice={}, benefit_users={}, thread={:?})",
                    notice,
                    benefit_users.len(),
                    thread_id
                );
                Ok(())
            }
            Err(e) => {
                common::telemetry::warn!(
                    "DbMessaging: failed to store message '{}': {}",
                    notice,
                    e
                );
                Err(AliothError::Internal(format!(
                    "Failed to store message: {}",
                    e
                )))
            }
        }
    }
}

#[async_trait]
impl MessagingService for DbMessagingService {
    /// 发送系统通知（写入 zc_id_message 表）
    async fn send_system_notification(
        &self,
        to: u64,
        title: &str,
        content: &str,
    ) -> Result<(), AliothError> {
        self.insert_message(title, content, to as i64, &[to as i64], None)
            .await
    }

    /// 发送单聊站内信（写入 zc_id_message，受益用户 = 接收方）
    async fn send_direct(&self, from: u64, to: u64, content: &str) -> Result<(), AliothError> {
        self.insert_message(content, content, from as i64, &[to as i64], None)
            .await
    }

    /// 发送群聊站内信（写入 zc_id_message，关联会话线程 fk_thread）
    async fn send_group(
        &self,
        from: u64,
        conversation_id: u64,
        content: &str,
    ) -> Result<(), AliothError> {
        self.insert_message(
            content,
            content,
            from as i64,
            &[],
            Some(conversation_id as i64),
        )
        .await
    }

    /// 广播消息到全部活跃用户（逐用户落库，单事务语义由单条 INSERT 保证）
    async fn broadcast(&self, from: u64, content: &str) -> Result<(), AliothError> {
        // 查询活跃用户 ID（is_active = true）
        let user_ids: Vec<i64> = match sqlx::query_scalar::<_, i64>(
            r#"
            SELECT id FROM isahl_auth.auth_users
            WHERE is_active = true
            "#,
        )
        .fetch_all(&self.pool)
        .await
        {
            Ok(ids) => ids,
            Err(e) => {
                common::telemetry::warn!("DbMessaging: failed to list active users: {}", e);
                return Err(AliothError::Internal(format!(
                    "Failed to list active users: {}",
                    e
                )));
            }
        };

        if user_ids.is_empty() {
            common::telemetry::warn!("DbMessaging: broadcast skipped, no active users");
            return Ok(());
        }

        // 批量 INSERT（unnest 展开用户数组，单语句）
        let result = sqlx::query(
            r#"
            INSERT INTO isahl."zc_id_msgs-system" (notice, comments, created_by_id, ak_benefit_user, deleted_at)
            SELECT $1, $2, $3, ARRAY[uid], NULL
            FROM unnest($4::bigint[]) AS uid
            "#,
        )
        .bind(content)
        .bind(content)
        .bind(from as i64)
        .bind(&user_ids)
        .execute(&self.pool)
        .await;

        match result {
            Ok(r) => {
                common::telemetry::info!(
                    "DbMessaging: broadcast delivered to {} users",
                    r.rows_affected()
                );
                Ok(())
            }
            Err(e) => {
                common::telemetry::warn!("DbMessaging: broadcast failed: {}", e);
                Err(AliothError::Internal(format!("Broadcast failed: {}", e)))
            }
        }
    }

    /// 发送分级告警（无投递目标参数，无法在 DB 直写下确定收件人——显式失败）
    async fn send_alert(
        &self,
        level: AlertLevel,
        title: &str,
        _content: &str,
    ) -> Result<(), AliothError> {
        Err(AliothError::NotImplemented(format!(
            "alert routing (level={:?}, title={}) requires a recipient-resolution backend",
            level, title
        )))
    }

    /// 向指定设备下发指令（设备通道需 ZChat/MQTT 适配器——显式失败）
    async fn send_device_command(
        &self,
        device_id: &str,
        command: DeviceCommand,
    ) -> Result<(), AliothError> {
        Err(AliothError::NotImplemented(format!(
            "device command (device={}, command={:?}) requires a device-channel adapter (ZChat/MQTT)",
            device_id, command
        )))
    }

    /// 广播指令到所有设备（同上，显式失败）
    async fn broadcast_device_command(&self, command: DeviceCommand) -> Result<(), AliothError> {
        Err(AliothError::NotImplemented(format!(
            "device broadcast (command={:?}) requires a device-channel adapter (ZChat/MQTT)",
            command
        )))
    }

    /// 发送原始消息（MQTT topic 通道——显式失败）
    async fn send_raw(
        &self,
        topic: &str,
        _payload: Vec<u8>,
        _qos: u8,
    ) -> Result<MessageDeliveryInfo, AliothError> {
        Err(AliothError::NotImplemented(format!(
            "raw topic '{}' requires an MQTT adapter",
            topic
        )))
    }
}
