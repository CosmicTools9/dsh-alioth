//! DMN 决策表 AI 起草/修订（add-decision-table-ai-authoring；migrate-dmn-assist-to-framework）
//!
//! 自然语言 → LLM 整表 DMN JSON → 服务端整表强校验（结构/hit-policy/
//! 逐 cell DSL 语法 + 引用字段 ⊆ 变量清单/输出 ∈ 出边 label 或兜底边）
//! → fail-closed 结构化返回（不落库）；面板逐条中文规则审阅后确认应用。
//!
//! 原实现于 Gateway approval_formula.rs（依赖 Gateway chat_sessions）——ns 服务化
//! 部署（AVIC commitment 等 scope 委托 approval::configure_routes）缺该端点 → AI
//! 起草 404。迁移至此：LLM 服务构建走 crate::llm_assist（Framework 级，读
//! zc_id_prot-llm_config + env 兜底）。本模块注册于 approval::configure_routes——
//! 所有委托方（Gateway 主裸路径 + ns 服务 scope）自动获得 dmn-assist/dmn-fix。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;

const DMN_INPUT_CAP: usize = 8;
const DMN_RULE_CAP: usize = 50;

#[derive(Debug, Deserialize)]
pub struct DmnAssistRequest {
    /// 自然语言规则诉求（如「金额≥10000 且客户等级 A → 大额审批，其余 → 普通审批」）
    pub message: String,
    /// 可用变量（上下文字段名）
    #[serde(default)]
    pub context_fields: Vec<String>,
    /// 出边 label 路由候选（去重后）；空 = 未提供（输出不受限）
    #[serde(default)]
    pub outputs: Vec<String>,
    /// 存在无 label 兜底边 → 输出不受出边 label 限制
    #[serde(default)]
    pub allow_unlisted: bool,
    /// dmn-fix 修订基座（起草不传）
    #[serde(default)]
    pub existing: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct DmnAssistResponse {
    pub valid: bool,
    pub errors: Vec<String>,
    /// 规范化后的整表（与 FlowNode.dmn 1:1；校验失败亦回传草案供修订）
    pub dmn: Option<Value>,
    /// 逐条中文规则说明（与 rules 对齐；前端审阅行）
    pub rules_explanation: Vec<String>,
    pub summary: Option<String>,
    pub variable_usage: Vec<String>,
}

const DMN_SYSTEM_PROMPT: &str = "你是 Alioth 流程设计器的决策表（DMN）生成器。根据用户诉求\
把业务规则整理为一张决策表：输入列（inputs）表示参与判定的上下文字段（列名 MUST 从可用变量中选取）；\
每条规则（rules）的 match 为该输入列的命中条件——空/不填 = 通配（恒命中），非空 = DSL 条件表达式\
（支持 == != < <= > >=、逻辑 && || !、算术 + - * / %、in [a, b]、contains、字段成员路径 _refs.xxx）；\
output 为命中后的路由输出值（MUST 从出边路由候选中选取，除非声明存在兜底边）。\
hitPolicy 取值：FIRST = 按行序首中（通配兜底行放最后）；UNIQUE = 恰一行命中（行间条件须互斥）；\
ANY = 多行命中但输出须一致；PRIORITY = 每行可带正整数 priority（越小越优先，缺省按行序）；\
COLLECT = 聚合全部命中（可选 aggregation：list 默认多输出/ count / sum / min / max 数值聚合）。\
输出必须是 JSON：{\"hitPolicy\": \"...\", \"inputs\": [{\"name\": \"...\"}], \
\"rules\": [{\"match\": [\"表达式或空\"], \"output\": \"...\"}], \
\"rules_explanation\": [\"当 ... 且 ... → 输出值（逐条中文，顺序与 rules 一一对应）\"], \
\"summary\": \"整表一句话中文说明\"}，不要输出其他内容。";

/// 规范化 LLM 草案 → 节点 dmn 形状（inputs 列名对象/match 空归一 null 通配），
/// 并整表强校验（fail-closed 语义与 validation.ts / publish 结构校验一致）。
fn normalize_and_validate_dmn(
    raw: &Value,
    context_fields: &[String],
    outputs: &[String],
    allow_unlisted: bool,
) -> (Option<Value>, Vec<String>) {
    let mut errors: Vec<String> = Vec::new();
    if !raw.is_object() {
        return (
            None,
            vec!["LLM 输出决策表结构非法（须为 JSON 对象）".to_string()],
        );
    }

    let hit_policy = raw
        .get("hitPolicy")
        .and_then(|v| v.as_str())
        .unwrap_or("FIRST")
        .trim()
        .to_string();
    if !["UNIQUE", "FIRST", "ANY", "PRIORITY", "COLLECT"].contains(&hit_policy.as_str()) {
        errors.push(format!(
            "非法命中策略「{hit_policy}」（须 UNIQUE/FIRST/ANY/PRIORITY/COLLECT）"
        ));
    }
    // COLLECT aggregation 合法（extend-dmn-decision-table-full）
    if hit_policy == "COLLECT" {
        if let Some(agg) = raw.get("aggregation").and_then(|v| v.as_str()) {
            if !["list", "count", "sum", "min", "max"].contains(&agg.trim()) {
                errors.push(format!(
                    "COLLECT aggregation「{agg}」非法（须 list/count/sum/min/max）"
                ));
            }
        }
    }

    let mut inputs: Vec<Value> = Vec::new();
    match raw.get("inputs").and_then(|v| v.as_array()) {
        Some(arr) => {
            if arr.is_empty() {
                errors.push("决策表至少需要 1 个输入列".to_string());
            }
            for (i, item) in arr.iter().enumerate() {
                let name = match item {
                    Value::String(s) => s.trim().to_string(),
                    Value::Object(m) => m
                        .get("name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                    _ => String::new(),
                };
                if name.is_empty() {
                    errors.push(format!("输入列 {} 名称缺失", i + 1));
                    continue;
                }
                if inputs
                    .iter()
                    .any(|v| v.get("name").and_then(|x| x.as_str()) == Some(name.as_str()))
                {
                    errors.push(format!("输入列名称重复: {name}"));
                    continue;
                }
                inputs.push(json!({ "name": name }));
            }
            if inputs.len() > DMN_INPUT_CAP {
                errors.push(format!("输入列超过上限 {DMN_INPUT_CAP}（UI 可读性约束）"));
            }
        }
        None => errors.push("决策表缺少 inputs 输入列定义".to_string()),
    }

    let ncols = inputs.len();
    let mut rules: Vec<Value> = Vec::new();
    match raw.get("rules").and_then(|v| v.as_array()) {
        Some(arr) => {
            if arr.is_empty() {
                errors.push("决策表至少需要 1 条规则".to_string());
            }
            for (ri, item) in arr.iter().enumerate() {
                if ri >= DMN_RULE_CAP {
                    errors.push(format!("规则超过上限 {DMN_RULE_CAP}"));
                    break;
                }
                if !item.is_object() {
                    errors.push(format!("规则 {} 结构非法（须为对象）", ri + 1));
                    continue;
                }
                let obj = item.as_object().unwrap();
                let output = obj
                    .get("output")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if output.is_empty() {
                    errors.push(format!("规则 {} 输出值缺失", ri + 1));
                }
                if !outputs.is_empty() && !allow_unlisted && !outputs.contains(&output) {
                    errors.push(format!(
                        "规则 {} 输出「{output}」无匹配出边 label（可选: {}）",
                        ri + 1,
                        outputs.join(" / ")
                    ));
                }
                let mut match_row: Vec<Value> = Vec::new();
                match obj.get("match") {
                    None => errors.push(format!("规则 {} 缺少 match 行", ri + 1)),
                    Some(Value::Array(cells)) => {
                        if cells.len() != ncols {
                            errors.push(format!(
                                "规则 {} 有 {} 个条件但输入列为 {ncols} 个",
                                ri + 1,
                                cells.len()
                            ));
                        }
                        for (ci, cell) in cells.iter().enumerate() {
                            let text = match cell {
                                Value::Null => String::new(),
                                Value::String(s) => s.trim().to_string(),
                                _ => {
                                    errors.push(format!(
                                        "规则 {} 第 {} 列条件须为表达式字符串或空（通配）",
                                        ri + 1,
                                        ci + 1
                                    ));
                                    String::new()
                                }
                            };
                            if !text.is_empty() {
                                let (valid, errs) = crate::llm_assist::validate_dsl_expression(
                                    &text,
                                    context_fields,
                                );
                                if !valid {
                                    let col: String = inputs
                                        .get(ci)
                                        .and_then(|v| v.get("name"))
                                        .and_then(|v| v.as_str())
                                        .map(str::to_string)
                                        .unwrap_or_else(|| format!("列{}", ci + 1));
                                    for e in errs {
                                        errors.push(format!(
                                            "规则 {} 列「{col}」表达式「{text}」: {e}",
                                            ri + 1
                                        ));
                                    }
                                }
                            }
                            match_row.push(if text.is_empty() {
                                Value::Null
                            } else {
                                Value::String(text)
                            });
                        }
                    }
                    Some(_) => errors.push(format!("规则 {} 的 match 须为数组", ri + 1)),
                }
                // PRIORITY 显式 priority 透传（≥1；缺省不写——运行时按行序）
                let mut rule_json = json!({ "match": match_row, "output": output });
                if let Some(p) = obj.get("priority").and_then(|v| v.as_i64()) {
                    if p < 1 {
                        errors.push(format!("规则 {} priority 须 ≥1（实际 {p}）", ri + 1));
                    } else {
                        rule_json["priority"] = json!(p);
                    }
                }
                rules.push(rule_json);
            }
        }
        None => errors.push("决策表缺少 rules 规则".to_string()),
    }

    let mut top = json!({ "hitPolicy": hit_policy, "inputs": inputs, "rules": rules });
    if hit_policy == "COLLECT" {
        if let Some(agg) = raw.get("aggregation").and_then(|v| v.as_str()) {
            top["aggregation"] = json!(agg.trim());
        }
    }
    (Some(top), errors)
}

/// 决策表全部非通配 cell 引用字段（去重排序；前端变量提示/校验展示）
fn dmn_variable_usage(dmn: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(rules) = dmn.get("rules").and_then(|v| v.as_array()) {
        for r in rules {
            if let Some(cells) = r.get("match").and_then(|v| v.as_array()) {
                for c in cells {
                    if let Some(s) = c.as_str() {
                        if !s.trim().is_empty() {
                            if let Ok(ast) = runtime_engine::parse_constraint_expression(s) {
                                collect_field_refs(&ast, &mut out);
                            }
                        }
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
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

fn invalid_dmn_response(errors: Vec<String>) -> DmnAssistResponse {
    DmnAssistResponse {
        valid: false,
        errors,
        dmn: None,
        rules_explanation: Vec::new(),
        summary: None,
        variable_usage: Vec::new(),
    }
}

fn fields_desc_for_prompt(context_fields: &[String]) -> String {
    const FIELDS_CAP: usize = 60;
    if context_fields.is_empty() {
        "（未提供变量清单——只能使用字面量）".to_string()
    } else if context_fields.len() > FIELDS_CAP {
        format!(
            "{}（共 {} 个字段，仅列出前 {FIELDS_CAP} 个，优先使用这些）",
            context_fields[..FIELDS_CAP].join(", "),
            context_fields.len()
        )
    } else {
        context_fields.join(", ")
    }
}

/// 进程内起草/修订传输层错误（HTTP 层映射 503/502；ai-agent 工具映射 fail-soft）
#[derive(Debug, Clone)]
pub enum DmnAssistCoreError {
    LlmUnavailable(String),
    LlmCallFailed(String),
    BadOutput(String),
}

/// dmn-assist / dmn-fix 共用执行链核心（进程内复用：HTTP handler 与 ai-agent 工具
/// 同源，防分叉——add-dock-dmn-authoring）：LLM 生成整表 → 规范化 + 强校验 →
/// 结构化返回（valid=false 亦回传草案与明细；传输失败以 Err 区分）。
pub async fn dmn_generate_core(
    pool: &PgPool,
    kind: &str,
    message: &str,
    context_fields: &[String],
    outputs: &[String],
    allow_unlisted: bool,
    existing: Option<&Value>,
) -> Result<DmnAssistResponse, DmnAssistCoreError> {
    let llm = match crate::llm_assist::load_llm_service(pool).await {
        Ok(s) => s,
        Err(e) => {
            common::telemetry::error!("{kind}: LLM unavailable: {e}");
            return Err(DmnAssistCoreError::LlmUnavailable(e));
        }
    };

    let outputs_desc = if outputs.is_empty() {
        "（未提供——输出可为任意业务值，发布时须有对应出边）".to_string()
    } else {
        outputs.join(" / ")
    };
    let mut prompt = format!(
        "可用变量（字段）：{}\n出边路由候选（output 必须从其中选取）：{}\n\
         无 label 兜底边：{}（允许时 output 允许不在候选中）\n用户诉求：{}",
        fields_desc_for_prompt(context_fields),
        outputs_desc,
        if allow_unlisted {
            "允许"
        } else {
            "不允许"
        },
        message
    );
    if let Some(ex) = existing {
        prompt.push_str(&format!(
            "\n当前决策表（修订基座——用户只描述对它的改动，语义须保持；输出为完整新表）：{}",
            serde_json::to_string(ex).unwrap_or_default()
        ));
    }
    prompt.push_str("\n请生成完整决策表。");

    let raw = match llm
        .generate_with_system_preamble(
            DMN_SYSTEM_PROMPT,
            &prompt,
            Some(0.2),
            Some(8192),
            None,
            None,
            None,
        )
        .await
    {
        Ok(text) => text,
        Err(e) => {
            common::telemetry::error!("{kind}: LLM call failed: {e}");
            return Err(DmnAssistCoreError::LlmCallFailed(e.to_string()));
        }
    };

    let json_text = match crate::llm_assist::extract_json(&raw) {
        Some(t) => t,
        None => {
            common::telemetry::warn!("{kind}: LLM output not JSON (len={})", raw.len());
            return Err(DmnAssistCoreError::BadOutput(
                "LLM 输出不可解析为 JSON（fail-closed），请重述诉求".to_string(),
            ));
        }
    };
    let parsed: Value = match serde_json::from_str(json_text) {
        Ok(v) => v,
        Err(e) => {
            common::telemetry::warn!("{kind}: LLM JSON invalid: {e}");
            return Err(DmnAssistCoreError::BadOutput(
                "LLM 输出 JSON 结构非法（fail-closed），请重述诉求".to_string(),
            ));
        }
    };

    let rules_explanation = parsed
        .get("rules_explanation")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let summary = parsed
        .get("summary")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let (normalized, errors) =
        normalize_and_validate_dmn(&parsed, context_fields, outputs, allow_unlisted);
    let variable_usage = normalized
        .as_ref()
        .map(dmn_variable_usage)
        .unwrap_or_default();

    Ok(DmnAssistResponse {
        valid: errors.is_empty(),
        errors,
        dmn: normalized,
        rules_explanation,
        summary,
        variable_usage,
    })
}

/// dmn-assist / dmn-fix HTTP 薄包装：认证 + 传输错误映射 503/502（行为与消息保持）
async fn dmn_llm_roundtrip(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    kind: &str,
    message: &str,
    context_fields: &[String],
    outputs: &[String],
    allow_unlisted: bool,
    existing: Option<&Value>,
) -> Result<HttpResponse, common::error::AliothError> {
    let _user_id = context::require_auth(&req)?;

    match dmn_generate_core(
        pool.get_ref(),
        kind,
        message,
        context_fields,
        outputs,
        allow_unlisted,
        existing,
    )
    .await
    {
        Ok(resp) => Ok(HttpResponse::Ok().json(resp)),
        Err(DmnAssistCoreError::LlmUnavailable(e)) => {
            common::telemetry::error!("{kind}: {e}");
            Ok(HttpResponse::ServiceUnavailable().json(DmnAssistResponse {
                valid: false,
                errors: vec!["LLM 服务不可用（fail-closed）".to_string()],
                dmn: None,
                rules_explanation: Vec::new(),
                summary: None,
                variable_usage: Vec::new(),
            }))
        }
        Err(DmnAssistCoreError::LlmCallFailed(e)) => {
            common::telemetry::error!("{kind}: LLM call failed: {e}");
            Ok(HttpResponse::BadGateway().json(DmnAssistResponse {
                valid: false,
                errors: vec!["LLM 调用失败（fail-closed）".to_string()],
                dmn: None,
                rules_explanation: Vec::new(),
                summary: None,
                variable_usage: Vec::new(),
            }))
        }
        Err(DmnAssistCoreError::BadOutput(msg)) => {
            Ok(HttpResponse::BadGateway().json(DmnAssistResponse {
                valid: false,
                errors: vec![msg],
                dmn: None,
                rules_explanation: Vec::new(),
                summary: None,
                variable_usage: Vec::new(),
            }))
        }
    }
}

/// POST /approval-flows/dmn-assist — 自然语言起草整表决策表
pub async fn dmn_assist(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<DmnAssistRequest>,
) -> Result<HttpResponse, common::error::AliothError> {
    if body.message.trim().is_empty() {
        return Ok(HttpResponse::BadRequest().json(invalid_dmn_response(vec![
            "请描述决策规则（message 不能为空）".to_string(),
        ])));
    }
    dmn_llm_roundtrip(
        pool,
        req,
        "dmn-assist",
        body.message.trim(),
        &body.context_fields,
        &body.outputs,
        body.allow_unlisted,
        None,
    )
    .await
}

/// POST /approval-flows/dmn-fix — 修订自愈：当前表/草案 + 诉求或错误 → 重新生成 + 同校验
pub async fn dmn_fix(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    body: web::Json<DmnAssistRequest>,
) -> Result<HttpResponse, common::error::AliothError> {
    if body.message.trim().is_empty() {
        return Ok(HttpResponse::BadRequest().json(invalid_dmn_response(vec![
            "请描述修订诉求（message 不能为空）".to_string(),
        ])));
    }
    if body.existing.is_none() {
        return Ok(HttpResponse::BadRequest().json(invalid_dmn_response(vec![
            "dmn-fix 需要携带 existing 当前决策表".to_string(),
        ])));
    }
    dmn_llm_roundtrip(
        pool,
        req,
        "dmn-fix",
        body.message.trim(),
        &body.context_fields,
        &body.outputs,
        body.allow_unlisted,
        body.existing.as_ref(),
    )
    .await
}

/// 路由注册（scope 由调用方委托：Gateway 主挂 `/api` 裸路径、ns 服务挂
/// `/service/{svc}` —— 路径为 scope 相对 `approval-flows/dmn-*`）
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route("/approval-flows/dmn-assist", web::post().to(dmn_assist))
        .route("/approval-flows/dmn-fix", web::post().to(dmn_fix));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn dmn_valid() -> serde_json::Value {
        json!({
            "hitPolicy": "FIRST",
            "inputs": [{"name": "amount"}, {"name": "level"}],
            "rules": [
                { "match": ["amount >= 10000", "level == 'A'"], "output": "go-big" },
                { "match": [null, ""], "output": "go-normal" }
            ]
        })
    }

    #[test]
    fn valid_table_passes() {
        let (dmn, errors) = normalize_and_validate_dmn(
            &dmn_valid(),
            &fields(&["amount", "level"]),
            &fields(&["go-big", "go-normal"]),
            false,
        );
        assert!(errors.is_empty(), "合法整表应通过: {errors:?}");
        let d = dmn.unwrap();
        assert_eq!(d["rules"][1]["match"][1], json!(null));
        assert_eq!(d["rules"][0]["output"], json!("go-big"));
    }

    #[test]
    fn unknown_field_cell_fail_closed() {
        let raw = json!({
            "hitPolicy": "FIRST",
            "inputs": [{"name": "amount"}],
            "rules": [{ "match": ["amount > 100"], "output": "go-a" }]
        });
        let (_, errors) =
            normalize_and_validate_dmn(&raw, &fields(&["total"]), &fields(&["go-a"]), false);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("amount") && e.contains("不在变量清单")),
            "未知字段必须 fail-closed: {errors:?}"
        );
    }

    #[test]
    fn cell_syntax_error_reports_coordinate() {
        let raw = json!({
            "hitPolicy": "FIRST",
            "inputs": [{"name": "amount"}],
            "rules": [{ "match": ["amount >> 100"], "output": "go-a" }]
        });
        let (_, errors) =
            normalize_and_validate_dmn(&raw, &fields(&["amount"]), &fields(&["go-a"]), false);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("规则 1") && e.contains("amount")),
            "语法错误应带规则与列坐标: {errors:?}"
        );
    }

    #[test]
    fn output_must_match_edge_label_unless_unlisted() {
        let raw = json!({
            "hitPolicy": "FIRST",
            "inputs": [{"name": "amount"}],
            "rules": [{ "match": ["amount >= 10000"], "output": "go-big" }]
        });
        let (_, errors) =
            normalize_and_validate_dmn(&raw, &fields(&["amount"]), &fields(&["go-normal"]), false);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("go-big") && e.contains("无匹配出边")),
            "无兜底边输出未命中 label 必须拒绝: {errors:?}"
        );
        let (_, errors2) =
            normalize_and_validate_dmn(&raw, &fields(&["amount"]), &fields(&["go-normal"]), true);
        assert!(errors2.is_empty(), "兜底边存在时应放行: {errors2:?}");
    }

    #[test]
    fn structure_violations() {
        let raw0 = json!({ "hitPolicy": "FIRST", "inputs": [{"name":"a"}], "rules": [] });
        let (_, e0) = normalize_and_validate_dmn(&raw0, &fields(&["a"]), &[], false);
        assert!(e0.iter().any(|e| e.contains("至少需要 1 条规则")));

        let raw2 = json!({ "hitPolicy": "FOO", "inputs": [{"name":"a"}], "rules": [{ "match": [""], "output": "x" }] });
        let (_, e2) = normalize_and_validate_dmn(&raw2, &fields(&["a"]), &[], false);
        assert!(e2.iter().any(|e| e.contains("非法命中策略")));

        // COLLECT + 非法 aggregation 拒绝；合法放行
        let bad_agg = json!({ "hitPolicy": "COLLECT", "aggregation": "avg", "inputs": [{"name":"a"}], "rules": [{ "match": [""], "output": "x" }] });
        let (_, e_agg) = normalize_and_validate_dmn(&bad_agg, &fields(&["a"]), &[], false);
        assert!(e_agg
            .iter()
            .any(|e| e.contains("aggregation") && e.contains("avg")));
        let ok_agg = json!({ "hitPolicy": "COLLECT", "aggregation": "sum", "inputs": [{"name":"a"}], "rules": [{ "match": [""], "output": "x" }] });
        let (out_agg, e_ok) = normalize_and_validate_dmn(&ok_agg, &fields(&["a"]), &[], false);
        assert!(e_ok.is_empty(), "COLLECT+sum 应合法: {e_ok:?}");
        assert_eq!(out_agg.unwrap()["aggregation"], "sum");

        // 列名重复
        let raw3 = json!({ "hitPolicy": "FIRST", "inputs": [{"name":"a"},{"name":"a"}], "rules": [{ "match": ["1","2"], "output": "x" }] });
        let (_, e3) = normalize_and_validate_dmn(&raw3, &fields(&["a"]), &[], false);
        assert!(e3.iter().any(|e| e.contains("名称重复")));

        // match 与输入列不等长
        let raw4 = json!({ "hitPolicy": "FIRST", "inputs": [{"name":"a"},{"name":"b"}], "rules": [{ "match": [""], "output": "x" }] });
        let (_, e4) = normalize_and_validate_dmn(&raw4, &fields(&["a", "b"]), &[], false);
        assert!(e4.iter().any(|e| e.contains("1 个条件但输入列为 2 个")));
    }

    #[test]
    fn variable_usage_union() {
        let (dmn, errors) = normalize_and_validate_dmn(
            &json!({
                "hitPolicy": "FIRST",
                "inputs": [{"name": "amount"}, {"name": "level"}],
                "rules": [
                    { "match": ["amount >= 10000", "level == 'A'"], "output": "x" },
                    { "match": ["amount < 10000", ""], "output": "y" }
                ]
            }),
            &fields(&["amount", "level"]),
            &[],
            false,
        );
        assert!(errors.is_empty());
        let usage = dmn_variable_usage(&dmn.unwrap());
        assert_eq!(usage, fields(&["amount", "level"]));
    }

    #[test]
    fn inputs_accepts_plain_string_names() {
        let raw = json!({
            "hitPolicy": "FIRST",
            "inputs": ["amount"],
            "rules": [{ "match": ["amount > 1"], "output": "x" }]
        });
        let (dmn, errors) = normalize_and_validate_dmn(&raw, &fields(&["amount"]), &[], false);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(dmn.unwrap()["inputs"][0]["name"], json!("amount"));
    }
}
