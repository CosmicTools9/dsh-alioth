//! 记忆作用域解析（refactor-chat-ai-subject-identity-memory）。
//!
//! 双层记忆的两侧键：
//! - **主体侧**（L1）：智能体主体 id 由 `AgentConfig.subject_id` 提供；其**联系方式**
//!   （`isahl.zc_id_contact_infos`）按约定 code `agent-<agent_code>` 解析，作为该主体
//!   发出消息的 `fk_sender-addr` 与「智能体侧」识别依据。未 materialize → None，
//!   调用方 MUST 显式失败，MUST NOT 回退共享联系人。
//! - **对话方侧**（L2 第二维）：由会话最后一条**含对话方**的消息的参与方集合
//!   （`fk_sender-addr` ∪ `zc_id_message_rr_recipients.ref_right` ∪
//!   `zc_id_message_rr_copy.ref_right`，均为联系方式 id）排除智能体侧后，
//!   聚合到联系人（`isahl.zc_id_contacts`）；均不可得时回退会话属主正向链。
//!
//! 方向契约：消息层身份空间是联系方式 id，联系人（`zc_id_contacts`）是聚合层产物
//! （反向单跳见 `framework_contacts::ContactsService::resolve_contact_of_info`）。

use sqlx::PgPool;

use framework_contacts::ContactsService;

/// 主体联系方式约定 code 前缀（种子按 `agent-<agent_code>` 落行）。
pub const SUBJECT_CONTACT_CODE_PREFIX: &str = "agent-";

/// 历史共享 AI 联系人 code（单一 `llm-agent`，本变更前所有主体共用）。
/// **只读识别**用：历史消息的发送方仍是该联系方式，判定其为 assistant 侧以免旧会话
/// 渲染/重生成回归；写侧 MUST NOT 再创建或使用该行（refactor-chat-ai-subject-identity-memory D-3）。
pub const LEGACY_SHARED_AI_CONTACT_CODE: &str = "llm-agent";

/// 智能体侧联系方式 code 匹配串（前缀，供 SQL LIKE 使用）。
pub fn subject_contact_code_prefix_like() -> String {
    format!("{SUBJECT_CONTACT_CODE_PREFIX}%")
}

/// 解析智能体主体的联系方式 id（`isahl.zc_id_contact_infos`，约定 code `agent-<code>`）。
pub async fn resolve_subject_contact_id(
    pool: &PgPool,
    agent_code: &str,
) -> Result<Option<i64>, String> {
    let code = format!("{SUBJECT_CONTACT_CODE_PREFIX}{agent_code}");
    let row: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl.zc_id_contact_infos
           WHERE code = $1 AND deleted_at IS NULL
           ORDER BY id LIMIT 1"#,
    )
    .bind(code)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("resolve subject contact failed: {}", e))?;
    Ok(row)
}

/// 主体人格（add-agent-persona-channel）：`zc_id_empl-agent.soul`（物理列，由
/// agent 管理面 `soul` 字段写入）。NULL / 空白 → `Ok(None)`（调用方不产段）；
/// 查询失败 → `Err`（调用方 warn 降级，不阻断该轮）。`settings` JSONB 是
/// AgentConfig 覆盖层，MUST NOT 作为人格来源。
pub async fn load_subject_persona(
    pool: &PgPool,
    subject_id: i64,
) -> Result<Option<String>, String> {
    let row: Option<Option<String>> = sqlx::query_scalar(
        r#"SELECT soul FROM isahl."zc_id_empl-agent"
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("load subject persona failed: {}", e))?;
    Ok(row
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()))
}

/// 单条消息的参与方（均为 `zc_id_contact_infos` id）。
#[derive(Debug, Clone)]
struct MessageParticipants {
    participants: Vec<i64>,
}

/// 对话方解析器。
pub struct CounterpartResolver {
    pool: PgPool,
}

impl CounterpartResolver {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 解析会话对话方联系人 id（L2 键第二维）。
    ///
    /// 1. 从最近消息向前找第一条含**非智能体侧**参与方的消息，取其参与方集合中
    ///    首个非智能体联系方式并聚合为联系人；
    /// 2. 全部不可得（历史消息无参与方行 / 仅智能体侧）→ 回退会话属主正向链
    ///    （`auth_users.entity_id → ... → zc_id_contacts`）。
    pub async fn resolve(
        &self,
        session_id: i64,
        owner_user_id: i64,
        agent_info_ids: &[i64],
    ) -> Result<Option<i64>, String> {
        for row in self.recent_participants(session_id).await? {
            let Some(info_id) = row
                .participants
                .iter()
                .copied()
                .find(|id| !agent_info_ids.contains(id))
            else {
                continue;
            };
            if let Some(contact_id) =
                ContactsService::resolve_contact_of_info(&self.pool, info_id).await?
            {
                return Ok(Some(contact_id));
            }
        }

        // 回退：会话属主正向链（账号 → 联系人 → 首选联系方式）
        let fallback = ContactsService::resolve_user_contact(&self.pool, owner_user_id).await?;
        Ok(fallback.map(|r| r.contact_id))
    }

    /// 取会话最近消息的参与方集合（新→旧），上限 20 条。
    async fn recent_participants(
        &self,
        session_id: i64,
    ) -> Result<Vec<MessageParticipants>, String> {
        let rows = sqlx::query_as::<_, (i64, Option<i64>, Option<Vec<i64>>, Option<Vec<i64>>)>(
            r#"
            SELECT m.id,
                   m."fk_sender-addr" AS sender,
                   r.recipients,
                   c.copies
              FROM isahl."zc_id_msgs-chat_ai" m
              LEFT JOIN LATERAL (
                  SELECT array_agg(rr.ref_right) AS recipients
                    FROM isahl."zc_id_message_rr_recipients" rr
                   WHERE rr.ref_left = m.id AND rr.deleted_at IS NULL
              ) r ON TRUE
              LEFT JOIN LATERAL (
                  SELECT array_agg(cc.ref_right) AS copies
                    FROM isahl."zc_id_message_rr_copy" cc
                   WHERE cc.ref_left = m.id AND cc.deleted_at IS NULL
              ) c ON TRUE
             WHERE m.fk_thread = $1 AND m.deleted_at IS NULL
             ORDER BY m.created_at DESC, m.id DESC
             LIMIT 20
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("load message participants failed: {}", e))?;

        Ok(rows
            .into_iter()
            .map(|(_id, sender, recipients, copies)| {
                let mut participants: Vec<i64> = Vec::new();
                if let Some(s) = sender {
                    participants.push(s);
                }
                for list in [recipients, copies].into_iter().flatten() {
                    for id in list {
                        if !participants.contains(&id) {
                            participants.push(id);
                        }
                    }
                }
                MessageParticipants { participants }
            })
            .collect())
    }
}

/// 智能体侧联系方式 id 集合——历史消息的角色判定（发送方 ∈ 集合 ⇒ assistant）与
/// 对话方解析的排除集。集合 = `agent-<code>` 前缀 ∪ 历史共享 `llm-agent`（只识别既有行，
/// 写侧不再创建）。
pub async fn agent_contact_id_set(pool: &PgPool) -> Result<std::collections::HashSet<i64>, String> {
    let rows: Vec<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl.zc_id_contact_infos
           WHERE deleted_at IS NULL
             AND (code LIKE $1 OR code = $2)"#,
    )
    .bind(subject_contact_code_prefix_like())
    .bind(LEGACY_SHARED_AI_CONTACT_CODE)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("load agent contacts failed: {}", e))?;
    Ok(rows.into_iter().collect())
}

/// 会话当前智能体 code：`agent_state.pinned_agent`（非空）→ 最近 assistant 消息
/// meta.agent_code；均无 → None。
pub async fn session_agent_code(pool: &PgPool, session_id: i64) -> Result<Option<String>, String> {
    let pinned: Option<String> = sqlx::query_scalar::<_, String>(
        r#"SELECT agent_state->>'pinned_agent'
           FROM isahl."zc_id_thre-ai_session" WHERE id = $1"#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .filter(|s| !s.is_empty());
    if pinned.is_some() {
        return Ok(pinned);
    }
    let last: Option<String> = sqlx::query_scalar(
        r#"SELECT cm.agent_code
             FROM isahl."zc_id_msgs-chat_ai" m
             JOIN isahl_auth.chat_message_meta cm ON cm.msg_id = m.id
            WHERE m.fk_thread = $1 AND m.deleted_at IS NULL AND cm.agent_code <> ''
            ORDER BY m.created_at DESC, m.id DESC LIMIT 1"#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    Ok(last)
}

/// 会话当前智能体的联系方式 id（无主体 code / 未 materialize 联系方式 → None）。
pub async fn session_agent_contact_id(
    pool: &PgPool,
    session_id: i64,
) -> Result<Option<i64>, String> {
    match session_agent_code(pool, session_id).await? {
        Some(code) => resolve_subject_contact_id(pool, &code).await,
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_contact_code_convention() {
        assert_eq!(
            format!("{SUBJECT_CONTACT_CODE_PREFIX}{}", "general"),
            "agent-general"
        );
    }
}
