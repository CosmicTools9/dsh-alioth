//! Assistant turn seam（consolidate-ai-chat-chain D3 / W3）
//!
//! Gateway 的 ad-hoc LLM 端点（公式生成/修复、NGAC 提案、知识扩展、单据识别）
//! 经本模块完成一次 turn：装配（`load_llm`，fail-closed）、prompt 分段组装
//! （`PromptSection` → `assemble_prompt`）、输出契约解析（`OutputContract::Text | JsonObject`）
//! 与错误分类（`TurnError`）。
//!
//! 纪律：
//! - prompt 文本逐字保留在调用点（本模块只做确定性拼接，不改写文本）。
//! - `load_llm` MUST NOT 自行读表/解密——LLM 装配单实现在 `approval::llm_assist`
//!   （经 `DbLlmConfigAdapter` 的 `LlmConfigPort`）。
//! - `JsonObject` 契约不合规 = 可分类错误（`NotJson` / `InvalidJson`）；
//!   MUST NOT panic、MUST NOT 静默返回原文当结构化结果。

use llm::{LlmService, TokenUsage};
use sqlx::PgPool;

use super::adapters::db_llm_config::DbLlmConfigAdapter;
use super::ports::LlmConfigPort;

/// prompt 分段：`heading` 为可选的静态标题（非空时单独成行），`body` 为段正文（逐字）。
/// 组装规则见 `assemble_prompt`——调用点产出的 prompt MUST 与改造前逐字一致。
pub struct PromptSection {
    pub heading: &'static str,
    pub body: String,
}

/// 输出契约：`Text` 原样返回；`JsonObject` 要求输出含 JSON 对象（容忍围栏/前后噪声）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputContract {
    Text,
    JsonObject,
}

/// 单次 turn 的声明式规格（场景只声明差异，组装归 seam）。
pub struct TurnSpec {
    /// system message（各端点既有 SYSTEM_PROMPT 常量，逐字保留）。
    pub system_preamble: &'static str,
    pub sections: Vec<PromptSection>,
    pub output: OutputContract,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

/// turn 结果：`text` 恒为模型原始文本；`json` 仅 `OutputContract::JsonObject` 时非空。
pub struct TurnOutcome {
    pub text: String,
    pub json: Option<serde_json::Value>,
    pub usage: Option<TokenUsage>,
}

/// turn 错误（可分类；端点据此映射状态码——装配失败 fail-closed）。
#[derive(Debug)]
pub enum TurnError {
    /// LLM 装配失败（配置缺失/DB 故障等）⇒ 端点 503 fail-closed。
    /// 文本已剥离内部哨兵，可直接进日志/用户可见响应。
    Config(String),
    /// LLM 调用失败（上游/超时/协议）⇒ 端点 502。
    Call(String),
    /// `OutputContract::JsonObject`：输出中提取不到 JSON 对象 ⇒ 端点 502。
    NotJson { raw_len: usize },
    /// `OutputContract::JsonObject`：提取到的 JSON 非法 ⇒ 端点 502。
    InvalidJson(String),
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(msg) => write!(f, "{msg}"),
            Self::Call(msg) => write!(f, "{msg}"),
            Self::NotJson { raw_len } => {
                write!(f, "LLM output is not a JSON object (len={raw_len})")
            }
            Self::InvalidJson(msg) => write!(f, "LLM output JSON invalid: {msg}"),
        }
    }
}

/// fail-closed 装配入口：委托既有 `LlmConfigPort`（`DbLlmConfigAdapter` →
/// `approval::llm_assist` 单实现），未配置哨兵在 seam 内剥离（用户可见文本 MUST NOT 含哨兵）。
pub async fn load_llm(pool: &PgPool) -> Result<LlmService, TurnError> {
    DbLlmConfigAdapter::new(pool.clone())
        .load_service()
        .await
        .map_err(|e| TurnError::Config(approval::llm_assist::visible_message(&e).to_string()))
}

/// 执行一次 assistant turn：组装 prompt → 调用 → 按契约解析。
pub async fn run_turn(llm: &LlmService, spec: TurnSpec) -> Result<TurnOutcome, TurnError> {
    let prompt = assemble_prompt(&spec.sections);
    let (text, usage) = llm
        .generate_detailed(
            spec.system_preamble,
            &prompt,
            spec.temperature.map(f64::from),
            spec.max_tokens.map(u64::from),
            None,
            None,
            None,
        )
        .await
        .map_err(|e| TurnError::Call(e.to_string()))?;
    let json = apply_contract(spec.output, &text)?;
    Ok(TurnOutcome { text, json, usage })
}

/// 分段组装（确定性）：段间以单个换行分隔；`heading` 非空时成行（后随换行）再续 `body`。
fn assemble_prompt(sections: &[PromptSection]) -> String {
    let mut prompt = String::new();
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            prompt.push('\n');
        }
        if !section.heading.is_empty() {
            prompt.push_str(section.heading);
            prompt.push('\n');
        }
        prompt.push_str(&section.body);
    }
    prompt
}

/// 输出契约解析（`Text` 恒通过；`JsonObject` 提取并解析，失败可分类）。
fn apply_contract(
    output: OutputContract,
    text: &str,
) -> Result<Option<serde_json::Value>, TurnError> {
    match output {
        OutputContract::Text => Ok(None),
        OutputContract::JsonObject => {
            let json_text = extract_json(text).ok_or(TurnError::NotJson {
                raw_len: text.len(),
            })?;
            let value = serde_json::from_str(json_text)
                .map_err(|e| TurnError::InvalidJson(e.to_string()))?;
            Ok(Some(value))
        }
    }
}

/// 从 LLM 输出提取 JSON（容忍围栏/前后噪声）。
///
/// 单实现在 `approval::llm_assist`（`dmn_assist` 消费同一实现）；本模块仅再导出，
/// MUST NOT 复制实现。
pub use approval::llm_assist::extract_json;

#[cfg(test)]
mod tests {
    use super::*;

    fn section(heading: &'static str, body: &str) -> PromptSection {
        PromptSection {
            heading,
            body: body.to_string(),
        }
    }

    /// 分段组装逐字性：与 nl-assist 现行拼接（标题行 + 段正文 + 空行分隔）逐字一致。
    #[test]
    fn assemble_prompt_matches_legacy_concatenation() {
        let sections = vec![
            section("== 标题一 ==", "snapshot\n"),
            section("== 标题二 ==", "管理员: 你好\n"),
            section("== 标题三 ==", "诉求"),
            section("", "\n请输出 JSON 提案。\n"),
        ];
        assert_eq!(
            assemble_prompt(&sections),
            "== 标题一 ==\nsnapshot\n\n== 标题二 ==\n管理员: 你好\n\n== 标题三 ==\n诉求\n\n请输出 JSON 提案。\n"
        );
    }

    /// 单段（无标题）原样透传：approval_formula 的 prompt 形态。
    #[test]
    fn assemble_prompt_single_body_is_verbatim() {
        let sections = vec![section("", "可用变量（字段）：a, b\n请生成表达式。")];
        assert_eq!(
            assemble_prompt(&sections),
            "可用变量（字段）：a, b\n请生成表达式。"
        );
    }

    /// 契约：Text 不解析；JsonObject 容忍围栏；缺 JSON / 非法 JSON 可分类。
    #[test]
    fn contract_text_and_json_object() {
        assert_eq!(
            apply_contract(OutputContract::Text, "任意文本").unwrap(),
            None
        );
        let json = apply_contract(
            OutputContract::JsonObject,
            "```json\n{\"expression\":\"a > 1\"}\n```",
        )
        .unwrap()
        .expect("JsonObject 契约必须产出 json");
        assert_eq!(json["expression"], "a > 1");
        assert!(matches!(
            apply_contract(OutputContract::JsonObject, "完全没有 JSON"),
            Err(TurnError::NotJson { .. })
        ));
        assert!(matches!(
            apply_contract(OutputContract::JsonObject, "{非法}"),
            Err(TurnError::InvalidJson(_))
        ));
    }

    /// extract_json（自 approval_formula / admin_ngac_assist 迁入）断言逐条保留。
    #[test]
    fn extract_json_strips_fences_and_noise() {
        assert_eq!(extract_json("```json\n{\"a\":1}\n```"), Some("{\"a\":1}"));
        assert_eq!(
            extract_json("前置噪声 {\"a\":1} 尾部噪声"),
            Some("{\"a\":1}")
        );
        assert_eq!(extract_json("完全没有大括号"), None);
        assert_eq!(extract_json(""), None);
    }

    /// 错误分类文案可读（装配/调用/契约；json 长度随原文字节数）。
    #[test]
    fn turn_error_display_is_classifiable() {
        assert_eq!(TurnError::Config("no key".into()).to_string(), "no key");
        assert_eq!(TurnError::Call("boom".into()).to_string(), "boom");
        assert_eq!(
            TurnError::NotJson { raw_len: 3 }.to_string(),
            "LLM output is not a JSON object (len=3)"
        );
        assert_eq!(
            TurnError::InvalidJson("bad".into()).to_string(),
            "LLM output JSON invalid: bad"
        );
    }
}
