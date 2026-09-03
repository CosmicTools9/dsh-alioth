//! DeepSeek provider backend.
//!
//! DeepSeek v4 (2026) offers:
//! - Chat completions: `POST {base_url}/v1/chat/completions`
//! - Bearer token authentication
//! - JSON request/response (OpenAI-compatible schema, but field semantics
//!   differ in `reasoning_content` and `reasoning_effort`).
//! - Streaming via SSE (`stream: true`).

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use super::{BackendError, CompletionRequest, CompletionResponse, LlmBackend, ToolCallResult};

const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
const DEFAULT_MODEL: &str = "deepseek-v4-pro";
const DEFAULT_FLASH_MODEL: &str = "deepseek-v4-flash";

/// DeepSeek-specific request body.
///
/// Notable differences from generic OpenAI:
/// - `reasoning_effort` accepts only `low|medium|high` (no `minimal`/`xhigh`).
/// - `response_format` uses `{"type": "json_object"}` (DeepSeek flavor).
/// - `tool_choice` is `auto` by default.
#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct DeepSeekRequest<'a> {
    model: &'a str,
    messages: Vec<DeepSeekMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<DeepSeekTool<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a str>,
    temperature: f64,
    max_tokens: u64,
    top_p: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<DeepSeekResponseFormat<'a>>,
    /// thinking 开关:{"type":"enabled"|"disabled"}。None = 不发送(默认 enabled)
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<DeepSeekThinking<'a>>,
    #[serde(default)]
    stream: bool,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct DeepSeekMessage<'a> {
    role: &'a str,
    content: super::MessageContent<'a>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct DeepSeekTool<'a> {
    #[serde(rename = "type")]
    tool_type: &'static str,
    function: DeepSeekFunction<'a>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct DeepSeekFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct DeepSeekResponseFormat<'a> {
    #[serde(rename = "type")]
    format_type: &'a str,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct DeepSeekThinking<'a> {
    #[serde(rename = "type")]
    thinking_type: &'a str,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DeepSeekResponse {
    id: Option<String>,
    model: Option<String>,
    choices: Vec<DeepSeekChoice>,
    usage: Option<DeepSeekUsage>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DeepSeekChoice {
    index: u32,
    message: DeepSeekAssistantMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DeepSeekAssistantMessage {
    role: String,
    /// Main answer content.
    content: Option<String>,
    /// DeepSeek-specific reasoning trace (chain-of-thought).
    reasoning_content: Option<String>,
    /// Tool calls requested by the model.
    tool_calls: Option<Vec<DeepSeekToolCall>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DeepSeekToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: DeepSeekFunctionCall,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DeepSeekFunctionCall {
    name: String,
    /// DeepSeek returns the arguments as a JSON-encoded string.
    arguments: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DeepSeekUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

/// DeepSeek error response body.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DeepSeekErrorBody {
    error: Option<DeepSeekErrorDetail>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DeepSeekErrorDetail {
    message: String,
    #[serde(rename = "type")]
    err_type: Option<String>,
    code: Option<String>,
}

pub struct DeepSeekBackend {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model: String,
    #[allow(dead_code)]
    flash_model: String,
    timeout_seconds: u64,
}

impl DeepSeekBackend {
    pub fn new(
        api_key: String,
        base_url: Option<String>,
        default_model: String,
        flash_model: String,
        timeout_seconds: u64,
    ) -> Result<Self, BackendError> {
        if api_key.is_empty() {
            return Err(BackendError::Auth("DeepSeek API key is empty".to_string()));
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(timeout_seconds))
                // 防御性优化：reqwest 的 .timeout() 已是总请求超时（含 connect/首字节/
                // 响应体）；connect_timeout 仅额外限制 TCP 连接建立阶段的等待上限，
                // 避免个别网络环境下 connect 阶段异常拖长。非根因修复——AppAgent
                // 观测到的 15 分钟挂起来自外层 call_with_retry 900s 预算 + 300s×重试链。
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .map_err(|e| BackendError::Config(e.to_string()))?,
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            api_key,
            default_model,
            flash_model,
            timeout_seconds,
        })
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        )
    }

    fn build_headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, "application/json".parse().unwrap());
        h.insert(
            AUTHORIZATION,
            format!("Bearer {}", self.api_key).parse().unwrap(),
        );
        // DeepSeek-specific user agent helps with API tier classification.
        h.insert("User-Agent", "AliothStudio/0.1 (deepseek)".parse().unwrap());
        h
    }

    fn build_body<'a>(&'a self, req: &'a CompletionRequest, stream: bool) -> DeepSeekRequest<'a> {
        let mut messages = Vec::new();
        if let Some(sys) = &req.system {
            if !sys.is_empty() {
                messages.push(DeepSeekMessage {
                    role: "system",
                    content: super::MessageContent::Text(sys.as_str()),
                });
            }
        }
        for (role, content) in &req.history {
            messages.push(DeepSeekMessage {
                role: role.as_str(),
                content: super::MessageContent::Text(content.as_str()),
            });
        }
        messages.push(DeepSeekMessage {
            role: "user",
            content: super::build_user_content(req.prompt.as_str(), &req.images),
        });

        let tools: Option<Vec<DeepSeekTool>> = if req.tools.is_empty() {
            None
        } else {
            Some(
                req.tools
                    .iter()
                    .map(|t| DeepSeekTool {
                        tool_type: "function",
                        function: DeepSeekFunction {
                            name: t.name.as_str(),
                            description: t.description.as_str(),
                            parameters: &t.parameters,
                        },
                    })
                    .collect(),
            )
        };

        // DeepSeek v4 官方档位 low/high/max（官方映射表：medium/xhigh→high，max 独立）
        // 2026-08-22 实测：`reasoning_effort=high` + `tools` + 长 prompt → API 返回
        // 200 但 content/reasoning_content/tool_calls 全空（会话 1/2 的 Empty response
        // 根因；无 reasoning 时同请求正常返回）。工具调用任务不需要深度推理——
        // tools 存在时强制不发送 reasoning_effort（走 API 默认），保工具响应。
        let reasoning_effort: Option<&'static str> = if req.tools.is_empty() {
            match req.reasoning_effort {
                Some(crate::types::ReasoningEffort::Minimal) => Some("low"),
                Some(crate::types::ReasoningEffort::Low) => Some("low"),
                Some(crate::types::ReasoningEffort::Medium) => None,
                Some(crate::types::ReasoningEffort::High) => Some("high"),
                Some(crate::types::ReasoningEffort::XHigh) => Some("high"),
                Some(crate::types::ReasoningEffort::Max) => Some("max"),
                None => None,
            }
        } else {
            None
        };

        let response_format: Option<DeepSeekResponseFormat> = req
            .response_format
            .as_deref()
            .map(|t| DeepSeekResponseFormat { format_type: t });

        // DeepSeek v4 thinking 开关（enabled/disabled；None = 默认 enabled 不发送）
        let thinking = req
            .thinking
            .as_deref()
            .filter(|t| *t == "disabled" || *t == "enabled")
            .map(|t| DeepSeekThinking { thinking_type: t });

        DeepSeekRequest {
            model: req.model.as_str(),
            messages,
            tools,
            tool_choice: if req.tools.is_empty() {
                None
            } else {
                Some("auto")
            },
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            top_p: req.top_p,
            frequency_penalty: if req.frequency_penalty == 0.0 {
                None
            } else {
                Some(req.frequency_penalty)
            },
            presence_penalty: if req.presence_penalty == 0.0 {
                None
            } else {
                Some(req.presence_penalty)
            },
            reasoning_effort,
            response_format,
            thinking,
            stream,
        }
    }

    fn parse_error(status: u16, body: &str) -> BackendError {
        // Try to parse DeepSeek's error envelope.
        if let Ok(parsed) = serde_json::from_str::<DeepSeekErrorBody>(body) {
            if let Some(detail) = parsed.error {
                return match status {
                    401 | 403 => BackendError::Auth(detail.message),
                    429 => BackendError::RateLimit(60),
                    _ => BackendError::ProviderStatus {
                        status,
                        body: detail.message,
                    },
                };
            }
        }
        // 429 with no JSON body still indicates rate limit.
        if status == 429 {
            return BackendError::RateLimit(60);
        }
        if status == 401 || status == 403 {
            return BackendError::Auth(body.to_string());
        }
        BackendError::ProviderStatus {
            status,
            body: body.to_string(),
        }
    }
}

#[async_trait]
impl LlmBackend for DeepSeekBackend {
    fn provider_name(&self) -> &'static str {
        "deepseek"
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, BackendError> {
        let url = self.endpoint();
        let body = self.build_body(&req, false);
        let headers = self.build_headers();

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    BackendError::Timeout(self.timeout_seconds)
                } else {
                    BackendError::Transport(e.to_string())
                }
            })?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Self::parse_error(status, &body));
        }

        let raw: serde_json::Value = response
            .json()
            .await
            .map_err(|e| BackendError::Parse(format!("response body: {}", e)))?;

        let parsed: DeepSeekResponse = serde_json::from_value(raw.clone())
            .map_err(|e| BackendError::Parse(format!("DeepSeek envelope: {}", e)))?;

        let choice = parsed
            .choices
            .first()
            .ok_or_else(|| BackendError::Parse("no choices in response".to_string()))?;

        let text = choice.message.content.clone().unwrap_or_default();
        // DeepSeek v4 may include reasoning_content. It is NOT prepended to the
        // returned text: mixed reasoning + JSON breaks callers that parse the
        // payload directly (Planning first-try failures, judge score scans).
        // Reasoning stays observable via `raw` on CompletionResponse.
        let _reasoning = choice.message.reasoning_content.as_deref();

        let tool_calls: Vec<ToolCallResult> = choice
            .message
            .tool_calls
            .as_ref()
            .map(|tcs| {
                tcs.iter()
                    .map(|tc| ToolCallResult {
                        id: tc.id.clone(),
                        name: tc.function.name.clone(),
                        arguments: serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(serde_json::Value::String(tc.function.arguments.clone())),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let usage = parsed
            .usage
            .as_ref()
            .and_then(|u| super::map_usage(u.prompt_tokens, u.completion_tokens));

        Ok(CompletionResponse {
            text: if text.is_empty() { None } else { Some(text) },
            tool_calls,
            usage,
            raw,
        })
    }

    /// 真流式（SSE）：`stream: true` + 逐 `delta.content` yield。
    /// 流内错误以 `Err` 项发出；`[DONE]` 标记正常结束。
    fn complete_stream(
        &self,
        req: CompletionRequest,
    ) -> futures_util::stream::BoxStream<'_, Result<String, BackendError>> {
        let client = self.client.clone();
        let url = self.endpoint();
        let headers = self.build_headers();
        let timeout = self.timeout_seconds;
        let model = self.default_model.clone();
        let system = req.system.clone();
        let prompt = req.prompt.clone();
        let history = req.history.clone();

        futures_util::stream::once(async move {
            // owned body（stream:true）
            let mut messages: Vec<serde_json::Value> = Vec::new();
            if let Some(sys) = &system {
                if !sys.is_empty() {
                    messages.push(serde_json::json!({"role": "system", "content": sys}));
                }
            }
            for (role, content) in &history {
                messages.push(serde_json::json!({"role": role, "content": content}));
            }
            messages.push(serde_json::json!({"role": "user", "content": prompt}));
            let body = serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": true,
                "temperature": req.temperature,
                "max_tokens": req.max_tokens,
                "top_p": req.top_p,
            });

            let response = match client.post(&url).headers(headers).json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    let err = if e.is_timeout() {
                        BackendError::Timeout(timeout)
                    } else {
                        BackendError::Transport(e.to_string())
                    };
                    return futures_util::stream::once(async move { Err(err) }).boxed();
                }
            };

            let status = response.status().as_u16();
            if !response.status().is_success() {
                let err_body = response.text().await.unwrap_or_default();
                let err = Self::parse_error(status, &err_body);
                return futures_util::stream::once(async move { Err(err) }).boxed();
            }

            // 共享 SSE 解析：response → channel → BoxStream
            let rx = super::sse::spawn_sse_parser(response);
            super::sse::channel_to_stream(rx)
        })
        .flatten()
        .boxed()
    }
}

pub const fn default_model() -> &'static str {
    DEFAULT_MODEL
}

pub const fn default_flash_model() -> &'static str {
    DEFAULT_FLASH_MODEL
}
