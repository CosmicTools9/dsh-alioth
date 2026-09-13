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

/// 把 JSON 值归类为 Schema type 名词（用于报错信息）。
fn value_type_name(value: &serde_json::Value) -> &'static str {
    if value.is_null() {
        "null"
    } else if value.is_boolean() {
        "boolean"
    } else if value.is_number() {
        "number"
    } else if value.is_string() {
        "string"
    } else if value.is_array() {
        "array"
    } else {
        "object"
    }
}

/// 值是否满足声明的 Schema type。
/// 仅校验工具支持的标量类型（string/number/integer/boolean）；
/// object/array/null 等其他声明不在表单字段校验范围，放行（不误报）。
fn value_matches_type(value: &serde_json::Value, declared: &str) -> bool {
    match declared {
        "string" => value.is_string(),
        "number" => value.is_number(),
        // JSON 数字经 f64 往返的 1.0 非整数——按整型声明即类型不符
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        _ => true,
    }
}

fn push_field_error(errors: &mut Vec<serde_json::Value>, field: &str, code: &str, message: String) {
    errors.push(json!({
        "field": field,
        "error": code,
        "message": message
    }));
}

/// 纯函数表单校验引擎：`form_schema`（JSON Schema 子集）逐字段校验
/// `field_values`。支持规则：required / type / enum / minimum / maximum /
/// minLength / maxLength / pattern。返回错误条目列表
/// （`{field, error, message}`）；空列表 = 校验通过。
///
/// 规则说明：
/// - required 按「值是否存在」判定（保留既有行为，null 视为已提供，交由 type 规则拦截）；
/// - type 不符时该字段跳过其余规则（避免级联噪音）；
/// - 数值规则（minimum/maximum）仅作用于数值；长度/pattern 仅作用于字符串；
/// - pattern 非法正则视为 schema 作者错误——跳过该规则，不阻断其余校验。
fn validate_form_schema(
    form_schema: &serde_json::Value,
    field_values: &serde_json::Value,
) -> Vec<serde_json::Value> {
    let mut errors: Vec<serde_json::Value> = Vec::new();
    let Some(props) = form_schema.get("properties").and_then(|p| p.as_object()) else {
        return errors;
    };

    // required：声明的必填字段缺值即报错（与 Schema 无关字段不在 properties 内）
    if let Some(required) = form_schema.get("required").and_then(|r| r.as_array()) {
        for name in required {
            let Some(name) = name.as_str() else { continue };
            if !props.contains_key(name) {
                continue;
            }
            if field_values.get(name).is_none() {
                push_field_error(
                    &mut errors,
                    name,
                    "required",
                    format!("字段 '{}' 为必填项", name),
                );
            }
        }
    }

    for (field_name, field_schema) in props {
        if !field_schema.is_object() {
            continue;
        }
        let Some(value) = field_values.get(field_name) else {
            continue;
        };

        // type
        if let Some(declared) = field_schema.get("type").and_then(|t| t.as_str()) {
            if !value_matches_type(value, declared) {
                push_field_error(
                    &mut errors,
                    field_name,
                    "type",
                    format!(
                        "字段 '{}' 类型应为 {}, 实际为 {}",
                        field_name,
                        declared,
                        value_type_name(value)
                    ),
                );
                continue;
            }
        }

        // enum
        if let Some(allowed) = field_schema.get("enum").and_then(|e| e.as_array()) {
            if !allowed.iter().any(|v| v == value) {
                push_field_error(
                    &mut errors,
                    field_name,
                    "enum",
                    format!("字段 '{}' 的值不在允许范围内", field_name),
                );
            }
        }

        // minimum / maximum（数值字段）
        if value.is_number() {
            if let Some(min) = field_schema.get("minimum").and_then(|m| m.as_f64()) {
                if let Some(actual) = value.as_f64() {
                    if actual < min {
                        push_field_error(
                            &mut errors,
                            field_name,
                            "minimum",
                            format!("字段 '{}' 不能小于 {}", field_name, min),
                        );
                    }
                }
            }
            if let Some(max) = field_schema.get("maximum").and_then(|m| m.as_f64()) {
                if let Some(actual) = value.as_f64() {
                    if actual > max {
                        push_field_error(
                            &mut errors,
                            field_name,
                            "maximum",
                            format!("字段 '{}' 不能大于 {}", field_name, max),
                        );
                    }
                }
            }
        }

        // minLength / maxLength / pattern（字符串字段，按 Unicode 标量计长）
        if let Some(s) = value.as_str() {
            let len = s.chars().count() as u64;
            if let Some(min) = field_schema.get("minLength").and_then(|m| m.as_u64()) {
                if len < min {
                    push_field_error(
                        &mut errors,
                        field_name,
                        "min_length",
                        format!("字段 '{}' 长度不能小于 {}", field_name, min),
                    );
                }
            }
            if let Some(max) = field_schema.get("maxLength").and_then(|m| m.as_u64()) {
                if len > max {
                    push_field_error(
                        &mut errors,
                        field_name,
                        "max_length",
                        format!("字段 '{}' 长度不能大于 {}", field_name, max),
                    );
                }
            }
            if let Some(pattern) = field_schema.get("pattern").and_then(|p| p.as_str()) {
                if let Ok(re) = regex::Regex::new(pattern) {
                    if !re.is_match(s) {
                        push_field_error(
                            &mut errors,
                            field_name,
                            "pattern",
                            format!("字段 '{}' 不符合格式要求", field_name),
                        );
                    }
                }
            }
        }
    }

    errors
}

/// 表单校验工具
pub struct ValidateFormTool;

impl ValidateFormTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "validate_form".to_string(),
            description: "校验表单字段值是否符合 Schema 定义的规则（类型、必填、范围、格式等）"
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "form_schema": { "type": "object", "description": "表单 Schema 定义" },
                    "field_values": { "type": "object", "description": "待校验的字段值映射" }
                },
                "required": ["form_schema", "field_values"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, String> {
        let args = call.arguments.clone();
        let form_schema = args.get("form_schema").cloned().unwrap_or(json!({}));
        let field_values = args.get("field_values").cloned().unwrap_or(json!({}));

        let errors = validate_form_schema(&form_schema, &field_values);
        let valid = errors.is_empty();
        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            success: valid,
            output: json!({ "valid": valid, "errors": errors }),
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
                    "action_type": { "type": "string", "enum": ["status_transition", "batch_approve", "generate_document", "send_notification"], "description": "动作类型" },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tool_orchestrator::ActionHandler;
    use crate::agents::ConfirmationLevel;
    use std::sync::{Arc, Mutex};

    /// 测试用 ActionHandler：固定确认级别 + 共享调用日志
    /// （锁约定与 crate 内既有 fake adapter 一致：std Mutex 直锁）
    struct FakeActionHandler {
        level: ConfirmationLevel,
        calls: Arc<Mutex<Vec<(String, Vec<i64>)>>>,
    }

    impl FakeActionHandler {
        fn boxed(
            level: ConfirmationLevel,
        ) -> (Arc<dyn ActionHandler>, Arc<Mutex<Vec<(String, Vec<i64>)>>>) {
            let calls: Arc<Mutex<Vec<(String, Vec<i64>)>>> = Arc::new(Mutex::new(vec![]));
            let handler: Arc<dyn ActionHandler> = Arc::new(FakeActionHandler {
                level,
                calls: calls.clone(),
            });
            (handler, calls)
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
            self.calls
                .lock()
                .unwrap()
                .push((action_type.to_string(), target_ids.to_vec()));
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
        }
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
            calls.lock().unwrap().is_empty(),
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
        assert!(calls.lock().unwrap().is_empty(), "未确认不得触达 handler");
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
        assert_eq!(calls.lock().unwrap().len(), 1);
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

    /// 返回所有错误条目的 error code 列表（断言辅助）
    fn codes(errors: &[serde_json::Value]) -> Vec<&str> {
        errors
            .iter()
            .map(|e| e["error"].as_str().unwrap_or(""))
            .collect()
    }

    #[test]
    fn type_rule_string_number_integer_boolean() {
        let schema = json!({
            "properties": {
                "title": { "type": "string" },
                "price": { "type": "number" },
                "count": { "type": "integer" },
                "enabled": { "type": "boolean" }
            }
        });
        // 正例：各类型匹配
        assert!(validate_form_schema(
            &schema,
            &json!({ "title": "x", "price": 1.5, "count": 3, "enabled": true })
        )
        .is_empty());
        // 反例：类型不符（错误按字段名排序，与 Map 迭代序一致）
        let errors = validate_form_schema(
            &schema,
            &json!({ "title": 42, "price": "贵", "count": 3.5, "enabled": "yes" }),
        );
        assert_eq!(codes(&errors), vec!["type", "type", "type", "type"]);
        let title_err = errors.iter().find(|e| e["field"] == "title").unwrap();
        assert_eq!(
            title_err["message"],
            "字段 'title' 类型应为 string, 实际为 number"
        );
        let count_err = errors.iter().find(|e| e["field"] == "count").unwrap();
        assert_eq!(
            count_err["message"],
            "字段 'count' 类型应为 integer, 实际为 number"
        );
        // 浮点数字面量 1.0 不算 integer
        let errors = validate_form_schema(&schema, &json!({ "count": 1.0 }));
        assert_eq!(codes(&errors), vec!["type"]);
    }

    #[test]
    fn required_rule_keeps_presence_semantics() {
        let schema = json!({
            "properties": { "name": { "type": "string" }, "age": { "type": "integer" } },
            "required": ["name"]
        });
        assert!(validate_form_schema(&schema, &json!({ "name": "张三" })).is_empty());
        let errors = validate_form_schema(&schema, &json!({ "age": 30 }));
        assert_eq!(codes(&errors), vec!["required"]);
        assert_eq!(errors[0]["message"], "字段 'name' 为必填项");
        // 显式 null 视为已提供 → 不报 required；是否合法交由 type 规则
        let errors = validate_form_schema(&schema, &json!({ "name": serde_json::Value::Null }));
        assert!(!codes(&errors).contains(&"required"));
        assert_eq!(codes(&errors), vec!["type"]);
    }

    #[test]
    fn enum_rule_accepts_declared_values_only() {
        let schema = json!({
            "properties": { "status": { "type": "string", "enum": ["draft", "submitted"] } }
        });
        assert!(validate_form_schema(&schema, &json!({ "status": "draft" })).is_empty());
        let errors = validate_form_schema(&schema, &json!({ "status": "approved" }));
        assert_eq!(codes(&errors), vec!["enum"]);
    }

    #[test]
    fn minimum_maximum_rules_bound_numbers() {
        let schema = json!({
            "properties": { "age": { "type": "integer", "minimum": 0, "maximum": 150 } }
        });
        assert!(validate_form_schema(&schema, &json!({ "age": 0 })).is_empty());
        assert!(validate_form_schema(&schema, &json!({ "age": 150 })).is_empty());
        let errors = validate_form_schema(&schema, &json!({ "age": -1 }));
        assert_eq!(codes(&errors), vec!["minimum"]);
        let errors = validate_form_schema(&schema, &json!({ "age": 151 }));
        assert_eq!(codes(&errors), vec!["maximum"]);
        // 字符串值不触发数值边界规则（无 type 声明时无任何规则命中）
        let untyped = json!({
            "properties": { "age": { "minimum": 0, "maximum": 150 } }
        });
        assert!(validate_form_schema(&untyped, &json!({ "age": "老" })).is_empty());
    }

    #[test]
    fn min_length_max_length_rules_measure_scalars() {
        let schema = json!({
            "properties": { "code": { "type": "string", "minLength": 2, "maxLength": 5 } }
        });
        assert!(validate_form_schema(&schema, &json!({ "code": "AB12" })).is_empty());
        let errors = validate_form_schema(&schema, &json!({ "code": "A" }));
        assert_eq!(codes(&errors), vec!["min_length"]);
        let errors = validate_form_schema(&schema, &json!({ "code": "ABCDEF" }));
        assert_eq!(codes(&errors), vec!["max_length"]);
        // 多字节字符按 Unicode 标量计数（"张"=1）
        let errors = validate_form_schema(&schema, &json!({ "code": "张" }));
        assert_eq!(codes(&errors), vec!["min_length"]);
    }

    #[test]
    fn pattern_rule_validates_regex() {
        let schema = json!({
            "properties": { "email": { "type": "string", "pattern": r"^[^@]+@[^@]+$" } }
        });
        assert!(validate_form_schema(&schema, &json!({ "email": "a@b.com" })).is_empty());
        let errors = validate_form_schema(&schema, &json!({ "email": "not-an-email" }));
        assert_eq!(codes(&errors), vec!["pattern"]);
        // 非法正则跳过该规则（不 panic、不误报）
        let bad_schema = json!({
            "properties": { "x": { "type": "string", "pattern": "([" } }
        });
        assert!(validate_form_schema(&bad_schema, &json!({ "x": "anything" })).is_empty());
    }

    #[test]
    fn multiple_fields_report_isolated_errors() {
        let schema = json!({
            "properties": {
                "title": { "type": "string", "minLength": 3 },
                "qty": { "type": "integer", "minimum": 1 },
                "status": { "enum": ["on"] }
            },
            "required": ["title", "qty"]
        });
        let errors = validate_form_schema(&schema, &json!({ "qty": 0, "status": "off" }));
        // required(title) 缺 + qty minimum 越界 + status enum 不匹配
        assert_eq!(codes(&errors), vec!["required", "minimum", "enum"]);
    }

    #[tokio::test]
    async fn execute_contract_reports_valid_flag_and_errors() {
        // 通过 ToolResult 契约层（execute 不需 DB；connect_lazy 不发起连接）
        let ctx = ToolContext {
            session_id: 1,
            user_id: None,
            db_pool: sqlx::PgPool::connect_lazy("postgres://u:p@localhost:5432/nodb").unwrap(),
            allowed_schemas: vec![],
            action_handler: None,
        };
        let call = ToolCall {
            id: "c1".to_string(),
            name: "validate_form".to_string(),
            arguments: json!({
                "form_schema": {
                    "properties": { "age": { "type": "integer", "minimum": 0 } }
                },
                "field_values": { "age": -5 }
            }),
        };
        let result = ValidateFormTool::execute(&call, &ctx).await.unwrap();
        assert!(!result.success);
        assert_eq!(result.output["valid"], false);
        assert_eq!(result.output["errors"][0]["error"], "minimum");
        assert!(result.error.is_some());
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
}
