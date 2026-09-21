//! doc-recognition — 文档识别/转换/类型提取的 **Framework 基础能力**（用户裁决 2026-09-16）。
//!
//! 定位（应用无关，三处同源消费）：
//! - **Gateway** 挂 `/api/service/doc-recognition/*` 端点（`DbLlmConfigAdapter` 解析
//!   `LlmService` 后注入本 crate）——OA 等外部门户经 `X-Service-Key` 服务间转调，
//!   **门户自身不接大模型**；
//! - **Meta**（多模态场景）可直接进程内消费——本 crate 收 `&LlmService`，
//!   配置源由调用方解析（env / DB / 其它），crate 不读环境不查库（LLM 调用面之外零依赖）；
//! - 各 ns Service（如 contract 的 PDF 导入）后续可迁移至本 crate 收口（复用优先）。
//!
//! 能力面：
//! - 转换：[`extract_pdf_text`]（PDF → 文本层；扫描件无文本层显式报错，不静默降级）；
//! - 识别/类型提取：[`structure_certificate`]（证照制式分析——LLM 结构化 + 类别白名单校验）；
//! - 多模态扩展位：`llm` crate 已有 `ImageContent`；图片扫描件识别沿用同一
//!   `LlmService` 注入形态扩展（本 crate 不预先实现，Meta 场景落地时增补）。

use llm::LlmService;
use serde::{Deserialize, Serialize};

/// 文本上限（超长截断并标记——识别质量与 token 成本平衡，同 contract 导入先例）。
pub const MAX_TEXT_CHARS: usize = 12_000;

/// 识别失败（fail-visible：调用方按变体映射 4xx/5xx，不静默降级）。
#[derive(Debug, thiserror::Error)]
pub enum RecognitionError {
    /// PDF 解析失败 / 扫描件无文本层（调用方映射 400）
    #[error("{0}")]
    NoTextLayer(String),
    /// LLM 调用/输出失败（调用方映射 502/503，携带可操作信息）
    #[error("{0}")]
    Llm(String),
}

/// 证照制式分析草稿（识别结果；字段缺失 = null，禁止编造）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CertDraft {
    /// 类别 code（调用方白名单内；未命中为 null——用户手选兜底）
    #[serde(default)]
    pub cert_type: Option<String>,
    /// LLM 原始识别类别（白名单外时保留供前端提示）
    #[serde(default)]
    pub raw_type: Option<String>,
    #[serde(default)]
    pub cert_no: Option<String>,
    /// YYYY-MM-DD
    #[serde(default)]
    pub valid_until: Option<String>,
    /// YYYY-MM-DD
    #[serde(default)]
    pub issued_at: Option<String>,
    #[serde(default)]
    pub issuer: Option<String>,
    /// 0.0–1.0（低置信度 → 前端要求人工核对）
    #[serde(default)]
    pub confidence: Option<f64>,
}

/// 转换：提取 PDF 文本层；扫描件/无文本层 → [`RecognitionError::NoTextLayer`]；
/// 超长截断（返回 `(文本, 是否截断)`）。
pub fn extract_pdf_text(bytes: &[u8]) -> Result<(String, bool), RecognitionError> {
    let raw = pdf_extract::extract_text_from_mem(bytes)
        .map_err(|e| RecognitionError::NoTextLayer(format!("PDF 解析失败（{e}）")))?;
    let text = raw.trim();
    if text.is_empty() {
        return Err(RecognitionError::NoTextLayer(
            "扫描版 PDF 无文本层，无法自动识别；请使用文本版 PDF 或手工录入".into(),
        ));
    }
    let truncated = text.chars().count() > MAX_TEXT_CHARS;
    let clipped: String = text.chars().take(MAX_TEXT_CHARS).collect();
    Ok((clipped, truncated))
}

const CERT_SYSTEM_PROMPT: &str = "你是企业资质证照制式识别引擎。将用户提供的证书文本整理为 JSON 输出，\
仅输出 JSON，不要任何额外文字或解释。\
输出结构：{\"cert_type\": 证书类别代码（必须从用户给出的可选类别代码中选择；无法判断则为 null）, \
\"cert_no\": 证书编号, \"valid_until\": 有效期至（YYYY-MM-DD）, \"issued_at\": 发证日期（YYYY-MM-DD）, \
\"issuer\": 发证机关全称, \"confidence\": 识别置信度（0 到 1 的数字）}。\
文本中未出现的字段输出 null。禁止编造文本中不存在的信息。";

/// LLM 输出解析：直接 JSON；失败则剥离 ```json 围栏重试（字符串处理，非正则解析）。
pub fn parse_llm_json(raw: &str) -> Result<CertDraft, String> {
    let trimmed = raw.trim();
    if let Ok(out) = serde_json::from_str::<CertDraft>(trimmed) {
        return Ok(out);
    }
    let candidate = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    serde_json::from_str::<CertDraft>(candidate).map_err(|e| format!("LLM 输出非 JSON: {e}"))
}

/// 白名单校验：命中 → `cert_type` 归一为白名单 code（大小写不敏感）；
/// 未命中 → `cert_type` 置 null、原值保留 `raw_type`（调用方提示用户手选，不拒绝整单）。
pub fn validate_against_whitelist(
    mut draft: CertDraft,
    whitelist: &[(String, String)],
) -> CertDraft {
    let matched = draft
        .cert_type
        .as_deref()
        .and_then(|t| {
            whitelist
                .iter()
                .find(|(code, _)| code.eq_ignore_ascii_case(t))
        })
        .map(|(code, _)| code.clone());
    match matched {
        Some(code) => {
            draft.raw_type = None;
            draft.cert_type = Some(code);
        }
        None => {
            draft.raw_type = draft.cert_type.take();
        }
    }
    draft
}

/// 识别 + 类型提取：证照制式分析（LLM 结构化 + 重试 1 次；`json_object` 模式）。
/// `whitelist` = 类别 code/名称活动行（调用方从字典表加载——企业类目落库后自动纳入）。
/// `LlmService` 由调用方注入（Gateway = DbLlmConfigAdapter；Meta = 自有配置源）。
pub async fn structure_certificate(
    service: &LlmService,
    text: &str,
    whitelist: &[(String, String)],
) -> Result<CertDraft, RecognitionError> {
    let options = whitelist
        .iter()
        .map(|(code, name)| format!("{code}={name}"))
        .collect::<Vec<_>>()
        .join("; ");
    let prompt = format!("可选类别代码：{options}\n\n请识别以下证书文本：\n\n{text}");
    let mut last_err: Option<String> = None;
    for _ in 0..2 {
        match service
            .generate_with_system_preamble_for_role(
                llm::ModelRole::Task,
                Some(CERT_SYSTEM_PROMPT),
                &prompt,
                None,
                None,
                None,
                Some("json_object"),
            )
            .await
        {
            Ok(raw) => match parse_llm_json(&raw) {
                Ok(out) => return Ok(validate_against_whitelist(out, whitelist)),
                Err(e) => last_err = Some(e),
            },
            Err(e) => last_err = Some(e.to_string()),
        }
    }
    Err(RecognitionError::Llm(format!(
        "证书识别失败: {}",
        last_err.unwrap_or_default()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whitelist() -> Vec<(String, String)> {
        vec![
            ("BUSINESS_LICENSE".into(), "营业执照".into()),
            ("ROAD_TRANSPORT_LICENSE".into(), "道路运输经营许可证".into()),
        ]
    }

    #[test]
    fn parse_direct_and_fenced_json() {
        let direct = r#"{"cert_type":"BUSINESS_LICENSE","cert_no":"9133","issuer":"市监局"}"#;
        let draft = parse_llm_json(direct).expect("直接 JSON");
        assert_eq!(draft.cert_type.as_deref(), Some("BUSINESS_LICENSE"));
        assert_eq!(draft.cert_no.as_deref(), Some("9133"));

        let fenced = "```json\n{\"cert_no\":\"X\"}\n```";
        let draft = parse_llm_json(fenced).expect("围栏剥离");
        assert_eq!(draft.cert_no.as_deref(), Some("X"));
        assert_eq!(draft.cert_type, None);

        assert!(parse_llm_json("不是 JSON").is_err());
    }

    #[test]
    fn whitelist_validation_hit_miss_and_case() {
        let wl = whitelist();
        let hit = validate_against_whitelist(
            CertDraft {
                cert_type: Some("business_license".into()),
                ..Default::default()
            },
            &wl,
        );
        assert_eq!(hit.cert_type.as_deref(), Some("BUSINESS_LICENSE"));
        assert_eq!(hit.raw_type, None, "命中后原值清空");

        let miss = validate_against_whitelist(
            CertDraft {
                cert_type: Some("SOMETHING_ELSE".into()),
                ..Default::default()
            },
            &wl,
        );
        assert_eq!(miss.cert_type, None, "未命中置 null");
        assert_eq!(
            miss.raw_type.as_deref(),
            Some("SOMETHING_ELSE"),
            "原识别值保留供提示"
        );

        let none = validate_against_whitelist(CertDraft::default(), &wl);
        assert_eq!(none.cert_type, None);
        assert_eq!(none.raw_type, None);
    }

    #[test]
    fn extract_rejects_garbage_bytes() {
        let err = extract_pdf_text(b"not a pdf").unwrap_err();
        assert!(matches!(err, RecognitionError::NoTextLayer(_)));
    }
}
