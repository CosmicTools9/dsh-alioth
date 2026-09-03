//! LLM Provider-specific backends.
//!
//! Each Provider (DeepSeek, Kimi, MiniMax) implements its own HTTP client that
//! calls the Provider's native API directly. There is no shared "OpenAI
//! compatible" layer; each backend handles its own request/response format,
//! authentication header, error mapping, and rate-limit reporting.

pub mod deepseek;
pub mod glm;
pub mod kimi;
pub mod minimax;
pub mod sse;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::types::{LlmProvider, ReasoningEffort, TokenUsage, ToolDefinition};

/// Provider-agnostic completion request.
///
/// Each backend maps this struct to its own request body via `serialize_body`.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    /// Model identifier as the backend should pass to its API.
    /// Usually the user-selected model (e.g. `deepseek-v4-pro`).
    pub model: String,
    /// System / developer prompt. Backend may map to its native key
    /// (`system` for OpenAI-compatible, `preamble` for rig, etc.).
    pub system: Option<String>,
    /// User prompt.
    pub prompt: String,
    /// Prior conversation turns as (role, content) pairs ("assistant" | "user"),
    /// inserted between system and the current user prompt. Empty = single-turn
    /// (existing behavior). Prefix-cache friendly: backends MUST append-only.
    pub history: Vec<(String, String)>,
    pub tools: Vec<ToolDefinition>,
    /// Sampling temperature.
    pub temperature: f64,
    /// Maximum output tokens.
    pub max_tokens: u64,
    /// Top-p nucleus sampling.
    pub top_p: f64,
    /// Frequency penalty.
    pub frequency_penalty: f64,
    /// Presence penalty.
    pub presence_penalty: f64,
    /// Reasoning effort hint (semantic level). Backend may translate to its
    /// native format (DeepSeek accepts `low|medium|high`; others may use
    /// thinking variants or different keys).
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Response format hint. Backend may translate to its native type
    /// (`json_object`, `{"type":"json_schema",...}`).
    pub response_format: Option<String>,
    /// 思考模式控制（DeepSeek enabled|disabled、MiniMax-M3 adaptive|disabled、
    /// Kimi K2.7 enabled）。None = 不显式发送。
    pub thinking: Option<String>,
    /// MiniMax 专属:请求准入服务层级 standard|priority。
    pub service_tier: Option<String>,
    /// MiniMax-M3 专属:输出拆分开关（thinking 拆分到 reasoning_content）。
    pub reasoning_split: Option<bool>,
    /// 图像输入（多模态）。非空时 backend 将用户消息编码为
    /// OpenAI 兼容 content 数组（text + image_url data URL）；空时保持纯文本。
    pub images: Vec<ImageContent>,
}

/// 图像内容（原始字节；backend 侧按 mime 做 base64 data URL 编码）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageContent {
    /// MIME 类型，如 `image/png` / `image/jpeg`
    pub mime: String,
    /// 图像原始字节
    pub data: Vec<u8>,
}

/// Provider-agnostic completion response.
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    /// Plain text content (if no tool calls).
    pub text: Option<String>,
    /// Tool/function calls requested by the model.
    pub tool_calls: Vec<ToolCallResult>,
    /// token 用量（provider 返回；缺失或解析失败时为 None，不得报错）
    pub usage: Option<TokenUsage>,
    /// Raw provider response (for logging / debugging).
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Rate-limit information reported by the backend (parsed from response headers).
#[derive(Debug, Clone, Default)]
pub struct RateLimitInfo {
    pub remaining_requests: Option<u64>,
    pub remaining_tokens: Option<u64>,
    pub reset_seconds: Option<u64>,
}

/// 消息内容：纯文本或 OpenAI 兼容多部分（文本 + 图像）。
/// `Text` 序列化为字符串（与既有请求逐字节一致）；`Parts` 序列化为 content 数组。
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum MessageContent<'a> {
    Text(&'a str),
    Parts(Vec<ContentPart<'a>>),
}

/// OpenAI 兼容 content part（`type` 标签区分）
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart<'a> {
    Text { text: &'a str },
    ImageUrl { image_url: ImageUrlPart<'a> },
}

/// `image_url` part：data URL（`data:{mime};base64,{b64}`）或 http(s) URL
#[derive(Debug, Serialize)]
pub struct ImageUrlPart<'a> {
    pub url: std::borrow::Cow<'a, str>,
}

/// 图像字节 → data URL（base64）
pub(crate) fn image_data_url(mime: &str, data: &[u8]) -> String {
    use base64::Engine as _;
    format!(
        "data:{};base64,{}",
        mime,
        base64::engine::general_purpose::STANDARD.encode(data)
    )
}

/// 按图像构造用户消息 content：无图像 → 纯文本（逐字节兼容）；有图像 → content 数组
pub(crate) fn build_user_content<'a>(
    prompt: &'a str,
    images: &'a [ImageContent],
) -> MessageContent<'a> {
    if images.is_empty() {
        return MessageContent::Text(prompt);
    }
    let mut parts = vec![ContentPart::Text { text: prompt }];
    for img in images {
        parts.push(ContentPart::ImageUrl {
            image_url: ImageUrlPart {
                url: std::borrow::Cow::Owned(image_data_url(&img.mime, &img.data)),
            },
        });
    }
    MessageContent::Parts(parts)
}

/// Common error type for all backends.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("HTTP transport error: {0}")]
    Transport(String),
    #[error("Request timeout after {0}s")]
    Timeout(u64),
    #[error("Provider returned status {status}: {body}")]
    ProviderStatus { status: u16, body: String },
    #[error("Failed to parse response: {0}")]
    Parse(String),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Authentication error: {0}")]
    Auth(String),
    #[error("Rate limit exceeded; reset in {0}s")]
    RateLimit(u64),
    /// 流式生成正常结束但从未产出任何非空 content（finish_reason 随错误携带，
    /// `length` = 思考耗尽 max_tokens 预算，其余 = 模型未输出正文）。
    #[error("Generation produced no content (finish_reason={0})")]
    NoContent(String),
}

/// 从 OpenAI 兼容的 usage 字段构造 TokenUsage。
/// prompt_tokens / completion_tokens 任一缺失时返回 None——不把缺失字段伪装成 0。
pub(crate) fn map_usage(
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
) -> Option<TokenUsage> {
    match (prompt_tokens, completion_tokens) {
        (Some(input_tokens), Some(output_tokens)) => Some(TokenUsage {
            input_tokens,
            output_tokens,
        }),
        _ => None,
    }
}

/// Provider-specific HTTP backend.
#[async_trait]
pub trait LlmBackend: Send + Sync {
    /// Provider name (matches `LlmProvider::as_str()`).
    fn provider_name(&self) -> &'static str;

    /// Send a completion request to the provider.
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, BackendError>;

    /// Stream a completion response chunk-by-chunk.
    ///
    /// 默认实现：调用 `complete` 后单 chunk 发完整文本（非流式后端零改动）。
    /// 支持 SSE 的后端（如 DeepSeek `stream: true`）应覆写为真流式。
    /// 每个 `Ok(String)` 为一个增量文本 chunk；流结束时正常结束。
    fn complete_stream(
        &self,
        req: CompletionRequest,
    ) -> futures_util::stream::BoxStream<'_, Result<String, BackendError>> {
        let backend = self;
        let fut = async move {
            match backend.complete(req).await {
                Ok(resp) => {
                    let text = resp.text.unwrap_or_default();
                    if text.is_empty() {
                        futures_util::stream::empty().boxed()
                    } else {
                        futures_util::stream::once(async move { Ok(text) }).boxed()
                    }
                }
                Err(e) => futures_util::stream::once(async move { Err(e) }).boxed(),
            }
        };
        // 将 async block 的流展平为 BoxStream
        futures_util::stream::once(async move {
            // future 返回流，此处需要间接——用 async_stream 语义：直接返回 future 产出的流
            let s = fut.await;
            s
        })
        .flatten()
        .boxed()
    }

    /// Probe with a tiny request to verify connectivity (e.g. on startup).
    /// 默认 top_p=1.0；kimi 会覆盖为 0.95（其 API 仅接受 0.95，否则 verify 误报 400）。
    async fn verify(&self) -> Result<(), BackendError> {
        // Default: send a 1-token "ping" request.
        let req = CompletionRequest {
            model: self.default_model().to_string(),
            system: None,
            prompt: "ping".to_string(),
            tools: vec![],
            history: vec![],
            temperature: 0.0,
            max_tokens: 1,
            top_p: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            images: vec![],
            reasoning_effort: None,
            response_format: None,
            thinking: None,
            service_tier: None,
            reasoning_split: None,
        };
        self.complete(req).await.map(|_| ())
    }

    /// Default model name to use for verify() and as a fallback.
    fn default_model(&self) -> &str;
}

/// Build the appropriate backend for the given provider + config.
pub fn build_backend(
    provider: LlmProvider,
    api_key: String,
    base_url: Option<String>,
    default_model: String,
    #[allow(dead_code)] flash_model: String,
    timeout_seconds: u64,
) -> Result<Box<dyn LlmBackend>, BackendError> {
    let backend: Box<dyn LlmBackend> = match provider {
        LlmProvider::DeepSeek => Box::new(deepseek::DeepSeekBackend::new(
            api_key,
            base_url,
            default_model,
            flash_model,
            timeout_seconds,
        )?),
        LlmProvider::Kimi => Box::new(kimi::KimiBackend::new(
            api_key,
            base_url,
            default_model,
            flash_model,
            timeout_seconds,
        )?),
        LlmProvider::MiniMax => Box::new(minimax::MiniMaxBackend::new(
            api_key,
            base_url,
            default_model,
            flash_model,
            timeout_seconds,
        )?),
        LlmProvider::Glm => Box::new(glm::GlmBackend::new(
            api_key,
            base_url,
            default_model,
            flash_model,
            timeout_seconds,
        )?),
    };
    Ok(backend)
}

#[cfg(test)]
mod usage_tests {
    use super::map_usage;

    #[test]
    fn test_map_usage_both_present() {
        let u = map_usage(Some(100), Some(50)).unwrap();
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.output_tokens, 50);
    }

    #[test]
    fn test_map_usage_missing_field_degrades_to_none() {
        assert!(map_usage(None, Some(50)).is_none());
        assert!(map_usage(Some(100), None).is_none());
        assert!(map_usage(None, None).is_none());
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;

    #[test]
    fn empty_images_serialize_as_plain_string() {
        let c = build_user_content("hello", &[]);
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, r#""hello""#);
    }

    #[test]
    fn images_serialize_as_content_array_with_data_url() {
        let img = ImageContent {
            mime: "image/png".to_string(),
            data: vec![1u8, 2, 3],
        };
        let imgs = [img];
        let c = build_user_content("describe", &imgs);
        let json: serde_json::Value = serde_json::to_value(&c).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "describe");
        assert_eq!(arr[1]["type"], "image_url");
        let url = arr[1]["image_url"]["url"].as_str().unwrap();
        assert_eq!(url, "data:image/png;base64,AQID");
    }

    #[test]
    fn data_url_encoding_matches_base64() {
        let url = image_data_url("image/jpeg", b"\xff\xd8");
        assert_eq!(url, "data:image/jpeg;base64,/9g=");
    }
}
