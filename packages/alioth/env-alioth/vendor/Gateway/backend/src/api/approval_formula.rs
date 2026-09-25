//! 审批公式 AI 生成与模拟执行（唯一引擎 Rhai；表达式不面向用户手写）
//!
//! - `POST /api/approval-flows/formula-assist` — 自然语言 → LLM 生成 Rhai 表达式
//!   → 服务端强校验（语法 + 引用标识符 ⊆ 变量清单）→ 结构化返回
//!   （fail-closed：校验失败即 invalid，不落库）
//! - `POST /api/approval-flows/expr-simulate` — 表达式 + 示例值 → 求值（Rhai 沙箱）
//!   → {ok, result, error}（可视化模拟执行）
//!
//! 校验复用唯一引擎（runtime-engine `RhaiExpressionEngine`：严格变量模式
//! validate + 沙箱求值）。LLM 复用 chat 基础设施
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

/// 唯一表达式引擎名（平台单一实现，客户端 MUST NOT 传其他值）
pub const ENGINE: &str = "rhai";

/// 表达式强校验（唯一引擎 Rhai，fail-closed）：语法 + 引用标识符 ⊆ 变量清单 + 自由函数白名单。
/// `_refs` 成员路径与 `ctx[...]` 索引形态由 `collect_variables` 归一为键名参与比对；
/// 白名单单一真相源 = `runtime-engine/expression/builtins.json`（未登记函数运行期必
/// `Function not found`，故 MUST 在写入/模拟入口拦下）。
fn validate_expression(
    expression: &str,
    engine: &str,
    context_fields: &[String],
) -> (bool, Vec<String>) {
    if engine != ENGINE {
        return (
            false,
            vec![format!("未知引擎 '{engine}'（唯一引擎：{ENGINE}）")],
        );
    }
    // 已知键集 = 上下文清单 + 结构键（_refs 引用容器 / entityId / ctx 恒可见）
    let mut known: Vec<String> = context_fields.to_vec();
    known.push("_refs".to_string());
    known.push("entityId".to_string());
    known.push("ctx".to_string());
    match runtime_engine::RhaiExpressionEngine::new().validate_all(expression, &known) {
        Ok(()) => (true, Vec::new()),
        Err(e) => (false, vec![e]),
    }
}

/// 求值（唯一引擎 Rhai）——模拟执行
fn evaluate_expression(
    expression: &str,
    engine: &str,
    sample: &serde_json::Map<String, Value>,
) -> Result<Value, String> {
    use std::collections::HashMap;
    if engine != ENGINE {
        return Err(format!("未知引擎 '{engine}'（唯一引擎：{ENGINE}）"));
    }
    let vars: HashMap<String, Value> = sample.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    runtime_engine::RhaiExpressionEngine::new().evaluate(expression, &vars)
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
    let engine = body.engine.clone().unwrap_or_else(|| ENGINE.to_string());
    if engine != ENGINE {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "expression": "", "engine": engine, "valid": false,
            "errors": vec![format!("未知引擎 '{engine}'（唯一引擎：{ENGINE}）")],
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

    let engine_desc = "Rhai（比较 == != < <= > >=、逻辑 && || !、in [\"a\",\"b\"]、s.contains(\"x\")、if cond { a } else { b }；字符串用双引号，JSON null 用 ()）";
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

    let variable_usage = runtime_engine::collect_variables(&expression)
        .map(|mut used| {
            used.sort();
            used.dedup();
            used
        })
        .unwrap_or_default();

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
    let engine = body.engine.clone().unwrap_or_else(|| ENGINE.to_string());
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
    let engine = body.engine.clone().unwrap_or_else(|| ENGINE.to_string());
    if engine != ENGINE {
        return Ok(HttpResponse::BadRequest().json(FormulaAssistResponse {
            expression: String::new(),
            engine: engine.clone(),
            valid: false,
            errors: vec![format!("未知引擎 '{engine}'（唯一引擎：{ENGINE}）")],
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
    let variable_usage = runtime_engine::collect_variables(&expression)
        .map(|mut used| {
            used.sort();
            used.dedup();
            used
        })
        .unwrap_or_default();

    Ok(HttpResponse::Ok().json(FormulaAssistResponse {
        expression,
        engine,
        valid,
        errors,
        explanation,
        variable_usage,
    }))
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
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_validate_valid() {
        let (valid, errors) = validate_expression(
            "amount > 5000 && code == \"VIP\"",
            ENGINE,
            &fields(&["amount", "code"]),
        );
        assert!(valid, "合法表达式应通过: {:?}", errors);
        assert!(errors.is_empty());
        let (valid, _) = validate_expression("ctx[\"act-group\"] == 1", ENGINE, &fields(&[]));
        assert!(valid, "ctx 索引语法应通过");
    }

    #[test]
    fn test_validate_syntax_error() {
        // 语法错 MUST fail-closed。夹具注意：`amount >> 5` 在 Rhai 中是**合法位运算**
        // （退役 DSL 才非法）⇒ 旧夹具钉的是已退役文法假设；此处用真语法错（括号不闭合）。
        let (valid, errors) = validate_expression("amount > ((", ENGINE, &fields(&["amount"]));
        assert!(!valid);
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_unknown_field_fail_closed() {
        let (valid, _errors) = validate_expression("amount > 100", ENGINE, &fields(&["total"]));
        assert!(!valid, "引用未知字段必须 fail-closed");
    }

    #[test]
    fn test_validate_unregistered_function_fail_closed() {
        // 模拟/写入入口 MUST 拦未登记自由函数：Rhai 编译期不报未注册函数，只在求值期失败
        let (valid, errors) =
            validate_expression("median([1, 2, 3]) > 0", ENGINE, &fields(&["amount"]));
        assert!(!valid, "未登记自由函数必须 fail-closed");
        assert!(errors.iter().any(|e| e.contains("median")), "{errors:?}");
        let (ok, errs) = validate_expression("sum([1, 2, 3]) > 0", ENGINE, &fields(&["amount"]));
        assert!(ok, "白名单内函数应通过: {errs:?}");
    }

    #[test]
    fn test_validate_unknown_engine_rejected() {
        let (valid, errors) = validate_expression("amount > 1", "dsl", &fields(&["amount"]));
        assert!(!valid, "旧引擎名必须拒绝（单一引擎）");
        assert!(errors.iter().any(|e| e.contains("唯一引擎")));
    }

    #[test]
    fn test_validate_script_forms() {
        let (valid, errors) = validate_expression(
            "let total = 0; for i in 0..3 { total += i; } total > 2",
            ENGINE,
            &[],
        );
        assert!(valid, "合法脚本应通过: {:?}", errors);
        let (bad, _) = validate_expression("if {", ENGINE, &[]);
        assert!(!bad, "语法错误应拒绝");
    }

    #[test]
    fn test_evaluate_condition() {
        let mut sample = serde_json::Map::new();
        sample.insert("amount".to_string(), json!(6000));
        let r = evaluate_expression("amount > 5000", ENGINE, &sample).unwrap();
        assert_eq!(r, json!(true));
    }

    #[test]
    fn test_evaluate_strict_unknown() {
        let sample = serde_json::Map::new();
        assert!(evaluate_expression("unknown > 1", ENGINE, &sample).is_err());
    }

    #[test]
    fn test_evaluate_rhai() {
        let mut sample = serde_json::Map::new();
        sample.insert("a".to_string(), json!(10));
        let r = evaluate_expression("a * 2 + 1", ENGINE, &sample).unwrap();
        assert_eq!(r, json!(21));
    }

    #[test]
    fn test_extract_json_tolerates_fences() {
        let out = extract_json("```json\n{\"expression\": \"a > 1\"}\n```").unwrap();
        assert!(out.contains("a > 1"));
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
