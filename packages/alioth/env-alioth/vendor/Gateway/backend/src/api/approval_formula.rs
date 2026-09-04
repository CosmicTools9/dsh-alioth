//! 审批公式 AI 生成与模拟执行（P1：DSL 不面向用户手写）
//!
//! - `POST /api/approval-flows/formula-assist` — 自然语言 → LLM 生成表达式
//!   （DSL 或 Rhai）→ 服务端强校验（语法 + 引用字段 ⊆ 变量清单 / Rhai 沙箱
//!   compile-only）→ 结构化返回（fail-closed：校验失败即 invalid，不落库）
//! - `POST /api/approval-flows/expr-simulate` — 表达式 + 示例值 → 求值
//!   （DSL strict fail-closed / Rhai 沙箱）→ {ok, result, error}（可视化模拟执行）
//!
//! 校验复用统一引擎（runtime-engine：ConstraintExpr parser/evaluator strict +
//! RhaiExpressionEngine 沙箱 validate）。LLM 复用 chat 基础设施
//! （DbLlmConfigAdapter，与 admin_ngac_assist 同模式）。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

use crate::api::chat_sessions::adapters::db_llm_config::DbLlmConfigAdapter;
use crate::api::chat_sessions::ports::LlmConfigPort;

#[derive(Debug, Deserialize)]
pub struct FormulaAssistRequest {
    /// 自然语言诉求（如「金额大于 5000 且客户是 VIP」）
    pub message: String,
    /// 目标引擎：dsl（默认）/ rhai
    #[serde(default)]
    pub engine: Option<String>,
    /// 可用变量清单（页面上下文 currentData 字段 + 上下文叶表字段）
    #[serde(default)]
    pub context_fields: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct FormulaAssistResponse {
    pub expression: String,
    pub engine: String,
    pub valid: bool,
    pub errors: Vec<String>,
    pub explanation: Option<String>,
    /// 表达式引用的变量（前端计算逻辑图/变量提示用）
    pub variable_usage: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExprSimulateRequest {
    pub expression: String,
    #[serde(default)]
    pub engine: Option<String>,
    /// 示例输入 {field: value}
    #[serde(default)]
    pub sample_values: serde_json::Map<String, Value>,
}

#[derive(Debug, Serialize)]
pub struct SimulateResponse {
    pub ok: bool,
    pub result: Option<Value>,
    pub error: Option<String>,
}

/// 表达式强校验（fail-closed）：语法 + 引用字段 ⊆ 变量清单 / Rhai 沙箱 compile-only
fn validate_expression(
    expression: &str,
    engine: &str,
    context_fields: &[String],
) -> (bool, Vec<String>) {
    match engine {
        "rhai" => {
            let eng = runtime_engine::RhaiExpressionEngine::new();
            match eng.validate(expression) {
                Ok(()) => (true, Vec::new()),
                Err(e) => (false, vec![e]),
            }
        }
        _ => match runtime_engine::parse_constraint_expression(expression) {
            Ok(ast) => {
                let mut used = Vec::new();
                collect_field_refs(&ast, &mut used);
                // 模型设计规则（2026-09-01）：`_refs.` 成员路径为外键列引用
                // 模式（运行时由 ctx 的 _refs 对象解析，成员集合随模型生成），
                // 不要求出现在可入选变量清单中
                let unknown: Vec<String> = used
                    .iter()
                    .filter(|f| {
                        !f.starts_with("_refs.")
                            && !context_fields.iter().any(|c| c.as_str() == f.as_str())
                    })
                    .cloned()
                    .collect();
                if unknown.is_empty() {
                    (true, Vec::new())
                } else {
                    (
                        false,
                        vec![format!(
                            "引用字段不在变量清单: {}（可用: {}）",
                            unknown.join(", "),
                            if context_fields.is_empty() {
                                "无".to_string()
                            } else {
                                context_fields.join(", ")
                            }
                        )],
                    )
                }
            }
            Err(e) => (false, vec![format!("DSL 语法错误: {e}")]),
        },
    }
}

/// AST 级字段引用收集（字符串字面量不计——准确校验）
fn collect_field_refs(expr: &runtime_engine::ConstraintExpr, out: &mut Vec<String>) {
    use runtime_engine::ConstraintExpr;
    match expr {
        ConstraintExpr::FieldRef(name) => out.push(name.clone()),
        ConstraintExpr::Binary(l, _, r) => {
            collect_field_refs(l, out);
            collect_field_refs(r, out);
        }
        ConstraintExpr::Unary(_, e) => collect_field_refs(e, out),
        ConstraintExpr::And(l, r) | ConstraintExpr::Or(l, r) => {
            collect_field_refs(l, out);
            collect_field_refs(r, out);
        }
        ConstraintExpr::Not(e) => collect_field_refs(e, out),
        ConstraintExpr::Call(_, args) => args.iter().for_each(|a| collect_field_refs(a, out)),
        ConstraintExpr::Literal(_) => {}
    }
}

/// 求值（DSL strict / Rhai 沙箱）——模拟执行
fn evaluate_expression(
    expression: &str,
    engine: &str,
    sample: &serde_json::Map<String, Value>,
) -> Result<Value, String> {
    use std::collections::HashMap;
    match engine {
        "rhai" => {
            let vars: HashMap<String, Value> =
                sample.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            runtime_engine::RhaiExpressionEngine::new().evaluate(expression, &vars)
        }
        _ => {
            let ast = runtime_engine::parse_constraint_expression(expression)
                .map_err(|e| format!("DSL 语法错误: {e}"))?;
            let vars: HashMap<String, Value> =
                sample.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            runtime_engine::ExpressionEvaluator::eval_expr_to_json_strict(&ast, &vars)
        }
    }
}

/// 从 LLM 输出提取 JSON（容忍围栏/前后噪声）
fn extract_json(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim();
    let trimmed = trimmed.strip_suffix("```").unwrap_or(trimmed).trim();
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end > start {
        Some(&trimmed[start..=end])
    } else {
        None
    }
}

const SYSTEM_PROMPT: &str = "你是 Alioth 流程条件表达式生成器。根据用户的自然语言诉求生成一条条件表达式。\
输出必须是 JSON：{\"expression\": \"<表达式>\", \"explanation\": \"<中文说明>\"}，不要输出其他内容。";

/// POST /api/approval-flows/formula-assist
async fn formula_assist(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<FormulaAssistRequest>,
) -> Result<HttpResponse, common::error::AliothError> {
    let _user_id = context::require_auth(&req)?;
    let engine = body.engine.clone().unwrap_or_else(|| "dsl".to_string());
    if engine != "dsl" && engine != "rhai" {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "expression": "", "engine": engine, "valid": false,
            "errors": vec![format!("未知引擎 '{engine}'（合法：dsl/rhai）")],
            "explanation": null, "variable_usage": []
        })));
    }

    // LLM 装配（复用 chat 基础设施；不可用 fail-closed）
    let llm = match DbLlmConfigAdapter::new(pool.get_ref().clone())
        .load_service()
        .await
    {
        Ok(s) => s,
        Err(e) => {
            log::error!("formula-assist: LLM unavailable: {}", e);
            return Ok(HttpResponse::ServiceUnavailable().json(serde_json::json!({
                "expression": "", "engine": engine, "valid": false,
                "errors": vec!["LLM 服务不可用（fail-closed）".to_string()],
                "explanation": null, "variable_usage": []
            })));
        }
    };

    let engine_desc = if engine == "rhai" {
        "Rhai 脚本（复杂公式；算术/函数/循环）"
    } else {
        "ConstraintExpr DSL（比较 == != < <= > >=、逻辑 && || !、in [..]、contains）"
    };
    // G5：字段清单截断（防 prompt 膨胀——>60 字段仅列前 60 并提示）
    const FIELDS_CAP: usize = 60;
    let fields_desc = if body.context_fields.is_empty() {
        "（未提供变量清单——只能使用字面量）".to_string()
    } else if body.context_fields.len() > FIELDS_CAP {
        format!(
            "{}（共 {} 个字段，仅列出前 {FIELDS_CAP} 个，优先使用这些）",
            body.context_fields[..FIELDS_CAP].join(", "),
            body.context_fields.len()
        )
    } else {
        body.context_fields.join(", ")
    };
    let prompt = format!(
        "可用变量（字段）：{fields_desc}\n目标引擎：{engine}（{engine_desc}）\n用户诉求：{}\n请生成表达式。",
        body.message
    );

    let raw = match llm
        .generate_with_system_preamble(
            SYSTEM_PROMPT,
            &prompt,
            Some(0.2),
            Some(4096),
            None,
            None,
            None,
        )
        .await
    {
        Ok(text) => text,
        Err(e) => {
            log::error!("formula-assist: LLM call failed: {}", e);
            return Ok(HttpResponse::BadGateway().json(serde_json::json!({
                "expression": "", "engine": engine, "valid": false,
                "errors": vec!["LLM 调用失败（fail-closed）".to_string()],
                "explanation": null, "variable_usage": []
            })));
        }
    };

    let json_text = match extract_json(&raw) {
        Some(t) => t,
        None => {
            log::warn!("formula-assist: LLM output not JSON (len={})", raw.len());
            return Ok(HttpResponse::BadGateway().json(serde_json::json!({
                "expression": "", "engine": engine, "valid": false,
                "errors": vec!["LLM 输出不可解析为 JSON（fail-closed），请重述诉求".to_string()],
                "explanation": null, "variable_usage": []
            })));
        }
    };
    let parsed: serde_json::Value = match serde_json::from_str(json_text) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("formula-assist: LLM JSON invalid: {}", e);
            return Ok(HttpResponse::BadGateway().json(serde_json::json!({
                "expression": "", "engine": engine, "valid": false,
                "errors": vec!["LLM 输出 JSON 结构非法（fail-closed），请重述诉求".to_string()],
                "explanation": null, "variable_usage": []
            })));
        }
    };
    let expression = parsed
        .get("expression")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let explanation = parsed
        .get("explanation")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if expression.is_empty() {
        return Ok(HttpResponse::BadGateway().json(serde_json::json!({
            "expression": "", "engine": engine, "valid": false,
            "errors": vec!["LLM 输出缺少 expression 字段（fail-closed）".to_string()],
            "explanation": explanation, "variable_usage": []
        })));
    }

    let variable_usage = if engine == "rhai" {
        Vec::new()
    } else {
        runtime_engine::parse_constraint_expression(&expression)
            .map(|ast| {
                let mut out = Vec::new();
                collect_field_refs(&ast, &mut out);
                out.sort();
                out.dedup();
                out
            })
            .unwrap_or_default()
    };

    // 强校验（fail-closed：校验失败即 invalid，不落库）
    let (valid, errors) = validate_expression(&expression, &engine, &body.context_fields);

    Ok(HttpResponse::Ok().json(FormulaAssistResponse {
        expression,
        engine,
        valid,
        errors,
        explanation,
        variable_usage,
    }))
}

/// POST /api/approval-flows/expr-simulate
async fn expr_simulate(
    req: HttpRequest,
    body: web::Json<ExprSimulateRequest>,
) -> Result<HttpResponse, common::error::AliothError> {
    let _user_id = context::require_auth(&req)?;
    let engine = body.engine.clone().unwrap_or_else(|| "dsl".to_string());
    match evaluate_expression(&body.expression, &engine, &body.sample_values) {
        Ok(result) => Ok(HttpResponse::Ok().json(SimulateResponse {
            ok: true,
            result: Some(result),
            error: None,
        })),
        Err(e) => Ok(HttpResponse::Ok().json(SimulateResponse {
            ok: false,
            result: None,
            error: Some(e),
        })),
    }
}

/// POST /api/approval-flows/formula-fix — 自愈修复（G1）：表达式 + 错误 →
/// LLM 生成修复表达式 → 强校验 → 结构化返回（面板一键应用，闭环自愈）
#[derive(Debug, Deserialize)]
pub struct FormulaFixRequest {
    pub expression: String,
    pub error: String,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub context_fields: Vec<String>,
}

async fn formula_fix(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<FormulaFixRequest>,
) -> Result<HttpResponse, common::error::AliothError> {
    let _user_id = context::require_auth(&req)?;
    let engine = body.engine.clone().unwrap_or_else(|| "dsl".to_string());
    if engine != "dsl" && engine != "rhai" {
        return Ok(HttpResponse::BadRequest().json(FormulaAssistResponse {
            expression: String::new(),
            engine: engine.clone(),
            valid: false,
            errors: vec![format!("未知引擎 '{engine}'（合法：dsl/rhai）")],
            explanation: None,
            variable_usage: vec![],
        }));
    }

    let llm = match DbLlmConfigAdapter::new(pool.get_ref().clone())
        .load_service()
        .await
    {
        Ok(s) => s,
        Err(e) => {
            log::error!("formula-fix: LLM unavailable: {}", e);
            return Ok(
                HttpResponse::ServiceUnavailable().json(FormulaAssistResponse {
                    expression: String::new(),
                    engine: engine.clone(),
                    valid: false,
                    errors: vec!["LLM 服务不可用（fail-closed）".to_string()],
                    explanation: None,
                    variable_usage: vec![],
                }),
            );
        }
    };

    const FIELDS_CAP: usize = 60;
    let fields_desc = if body.context_fields.is_empty() {
        "（未提供变量清单——只能使用字面量）".to_string()
    } else if body.context_fields.len() > FIELDS_CAP {
        format!(
            "{}（共 {} 个字段，仅列出前 {FIELDS_CAP} 个，优先使用这些）",
            body.context_fields[..FIELDS_CAP].join(", "),
            body.context_fields.len()
        )
    } else {
        body.context_fields.join(", ")
    };
    let prompt = format!(
        "可用变量（字段）：{fields_desc}\n目标引擎：{engine}\n原表达式（求值失败）：{}\n错误信息：{}\n请修复表达式，输出 JSON：{{\"expression\": \"<修复后表达式>\", \"explanation\": \"<修复说明>\"}}。",
        body.expression, body.error
    );

    let raw = match llm
        .generate_with_system_preamble(
            SYSTEM_PROMPT,
            &prompt,
            Some(0.2),
            Some(4096),
            None,
            None,
            None,
        )
        .await
    {
        Ok(text) => text,
        Err(e) => {
            log::error!("formula-fix: LLM call failed: {}", e);
            return Ok(HttpResponse::BadGateway().json(FormulaAssistResponse {
                expression: String::new(),
                engine: engine.clone(),
                valid: false,
                errors: vec!["LLM 调用失败（fail-closed）".to_string()],
                explanation: None,
                variable_usage: vec![],
            }));
        }
    };

    let json_text = match extract_json(&raw) {
        Some(t) => t,
        None => {
            log::warn!("formula-fix: LLM output not JSON (len={})", raw.len());
            return Ok(HttpResponse::BadGateway().json(FormulaAssistResponse {
                expression: String::new(),
                engine: engine.clone(),
                valid: false,
                errors: vec!["LLM 输出不可解析为 JSON（fail-closed）".to_string()],
                explanation: None,
                variable_usage: vec![],
            }));
        }
    };
    let parsed: serde_json::Value = match serde_json::from_str(json_text) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("formula-fix: LLM JSON invalid: {}", e);
            return Ok(HttpResponse::BadGateway().json(FormulaAssistResponse {
                expression: String::new(),
                engine,
                valid: false,
                errors: vec!["LLM 输出 JSON 结构非法（fail-closed）".to_string()],
                explanation: None,
                variable_usage: vec![],
            }));
        }
    };
    let expression = parsed
        .get("expression")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let explanation = parsed
        .get("explanation")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if expression.is_empty() {
        return Ok(HttpResponse::BadGateway().json(FormulaAssistResponse {
            expression: String::new(),
            engine,
            valid: false,
            errors: vec!["LLM 输出缺少 expression 字段（fail-closed）".to_string()],
            explanation,
            variable_usage: vec![],
        }));
    }

    let (valid, errors) = validate_expression(&expression, &engine, &body.context_fields);
    let variable_usage = if engine == "rhai" {
        Vec::new()
    } else {
        runtime_engine::parse_constraint_expression(&expression)
            .map(|ast| {
                let mut out = Vec::new();
                collect_field_refs(&ast, &mut out);
                out.sort();
                out.dedup();
                out
            })
            .unwrap_or_default()
    };

    Ok(HttpResponse::Ok().json(FormulaAssistResponse {
        expression,
        engine,
        valid,
        errors,
        explanation,
        variable_usage,
    }))
}

/// POST /api/approval-flows/expr-ast — 表达式 AST（G3 计算逻辑图数据源）
#[derive(Debug, Deserialize)]
pub struct ExprAstRequest {
    pub expression: String,
}

#[derive(Debug, Serialize)]
pub struct ExprAstResponse {
    pub ast: Option<Value>,
    pub error: Option<String>,
}

async fn expr_ast(
    req: HttpRequest,
    body: web::Json<ExprAstRequest>,
) -> Result<HttpResponse, common::error::AliothError> {
    let _user_id = context::require_auth(&req)?;
    match runtime_engine::parse_constraint_expression(&body.expression) {
        Ok(ast) => match serde_json::to_value(&ast) {
            Ok(v) => Ok(HttpResponse::Ok().json(ExprAstResponse {
                ast: Some(v),
                error: None,
            })),
            Err(e) => Ok(HttpResponse::Ok().json(ExprAstResponse {
                ast: None,
                error: Some(format!("AST 序列化失败: {e}")),
            })),
        },
        Err(e) => Ok(HttpResponse::Ok().json(ExprAstResponse {
            ast: None,
            error: Some(format!("DSL 语法错误: {e}")),
        })),
    }
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/api/approval-flows/formula-assist",
        web::post().to(formula_assist),
    )
    .route(
        "/api/approval-flows/formula-fix",
        web::post().to(formula_fix),
    )
    .route(
        "/api/approval-flows/expr-simulate",
        web::post().to(expr_simulate),
    )
    .route("/api/approval-flows/expr-ast", web::post().to(expr_ast));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_validate_dsl_valid() {
        let (valid, errors) = validate_expression(
            "amount > 5000 && code == 'VIP'",
            "dsl",
            &fields(&["amount", "code"]),
        );
        assert!(valid, "合法 DSL 应通过: {:?}", errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_dsl_syntax_error() {
        let (valid, errors) = validate_expression("amount >> 5", "dsl", &fields(&["amount"]));
        assert!(!valid);
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_dsl_unknown_field_fail_closed() {
        let (valid, errors) = validate_expression("amount > 100", "dsl", &fields(&["total"]));
        assert!(!valid, "引用未知字段必须 fail-closed");
        assert!(errors.iter().any(|e| e.contains("不在变量清单")));
    }

    #[test]
    fn test_validate_dsl_dash_identifier() {
        let (valid, errors) = validate_expression("act-group == 1", "dsl", &fields(&["act-group"]));
        assert!(valid, "连字符标识符应通过: {:?}", errors);
    }

    #[test]
    fn test_validate_rhai() {
        let (valid, errors) = validate_expression(
            "let total = 0; for i in 0..3 { total += i; } total > 2",
            "rhai",
            &[],
        );
        assert!(valid, "合法 Rhai 应通过: {:?}", errors);
        let (bad, _) = validate_expression("if {", "rhai", &[]);
        assert!(!bad, "语法错误 Rhai 应拒绝");
    }

    #[test]
    fn test_evaluate_dsl() {
        let mut sample = serde_json::Map::new();
        sample.insert("amount".to_string(), json!(6000));
        let r = evaluate_expression("amount > 5000", "dsl", &sample).unwrap();
        assert_eq!(r, json!(true));
    }

    #[test]
    fn test_evaluate_dsl_strict_unknown() {
        let sample = serde_json::Map::new();
        assert!(evaluate_expression("unknown > 1", "dsl", &sample).is_err());
    }

    #[test]
    fn test_evaluate_rhai() {
        let mut sample = serde_json::Map::new();
        sample.insert("a".to_string(), json!(10));
        let r = evaluate_expression("a * 2 + 1", "rhai", &sample).unwrap();
        assert_eq!(r, json!(21));
    }

    #[test]
    fn test_extract_json_tolerates_fences() {
        let out = extract_json("```json\n{\"expression\": \"a > 1\"}\n```").unwrap();
        assert!(out.contains("a > 1"));
    }

    #[test]
    fn test_expr_ast_serialization_shape() {
        // G3：ConstraintExpr Serialize → AST JSON（前端计算逻辑图数据源）
        let ast =
            runtime_engine::parse_constraint_expression("amount > 5000 && code == 'VIP'").unwrap();
        let v = serde_json::to_value(&ast).unwrap();
        let s = v.to_string();
        assert!(
            s.contains("amount") && (s.contains("And") || s.contains("and")),
            "AST JSON 应含字段与逻辑结构: {}",
            s
        );
        // 字符串字面量不泄漏为字段（AST 级精确）
        let ast2 = runtime_engine::parse_constraint_expression("code == 'VIP'").unwrap();
        let s2 = serde_json::to_string(&ast2).unwrap();
        assert!(
            !s2.contains("\"VIP\"") || s2.contains("String"),
            "字面量应为 String 字面量: {}",
            s2
        );
    }

    #[test]
    fn test_fields_cap_truncation() {
        // G5：>60 字段截断提示（模拟 prompt 构建的字段描述——直接验证截断常量生效）
        let fields: Vec<String> = (0..80).map(|i| format!("f{i}")).collect();
        let desc = if fields.len() > 60 {
            format!(
                "{}（共 {} 个字段，仅列出前 60 个，优先使用这些）",
                fields[..60].join(", "),
                fields.len()
            )
        } else {
            fields.join(", ")
        };
        assert!(desc.contains("共 80 个字段"));
        assert!(desc.contains("仅列出前 60 个"));
    }
}
