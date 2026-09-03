//! 工具执行器实现

use super::row_value;
use super::{ToolCall, ToolContext, ToolResult};
use crate::agents::ToolDefinition;
use serde_json::json;
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use sqlx::{AssertSqlSafe, Column, Row};

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

        // TODO: 实现真正的 Schema 校验（集成 jsonschema 或手写规则引擎）
        let mut errors = Vec::new();
        if let Some(props) = form_schema.get("properties").and_then(|p| p.as_object()) {
            for (field_name, _schema) in props {
                let required = form_schema
                    .get("required")
                    .and_then(|r| r.as_array())
                    .map(|arr| arr.iter().any(|v| v.as_str() == Some(field_name)))
                    .unwrap_or(false);
                let has_value = field_values.get(field_name).is_some();
                if required && !has_value {
                    errors.push(json!({
                        "field": field_name,
                        "error": "required",
                        "message": format!("字段 '{}' 为必填项", field_name)
                    }));
                }
            }
        }

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

/// 单据查询工具
pub struct QueryDocumentTool;

impl QueryDocumentTool {
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "query_document".to_string(),
            description:
                "查询业务单据记录（订单、采购单、入库单等），支持按状态、时间范围、关联实体筛选"
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "document_type": { "type": "string", "description": "单据类型代码" },
                    "status": { "type": "string", "description": "状态筛选" },
                    "limit": { "type": "integer", "description": "返回条数上限", "default": 50 },
                    "filters": { "type": "object", "description": "额外筛选条件" }
                },
                "required": ["document_type"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, String> {
        let args = call.arguments.clone();
        let doc_type = args
            .get("document_type")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'document_type' parameter")?;
        let status = args.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(50)
            .min(1000) as i64;

        // 查询 zc_id_entity 子表，通过 _f_/_t_ 匹配单据类型
        // 实际实现需根据 document_type 映射到具体表名
        let sql = format!(
            r#"SELECT id, code, notice, "_f_", "_t_", created_at
               FROM isahl.zc_id_entity
               WHERE "_f_" = $1 AND deleted_at IS NULL
               {} ORDER BY created_at DESC LIMIT $2"#,
            if status.is_empty() {
                ""
            } else {
                "AND \"_t_\" = $3"
            }
        );

        let mut query = sqlx::query(AssertSqlSafe(sql.as_str()))
            .bind(doc_type)
            .bind(limit);
        if !status.is_empty() {
            query = query.bind(status);
        }

        let rows = query
            .fetch_all(&ctx.db_pool)
            .await
            .map_err(|e| format!("Document query error: {}", e))?;

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
            output: json!({ "document_type": doc_type, "rows": results, "count": results.len() }),
            error: None,
        })
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
                    "params": { "type": "object", "description": "动作参数" }
                },
                "required": ["action_type", "target_ids"]
            }),
            execution_target: crate::agents::ExecutionTarget::Backend,
        }
    }

    pub async fn execute(call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, String> {
        let args = call.arguments.clone();
        let action_type = args
            .get("action_type")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'action_type' parameter")?;
        let target_ids = args
            .get("target_ids")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // TODO: 实际动作执行需接入 Trigger Registry 或业务服务
        // 当前返回预览结果，不实际修改数据
        let preview = json!({
            "action_type": action_type,
            "target_count": target_ids.len(),
            "targets": target_ids,
            "status": "preview",
            "message": "动作已预览，需用户确认后执行"
        });

        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            success: true,
            output: preview,
            error: None,
        })
    }
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

        match &statements[0] {
            sqlparser::ast::Statement::Query(_) => {}
            _ => {
                return Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    success: false,
                    output: json!(null),
                    error: Some("Only SELECT statements are allowed".to_string()),
                });
            }
        }

        // schema 白名单过滤
        if !ctx.allowed_schemas.is_empty() {
            let has_allowed = ctx.allowed_schemas.iter().any(|s| sql.contains(s));
            if !has_allowed {
                return Ok(ToolResult {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    success: false,
                    output: json!(null),
                    error: Some(format!(
                        "SQL must reference allowed schemas: {:?}",
                        ctx.allowed_schemas
                    )),
                });
            }
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
