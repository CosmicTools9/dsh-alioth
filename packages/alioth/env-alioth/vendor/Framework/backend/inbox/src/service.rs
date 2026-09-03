//! 站内信业务逻辑 — 读写操作 + 鉴权规则
//!
//! 所有权/受益规则：
//! - mark_read: 任何已认证用户可标记消息为已读（反馈按用户隔离）
//! - delete: 仅消息创建者可删除（created_by_id）

use chrono::Utc;
use sqlx::PgPool;

use crate::models::InboxActionResponse;

pub struct InboxService;

impl InboxService {
    /// 标记消息为已读（幂等）
    /// 返回 (success, message)
    pub async fn mark_read(pool: &PgPool, msg_id: i64, user_id: i64) -> InboxActionResponse {
        // 检查消息是否存在
        let exists: bool = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM isahl.zc_id_message WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(msg_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
            > 0;

        if !exists {
            return InboxActionResponse::fail("消息不存在");
        }

        // 在 zc_id_message_rr_contact-info 中记录该用户反馈为已读
        let r1 = sqlx::query(
            r#"UPDATE isahl."zc_id_message_rr_contact-info"
               SET deleted_at = NULL, feedback = 'read'::isahl.zc_id_message_rr_contact_info_feedback_enum,
                   updated_at = NOW()
               WHERE ref_left = $1 AND ref_right = $2"#,
        )
        .bind(msg_id)
        .bind(user_id)
        .execute(pool)
        .await;

        if let Err(e) = r1 {
            return InboxActionResponse::fail(format!("标记已读失败(contact-info): {}", e));
        }

        // 若没有已有记录，插入新行
        let r1b = sqlx::query(
            r#"INSERT INTO isahl."zc_id_message_rr_contact-info" (ref_left, ref_right, feedback)
               SELECT $1, $2, 'read'::isahl.zc_id_message_rr_contact_info_feedback_enum
               WHERE NOT EXISTS (
                   SELECT 1 FROM isahl."zc_id_message_rr_contact-info"
                   WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL
               )"#,
        )
        .bind(msg_id)
        .bind(user_id)
        .execute(pool)
        .await;

        if let Err(e) = r1b {
            return InboxActionResponse::fail(format!("标记已读失败(contact-info insert): {}", e));
        }

        // 更新消息生命周期状态为"已读"
        let status_id = Self::ensure_read_status(pool).await;

        let status_id = match status_id {
            Some(id) => id,
            None => return InboxActionResponse::fail("创建已读状态失败"),
        };

        let row = match crud::audit_outbox::fetch_primary_status_row(pool, msg_id).await {
            Ok(r) => r,
            Err(e) => {
                return InboxActionResponse::fail(format!("读取当前状态失败: {}", e));
            }
        };
        // ref_left 全表唯一 → 活跃行原地 UPDATE / 软删行 restore / 无行 INSERT
        let old_status = row.filter(|(_, active)| *active).map(|(s, _)| s);

        let r2 = match row {
            Some((_, true)) => {
                sqlx::query(
                    r#"UPDATE isahl."zc_id_lifecycle_r_primary-status" SET ref_right = $1, updated_at = NOW() WHERE ref_left = $2 AND deleted_at IS NULL"#,
                )
                .bind(status_id)
                .bind(msg_id)
                .execute(pool)
                .await
            }
            Some((_, false)) => {
                sqlx::query(
                    r#"UPDATE isahl."zc_id_lifecycle_r_primary-status" SET ref_right = $1, deleted_at = NULL, updated_at = NOW() WHERE ref_left = $2 AND deleted_at IS NOT NULL"#,
                )
                .bind(status_id)
                .bind(msg_id)
                .execute(pool)
                .await
            }
            None => {
                sqlx::query(
                    r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (ref_left, ref_right) VALUES ($1, $2)"#,
                )
                .bind(msg_id)
                .bind(status_id)
                .execute(pool)
                .await
            }
        };

        match r2 {
            Ok(_) => {
                // ADR D-010：主状态变更审计（pool 直插路径；失败不回滚业务）
                if let Err(e) = crud::audit_outbox::audit_primary_status(
                    pool,
                    msg_id,
                    old_status,
                    status_id,
                    Some(user_id),
                )
                .await
                {
                    common::telemetry::warn!(
                        "audit_primary_status enqueue failed (inbox {}): {}",
                        msg_id,
                        e
                    );
                }
                InboxActionResponse::ok("已标记为已读")
            }
            Err(e) => InboxActionResponse::fail(format!("更新生命周期状态失败: {}", e)),
        }
    }

    /// 软删除消息（仅创建者可删）
    pub async fn delete(pool: &PgPool, msg_id: i64, user_id: i64) -> InboxActionResponse {
        let result = sqlx::query(
            "UPDATE isahl.zc_id_message SET deleted_at = $1 WHERE id = $2 AND created_by_id = $3 AND deleted_at IS NULL",
        )
        .bind(Utc::now())
        .bind(msg_id)
        .bind(user_id)
        .execute(pool)
        .await;

        match result {
            Ok(r) if r.rows_affected() > 0 => InboxActionResponse::ok("消息已删除"),
            Ok(_) => InboxActionResponse::fail("消息不存在或无权删除"),
            Err(e) => InboxActionResponse::fail(format!("删除失败: {}", e)),
        }
    }

    async fn ensure_read_status(pool: &PgPool) -> Option<i64> {
        let existing = sqlx::query_scalar::<_, i64>(
            r#"SELECT id FROM isahl."zc_id_stus-message" WHERE code = 'read' AND deleted_at IS NULL LIMIT 1"#,
        )
        .fetch_optional(pool)
        .await
        .unwrap_or(None);

        if let Some(id) = existing {
            return Some(id);
        }

        sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_stus-message" (code, notice, enable) VALUES ('read', '已读', true) RETURNING id"#,
        )
        .fetch_one(pool)
        .await
        .ok()
    }
    /// 发送站内信（单事务：插消息 + 插收件人记录）
    pub async fn send(
        pool: &PgPool,
        sender_id: i64,
        req: crate::models::SendMessageRequest,
    ) -> InboxActionResponse {
        if let Err(e) = req.validate() {
            return InboxActionResponse::fail(e);
        }

        let mut tx = match pool.begin().await {
            Ok(t) => t,
            Err(e) => return InboxActionResponse::fail(format!("开启事务失败: {}", e)),
        };

        // 计算 fk_thread：如果回复，继承原消息的 fk_thread；否则 NULL
        let fk_thread = if let Some(prev_id) = req.previous_id {
            let thread_id: Option<i64> = sqlx::query_scalar(
                "SELECT fk_thread FROM isahl.zc_id_message WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(prev_id)
            .fetch_optional(&mut *tx)
            .await
            .unwrap_or(None);
            // 若原消息无 thread，用自己的 id 作为 thread；否则继承原 thread
            thread_id.or(Some(prev_id))
        } else {
            None
        };

        // 插入消息主体（叶表 zc_id_msgs-system；zc_id_message 有子表，父表直写违规）
        let msg_id: i64 = match sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_msgs-system"
                (notice, content, created_by_id, fk_previous, fk_thread, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
            RETURNING id"#,
        )
        .bind(&req.title)
        .bind(&req.content)
        .bind(sender_id)
        .bind(req.previous_id)
        .bind(fk_thread)
        .fetch_one(&mut *tx)
        .await
        {
            Ok(id) => id,
            Err(e) => {
                let _ = tx.rollback().await;
                return InboxActionResponse::fail(format!("发送消息失败: {}", e));
            }
        };

        // 为每个收件人插入 rr_recipients 记录（写入子表 zc_id_message_rr_contact-info）
        for &recipient_id in &req.recipient_ids {
            let r = sqlx::query(
                r#"INSERT INTO isahl."zc_id_message_rr_contact-info"
                    (ref_left, ref_right, feedback)
                VALUES ($1, $2, NULL::isahl.zc_id_message_rr_contact_info_feedback_enum)"#,
            )
            .bind(msg_id)
            .bind(recipient_id)
            .execute(&mut *tx)
            .await;

            if let Err(e) = r {
                let _ = tx.rollback().await;
                return InboxActionResponse::fail(format!("添加收件人失败: {}", e));
            }
        }

        match tx.commit().await {
            Ok(_) => InboxActionResponse::ok("消息已发送"),
            Err(e) => InboxActionResponse::fail(format!("提交事务失败: {}", e)),
        }
    }
}
