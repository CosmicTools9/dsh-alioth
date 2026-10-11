//! 工具执行器实现

use super::row_value;
use super::{ToolCall, ToolContext, ToolResult};
use crate::agents::tool_orchestrator::level_requires_confirmation;
use crate::agents::ToolDefinition;
use approval::handlers::dmn_assist::dmn_generate_core;
use serde_json::json;
use sqlparser::ast::{
    AccessExpr, Distinct, Expr, FunctionArg, FunctionArgExpr, FunctionArgumentClause,
    FunctionArguments, GroupByExpr, JoinConstraint, JoinOperator, LimitClause, NamedWindowExpr,
    ObjectName, OrderByKind, Query, Select, SelectItem, SetExpr, Subscript, TableFactor,
    TableWithJoins, WindowFrameBound, WindowSpec, WindowType, XmlTableColumnOption,
};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use sqlx::{AssertSqlSafe, Column, Row};

/// 表单校验工具（change `add-form-contract-seam`，ADR D-053）
///
/// 校验口径 = **当前页面登记的表单契约**（`ToolContext.form_contract`），MUST NOT 接受
/// 模型自备 schema——后者是「可校验任意 schema 的第二实现」，已随本 change 退役。
pub struct ValidateFormTool;

impl ValidateFormTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "validate_form".to_string(),
            description: "按**当前页面登记的表单契约**校验字段值（类型/必填/枚举/范围/格式）。\
                契约由页面提供（无需也不接受入参 schema）；契约不可用时返回 CONTRACT_UNAVAILABLE。"
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "field_values": { "type": "object", "description": "待校验的字段值映射" }
                },
                "required": ["field_values"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String> {
        let args = call.arguments.clone();
        let field_values = args.get("field_values").cloned().unwrap_or(json!({}));

        // fail-closed（design D7）：未登记 / 不可解析 / 超限截断的契约一律「不可用」，
        // MUST NOT 放行、MUST NOT 回落模型自备 schema。
        let Some(contract) = ctx.form_contract.as_ref() else {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: json!({
                    "valid": false,
                    "error": "CONTRACT_UNAVAILABLE",
                    "errors": [],
                    "uncovered": [],
                    "appliable": false,
                }),
                error: Some(
                    "CONTRACT_UNAVAILABLE: 当前页面未登记表单契约，无法校验\
                     （MUST NOT 以模型自备 schema 兜底）"
                        .to_string(),
                ),
            });
        };

        let outcome = super::form_contract::validate_against_contract(contract, &field_values);
        let valid = outcome.errors.is_empty();
        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            success: valid,
            output: json!({
                "valid": valid,
                "errors": outcome.errors,
                "uncovered": outcome.uncovered,
                "appliable": outcome.appliable,
            }),
            error: if valid {
                None
            } else {
                Some("表单校验失败".to_string())
            },
        })
    }
}

/// 决策表起草工具（add-dock-dmn-authoring）：NL → approval dmn_generate_core（进程内
/// 复用 HTTP dmn-assist 同源核心）→ 整表强校验 → 结构化返回（供 LLM 组装 kind=execute
/// apply_dmn 动作写回画布选中节点）。
pub struct DmnDraftTool;

impl DmnDraftTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "dmn_draft".to_string(),
            description:
                "为当前选中的决策表节点从自然语言生成完整决策表（DMN JSON），经服务端强校验后返回整表与逐条中文规则说明"
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "自然语言决策规则诉求（如：金额≥10000 且客户等级 A → 大额审批；其余 → 普通审批）" },
                    "context_fields": { "type": "array", "items": { "type": "string" }, "description": "可用上下文字段名清单" },
                    "outputs": { "type": "array", "items": { "type": "string" }, "description": "出边 label 路由候选" },
                    "allow_unlisted": { "type": "boolean", "description": "存在无 label 兜底边时允许输出不在候选" }
                },
                "required": ["message"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String> {
        let args = call.arguments.clone();
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        if message.is_empty() {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: serde_json::Value::Null,
                error: Some("请描述决策规则（message 不能为空）".to_string()),
            });
        }
        let context_fields = args
            .get("context_fields")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let outputs = args
            .get("outputs")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let allow_unlisted = args
            .get("allow_unlisted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        match dmn_generate_core(
            &ctx.db_pool,
            "dmn-draft",
            &message,
            &context_fields,
            &outputs,
            allow_unlisted,
            None,
        )
        .await
        {
            Ok(resp) => Ok(dmn_assist_tool_result(call, resp)),
            Err(e) => Ok(dmn_assist_core_error_result(call, &e)),
        }
    }
}

/// 决策表修订工具（add-dock-dmn-authoring）：携带现有表 + 修订诉求 → dmn_generate_core
/// （dmn-fix 语义：完整新表输出，同强校验）。
pub struct DmnReviseTool;

impl DmnReviseTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "dmn_revise".to_string(),
            description:
                "修订当前选中决策表节点的既有决策表（携带现有表与修订诉求），重新生成完整决策表并经服务端强校验"
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "对现有表的修订诉求或错误描述" },
                    "existing": { "type": "object", "description": "当前决策表节点 dmn JSON（修订基座）" },
                    "context_fields": { "type": "array", "items": { "type": "string" }, "description": "可用上下文字段名清单" },
                    "outputs": { "type": "array", "items": { "type": "string" }, "description": "出边 label 路由候选" },
                    "allow_unlisted": { "type": "boolean", "description": "存在无 label 兜底边时允许输出不在候选" }
                },
                "required": ["message", "existing"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String> {
        let args = call.arguments.clone();
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let existing = args.get("existing").cloned();
        if message.is_empty() {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: serde_json::Value::Null,
                error: Some("请描述修订诉求（message 不能为空）".to_string()),
            });
        }
        let Some(existing) = existing else {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: serde_json::Value::Null,
                error: Some("dmn_revise 需要携带 existing 当前决策表".to_string()),
            });
        };
        let context_fields = args
            .get("context_fields")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let outputs = args
            .get("outputs")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let allow_unlisted = args
            .get("allow_unlisted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        match dmn_generate_core(
            &ctx.db_pool,
            "dmn-revise",
            &message,
            &context_fields,
            &outputs,
            allow_unlisted,
            Some(&existing),
        )
        .await
        {
            Ok(resp) => Ok(dmn_assist_tool_result(call, resp)),
            Err(e) => Ok(dmn_assist_core_error_result(call, &e)),
        }
    }
}

/// 校验通过/失败的结构化返回（fail-closed：valid=false 亦回传 errors，不落写）
fn dmn_assist_tool_result(
    call: &ToolCall,
    resp: approval::handlers::dmn_assist::DmnAssistResponse,
) -> ToolResult {
    ToolResult {
        tool_call_id: call.id.clone(),
        name: call.name.clone(),
        success: resp.valid,
        output: json!({
            "valid": resp.valid,
            "errors": resp.errors,
            "dmn": resp.dmn,
            "rules_explanation": resp.rules_explanation,
            "summary": resp.summary,
            "variable_usage": resp.variable_usage,
        }),
        error: if resp.valid {
            None
        } else {
            Some(if resp.errors.is_empty() {
                "决策表校验未通过".to_string()
            } else {
                resp.errors.join("；")
            })
        },
    }
}

/// 传输层错误（LLM 不可用/调用失败/输出不可解析）→ fail-soft 工具错误结果
fn dmn_assist_core_error_result(
    call: &ToolCall,
    e: &approval::handlers::dmn_assist::DmnAssistCoreError,
) -> ToolResult {
    let msg = match e {
        approval::handlers::dmn_assist::DmnAssistCoreError::LlmUnavailable(_) => {
            "LLM 服务不可用（决策表起草暂不可用）".to_string()
        }
        approval::handlers::dmn_assist::DmnAssistCoreError::LlmCallFailed(_) => {
            "LLM 调用失败（请重试或调整描述）".to_string()
        }
        approval::handlers::dmn_assist::DmnAssistCoreError::BadOutput(m) => m.clone(),
    };
    ToolResult {
        tool_call_id: call.id.clone(),
        name: call.name.clone(),
        success: false,
        output: serde_json::Value::Null,
        error: Some(msg),
    }
}

/// 执行预定义动作工具
pub struct ExecuteActionTool;

impl ExecuteActionTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "execute_action".to_string(),
            description:
                "执行预定义业务动作（如状态推进、批量审批、生成单据等）。需要用户确认级别检查。"
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    // 枚举由动作目录生成（单一真相源，change add-action-catalogue）：
                    // MUST NOT 手写字面量清单。仅 `expose_to_llm` 项可见（NGAC /
                    // 系统设置动作只经提案 actions[] 可达，行为不变）。
                    "action_type": { "type": "string", "enum": crate::tools::action_catalogue::ActionCatalogue::llm_exposed_ids(), "description": "动作类型" },
                    "target_ids": { "type": "array", "items": { "type": "integer" }, "description": "目标实体 ID 列表" },
                    "params": { "type": "object", "description": "动作参数" },
                    "confirmed": { "type": "boolean", "description": "已废弃——Explicit/High/Critical 级动作一律需走用户确认通道（HTTP execute-action），本工具恒返回 needs_confirmation 提案，confirmed 值不再影响执行" }
                },
                "required": ["action_type", "target_ids"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String> {
        let args = call.arguments.clone();
        let action_type = args
            .get("action_type")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'action_type' parameter")?;
        let targets_raw = args
            .get("target_ids")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let params = args.get("params").cloned().unwrap_or_else(|| json!({}));

        // 无 handler（测试/无业务注入场景）：维持现状返回预览，不实际执行
        let Some(handler) = ctx.action_handler.as_deref() else {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: true,
                output: json!({
                    "action_type": action_type,
                    "target_count": targets_raw.len(),
                    "targets": targets_raw,
                    "status": "preview",
                    "message": "动作已预览，需用户确认后执行"
                }),
                error: None,
            });
        };

        // 确认门禁（S1 强化）：Explicit/High/Critical 级动作的确认不可由 LLM
        // 自证——无视 args.confirmed 值，恒返回 needs_confirmation 结构化提案；
        // 真实执行仅经 HTTP execute-action 端点（confirmed 来自用户确认弹窗点击）。
        // None/Preview/Low 级保持既有语义（直接执行）。
        let level = handler.confirmation_level(action_type);
        if level_requires_confirmation(&level) {
            let id = format!("agent_action:{}", action_type);
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: true,
                output: json!({
                    "id": id.to_string(),
                    "action_type": action_type,
                    "target_ids": targets_raw, // id-json-ok：LLM 工具参数原样回传（Rust 侧 i64 精确），执行端经 HTTP params 中继的 JS 精度面属 Gateway 流程
                    "target_count": targets_raw.len(),
                    "params": params,
                    "status": "needs_confirmation",
                    "confirm_url_hint": format!("/api/chat-sessions/{}/execute-action", ctx.session_id),
                    "message": "该动作需用户确认后经 HTTP execute-action 执行（LLM 工具路径不可自证确认）"
                }),
                error: None,
            });
        }

        // 真实执行：委托宿主 ActionHandler（非整数 target 条目被过滤）
        let target_ids: Vec<i64> = targets_raw.iter().filter_map(|v| v.as_i64()).collect();
        match handler
            .execute(action_type, &target_ids, &params, ctx)
            .await
        {
            Ok(output) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: true,
                output,
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: json!(null),
                error: Some(e),
            }),
        }
    }
}

/// 权限规范（org-policy）提案工具
/// （route-ngac-policy-authoring-through-ai-assist 2.1）。
///
/// 只把 LLM 整理的规范操作打包为 `needs_confirmation` 提案——工具路径
/// MUST NOT 落库、MUST NOT 自证确认（ADR D-051）；真实写入仅经 HTTP
/// `execute-action`（确认来自用户点击）→ Gateway 动作 `ngac_org_policy_apply`
/// → SSO org-policy 服务层（Rhai 强校验 + 状态机 + 审计等价保留）。
///
/// 表达式字段（condition/obligation/prohibition_template）值 MUST 为 Rhai
/// 源码字符串（结构化占位在工具层即拒）；标识符级强校验在服务端写径完成
/// （唯一引擎）。
pub struct NgacPolicyDraftTool;

/// org-policy 提案操作白名单（与 SSO 服务层内容写面一一对应；状态机迁移不在列）。
const NGAC_POLICY_OPS: &[&str] = &[
    "upsert_class",
    "patch_class",
    "create_rule",
    "patch_rule",
    "delete_rule",
    "create_label",
    "patch_label",
    "delete_label",
];

/// 表达式类字段（值 MUST 为 Rhai 源码字符串；空串 = 未设置）。
const NGAC_EXPR_FIELDS: &[&str] = &["condition", "obligation", "prohibition_template"];

/// 单操作形状校验（工具层 fail-closed；错误文本可行动，供 LLM 修正重试）。
fn validate_policy_operation(op: &serde_json::Value) -> Result<(), String> {
    let o = op
        .as_object()
        .ok_or_else(|| "operation 必须为 JSON 对象".to_string())?;
    let name = o
        .get("op")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "operation 缺少 op 字段".to_string())?;
    if !NGAC_POLICY_OPS.contains(&name) {
        return Err(format!(
            "未知 op '{name}'（白名单：{}）",
            NGAC_POLICY_OPS.join("/")
        ));
    }
    let need_str = |f: &str| -> Result<(), String> {
        if o.get(f)
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            == Some(true)
        {
            Ok(())
        } else {
            Err(format!("{name}: 缺少必填字符串字段 {f}"))
        }
    };
    let need_id = |f: &str| -> Result<(), String> {
        if o.get(f).and_then(|v| v.as_i64()).is_some() {
            Ok(())
        } else {
            Err(format!("{name}: 缺少必填整数 id 字段 {f}"))
        }
    };
    match name {
        "upsert_class" => {
            need_str("code")?;
            need_str("notice")?;
        }
        "patch_class" => {
            need_id("class_id")?;
        }
        "create_rule" => {
            need_id("policy_class_id")?;
            need_str("subject_code")?;
            need_str("resource_type")?;
            if o.get("actions").and_then(|v| v.as_array()).is_none() {
                return Err(format!("{name}: 缺少必填数组字段 actions"));
            }
        }
        "patch_rule" | "delete_rule" => {
            need_id("rule_id")?;
        }
        "create_label" => {
            need_str("code")?;
            need_id("rank")?;
            need_str("notice")?;
        }
        "patch_label" | "delete_label" => {
            need_str("code")?;
        }
        _ => unreachable!("op 已在白名单内"),
    }
    // 表达式字段：存在则为非空白字符串（Rhai 源码；结构化占位拒绝）
    for f in NGAC_EXPR_FIELDS {
        match o.get(*f) {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {}
            Some(_) => {
                return Err(format!(
                    "{name}: {f} MUST 为 Rhai 源码字符串（结构化占位不受支持）"
                ));
            }
        }
    }
    Ok(())
}

impl NgacPolicyDraftTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "ngac_policy_draft".to_string(),
            description: "将权限规范（org_policy）新建/修改意图整理为待用户确认的提案（不落库）。\
经用户确认后经确认通道执行（服务端 admin 门控 + Rhai 强校验 + 状态机 + 审计）。\
表达式字段（condition/obligation/prohibition_template）值 MUST 为 Rhai 源码字符串。\
可用键按字段区分：condition / prohibition_template = user_id、resource、action、\
subject_attributes、object_attributes（**不含** decision、permitted——条件在判决终态前求值，\
引用即被写径拒绝）；obligation（事件模式 / 义务算子）= 上述五键 + decision、permitted。\
状态机迁移（提交评审/激活/退役）不在本工具范围。"
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "operations": {
                        "type": "array",
                        "minItems": 1,
                        "description": "按序执行的规范内容操作",
                        "items": {
                            "type": "object",
                            "required": ["op"],
                            "properties": {
                                "op": {
                                    "type": "string",
                                    "enum": ["upsert_class", "patch_class", "create_rule",
                                             "patch_rule", "delete_rule", "create_label",
                                             "patch_label", "delete_label"],
                                    "description": "upsert_class=新建类（code 未被未退休行占用）；patch_class=更新类内容（仅 draft/in_review）；create_rule=挂接规则行（仅所属类 draft/in_review）；delete_rule=软删规则行"
                                },
                                "code": { "type": "string", "description": "upsert_class：规范码；create/patch/delete_label：分级码" },
                                "class_id": { "type": "integer", "description": "patch_class：目标类 id" },
                                "policy_class_id": { "type": "integer", "description": "create_rule：所属类 id" },
                                "rule_id": { "type": "integer", "description": "patch_rule / delete_rule：目标规则 id" },
                                "notice": { "type": "string", "description": "名称/说明（upsert_class、create_label 必填）" },
                                "scope": { "type": "object", "description": "{\"resource_types\": [\"...\"], \"actions\": [\"...\"]} 白名单域" },
                                "ua_template": { "type": "object", "description": "{\"name_rule\": \"...\"} 主体 UA 模板" },
                                "label_code": { "type": "string" },
                                "audit_required": { "type": "boolean" },
                                "comments": { "type": "string" },
                                "subject_code": { "type": "string", "description": "create_rule 必填：主体代码（UA 名或模板）" },
                                "resource_type": { "type": "string", "description": "create_rule 必填" },
                                "actions": { "type": "array", "items": { "type": "string" }, "description": "create_rule 必填：动作清单" },
                                "condition": { "type": "string", "description": "Rhai 条件源码（空串 = 未设置）" },
                                "obligation": { "type": "string", "description": "Rhai 义务源码" },
                                "prohibition_template": { "type": "string", "description": "Rhai 禁止模板源码" },
                                "rank": { "type": "integer", "description": "create_label 必填：分级序（越大越敏感）" },
                                "domain": { "type": "string", "description": "create/patch_label：分级域（缺省 security）" },
                                "summary": { "type": "string", "description": "一句话中文摘要（确认弹窗展示用）" }
                            }
                        }
                    }
                },
                "required": ["operations"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String> {
        let ops = call
            .arguments
            .get("operations")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if ops.is_empty() {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: json!(null),
                error: Some("operations 不能为空（至少一项规范操作）".to_string()),
            });
        }
        let mut cleaned: Vec<serde_json::Value> = Vec::with_capacity(ops.len());
        for (i, op) in ops.iter().enumerate() {
            if let Err(e) = validate_policy_operation(op) {
                return Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    success: false,
                    output: json!(null),
                    error: Some(format!("operations[{i}] 形状不合法：{e}")),
                });
            }
            cleaned.push(op.clone());
        }
        // 提案（id 非 `agent_action:` 前缀：确认经 structured.actions 记录解析，
        // params 承载 operations；工具路径零写入）
        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            success: true,
            output: json!({
                "id": "ngac:org-policy-apply",
                "label": format!("应用 {} 项权限规范变更", cleaned.len()),
                "kind": "execute",
                "action_type": "ngac_org_policy_apply",
                "target_ids": [],
                "params": { "operations": cleaned },
                "status": "needs_confirmation",
                "confirm_url_hint": format!("/api/chat-sessions/{}/execute-action", ctx.session_id),
                "message": "提案已生成：请用户确认后经 HTTP execute-action 执行（工具路径不落库）"
            }),
            error: None,
        })
    }
}

/// NGAC 策略操作提案工具（wire-nl-assist-into-assistant）。
///
/// 只把 LLM 整理的策略操作（关联/禁止/UA 指派）打包为 `needs_confirmation` 提案——
/// 工具路径 MUST NOT 落库、MUST NOT 自证确认（提案-确认纪律，ADR D-051）；
/// 真实写入仅经 HTTP `execute-action`（确认来自用户点击）→ Gateway 动作
/// `ngac_strategy_apply` → 单事务「业务写 + 行镜像 + 策略变更审计」（admin 门控）。
///
/// 操作白名单单一真相源 = [`crate::tools::ngac_ops::POLICY_OPS`]（nl-assist 校验 /
/// 本工具层提案校验 / 执行体分支集合三处同源；change `consolidate-ngac-write-path`）。
pub struct NgacStrategyDraftTool;

impl NgacStrategyDraftTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "ngac_strategy_draft".to_string(),
            description:
                "将 NGAC 策略操作意图（关联/禁止/UA 指派）整理为待用户确认的提案（不落库）。\
经用户确认后经确认通道执行（服务端 admin 门控 + 审计）。\
操作类型：create/update/delete association、create/update/delete prohibition、assign/remove_ua_assignment。\
UA/OA/用户 id MUST 先用 query_sql 查实，MUST NOT 编造。"
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "operations": {
                        "type": "array",
                        "minItems": 1,
                        "description": "按序执行的策略操作",
                        "items": {
                            "type": "object",
                            "required": ["op"],
                            "properties": {
                                "op": {
                                    "type": "string",
                                    "enum": crate::tools::ngac_ops::POLICY_OPS,
                                    "description": "操作类型"
                                },
                                "ua_id": { "type": "integer", "description": "create/update/delete association、create/update/delete prohibition、assign/remove_ua_assignment：目标 UA id" },
                                "oa_id": { "type": "integer", "description": "create association、create prohibition：目标 OA id" },
                                "association_id": { "type": "integer", "description": "update/delete_association：目标关联 id" },
                                "prohibition_id": { "type": "integer", "description": "update/delete_prohibition：目标禁止 id" },
                                "user_id": { "type": "integer", "description": "assign_ua / remove_ua_assignment：目标用户 id" },
                                "actions": { "type": "array", "items": { "type": "string" }, "description": "授权动作白名单（如 [\"read\",\"write\"]）" },
                                "o_name": { "type": "string", "description": "update_association：新的关联名" },
                                "conditions": { "type": "object", "description": "update_*：新的条件对象" },
                                "is_active": { "type": "boolean", "description": "update_prohibition：是否生效" }
                            }
                        }
                    }
                },
                "required": ["operations"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, String> {
        let args = &call.arguments;
        let ops = args
            .get("operations")
            .and_then(|v| v.as_array())
            .ok_or("Missing 'operations' parameter")?;
        if ops.is_empty() {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: serde_json::Value::Null,
                error: Some("EMPTY_OPERATIONS: operations 不能为空".to_string()),
            });
        }
        for (i, op) in ops.iter().enumerate() {
            let op_type = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
            if !crate::tools::ngac_ops::POLICY_OPS.contains(&op_type) {
                return Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    success: false,
                    output: serde_json::Value::Null,
                    error: Some(format!(
                        "INVALID_OP[{}]: '{}' 不在白名单 {:?}",
                        i,
                        op_type,
                        crate::tools::ngac_ops::POLICY_OPS
                    )),
                });
            }
        }
        // 提案-确认纪律：恒返回 needs_confirmation，零写入
        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            success: true,
            output: json!({
                "status": "needs_confirmation",
                "action_type": "ngac_strategy_apply",
                "confirm_url_hint": format!("/api/chat-sessions/{}/execute-action", _ctx.session_id),
                "message": "该策略操作需用户确认后经 HTTP execute-action 执行（LLM 工具路径不可自证确认）",
                "operations": ops,
            }),
            error: None,
        })
    }
}

/// 系统设置提案工具（add-system-config-ai-action 2.2）。
///
/// 只把 LLM 整理的系统设置变更打包为 `needs_confirmation` 提案——工具路径
/// MUST NOT 落库、MUST NOT 自证确认（提案-确认纪律）；真实写入仅经 HTTP
/// `execute-action`（确认来自用户点击）→ Gateway 动作 `system_config_apply`
/// → `system-config` 服务层（凭证加密 + change D 审计口径等价）。
///
/// 凭证：`credentials` 仅**专属键**承载（MUST NOT 混入 `settings`；非对象拒绝）；
/// 动作回执/审计 metadata/日志 MUST NOT 回显凭证值。
pub struct SystemConfigDraftTool;

/// 系统设置提案操作白名单（与服务层 CRUD 一一对应）。
const SYSTEM_CONFIG_OPS: &[&str] = &["create_config", "update_config", "delete_config"];

/// 单操作形状校验（工具层 fail-closed；错误文本可行动，供 LLM 修正重试）。
fn validate_system_config_operation(op: &serde_json::Value) -> Result<(), String> {
    let o = op
        .as_object()
        .ok_or_else(|| "operation 必须为 JSON 对象".to_string())?;
    let name = o
        .get("op")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "operation 缺少 op 字段".to_string())?;
    if !SYSTEM_CONFIG_OPS.contains(&name) {
        return Err(format!(
            "未知 op '{name}'（白名单：{}）",
            SYSTEM_CONFIG_OPS.join("/")
        ));
    }
    let need_str = |f: &str| -> Result<(), String> {
        if o.get(f)
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            == Some(true)
        {
            Ok(())
        } else {
            Err(format!("{name}: 缺少必填字符串字段 {f}"))
        }
    };
    let need_id = |f: &str| -> Result<(), String> {
        if o.get(f).and_then(|v| v.as_i64()).is_some() {
            Ok(())
        } else {
            Err(format!("{name}: 缺少必填整数 id 字段 {f}"))
        }
    };
    // 对象型可选字段（凭证专属键约束：非对象拒绝）
    for f in ["settings", "credentials"] {
        match o.get(f) {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::Object(_)) => {}
            Some(_) => return Err(format!("{name}: {f} MUST 为 JSON 对象")),
        }
    }
    match name {
        "create_config" => {
            need_str("category")?;
            need_str("provider")?;
        }
        "update_config" | "delete_config" => {
            need_id("id")?;
        }
        _ => unreachable!("op 已在白名单内"),
    }
    Ok(())
}

impl SystemConfigDraftTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "system_config_draft".to_string(),
            description: "将系统设置（system-config：LLM/email/sms/storage 等 provider 行）变更意图整理为待用户确认的提案（不落库）。经用户确认后经确认通道执行（admin 门控 + 凭证加密 + system_config.* 审计）。凭证（api_key/password 等）MUST 放 credentials 对象（禁止混入 settings），且 MUST NOT 在你的回复正文中重复凭证值；不确定 category/provider 取值时先读列表/分类。"
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "operations": {
                        "type": "array",
                        "minItems": 1,
                        "description": "按序执行的系统设置操作",
                        "items": {
                            "type": "object",
                            "required": ["op"],
                            "properties": {
                                "op": { "type": "string", "enum": ["create_config", "update_config", "delete_config"], "description": "create_config=新建 provider 行；update_config=更新（id 必填）；delete_config=软删（id 必填）" },
                                "id": { "type": "integer", "description": "update_config / delete_config：目标行 id" },
                                "category": { "type": "string", "description": "create_config 必填：分类码（如 llm/email/sms/storage）" },
                                "provider": { "type": "string", "description": "create_config 必填：provider 码（如 deepseek/smtp）" },
                                "code": { "type": "string", "description": "行编码（可选）" },
                                "notice": { "type": "string" },
                                "settings": { "type": "object", "description": "非敏感设置（MUST NOT 混入凭证键）" },
                                "credentials": { "type": "object", "description": "敏感凭证专属键（服务端按 schema 加密；MUST NOT 出现在 settings / 回复正文）" },
                                "enabled": { "type": "boolean" },
                                "is_default": { "type": "boolean" },
                                "domain_": { "type": "string" },
                                "comments": { "type": "string" },
                                "summary": { "type": "string", "description": "一句话中文摘要（确认弹窗展示用）" }
                            }
                        }
                    }
                },
                "required": ["operations"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String> {
        let ops = call
            .arguments
            .get("operations")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if ops.is_empty() {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: json!(null),
                error: Some("operations 不能为空（至少一项系统设置操作）".to_string()),
            });
        }
        let mut cleaned: Vec<serde_json::Value> = Vec::with_capacity(ops.len());
        for (i, op) in ops.iter().enumerate() {
            if let Err(e) = validate_system_config_operation(op) {
                return Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    success: false,
                    output: json!(null),
                    error: Some(format!("operations[{i}] 形状不合法：{e}")),
                });
            }
            cleaned.push(op.clone());
        }
        // 提案（id 非 `agent_action:` 前缀：确认经 structured 记录解析，params 承载 operations）
        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            success: true,
            output: json!({
                "id": "syscfg:apply",
                "label": format!("应用 {} 项系统设置变更", cleaned.len()),
                "kind": "execute",
                "action_type": "system_config_apply",
                "target_ids": [],
                "params": { "operations": cleaned },
                "status": "needs_confirmation",
                "confirm_url_hint": format!("/api/chat-sessions/{}/execute-action", ctx.session_id),
                "message": "提案已生成：请用户确认后经 HTTP execute-action 执行（工具路径不落库）"
            }),
            error: None,
        })
    }
}

// =====================================================================
// S3/S4：query_sql 的 AST 级安全检查（零新依赖，手写递归遍历）
//
// 替换原先对原始 SQL 文本做 contains 的脆弱校验——注释/字符串字面量可伪造
// 关键字绕过 contains，AST 提取则天然无视注释与字面量。sqlparser 的 visitor
// feature 未启用（A2：零依赖零 lockfile churn），此处手写递归覆盖 PostgreSQL
// 可解析的全部 Query 形状：FROM/JOIN/派生表/CTE/集合运算/表达式内嵌子查询
// （IN/EXISTS/标量子查询/函数参数/窗口子句/XMLTABLE 等）。
// =====================================================================

/// 单条 SELECT Query 的用法审计结果。
#[derive(Default)]
struct QueryUsage {
    /// 表引用：(显式 schema, 完整引用名)。schema = None 表示单段表名——
    /// 由搜索路径解析，不算显式外部引用。
    tables: Vec<(Option<String>, String)>,
    /// 函数调用名（取限定名末段并转小写，规避大小写/限定名绕过）。
    functions: Vec<String>,
}

/// S4 副作用/管理函数黑名单（精确名，小写）——进程控制、写/重置会话、
/// 阻塞、通知、序列推进、服务端文件读取、远程查询执行等对只读查询工具
/// 一律拒绝。
const FUNCTION_BLACKLIST: &[&str] = &[
    "pg_terminate_backend",
    "pg_cancel_backend",
    "set_config",
    "pg_sleep",
    "pg_notify",
    "nextval",
    "setval",
    "pg_read_file",
    "pg_ls_dir",
    "pg_read_binary_file",
    // dblink 裸名=远程查询执行（dblink_connect/dblink_exec 等另由前缀族覆盖）
    "dblink",
];

/// S4 黑名单前缀族（小写）——远程数据库调用（dblink_*）与大对象读写（lo_*）。
const FUNCTION_BLACKLIST_PREFIXES: &[&str] = &["dblink_", "lo_"];

/// 函数对象名 → 末段小写（未限定函数名即其本身）。
fn function_name_key(name: &ObjectName) -> String {
    name.0
        .last()
        .and_then(|part| part.as_ident())
        .map(|ident| ident.value.to_lowercase())
        .unwrap_or_default()
}

/// 收集一条 Query（含 CTE/子查询/集合运算/ORDER BY/分页表达式槽）的全部
/// 表引用与函数调用。
fn collect_query_usage(q: &Query, usage: &mut QueryUsage) {
    if let Some(with) = &q.with {
        for cte in &with.cte_tables {
            collect_query_usage(&cte.query, usage);
        }
    }
    collect_set_expr_usage(&q.body, usage);
    if let Some(order_by) = &q.order_by {
        if let OrderByKind::Expressions(exprs) = &order_by.kind {
            for ob in exprs {
                collect_expr_usage(&ob.expr, usage);
            }
        }
    }
    // LIMIT/OFFSET 表达式槽（limit 与 offset.value 均可是表达式，如
    // `LIMIT pg_sleep(1)`；含 MySQL 逗号形态与 ClickHouse LIMIT BY）
    if let Some(lc) = &q.limit_clause {
        match lc {
            LimitClause::LimitOffset {
                limit,
                offset,
                limit_by,
            } => {
                if let Some(limit) = limit {
                    collect_expr_usage(limit, usage);
                }
                if let Some(offset) = offset {
                    collect_expr_usage(&offset.value, usage);
                }
                for e in limit_by {
                    collect_expr_usage(e, usage);
                }
            }
            LimitClause::OffsetCommaLimit { offset, limit } => {
                collect_expr_usage(offset, usage);
                collect_expr_usage(limit, usage);
            }
        }
    }
    // FETCH FIRST <quantity> ROWS 表达式槽
    if let Some(fetch) = &q.fetch {
        if let Some(quantity) = &fetch.quantity {
            collect_expr_usage(quantity, usage);
        }
    }
}

fn collect_set_expr_usage(se: &SetExpr, usage: &mut QueryUsage) {
    match se {
        SetExpr::Select(select) => collect_select_usage(select, usage),
        SetExpr::Query(q) => collect_query_usage(q, usage),
        SetExpr::SetOperation { left, right, .. } => {
            collect_set_expr_usage(left, usage);
            collect_set_expr_usage(right, usage);
        }
        SetExpr::Values(values) => {
            for row in &values.rows {
                for e in &row.content {
                    collect_expr_usage(e, usage);
                }
            }
        }
        // `TABLE <name>`（等价 SELECT * FROM <name>）
        SetExpr::Table(tbl) => {
            let full = match (&tbl.schema_name, &tbl.table_name) {
                (Some(schema), Some(name)) => format!("{schema}.{name}"),
                (_, Some(name)) => name.clone(),
                _ => String::new(),
            };
            if !full.is_empty() {
                usage.tables.push((tbl.schema_name.clone(), full));
            }
        }
        // DML 不可能出现在已被放行的单条 SELECT 内（防御性占位）
        SetExpr::Insert(_) | SetExpr::Update(_) | SetExpr::Delete(_) | SetExpr::Merge(_) => {}
    }
}

fn collect_select_usage(s: &Select, usage: &mut QueryUsage) {
    // DISTINCT ON (...) 表达式槽（PG 扩展，表达式可为函数调用）
    if let Some(Distinct::On(exprs)) = &s.distinct {
        for e in exprs {
            collect_expr_usage(e, usage);
        }
    }
    for item in &s.projection {
        match item {
            SelectItem::UnnamedExpr(e) => collect_expr_usage(e, usage),
            SelectItem::ExprWithAlias { expr, .. } => collect_expr_usage(expr, usage),
            SelectItem::ExprWithAliases { expr, .. } => collect_expr_usage(expr, usage),
            SelectItem::QualifiedWildcard(..) | SelectItem::Wildcard(_) => {}
        }
    }
    for twj in &s.from {
        collect_from_usage(twj, usage);
    }
    if let Some(selection) = &s.selection {
        collect_expr_usage(selection, usage);
    }
    if let GroupByExpr::Expressions(exprs, _) = &s.group_by {
        for e in exprs {
            collect_expr_usage(e, usage);
        }
    }
    if let Some(having) = &s.having {
        collect_expr_usage(having, usage);
    }
    if let Some(qualify) = &s.qualify {
        collect_expr_usage(qualify, usage);
    }
    for nwd in &s.named_window {
        if let NamedWindowExpr::WindowSpec(spec) = &nwd.1 {
            collect_window_spec_usage(spec, usage);
        }
    }
}

fn collect_from_usage(twj: &TableWithJoins, usage: &mut QueryUsage) {
    collect_table_factor_usage(&twj.relation, usage);
    for join in &twj.joins {
        collect_table_factor_usage(&join.relation, usage);
        match &join.join_operator {
            JoinOperator::Join(c)
            | JoinOperator::Inner(c)
            | JoinOperator::Left(c)
            | JoinOperator::LeftOuter(c)
            | JoinOperator::Right(c)
            | JoinOperator::RightOuter(c)
            | JoinOperator::FullOuter(c)
            | JoinOperator::CrossJoin(c)
            | JoinOperator::Semi(c)
            | JoinOperator::LeftSemi(c)
            | JoinOperator::RightSemi(c)
            | JoinOperator::Anti(c)
            | JoinOperator::LeftAnti(c)
            | JoinOperator::RightAnti(c)
            | JoinOperator::StraightJoin(c) => collect_join_constraint_usage(c, usage),
            JoinOperator::AsOf {
                match_condition,
                constraint,
            } => {
                collect_expr_usage(match_condition, usage);
                collect_join_constraint_usage(constraint, usage);
            }
            JoinOperator::CrossApply
            | JoinOperator::OuterApply
            | JoinOperator::ArrayJoin
            | JoinOperator::LeftArrayJoin
            | JoinOperator::InnerArrayJoin => {}
        }
    }
}

fn collect_join_constraint_usage(constraint: &JoinConstraint, usage: &mut QueryUsage) {
    if let JoinConstraint::On(expr) = constraint {
        collect_expr_usage(expr, usage);
    }
}

fn collect_table_factor_usage(tf: &TableFactor, usage: &mut QueryUsage) {
    match tf {
        TableFactor::Table { name, args, .. } => {
            let full = name.to_string();
            // 多段名（schema.table / catalog.schema.table）——首段为显式 schema；
            // 单段表名走搜索路径（None）。ObjectNamePart 的 Function 形态非 PG
            // 可解析，按无显式 schema 处理。
            let explicit_schema = (name.0.len() >= 2)
                .then(|| name.0.first().and_then(|part| part.as_ident()))
                .flatten()
                .map(|ident| ident.value.clone());
            usage.tables.push((explicit_schema, full));
            // PG 中 `FROM func(...)` 解析为带 args 的表因子——名字本身即一次
            // 函数调用（纳入 S4 黑名单比对，如 FROM pg_ls_dir('/')），
            // 参数内嵌表达式递归（可能含子查询）。
            if let Some(table_args) = args {
                usage.functions.push(function_name_key(name));
                for arg in &table_args.args {
                    collect_fn_arg_usage(arg, usage);
                }
            }
        }
        TableFactor::Derived { subquery, .. } => collect_query_usage(subquery, usage),
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => collect_from_usage(table_with_joins, usage),
        TableFactor::UNNEST { array_exprs, .. } => {
            for e in array_exprs {
                collect_expr_usage(e, usage);
            }
        }
        // 非 PG 方言形状，兜底递归不遗漏内嵌表达式
        TableFactor::TableFunction { expr, .. } => collect_expr_usage(expr, usage),
        TableFactor::Function { name, args, .. } => {
            usage.functions.push(function_name_key(name));
            for arg in args {
                collect_fn_arg_usage(arg, usage);
            }
        }
        // XMLTABLE（PG 可解析）：PASSING/行表达式/列 PATH/DEFAULT 均可携带函数调用
        TableFactor::XmlTable {
            namespaces,
            row_expression,
            passing,
            columns,
            ..
        } => {
            for ns in namespaces {
                collect_expr_usage(&ns.uri, usage);
            }
            collect_expr_usage(row_expression, usage);
            for arg in &passing.arguments {
                collect_expr_usage(&arg.expr, usage);
            }
            for col in columns {
                if let XmlTableColumnOption::NamedInfo { path, default, .. } = &col.option {
                    if let Some(p) = path {
                        collect_expr_usage(p, usage);
                    }
                    if let Some(d) = default {
                        collect_expr_usage(d, usage);
                    }
                }
            }
        }
        TableFactor::JsonTable { json_expr, .. } => collect_expr_usage(json_expr, usage),
        TableFactor::OpenJsonTable { json_expr, .. } => collect_expr_usage(json_expr, usage),
        // Pivot/Unpivot/MatchRecognize/SemanticView——PostgreSQL 方言不可解析
        _ => {}
    }
}

/// 递归遍历表达式树：收集函数调用（含窗口函数 OVER 子句内表达式）与
/// 内嵌子查询（IN/EXISTS/标量/函数参数等）的用法。
fn collect_expr_usage(e: &Expr, usage: &mut QueryUsage) {
    match e {
        // —— 叶子：无嵌套表达式/子查询 ——
        Expr::Identifier(_)
        | Expr::CompoundIdentifier(_)
        | Expr::Value(_)
        | Expr::TypedString(_)
        | Expr::Wildcard(_)
        | Expr::QualifiedWildcard(_, _)
        | Expr::MatchAgainst { .. }
        | Expr::Dictionary(_)
        | Expr::Map(_)
        | Expr::Lambda(_)
        | Expr::MemberOf(_) => {}

        // —— 单子表达式包装 ——
        Expr::IsFalse(x)
        | Expr::IsNotFalse(x)
        | Expr::IsTrue(x)
        | Expr::IsNotTrue(x)
        | Expr::IsNull(x)
        | Expr::IsNotNull(x)
        | Expr::IsUnknown(x)
        | Expr::IsNotUnknown(x)
        | Expr::UnaryOp { expr: x, .. }
        | Expr::Nested(x)
        | Expr::Cast { expr: x, .. }
        | Expr::Extract { expr: x, .. }
        | Expr::Ceil { expr: x, .. }
        | Expr::Floor { expr: x, .. }
        | Expr::Collate { expr: x, .. }
        | Expr::OuterJoin(x)
        | Expr::Prior(x)
        | Expr::IsNormalized { expr: x, .. }
        | Expr::IsJson { expr: x, .. }
        | Expr::Named { expr: x, .. }
        | Expr::Prefixed { value: x, .. }
        | Expr::JsonAccess { value: x, .. } => collect_expr_usage(x, usage),

        // —— 双子表达式 ——
        Expr::IsDistinctFrom(l, r) | Expr::IsNotDistinctFrom(l, r) => {
            collect_expr_usage(l, usage);
            collect_expr_usage(r, usage);
        }
        Expr::BinaryOp { left, right, .. }
        | Expr::AnyOp { left, right, .. }
        | Expr::AllOp { left, right, .. } => {
            collect_expr_usage(left, usage);
            collect_expr_usage(right, usage);
        }
        Expr::Like { expr, pattern, .. }
        | Expr::ILike { expr, pattern, .. }
        | Expr::SimilarTo { expr, pattern, .. }
        | Expr::RLike { expr, pattern, .. } => {
            collect_expr_usage(expr, usage);
            collect_expr_usage(pattern, usage);
        }
        Expr::AtTimeZone {
            timestamp,
            time_zone,
        } => {
            collect_expr_usage(timestamp, usage);
            collect_expr_usage(time_zone, usage);
        }
        Expr::Position { expr, r#in } => {
            collect_expr_usage(expr, usage);
            collect_expr_usage(r#in, usage);
        }

        // —— 三/多子表达式 ——
        Expr::Between {
            expr, low, high, ..
        } => {
            collect_expr_usage(expr, usage);
            collect_expr_usage(low, usage);
            collect_expr_usage(high, usage);
        }
        Expr::Convert { expr, styles, .. } => {
            collect_expr_usage(expr, usage);
            for s in styles {
                collect_expr_usage(s, usage);
            }
        }
        Expr::Substring {
            expr,
            substring_from,
            substring_for,
            ..
        } => {
            collect_expr_usage(expr, usage);
            if let Some(from) = substring_from {
                collect_expr_usage(from, usage);
            }
            if let Some(for_expr) = substring_for {
                collect_expr_usage(for_expr, usage);
            }
        }
        Expr::Trim {
            trim_what,
            expr,
            trim_characters,
            ..
        } => {
            if let Some(what) = trim_what {
                collect_expr_usage(what, usage);
            }
            collect_expr_usage(expr, usage);
            if let Some(chars) = trim_characters {
                for c in chars {
                    collect_expr_usage(c, usage);
                }
            }
        }
        Expr::Overlay {
            expr,
            overlay_what,
            overlay_from,
            overlay_for,
        } => {
            collect_expr_usage(expr, usage);
            collect_expr_usage(overlay_what, usage);
            collect_expr_usage(overlay_from, usage);
            if let Some(for_expr) = overlay_for {
                collect_expr_usage(for_expr, usage);
            }
        }
        Expr::Case {
            operand,
            conditions,
            else_result,
            ..
        } => {
            if let Some(op) = operand {
                collect_expr_usage(op, usage);
            }
            for when in conditions {
                collect_expr_usage(&when.condition, usage);
                collect_expr_usage(&when.result, usage);
            }
            if let Some(otherwise) = else_result {
                collect_expr_usage(otherwise, usage);
            }
        }
        Expr::InList { expr, list, .. } => {
            collect_expr_usage(expr, usage);
            for item in list {
                collect_expr_usage(item, usage);
            }
        }
        Expr::InUnnest {
            expr, array_expr, ..
        } => {
            collect_expr_usage(expr, usage);
            collect_expr_usage(array_expr, usage);
        }
        Expr::GroupingSets(groups) | Expr::Cube(groups) | Expr::Rollup(groups) => {
            for group in groups {
                for e in group {
                    collect_expr_usage(e, usage);
                }
            }
        }
        Expr::Tuple(exprs) => {
            for e in exprs {
                collect_expr_usage(e, usage);
            }
        }
        Expr::Struct { values, .. } => {
            for v in values {
                collect_expr_usage(v, usage);
            }
        }
        Expr::Array(array) => {
            for e in &array.elem {
                collect_expr_usage(e, usage);
            }
        }
        Expr::Interval(interval) => collect_expr_usage(&interval.value, usage),

        // —— 复合字段访问（数组下标/切片等，下标可含表达式）——
        Expr::CompoundFieldAccess { root, access_chain } => {
            collect_expr_usage(root, usage);
            for acc in access_chain {
                match acc {
                    AccessExpr::Dot(inner) => collect_expr_usage(inner, usage),
                    AccessExpr::Subscript(sub) => match sub {
                        Subscript::Index { index } => collect_expr_usage(index, usage),
                        Subscript::Slice {
                            lower_bound,
                            upper_bound,
                            stride,
                        } => {
                            if let Some(e) = lower_bound {
                                collect_expr_usage(e, usage);
                            }
                            if let Some(e) = upper_bound {
                                collect_expr_usage(e, usage);
                            }
                            if let Some(e) = stride {
                                collect_expr_usage(e, usage);
                            }
                        }
                    },
                }
            }
        }

        // —— 子查询载体 ——
        Expr::InSubquery { expr, subquery, .. } => {
            collect_expr_usage(expr, usage);
            collect_query_usage(subquery, usage);
        }
        Expr::Exists { subquery, .. } => collect_query_usage(subquery, usage),
        Expr::Subquery(q) => collect_query_usage(q, usage),

        // —— 函数调用（含聚合/窗口函数）——
        Expr::Function(f) => {
            usage.functions.push(function_name_key(&f.name));
            collect_fn_args_usage(&f.parameters, usage);
            collect_fn_args_usage(&f.args, usage);
            if let Some(filter_expr) = &f.filter {
                collect_expr_usage(filter_expr, usage);
            }
            if let Some(over) = &f.over {
                collect_window_type_usage(over, usage);
            }
            for order_by in &f.within_group {
                collect_expr_usage(&order_by.expr, usage);
            }
        }
    }
}

fn collect_fn_args_usage(args: &FunctionArguments, usage: &mut QueryUsage) {
    match args {
        FunctionArguments::None => {}
        // `func((SELECT ...))` 单参数子查询形式
        FunctionArguments::Subquery(q) => collect_query_usage(q, usage),
        FunctionArguments::List(list) => {
            for arg in &list.args {
                collect_fn_arg_usage(arg, usage);
            }
            // 参数内嵌子句：string_agg(x, ',' ORDER BY y) / LIMIT / HAVING
            for clause in &list.clauses {
                match clause {
                    FunctionArgumentClause::OrderBy(exprs) => {
                        for ob in exprs {
                            collect_expr_usage(&ob.expr, usage);
                        }
                    }
                    FunctionArgumentClause::Limit(limit_expr) => {
                        collect_expr_usage(limit_expr, usage);
                    }
                    FunctionArgumentClause::Having(bound) => {
                        collect_expr_usage(&bound.1, usage);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn collect_fn_arg_usage(arg: &FunctionArg, usage: &mut QueryUsage) {
    match arg {
        FunctionArg::Named { arg: inner, .. } | FunctionArg::ExprNamed { arg: inner, .. } => {
            collect_fn_arg_expr_usage(inner, usage);
        }
        FunctionArg::Unnamed(inner) => collect_fn_arg_expr_usage(inner, usage),
    }
}

fn collect_fn_arg_expr_usage(arg_expr: &FunctionArgExpr, usage: &mut QueryUsage) {
    if let FunctionArgExpr::Expr(e) = arg_expr {
        collect_expr_usage(e, usage);
    }
}

fn collect_window_type_usage(wt: &WindowType, usage: &mut QueryUsage) {
    match wt {
        WindowType::NamedWindow(_) => {}
        WindowType::WindowSpec(spec) => collect_window_spec_usage(spec, usage),
    }
}

fn collect_window_spec_usage(spec: &WindowSpec, usage: &mut QueryUsage) {
    for e in &spec.partition_by {
        collect_expr_usage(e, usage);
    }
    for ob in &spec.order_by {
        collect_expr_usage(&ob.expr, usage);
    }
    if let Some(frame) = &spec.window_frame {
        collect_frame_bound_usage(&frame.start_bound, usage);
        if let Some(end) = &frame.end_bound {
            collect_frame_bound_usage(end, usage);
        }
    }
}

fn collect_frame_bound_usage(bound: &WindowFrameBound, usage: &mut QueryUsage) {
    match bound {
        WindowFrameBound::Preceding(Some(e)) | WindowFrameBound::Following(Some(e)) => {
            collect_expr_usage(e, usage);
        }
        _ => {}
    }
}

/// S3：返回首个越界表引用的拒绝信息；None = 通过。
/// 语义（D2.2）：显式 schema 必须 ∈ allowed_schemas；单段表名由搜索路径
/// 解析（放行，与 design 一致）；空 allowed_schemas = 不过滤（调用方保证）。
fn schema_whitelist_violation(query: &Query, allowed_schemas: &[String]) -> Option<String> {
    let mut usage = QueryUsage::default();
    collect_query_usage(query, &mut usage);
    for (schema, table) in &usage.tables {
        if let Some(schema) = schema {
            if !allowed_schemas.iter().any(|allowed| allowed == schema) {
                return Some(format!(
                    "SQL references table \"{table}\" in schema \"{schema}\" outside allowed schemas: {}",
                    allowed_schemas.join(", ")
                ));
            }
        }
    }
    None
}

/// S4：返回首个黑名单函数命中的拒绝信息；None = 通过。
fn function_blacklist_violation(query: &Query) -> Option<String> {
    let mut usage = QueryUsage::default();
    collect_query_usage(query, &mut usage);
    for f in &usage.functions {
        if FUNCTION_BLACKLIST.contains(&f.as_str())
            || FUNCTION_BLACKLIST_PREFIXES
                .iter()
                .any(|prefix| f.starts_with(prefix))
        {
            return Some(format!("SQL calls blacklisted function \"{f}\""));
        }
    }
    None
}

/// 查询 SQL 工具（只读 SELECT）
pub struct QuerySqlTool;

impl QuerySqlTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "query_sql".to_string(),
            description: "执行只读 SQL 查询（SELECT），返回查询结果。自动拒绝 DML/DDL 语句。"
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "sql": { "type": "string", "description": "SQL SELECT 语句" }
                },
                "required": ["sql"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String> {
        let args = call.arguments.clone();
        let sql = args
            .get("sql")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'sql' parameter")?;

        // 安全检查：AST 级解析，仅允许单条 SELECT
        let dialect = PostgreSqlDialect {};
        let statements = match Parser::parse_sql(&dialect, sql) {
            Ok(stmts) => stmts,
            Err(e) => {
                return Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    success: false,
                    output: json!(null),
                    error: Some(format!("SQL syntax error: {}", e)),
                });
            }
        };

        if statements.len() != 1 {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: json!(null),
                error: Some("Only a single SELECT statement is allowed".to_string()),
            });
        }

        let query = match &statements[0] {
            sqlparser::ast::Statement::Query(q) => q,
            _ => {
                return Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    success: false,
                    output: json!(null),
                    error: Some("Only SELECT statements are allowed".to_string()),
                });
            }
        };

        // S3：schema 白名单——AST 递归提取表引用（FROM/JOIN/子查询/CTE）后校验。
        // 显式 schema 必须 ∈ allowed_schemas；单段表名按搜索路径解析（放行）。
        // 替换字符串 contains：注释/字面量伪造（如 `-- isahl`）不再能绕过。
        // 空 allowed_schemas = 不过滤（既有语义）。
        if !ctx.allowed_schemas.is_empty() {
            if let Some(reason) = schema_whitelist_violation(query, &ctx.allowed_schemas) {
                return Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    success: false,
                    output: json!(null),
                    error: Some(reason),
                });
            }
        }

        // S4：副作用函数黑名单——AST 递归收集全部函数调用（SELECT 目标/
        // DISTINCT ON/WHERE/HAVING/JOIN ON/子查询/CTE/窗口子句/LIMIT/OFFSET/
        // FETCH 表达式槽），命中黑名单族（管理/写/耗时/远程函数）即拒；
        // 聚合与纯函数（count/sum/abs 等）放行。
        if let Some(reason) = function_blacklist_violation(query) {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: json!(null),
                error: Some(reason),
            });
        }

        // 添加 LIMIT 守卫（基于原始 SQL 的小写检查）
        let normalized_lower = sql.trim().to_lowercase();
        let guarded_sql = if !normalized_lower.contains("limit") {
            format!("{} LIMIT 1000", sql.trim_end_matches(';'))
        } else {
            sql.to_string()
        };

        // 执行查询
        let rows = sqlx::query(AssertSqlSafe(guarded_sql.as_str()))
            .fetch_all(&ctx.db_pool)
            .await
            .map_err(|e| format!("SQL execution error: {}", e))?;

        let mut results = Vec::new();
        for row in rows {
            let mut obj = serde_json::Map::new();
            for (i, col) in row.columns().iter().enumerate() {
                let val = row_value::row_value_to_json(&row, i);
                obj.insert(col.name().to_string(), val);
            }
            results.push(serde_json::Value::Object(obj));
        }

        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            success: true,
            output: json!({ "rows": results, "count": results.len() }),
            error: None,
        })
    }
}

/// 查询 Schema 工具（通过 information_schema）
pub struct QuerySchemaTool;

impl QuerySchemaTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "query_schema".to_string(),
            description: "查询数据库表结构信息（列名、数据类型、约束等）".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "table_name": { "type": "string", "description": "表名（支持通配符 %）" },
                    "schema_name": { "type": "string", "description": "schema 名，默认 isahl" }
                },
                "required": ["table_name"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String> {
        let args = call.arguments.clone();
        let table_pattern = args
            .get("table_name")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'table_name' parameter")?;
        let schema_name = args
            .get("schema_name")
            .and_then(|v| v.as_str())
            .unwrap_or("isahl");

        // 白名单检查
        if !ctx.allowed_schemas.is_empty()
            && !ctx.allowed_schemas.contains(&schema_name.to_string())
        {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: json!(null),
                error: Some(format!(
                    "Schema '{}' not in allowed list: {:?}",
                    schema_name, ctx.allowed_schemas
                )),
            });
        }

        let rows = sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>)>(
            r#"SELECT column_name, data_type, is_nullable,
                      column_default, character_maximum_length::text
               FROM information_schema.columns
               WHERE table_schema = $1 AND table_name LIKE $2
               ORDER BY ordinal_position"#,
        )
        .bind(schema_name)
        .bind(table_pattern)
        .fetch_all(&ctx.db_pool)
        .await
        .map_err(|e| format!("Schema query error: {}", e))?;

        let columns: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|(name, dtype, nullable, default, max_len)| {
                json!({
                    "column_name": name,
                    "data_type": dtype,
                    "is_nullable": nullable,
                    "column_default": default,
                    "max_length": max_len,
                })
            })
            .collect();

        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            success: true,
            output: json!({ "schema": schema_name, "columns": columns }),
            error: None,
        })
    }
}

/// NGAC 候选补丁影响预演工具（change `wire-org-policy-declaration-fields` tasks 3.3）。
///
/// 承载 `POST /api/admin/ngac/analysis/dry-run` 的产品入口：**只读**（克隆图求值，
/// 关系表零写入）⇒ 不产生提案、不落库。执行经宿主 `ActionHandler`（地址 = 动作目录
/// 的 `ngac_dry_run`；该条目 `expose_to_llm: false` ⇒ **能力不外溢**到其它可执行动作，
/// 模型只能经本工具触达该分析）。
pub struct NgacDryRunTool;

impl NgacDryRunTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "ngac_dry_run".to_string(),
            description: "预演 NGAC 候选策略补丁的判决影响（只读、零写库）。\
输入 patches（association/prohibition 增删）与 checks（user_id/resource_type/fk_resource/action），\
逐点返回 before/after/flipped（Permit/Deny/NotApplicable）。提案前用它给出影响清单；\
结果只作参考，真实生效仍经用户确认后的策略写入通道。"
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "patches": { "type": "array", "description": "候选补丁：{kind: association|prohibition, remove, id, fk_user_attribute, fk_object_attribute, rights[], is_active}" },
                    "checks": { "type": "array", "description": "判决点：{user_id, resource_type, fk_resource, action}" }
                },
                "required": ["patches", "checks"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String> {
        let params = call.arguments.clone();
        let Some(handler) = ctx.action_handler.as_deref() else {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: json!(null),
                error: Some("NGAC_ANALYSIS_UNAVAILABLE: 未注入动作执行器".to_string()),
            });
        };
        match handler.analyze("ngac_dry_run", &params, ctx).await {
            Ok(output) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: true,
                output,
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: json!(null),
                error: Some(e),
            }),
        }
    }
}

/// NGAC 图路径证据工具（tasks 3.3；只读）。
///
/// 承载 `POST /api/admin/ngac/analysis/explain-paths` 的产品入口：给定资源标识
/// （`{resource_type}:{id}`）返回 UA→OA 的 ASSOCIATION/PROHIBITION 边证据——
/// 回答「这个资源为什么这样授权」。同 [`NgacDryRunTool`] 经宿主 `ActionHandler`
/// 收窄可达（目录条目 `expose_to_llm: false`）。
pub struct NgacExplainPathsTool;

impl NgacExplainPathsTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "ngac_explain_paths".to_string(),
            description: "查询 NGAC 图路径证据（只读）：给定资源标识 `{resource_type}:{id}`，\
返回该资源 OA 闭包内全部 UA→OA 边（边类 ASSOCIATION/PROHIBITION + 权利 + 两端名）。\
用于解释存量授权的来源；不改变任何状态。"
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "resource": { "type": "string", "description": "资源标识 `{resource_type}:{id}`（与 PEP 同构）" },
                    "action": { "type": "string", "description": "动作名（契约字段；路径按 OA 闭包枚举，不按动作过滤）" }
                },
                "required": ["resource", "action"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String> {
        let params = call.arguments.clone();
        let Some(handler) = ctx.action_handler.as_deref() else {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: json!(null),
                error: Some("NGAC_ANALYSIS_UNAVAILABLE: 未注入动作执行器".to_string()),
            });
        };
        match handler.analyze("ngac_explain_paths", &params, ctx).await {
            Ok(output) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: true,
                output,
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: json!(null),
                error: Some(e),
            }),
        }
    }
}

/// NGAC 访问检查工具（`wire-ngac-zero-consumer-faces`；只读，批量）。
///
/// 承载 `/api/ngac/pdp/check{,/batch}` 的**能力面**产品入口：逐点返回 `permitted` / `reason`。
/// 执行经宿主 `ActionHandler::analyze`（**不入动作目录** ⇒ 与 `execute_action` 的枚举/确认级面解耦，
/// 能力不外溢），宿主侧实现 = `common::permissions::require_resource_access`（**决策唯一实现**）。
pub struct NgacCheckAccessTool;

impl NgacCheckAccessTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "ngac_check_access".to_string(),
            description: "批量检查 NGAC 访问判定（只读）：输入 checks（user_id / resource_type / \
fk_resource / action），逐点返回 permitted 与原因。用于回答「某用户能否对某资源做某动作」；\
不改变任何状态，判定与 PEP/内层门同源。"
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "checks": {
                        "type": "array",
                        "description": "判定点：{user_id, resource_type, fk_resource, action}",
                        "items": {
                            "type": "object",
                            "properties": {
                                "user_id": { "type": "integer" },
                                "resource_type": { "type": "string" },
                                "fk_resource": { "type": "integer" },
                                "action": { "type": "string" }
                            },
                            "required": ["user_id", "resource_type", "action"]
                        }
                    }
                },
                "required": ["checks"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String> {
        let params = call.arguments.clone();
        let Some(handler) = ctx.action_handler.as_deref() else {
            return Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: json!(null),
                error: Some("NGAC_ANALYSIS_UNAVAILABLE: 未注入动作执行器".to_string()),
            });
        };
        match handler.analyze("ngac_check_access", &params, ctx).await {
            Ok(output) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: true,
                output,
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                success: false,
                output: json!(null),
                error: Some(e),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tool_orchestrator::ActionHandler;
    use crate::agents::ConfirmationLevel;
    use std::sync::Arc;

    /// 测试用 ActionHandler：固定确认级别 + 调用日志（mpsc 通道，无 Mutex——
    /// 满足 `rs-parking-lot` 规则：新代码 MUST NOT 出现 `std::sync::Mutex` + `.lock().unwrap()`；
    /// 调用方按需 `try_recv`/`try_iter` 读取，语义与旧共享 Vec 一致）。
    /// FakeActionHandler 调用日志型（工具名 + 参数 id 集）
    type CallLog = std::sync::mpsc::Receiver<(String, Vec<i64>)>;

    struct FakeActionHandler {
        level: ConfirmationLevel,
        tx: std::sync::mpsc::Sender<(String, Vec<i64>)>,
    }

    impl FakeActionHandler {
        fn boxed(level: ConfirmationLevel) -> (Arc<dyn ActionHandler>, CallLog) {
            let (tx, rx) = std::sync::mpsc::channel::<(String, Vec<i64>)>();
            let handler: Arc<dyn ActionHandler> = Arc::new(FakeActionHandler { level, tx });
            (handler, rx)
        }
    }

    #[async_trait::async_trait]
    impl ActionHandler for FakeActionHandler {
        async fn execute(
            &self,
            action_type: &str,
            target_ids: &[i64],
            params: &serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<serde_json::Value, String> {
            let _ = self.tx.send((action_type.to_string(), target_ids.to_vec()));
            Ok(json!({
                "executed": action_type,
                "count": target_ids.len(),
                "note": params.get("note").cloned().unwrap_or(serde_json::Value::Null)
            }))
        }

        fn confirmation_level(&self, _action_type: &str) -> ConfirmationLevel {
            self.level.clone()
        }
    }

    fn action_ctx(handler: Option<Arc<dyn ActionHandler>>) -> ToolContext {
        ToolContext {
            session_id: 1,
            user_id: None,
            db_pool: sqlx::PgPool::connect_lazy("postgres://u:p@localhost:5432/nodb").unwrap(),
            allowed_schemas: vec![],
            action_handler: handler,
            form_contract: None,
        }
    }

    // ── 只读分析工具（ngac_dry_run / ngac_explain_paths，tasks 3.3）────────────

    /// 记录调用而不引入 `std::sync::Mutex`（`rs-parking-lot`：新代码不得
    /// `.lock().unwrap()`）——用 mpsc 通道回传 (tool, target_ids)。
    struct ChannelledHandler {
        tx: std::sync::mpsc::Sender<(String, Vec<i64>)>,
    }

    #[async_trait::async_trait]
    impl ActionHandler for ChannelledHandler {
        async fn execute(
            &self,
            action_type: &str,
            target_ids: &[i64],
            _params: &serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<serde_json::Value, String> {
            let _ = self.tx.send((action_type.to_string(), target_ids.to_vec()));
            Ok(json!({ "executed": action_type }))
        }

        async fn analyze(
            &self,
            tool: &str,
            params: &serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<serde_json::Value, String> {
            let _ = self.tx.send((tool.to_string(), vec![]));
            Ok(json!({
                "executed": tool,
                "note": params.get("resource").cloned().unwrap_or(serde_json::Value::Null)
            }))
        }

        fn confirmation_level(&self, _action_type: &str) -> ConfirmationLevel {
            ConfirmationLevel::None
        }
    }

    /// 分析工具经 `ToolContext.action_handler` 收窄执行：动作地址 = 工具名、
    /// target_ids 空、params 原样透传；**不产生提案**（只读）；无 handler ⇒ 失败（不静默）。
    #[tokio::test]
    async fn ngac_analysis_tools_dispatch_to_handler_without_proposal() {
        let (tx, rx) = std::sync::mpsc::channel();
        let handler: Arc<dyn ActionHandler> = Arc::new(ChannelledHandler { tx });
        let ctx = action_ctx(Some(handler));
        let registry = crate::tools::registry::ToolRegistry::new();

        let call = ToolCall {
            id: "t-dry".to_string(),
            name: "ngac_dry_run".to_string(),
            arguments: json!({
                "patches": [ { "kind": "association", "remove": true, "id": 5 } ],
                "checks": [ { "user_id": 9, "resource_type": "invoices", "action": "read" } ]
            }),
        };
        let r = registry.execute(&call, &ctx).await.expect("dry-run tool");
        assert!(r.success, "dry-run 工具应成功：{r:?}");
        assert_eq!(r.output["executed"], "ngac_dry_run");
        assert!(
            r.output.get("status").is_none() || r.output["status"] != "needs_confirmation",
            "只读分析 MUST NOT 产生确认提案：{r:?}"
        );

        let call2 = ToolCall {
            id: "t-paths".to_string(),
            name: "ngac_explain_paths".to_string(),
            arguments: json!({ "resource": "invoices:7", "action": "read" }),
        };
        let r2 = registry.execute(&call2, &ctx).await.expect("explain tool");
        assert!(r2.success, "explain-paths 工具应成功：{r2:?}");
        assert_eq!(r2.output["executed"], "ngac_explain_paths");

        let call3 = ToolCall {
            id: "t-check".to_string(),
            name: "ngac_check_access".to_string(),
            arguments: json!({ "checks": [ { "user_id": 7, "resource_type": "invoices", "fk_resource": 0, "action": "read" } ] }),
        };
        let r3 = registry.execute(&call3, &ctx).await.expect("check tool");
        assert!(r3.success, "check_access 工具应成功：{r3:?}");
        assert_eq!(r3.output["executed"], "ngac_check_access");

        let first = rx.recv().expect("第一次调用记录");
        let second = rx.recv().expect("第二次调用记录");
        let third = rx.recv().expect("第三次调用记录");
        assert_eq!(first.0, "ngac_dry_run");
        assert_eq!(second.0, "ngac_explain_paths");
        assert_eq!(third.0, "ngac_check_access");
        assert!(
            first.1.is_empty() && second.1.is_empty() && third.1.is_empty(),
            "分析工具 MUST NOT 传 target_ids"
        );

        // 无 handler（测试/未注入场景）⇒ 显式失败（MUST NOT 静默返回空结果）
        let bare = action_ctx(None);
        let r3 = registry.execute(&call, &bare).await.expect("tool result");
        assert!(!r3.success);
        assert!(
            r3.error
                .unwrap_or_default()
                .contains("NGAC_ANALYSIS_UNAVAILABLE"),
            "未注入执行器 MUST 显式失败"
        );
    }

    fn action_call(action_type: &str, confirmed: bool) -> ToolCall {
        ToolCall {
            id: "c1".to_string(),
            name: "execute_action".to_string(),
            arguments: json!({
                "action_type": action_type,
                "target_ids": [11, 22],
                "params": { "note": "x" },
                "confirmed": confirmed
            }),
        }
    }

    #[tokio::test]
    async fn execute_action_without_handler_returns_preview() {
        let registry = crate::tools::registry::ToolRegistry::new();
        let ctx = action_ctx(None);
        let result = registry
            .execute(&action_call("batch_approve", false), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output["status"], "preview");
        assert_eq!(result.output["target_count"], 2);
    }

    #[tokio::test]
    async fn execute_action_explicit_with_confirmed_llm_still_needs_confirmation() {
        // S1：Explicit 级确认不可由 LLM 自证——即使携带 confirmed=true 也恒返回
        // 结构化提案，handler 零调用（数据零变更）；真实执行仅经 HTTP 端点。
        let (handler, calls) = FakeActionHandler::boxed(ConfirmationLevel::Explicit);
        let registry = crate::tools::registry::ToolRegistry::new();
        let ctx = action_ctx(Some(handler));
        let result = registry
            .execute(&action_call("batch_approve", true), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output["status"], "needs_confirmation");
        // 结构化提案字段：HTTP execute-action 可解析的稳定 id + 完整动作形态
        assert_eq!(result.output["id"], "agent_action:batch_approve");
        assert_eq!(result.output["action_type"], "batch_approve");
        assert_eq!(result.output["target_ids"], json!([11, 22]));
        assert_eq!(result.output["params"], json!({ "note": "x" }));
        assert!(result.output["confirm_url_hint"].is_string());
        assert!(
            calls.try_recv().is_err(),
            "LLM 自称确认不得触达 handler（数据零变更）"
        );
    }

    #[tokio::test]
    async fn execute_action_explicit_without_confirmed_returns_needs_confirmation() {
        let (handler, calls) = FakeActionHandler::boxed(ConfirmationLevel::Explicit);
        let registry = crate::tools::registry::ToolRegistry::new();
        let ctx = action_ctx(Some(handler));
        let result = registry
            .execute(&action_call("batch_approve", false), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output["status"], "needs_confirmation");
        assert_eq!(result.output["id"], "agent_action:batch_approve");
        assert_eq!(result.output["target_ids"], json!([11, 22]));
        assert_eq!(result.output["params"], json!({ "note": "x" }));
        assert!(result.output["confirm_url_hint"].is_string());
        assert!(calls.try_recv().is_err(), "未确认不得触达 handler");
    }

    #[tokio::test]
    async fn execute_action_none_level_executes_without_confirmed() {
        let (handler, calls) = FakeActionHandler::boxed(ConfirmationLevel::None);
        let registry = crate::tools::registry::ToolRegistry::new();
        let ctx = action_ctx(Some(handler));
        let result = registry
            .execute(&action_call("generate_document", false), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output["executed"], "generate_document");
        assert_eq!(calls.try_iter().count(), 1);
    }

    #[tokio::test]
    async fn execute_action_handler_error_becomes_failed_result() {
        // level None + confirmed true → 执行路径；handler 报错 → success=false
        struct FailingHandler;
        #[async_trait::async_trait]
        impl ActionHandler for FailingHandler {
            async fn execute(
                &self,
                _action_type: &str,
                _target_ids: &[i64],
                _params: &serde_json::Value,
                _ctx: &ToolContext,
            ) -> Result<serde_json::Value, String> {
                Err("backend rejected".to_string())
            }
            fn confirmation_level(&self, _action_type: &str) -> ConfirmationLevel {
                ConfirmationLevel::None
            }
        }
        let ctx = action_ctx(Some(Arc::new(FailingHandler) as Arc<dyn ActionHandler>));
        let result = ExecuteActionTool::execute(&action_call("send_notification", true), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("backend rejected"));
    }

    /// 构造带契约的 ctx（change `add-form-contract-seam`：`validate_form` 的校验口径 =
    /// 当前页面登记的契约，MUST NOT 用入参 schema）
    fn contract_ctx(contract: Option<crate::tools::form_contract::FormContract>) -> ToolContext {
        ToolContext {
            session_id: 1,
            user_id: None,
            db_pool: sqlx::PgPool::connect_lazy("postgres://u:p@localhost:5432/nodb").unwrap(),
            allowed_schemas: vec![],
            action_handler: None,
            form_contract: contract,
        }
    }

    fn simple_contract() -> crate::tools::form_contract::FormContract {
        serde_json::from_value(json!({
            "fields": [
                { "name": "age", "type": "integer", "required": true, "minimum": 0, "maximum": 150 }
            ]
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn validate_form_uses_page_contract_and_reports_errors() {
        let ctx = contract_ctx(Some(simple_contract()));
        let call = ToolCall {
            id: "c1".to_string(),
            name: "validate_form".to_string(),
            arguments: json!({ "field_values": { "age": -5 } }),
        };
        let result = ValidateFormTool::execute(&call, &ctx).await.unwrap();
        assert!(!result.success);
        assert_eq!(result.output["valid"], false);
        assert_eq!(result.output["errors"][0]["error"], "minimum");
        assert_eq!(result.output["appliable"], false);
        assert!(result.error.is_some());
    }

    #[tokio::test]
    async fn validate_form_passes_when_values_match_contract() {
        let ctx = contract_ctx(Some(simple_contract()));
        let call = ToolCall {
            id: "c2".to_string(),
            name: "validate_form".to_string(),
            arguments: json!({ "field_values": { "age": 30 } }),
        };
        let result = ValidateFormTool::execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        assert_eq!(result.output["valid"], true);
        assert_eq!(result.output["appliable"], true);
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn validate_form_without_contract_fails_closed() {
        // 未登记契约 ⇒ CONTRACT_UNAVAILABLE，MUST NOT 放行（design D7 fail-closed）
        let ctx = contract_ctx(None);
        let call = ToolCall {
            id: "c3".to_string(),
            name: "validate_form".to_string(),
            arguments: json!({ "field_values": { "age": 30 } }),
        };
        let result = ValidateFormTool::execute(&call, &ctx).await.unwrap();
        assert!(!result.success);
        assert_eq!(result.output["error"], "CONTRACT_UNAVAILABLE");
        assert_eq!(result.output["appliable"], false);
        assert!(result
            .error
            .as_deref()
            .unwrap_or_default()
            .starts_with("CONTRACT_UNAVAILABLE"));
    }

    #[tokio::test]
    async fn validate_form_ignores_model_supplied_schema() {
        // 模型自备 schema 不再被消费（「可校验任意 schema」的第二实现已退役）：
        // 入参声明 age 为 string，而契约是 integer ⇒ 按契约判，30 合法。
        let ctx = contract_ctx(Some(simple_contract()));
        let call = ToolCall {
            id: "c4".to_string(),
            name: "validate_form".to_string(),
            arguments: json!({
                "form_schema": { "properties": { "age": { "type": "string" } } },
                "field_values": { "age": 30 }
            }),
        };
        let result = ValidateFormTool::execute(&call, &ctx).await.unwrap();
        assert!(result.success, "MUST 按契约判定，MUST NOT 按入参 schema");
    }

    // ============ S3/S4：query_sql AST 级校验（design D2.2/D2.3） ============

    /// 解析单条 SELECT 语句为 Query（测试辅助；与 execute 同走
    /// PostgreSqlDialect，保证校验面与实际执行面一致）。
    fn parse_select(sql: &str) -> Query {
        let statements = Parser::parse_sql(&PostgreSqlDialect {}, sql).expect("SQL 必须可解析");
        assert_eq!(statements.len(), 1, "样本必须是单条语句");
        match &statements[0] {
            sqlparser::ast::Statement::Query(q) => (**q).clone(),
            other => panic!("预期单条 SELECT，实际 {other:?}"),
        }
    }

    #[test]
    fn s3_comment_forged_schema_is_rejected() {
        // 注释/字面量含 allowed schema 字样（`-- isahl`）不再能绕过——
        // AST 提取无视注释，显式越界 schema 必拒。
        let allowed = vec!["isahl".to_string()];
        let q = parse_select("SELECT * FROM evil.ledger -- isahl");
        let reason = schema_whitelist_violation(&q, &allowed);
        let reason = reason.expect("注释伪造的越界 schema 必须被拒");
        assert!(
            reason.contains("evil.ledger"),
            "拒绝信息应点名越界表: {reason}"
        );
    }

    #[test]
    fn s3_whitelisted_schema_and_search_path_pass() {
        let allowed = vec!["isahl".to_string()];
        // 显式 schema ∈ 白名单 → 放行
        assert!(
            schema_whitelist_violation(&parse_select("SELECT * FROM isahl.ledger"), &allowed)
                .is_none()
        );
        // 无表引用 / 单段表名（搜索路径解析）→ 放行
        assert!(schema_whitelist_violation(&parse_select("SELECT 1"), &allowed).is_none());
        assert!(
            schema_whitelist_violation(&parse_select("SELECT * FROM ledger"), &allowed).is_none()
        );
    }

    #[tokio::test]
    async fn s3_execute_rejects_foreign_schema_before_touching_db() {
        // 端到端（S3 接线）：allowed_schemas=["isahl"] 时越界 schema 在 DB 执行前被拒，
        // 注释含 `isahl` 字样不再能绕过——错误必须点名越界表。
        let ctx = ToolContext {
            session_id: 1,
            user_id: None,
            db_pool: sqlx::PgPool::connect_lazy("postgres://u:p@localhost:5432/nodb").unwrap(),
            allowed_schemas: vec!["isahl".to_string()],
            action_handler: None,
            form_contract: None,
        };
        let call = ToolCall {
            id: "c1".to_string(),
            name: "query_sql".to_string(),
            arguments: json!({ "sql": "SELECT * FROM evil.ledger -- isahl LIMIT 1" }),
        };
        let result = QuerySqlTool::execute(&call, &ctx).await.unwrap();
        assert!(!result.success, "越界 schema 必须被拒");
        let err = result.error.unwrap_or_default();
        assert!(
            err.contains("outside allowed schemas"),
            "拒绝信息须指向 schema 越界: {err}"
        );
        assert!(err.contains("evil.ledger"), "拒绝信息须点名越界表: {err}");
    }

    #[tokio::test]
    async fn s3_execute_empty_allowed_skips_schema_gate() {
        // 空 allowed_schemas = 不过滤（既有语义）：同 SQL 不被 schema 门禁拦截，
        // 放行至 DB 层（测试环境无 DB → 连接错误；错误内容不得指向 schema 越界）。
        let ctx = ToolContext {
            session_id: 1,
            user_id: None,
            db_pool: sqlx::PgPool::connect_lazy("postgres://u:p@localhost:5432/nodb").unwrap(),
            allowed_schemas: vec![],
            action_handler: None,
            form_contract: None,
        };
        let call = ToolCall {
            id: "c1".to_string(),
            name: "query_sql".to_string(),
            arguments: json!({ "sql": "SELECT * FROM evil.ledger LIMIT 1" }),
        };
        match QuerySqlTool::execute(&call, &ctx).await {
            Ok(r) => {
                let err = r.error.unwrap_or_default();
                assert!(
                    !err.contains("outside allowed schemas"),
                    "空白名单不得触发 schema 拒绝: {err}"
                );
            }
            Err(e) => {
                assert!(
                    !e.contains("outside allowed schemas"),
                    "空白名单不得触发 schema 拒绝: {e}"
                );
            }
        }
    }

    #[test]
    fn s3_whitelist_recurses_into_subqueries_and_cte() {
        let allowed = vec!["isahl".to_string()];
        // 子查询内越界 schema → 拒
        let q = parse_select("SELECT * FROM isahl.a WHERE id IN (SELECT id FROM audit.logs)");
        let reason = schema_whitelist_violation(&q, &allowed);
        assert!(reason.is_some(), "子查询内越界 schema 必须被拒");
        // CTE 内越界 schema → 拒
        let q = parse_select("WITH x AS (SELECT * FROM audit.logs) SELECT * FROM isahl.a");
        assert!(schema_whitelist_violation(&q, &allowed).is_some());
        // JOIN 两侧与子查询均合法 → 放行
        let q = parse_select(
            "SELECT * FROM isahl.a JOIN isahl.b ON a.id = b.id \
             WHERE b.x IN (SELECT id FROM isahl.c)",
        );
        assert!(schema_whitelist_violation(&q, &allowed).is_none());
    }

    #[test]
    fn s4_blacklisted_functions_rejected() {
        // 精确名 + 前缀族（dblink_*/lo_*）都拒；大小写/限定名不构成绕过
        for sql in [
            "SELECT pg_terminate_backend(42)",
            "SELECT set_config('search_path', 'evil', false)",
            "SELECT pg_sleep(10)",
            "SELECT nextval('order_seq')",
            "SELECT pg_read_file('/etc/passwd')",
            "SELECT dblink_exec('conn', 'DROP TABLE t')",
            "SELECT dblink('conn', 'SELECT 1')",
            "SELECT lo_import('/tmp/x')",
            "SELECT public.pg_cancel_backend(7)",
            "SELECT PG_SLEEP(3)",
        ] {
            assert!(
                function_blacklist_violation(&parse_select(sql)).is_some(),
                "黑名单函数必须被拒: {sql}"
            );
        }
    }

    #[test]
    fn s4_blacklist_covers_distinct_on_and_paging_slots() {
        // DISTINCT ON(...)/LIMIT/OFFSET 表达式槽均收录（评审 P2：收集器曾漏这
        // 四槽——黑名单函数藏在这些位置可绕过）。FETCH quantity 槽同样已遍历，
        // 但 PG 方言解析器把 FETCH 数量限为具体值（函数不可达），无解析样本。
        for sql in [
            "SELECT DISTINCT ON (pg_sleep(1)) a FROM isahl.t",
            "SELECT 1 LIMIT pg_sleep(1)",
            "SELECT 1 LIMIT 1 OFFSET pg_sleep(1)",
            "SELECT DISTINCT ON (dblink('conn', 'SELECT 1')) a FROM isahl.t",
        ] {
            assert!(
                function_blacklist_violation(&parse_select(sql)).is_some(),
                "表达式槽内黑名单函数必须被拒: {sql}"
            );
        }
        // 槽内纯函数不误伤
        let q = parse_select("SELECT DISTINCT ON (abs(x)) x FROM isahl.t LIMIT 3 OFFSET 1");
        assert!(function_blacklist_violation(&q).is_none());
    }

    #[test]
    fn s4_aggregates_and_pure_functions_allowed() {
        // count/sum/abs 等聚合与纯函数放行（白名单外函数不误伤）
        let q = parse_select("SELECT count(*), sum(amount), abs(balance) FROM isahl.ledger");
        assert!(function_blacklist_violation(&q).is_none());
    }

    #[test]
    fn s4_blacklist_reaches_into_subqueries() {
        // 标量子查询 / EXISTS / 派生表 / CTE 内的黑名单调用均应收录
        for sql in [
            "SELECT (SELECT pg_sleep(1))",
            "SELECT 1 WHERE EXISTS (SELECT 1 WHERE pg_ls_dir('/') IS NOT NULL)",
            "SELECT * FROM (SELECT pg_terminate_backend(1)) t",
            "WITH x AS (SELECT nextval('seq')) SELECT * FROM x",
        ] {
            assert!(
                function_blacklist_violation(&parse_select(sql)).is_some(),
                "子查询内黑名单函数必须被拒: {sql}"
            );
        }
        // 派生表内纯聚合 → 放行
        let q = parse_select("SELECT * FROM (SELECT count(*) c FROM isahl.a) t");
        assert!(function_blacklist_violation(&q).is_none());
    }

    // ── ngac_policy_draft（route-ngac-policy-authoring-through-ai-assist 2.1）──

    fn ngac_call(operations: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "n1".to_string(),
            name: "ngac_policy_draft".to_string(),
            arguments: json!({ "operations": operations }),
        }
    }

    #[tokio::test]
    async fn ngac_policy_draft_returns_confirmation_proposal_without_writes() {
        let (handler, calls) = FakeActionHandler::boxed(ConfirmationLevel::Explicit);
        let registry = crate::tools::registry::ToolRegistry::new();
        let ctx = action_ctx(Some(handler));
        let call = ngac_call(json!([
            {
                "op": "upsert_class", "code": "finance-approval", "notice": "财务审批规范",
                "scope": { "resource_types": ["invoice"], "actions": ["approve"] },
                "prohibition_template": "action == \"export\" && !permitted",
                "summary": "新建财务审批规范"
            },
            {
                "op": "create_rule", "policy_class_id": 7, "subject_code": "position:cfo",
                "resource_type": "invoice", "actions": ["approve"],
                "condition": "action == \"approve\" && permitted"
            }
        ]));
        let result = registry.execute(&call, &ctx).await.unwrap();
        assert!(result.success, "{:?}", result.error);
        assert_eq!(result.output["id"], "ngac:org-policy-apply");
        assert_eq!(result.output["kind"], "execute");
        assert_eq!(result.output["action_type"], "ngac_org_policy_apply");
        assert_eq!(result.output["status"], "needs_confirmation");
        assert!(result.output["label"].as_str().unwrap().contains('2'));
        assert_eq!(
            result.output["params"]["operations"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        // 工具路径零动作调用（MUST NOT 落库/自证确认）
        assert!(calls.try_recv().is_err());
    }

    #[tokio::test]
    async fn ngac_policy_draft_rejects_unknown_op() {
        let registry = crate::tools::registry::ToolRegistry::new();
        let ctx = action_ctx(None);
        let result = registry
            .execute(&ngac_call(json!([{ "op": "drop_table" }])), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("白名单"));
    }

    #[tokio::test]
    async fn ngac_policy_draft_rejects_structured_expression_placeholder() {
        let registry = crate::tools::registry::ToolRegistry::new();
        let ctx = action_ctx(None);
        let result = registry
            .execute(
                &ngac_call(json!([{
                    "op": "create_rule", "policy_class_id": 7, "subject_code": "s",
                    "resource_type": "r", "actions": ["read"],
                    "condition": { "and": [] }
                }])),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("Rhai 源码字符串"));
    }

    #[tokio::test]
    async fn ngac_policy_draft_rejects_empty_operations() {
        let registry = crate::tools::registry::ToolRegistry::new();
        let ctx = action_ctx(None);
        let result = registry.execute(&ngac_call(json!([])), &ctx).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("不能为空"));
    }

    // ── system_config_draft（add-system-config-ai-action 2.2）──

    fn syscfg_call(operations: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "s1".to_string(),
            name: "system_config_draft".to_string(),
            arguments: json!({ "operations": operations }),
        }
    }

    #[tokio::test]
    async fn system_config_draft_returns_confirmation_proposal_without_writes() {
        let (handler, calls) = FakeActionHandler::boxed(ConfirmationLevel::Explicit);
        let registry = crate::tools::registry::ToolRegistry::new();
        let ctx = action_ctx(Some(handler));
        let call = syscfg_call(json!([
            {
                "op": "create_config", "category": "llm", "provider": "deepseek",
                "code": "llm-primary", "notice": "主模型",
                "settings": { "model": "deepseek-v4", "base_url": "https://api.example" },
                "credentials": { "api_key": "sk-***" },
                "enabled": true, "is_default": true, "summary": "新建主 LLM 行"
            },
            { "op": "update_config", "id": 7, "enabled": false, "summary": "停用旧行" }
        ]));
        let result = registry.execute(&call, &ctx).await.unwrap();
        assert!(result.success, "{:?}", result.error);
        assert_eq!(result.output["id"], "syscfg:apply");
        assert_eq!(result.output["kind"], "execute");
        assert_eq!(result.output["action_type"], "system_config_apply");
        assert_eq!(result.output["status"], "needs_confirmation");
        assert!(result.output["label"].as_str().unwrap().contains('2'));
        assert_eq!(
            result.output["params"]["operations"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        // 工具路径零动作调用（MUST NOT 落库/自证确认）
        assert!(calls.try_recv().is_err());
    }

    #[tokio::test]
    async fn system_config_draft_requires_category_and_provider_on_create() {
        let registry = crate::tools::registry::ToolRegistry::new();
        let ctx = action_ctx(None);
        let result = registry
            .execute(
                &syscfg_call(json!([{ "op": "create_config", "category": "llm" }])),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("provider"));
    }

    #[tokio::test]
    async fn system_config_draft_rejects_non_object_credentials() {
        let registry = crate::tools::registry::ToolRegistry::new();
        let ctx = action_ctx(None);
        let result = registry
            .execute(
                &syscfg_call(json!([{
                    "op": "update_config", "id": 3, "credentials": "sk-plain-text"
                }])),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result
            .error
            .unwrap()
            .contains("credentials MUST 为 JSON 对象"));
    }

    #[tokio::test]
    async fn system_config_draft_rejects_unknown_op_and_empty() {
        let registry = crate::tools::registry::ToolRegistry::new();
        let ctx = action_ctx(None);
        let unknown = registry
            .execute(&syscfg_call(json!([{ "op": "drop_config" }])), &ctx)
            .await
            .unwrap();
        assert!(!unknown.success);
        assert!(unknown.error.unwrap().contains("白名单"));
        let empty = registry
            .execute(&syscfg_call(json!([])), &ctx)
            .await
            .unwrap();
        assert!(!empty.success);
        assert!(empty.error.unwrap().contains("不能为空"));
    }
}
