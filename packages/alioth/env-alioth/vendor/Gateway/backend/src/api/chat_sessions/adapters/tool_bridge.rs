//! # M1-SEAM: depends on ai-agent M1 APIs (see design.md D2.1-D2.4)
//!
//! M1（ai-agent/llm）与 M2（Gateway）跨层契约的唯一接缝文件：所有消费 M1 新
//! API 的代码集中在本文件（ActionHandler / ToolStreamEvent / with_allowed_tools /
//! with_event_sink / ToolRunResult.usage）。M1 已 merge（d34528ee7e），本文件
//! 必须真实编译通过。导出面保持 Gateway 内部类型为主，M1 类型仅经 trait
//! object / 工具端口适配使用。
//!
//! 内容：
//! - `GatewayActionHandler`：业务动作真实执行侧（batch_approve → approvals
//!   服务层；send_notification → system_push 落库服务；status_transition →
//!   trigger_crud 通用 UPDATE（agent allowed_schemas 域门禁 + 触发器 NGAC
//!   钩子）；generate_document → 草稿 assistant 消息落库）
//! - `GatewayToolPort`：DbToolAdapter 包装——为 execute_action 工具注入
//!   action_handler（M1 的 DbToolAdapter 面向纯 DB 工具，handler 为 None）
//! - `tool_result_usage`：ToolRunResult.usage（D2.4）→ canonical JSON

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;

use ai_agent::agents::tool_orchestrator::{
    ActionHandler, DbToolAdapter, ToolExecutionPort, ToolRunResult, ToolStreamEvent,
};
use ai_agent::agents::ConfirmationLevel;
use ai_agent::tools::registry::ToolRegistry;
use ai_agent::tools::{ToolCall, ToolContext, ToolResult};

use common::messaging::MessagingService;
use framework_workspace_approval::{ApprovalActor, ApprovalService};
use i18n::Locale;

use crate::api::chat_sessions::orchestrator::TurnStreamEvent;
use crate::api::chat_sessions::ports::{AIContactPort, MessageStorePort};
use crate::notification::db_messaging::DbMessagingService;

/// M1 ToolStreamEvent → Gateway TurnStreamEvent（D2.3/D2.10）：M1 事件语义与
/// WS typed 帧一一对应；映射集中在接缝文件。
pub fn map_tool_stream_event(ev: ToolStreamEvent) -> Option<TurnStreamEvent> {
    match ev {
        ToolStreamEvent::Chunk(text) => Some(TurnStreamEvent::Chunk(text)),
        ToolStreamEvent::ToolStart { name, arguments } => {
            Some(TurnStreamEvent::ToolStart { name, arguments })
        }
        ToolStreamEvent::ToolEnd {
            name,
            success,
            output,
        } => Some(TurnStreamEvent::ToolEnd {
            name,
            success,
            output,
        }),
    }
}

/// 工具路径本轮 token 用量（D2.4）：ToolRunResult.usage 归一为
/// ai-agent TokenUsage JSON（prompt/completion/total，与 orchestrator
/// 无工具路径形态同构）。
pub fn tool_result_usage(result: &ToolRunResult) -> Option<Value> {
    serde_json::to_value(&result.usage).ok()
}

// ============================================
// C3 工具结果截断（fix-chat-ai-capability-gaps D2.2）
// ============================================

/// 工具结果序列化上限：超过即截断（LLM 看到的单工具结果正文 ≤4000 字符）。
const TOOL_OUTPUT_CAP: usize = 4000;

/// C3 纯函数：`text` 为工具 output 的序列化字符串。>4000 字符 → 保留前
/// 4000 字符 + 追加「…[结果截断，共 N 行 / 共 N 字符]」（N 从原文统计）；
/// ≤4000 原样返回。边界（恰 4000/超/行数统计）单测覆盖。
pub fn truncate_tool_output(text: &str) -> String {
    if text.chars().count() <= TOOL_OUTPUT_CAP {
        return text.to_string();
    }
    let head: String = text.chars().take(TOOL_OUTPUT_CAP).collect();
    let lines = text.lines().count();
    let chars = text.chars().count();
    format!("{}…[结果截断，共 {} 行 / 共 {} 字符]", head, lines, chars)
}

/// Gateway 包装层截断器：ToolResult 出向（回 ai-agent）前，output 序列化
/// 超限 → 以截断文本替换 output（ai-agent execute_one 仅 output.to_string()
/// 渲染——续问 prompt 与 ToolEnd 事件同受此限）。不达标原样返回。
fn cap_tool_result(mut result: ToolResult) -> ToolResult {
    let serialized = result.output.to_string();
    let capped = truncate_tool_output(&serialized);
    if capped != serialized {
        result.output = Value::String(capped);
    }
    result
}

/// C8/D2.4：审计 user_email 解析——会话属主 auth_users.email；缺省（测试/
/// 历史用户等）回落 `user:{id}`（audit user_email NOT NULL 且非空校验——
/// 保证审计行必可写、不因邮箱缺失静默丢行）。
pub(crate) async fn resolve_user_email(pool: &PgPool, user_id: i64) -> String {
    match sqlx::query_scalar::<_, String>("SELECT email FROM isahl_auth.auth_users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
    {
        Ok(Some(email)) if !email.is_empty() => email,
        _ => format!("user:{}", user_id),
    }
}

// ============================================
// GatewayActionHandler
// ============================================

/// 业务动作真实执行侧（fix-chat-ai-feature-gaps D2.1）。
///
/// 确认级别映射（design D2.1 定）：batch_approve / status_transition = Explicit
/// （必须 confirmed=true）；send_notification = Preview；generate_document = None。
/// HTTP execute-action 端点与 LLM execute_action 工具（经 GatewayToolPort 注入
/// ToolContext.action_handler）共用本实现——单实现无 HTTP 自调用。
pub struct GatewayActionHandler {
    pool: PgPool,
    i18n: crate::i18n::I18nManagerRef,
}

impl GatewayActionHandler {
    pub fn new(pool: PgPool, i18n: crate::i18n::I18nManagerRef) -> Self {
        Self { pool, i18n }
    }

    /// HTTP 直接执行入口（orchestrator.execute_action 调用）：
    /// 与工具路径同实现，仅 ctx 由调用方构造（allowed_schemas 来自 agent 配置）。
    pub async fn run_action(
        &self,
        action_type: &str,
        target_ids: &[i64],
        params: &Value,
        ctx: &ToolContext,
    ) -> Result<Value, String> {
        match action_type {
            "batch_approve" => self.batch_approve(target_ids, params, ctx).await,
            "send_notification" => self.send_notification(target_ids, params, ctx).await,
            "status_transition" => self.status_transition(target_ids, params, ctx).await,
            "generate_document" => self.generate_document(target_ids, params, ctx).await,
            other => Err(format!("UNKNOWN_ACTION_TYPE: {}", other)),
        }
    }

    /// C8/D2.4：工具路径动作审计——成功 Permit / 失败 Deny + error；
    /// metadata{action_type,target_count,confirmed:false}（工具路径无用户确认
    /// 通道）；object_path=chat-sessions/{sid}/actions/agent_action:{type}。
    /// 无用户上下文（仅测试/内部）不记；await + 错误仅 telemetry 不阻断主链。
    async fn audit_execution(
        &self,
        ctx: &ToolContext,
        action_type: &str,
        target_count: usize,
        outcome: &Result<Value, String>,
    ) {
        use common::audit::{record_audit_event_with_metadata, Decision};
        let Some(user_id) = ctx.user_id else {
            return;
        };
        let email = resolve_user_email(&self.pool, user_id).await;
        let (decision, metadata) = match outcome {
            Ok(_) => (
                Decision::Permit,
                serde_json::json!({
                    "action_type": action_type,
                    "target_count": target_count,
                    "confirmed": false,
                }),
            ),
            Err(e) => (
                Decision::Deny,
                serde_json::json!({
                    "action_type": action_type,
                    "target_count": target_count,
                    "confirmed": false,
                    "error": e,
                }),
            ),
        };
        if let Err(e) = record_audit_event_with_metadata(
            &self.pool,
            user_id,
            &email,
            &format!(
                "chat-sessions/{}/actions/agent_action:{}",
                ctx.session_id, action_type
            ),
            "chat_ai.action.execute",
            &decision,
            metadata,
        )
        .await
        {
            common::telemetry::warn!("chat_ai.action.execute audit failed: {}", e);
        }
    }

    async fn batch_approve(
        &self,
        target_ids: &[i64],
        params: &Value,
        ctx: &ToolContext,
    ) -> Result<Value, String> {
        // action: "approve"（默认）| "reject"；opinion 可选意见
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("approve");
        let status_code = match action {
            "approve" => "approved",
            "reject" => "rejected",
            other => {
                return Err(format!(
                    "INVALID_PARAM: action must be approve|reject, got {}",
                    other
                ))
            }
        };
        let opinion = params
            .get("opinion")
            .and_then(|v| v.as_str())
            .map(String::from);
        let actor = ctx.user_id.map(|uid| ApprovalActor {
            user_id: uid,
            opinion: opinion.clone(),
        });

        let mut failed: Vec<Value> = Vec::new();
        for &approval_id in target_ids {
            // 服务层自带鉴权（fk_operator 非 NULL 时必须 == actor.user_id）；
            // actor=None（无用户上下文）时服务层跳过鉴权——仅测试/内部场景。
            let resp = ApprovalService::execute(
                &self.pool,
                approval_id,
                status_code,
                actor.clone(),
                None::<&dyn framework_workspace_approval::ApprovalHook>,
            )
            .await;
            if !resp.success {
                failed
                    .push(json!({ "approval_id": approval_id.to_string(), "error": resp.message }));
            }
        }
        if !failed.is_empty() {
            return Err(format!(
                "APPROVAL_FAILED: {}",
                serde_json::to_string(&failed).unwrap_or_default()
            ));
        }
        Ok(json!({
            "action": "batch_approve",
            "status": status_code,
            "processed": target_ids.len(),
        }))
    }

    async fn send_notification(
        &self,
        target_ids: &[i64],
        params: &Value,
        ctx: &ToolContext,
    ) -> Result<Value, String> {
        let title = params
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or("MISSING_PARAM: title")?;
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or("MISSING_PARAM: content")?;
        // 接收人：params.user_id > target_ids[0] > 当前用户
        let to = params
            .get("user_id")
            .and_then(|v| v.as_i64())
            .or_else(|| target_ids.first().copied())
            .or(ctx.user_id)
            .ok_or("MISSING_PARAM: no recipient (user_id / target_ids)")?;

        let messaging = DbMessagingService::new(self.pool.clone());
        messaging
            .send_system_notification(to as u64, title, content)
            .await
            .map_err(|e| format!("NOTIFY_FAILED: {}", e))?;
        Ok(json!({ "action": "send_notification", "sent_to": to, "title": title }))
    }

    async fn status_transition(
        &self,
        target_ids: &[i64],
        params: &Value,
        ctx: &ToolContext,
    ) -> Result<Value, String> {
        let table = params
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or("MISSING_PARAM: table")?;
        if table.is_empty()
            || !table
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!("INVALID_PARAM: table name {}", table));
        }
        let status = params
            .get("status")
            .and_then(|v| v.as_str())
            .ok_or("MISSING_PARAM: status")?;
        let column = params
            .get("column")
            .and_then(|v| v.as_str())
            .unwrap_or("status");

        // agent 域门禁：allowed_schemas 为空拒绝；目标表必须落在白名单内
        // （isahl.* 或精确表名；与 query_sql 工具同语义的 schema 域约束）。
        // 行级 NGAC 由 trigger_crud before-update 触发器链承载（TriggerBlocked
        // 会冒泡为错误，不静默）。
        if ctx.allowed_schemas.is_empty() {
            return Err(
                "ACTION_DENIED: agent 无 allowed_schemas（status_transition 拒绝）".to_string(),
            );
        }
        let domain_ok = ctx
            .allowed_schemas
            .iter()
            .any(|s| s == "isahl" || s == table || table.starts_with(&format!("{}.", s)));
        if !domain_ok {
            return Err(format!(
                "ACTION_DENIED: 目标表 {} 不在 agent allowed_schemas {:?} 内",
                table, ctx.allowed_schemas
            ));
        }

        let mut updated = 0u32;
        for &id in target_ids {
            // 旧记录（update_with_triggers 前置输入；含 deleted_at 行级守卫）
            let old_sql = format!(
                r#"SELECT to_jsonb(e) AS record FROM isahl."{}" AS e WHERE e.id = $1 AND e.deleted_at IS NULL"#,
                table
            );
            let old_record: Option<(Value,)> =
                sqlx::query_as(sqlx::AssertSqlSafe(old_sql.as_str()))
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| format!("DB_ERROR: {}", e))?;
            let Some((old_json,)) = old_record else {
                return Err(format!("TARGET_NOT_FOUND: {} {} 不存在或已删除", table, id));
            };
            let old_map: HashMap<String, Value> =
                serde_json::from_value(old_json).map_err(|e| format!("DB_ERROR: {}", e))?;

            let mut record = HashMap::new();
            record.insert(column.to_string(), Value::String(status.to_string()));
            crate::trigger_crud::update_with_triggers(
                &self.pool,
                table,
                id,
                record,
                &old_map,
                ctx.user_id,
            )
            .await
            .map_err(|e| format!("TRANSITION_FAILED: {} {} → {}: {}", table, id, status, e))?;
            updated += 1;
        }
        Ok(json!({
            "action": "status_transition",
            "table": table,
            "column": column,
            "status": status,
            "updated": updated,
        }))
    }

    async fn generate_document(
        &self,
        target_ids: &[i64],
        params: &Value,
        ctx: &ToolContext,
    ) -> Result<Value, String> {
        // 草稿正文：params.draft_text > params.content；target_ids[0] 可选关联 id
        let draft = params
            .get("draft_text")
            .and_then(|v| v.as_str())
            .or_else(|| params.get("content").and_then(|v| v.as_str()))
            .ok_or("MISSING_PARAM: draft_text/content")?;
        let doc_type = params
            .get("doc_type")
            .and_then(|v| v.as_str())
            .unwrap_or("document");
        let content = format!(
            "【{} 草稿】{}",
            doc_type,
            if target_ids.is_empty() {
                draft.to_string()
            } else {
                format!("{}（关联 {}", draft, target_ids[0])
            }
        );

        // AI contact（经既有 adapter 解析/自建——code 固定 llm-agent，i18n 兜底
        // 命名；复用其进程级缓存与建行逻辑，避免本文件重复 INSERT 走绕基线）
        let contact_adapter =
            crate::api::chat_sessions::adapters::db_ai_contact::DbAIContactAdapter::new(
                self.pool.clone(),
                self.i18n.clone(),
            );
        let sender = contact_adapter
            .resolve_ai_contact_id(&Locale::new("zh-CN"))
            .await?
            .ok_or("AI_CONTACT_UNAVAILABLE")?;

        // 草稿以 assistant 消息落库（经 MessageStorePort 适配器——叶表写入
        // 归口既有 db_message.rs 基线，不在本文件重复裸 INSERT）
        let msg_store = crate::api::chat_sessions::adapters::db_message::SqlxMessageAdapter::new(
            self.pool.clone(),
        );
        let row = msg_store
            .add_message(ctx.session_id, &content, Some(sender))
            .await?;

        Ok(json!({
            "action": "generate_document",
            "session_id": ctx.session_id,
            "message_id": row.id.to_string(),
            "doc_type": doc_type,
        }))
    }
}

#[async_trait]
impl ActionHandler for GatewayActionHandler {
    async fn execute(
        &self,
        action_type: &str,
        target_ids: &[i64],
        params: &Value,
        ctx: &ToolContext,
    ) -> Result<Value, String> {
        let outcome = self.run_action(action_type, target_ids, params, ctx).await;
        // C8/D2.4：工具路径真实执行审计（Explicit/High/Critical 已被 S1 门禁
        // 挡在 needs_confirmation 提案——能到 handler 的仅 None/Preview/Low
        // 直接执行；预览不经本方法故不记）。await + 错误仅 telemetry。
        self.audit_execution(ctx, action_type, target_ids.len(), &outcome)
            .await;
        outcome
    }

    fn confirmation_level(&self, action_type: &str) -> ConfirmationLevel {
        match action_type {
            "send_notification" => ConfirmationLevel::Preview,
            "generate_document" => ConfirmationLevel::None,
            // batch_approve / status_transition（写操作）→ Explicit；未知动作
            // fail-safe 归 Explicit（宁可多确认不裸执行）
            _ => ConfirmationLevel::Explicit,
        }
    }
}

/// 供 orchestrator/HTTP 层做门禁判定的同口径助手（ai-agent 的
/// level_requires_confirmation 是 pub(crate)，Gateway 侧不能引用）。
pub fn requires_confirmation(level: &ConfirmationLevel) -> bool {
    matches!(
        level,
        ConfirmationLevel::Explicit | ConfirmationLevel::High | ConfirmationLevel::Critical
    )
}

// ============================================
// GatewayToolPort（execute_action 工具注入 handler）
// ============================================

/// DbToolAdapter 包装：DB 工具全委托（含 allowed_tools 过滤）；execute_action
/// 单独以带 action_handler 的 ToolContext 经 ToolRegistry 执行（M1 DbToolAdapter
/// 恒 action_handler: None，业务动作注入只能在此层完成）。
pub struct GatewayToolPort {
    inner: DbToolAdapter,
    handler: Arc<dyn ActionHandler>,
    pool: PgPool,
    /// execute_action 是否在白名单（空 = 不过滤）
    action_allowed: bool,
}

impl GatewayToolPort {
    pub fn new(
        inner: DbToolAdapter,
        handler: Arc<dyn ActionHandler>,
        pool: PgPool,
        allowed_names: &[String],
    ) -> Self {
        let action_allowed =
            allowed_names.is_empty() || allowed_names.iter().any(|n| n == "execute_action");
        Self {
            inner,
            handler,
            pool,
            action_allowed,
        }
    }
}

#[async_trait]
impl ToolExecutionPort for GatewayToolPort {
    fn list_tools(&self) -> Vec<ai_agent::agents::ToolDefinition> {
        self.inner.list_tools()
    }

    async fn execute(
        &self,
        call: &ToolCall,
        session_id: i64,
        user_id: Option<i64>,
        allowed_schemas: &[String],
    ) -> Result<ToolResult, String> {
        // C3：所有出向 ToolResult（execute_action 与 DB 工具）统一经
        // cap_tool_result——output 序列化 >4000 字符截断 + 提示。exec Err
        //（<tool_error> 渲染路径）短小不进截断。
        let outcome = if call.name == "execute_action" {
            if !self.action_allowed {
                Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    success: false,
                    output: Value::Null,
                    error: Some("tool not allowed for agent".to_string()),
                })
            } else {
                let ctx = ToolContext {
                    session_id,
                    user_id,
                    db_pool: self.pool.clone(),
                    allowed_schemas: allowed_schemas.to_vec(),
                    action_handler: Some(self.handler.clone()),
                };
                let registry = ToolRegistry::new();
                registry.execute(call, &ctx).await
            }
        } else {
            self.inner
                .execute(call, session_id, user_id, allowed_schemas)
                .await
        };
        outcome.map(cap_tool_result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result_with(output: Value) -> ToolResult {
        ToolResult {
            tool_call_id: "t1".to_string(),
            name: "query_sql".to_string(),
            success: true,
            output,
            error: None,
        }
    }

    // ── C3 工具结果截断（fix-chat-ai-capability-gaps D2.2）────────────

    #[test]
    fn test_truncate_under_cap_unchanged() {
        let text = "x".repeat(4000);
        assert_eq!(truncate_tool_output(&text), text, "恰 4000 原样返回");
        let short = "{\"rows\":[1,2,3]}";
        assert_eq!(truncate_tool_output(short), short);
    }

    #[test]
    fn test_truncate_over_cap_with_marker() {
        let text = "y".repeat(5000);
        let out = truncate_tool_output(&text);
        assert!(out.ends_with("…[结果截断，共 1 行 / 共 5000 字符]"));
        assert_eq!(
            out.chars().count(),
            4000 + "…[结果截断，共 1 行 / 共 5000 字符]".chars().count()
        );
        assert!(out.starts_with(&"y".repeat(4000)));
    }

    #[test]
    fn test_truncate_line_count_stats() {
        // 1000 行 × 5 字符（含换行）= 5000 字符 > 4000；行数从原文统计
        let text = "row-\n".repeat(1000);
        let out = truncate_tool_output(&text);
        assert!(out.contains("共 1000 行"), "行数提示: {}", out);
        assert!(out.contains("共 5000 字符"));
    }

    #[test]
    fn test_cap_tool_result_small_untouched() {
        let small = json!({ "rows": 3, "ok": true });
        let capped = cap_tool_result(result_with(small.clone()));
        assert_eq!(capped.output, small, "≤4000 原样（Value 结构保留）");
        assert!(capped.success);
    }

    #[test]
    fn test_cap_tool_result_large_replaced_with_notice() {
        // 1000 行 × 每行 ~20 字符 → 序列化远超 4000 → 替换为截断文本
        let big_rows: Vec<Value> = (0..1000)
            .map(|i| json!({ "id": i.to_string(), "payload": "some-long-row-value-abcdefgh" }))
            .collect();
        let capped = cap_tool_result(result_with(Value::Array(big_rows)));
        match &capped.output {
            Value::String(s) => {
                assert!(s.contains("结果截断"), "必须含截断提示");
                assert!(s.chars().count() <= 4200, "截断结果整体 ≤4000+提示尾巴");
            }
            other => panic!("大结果必须替换为截断字符串，实际 {:?}", other),
        }
    }
}
