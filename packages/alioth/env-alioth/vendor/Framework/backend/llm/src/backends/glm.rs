//! GLM（智谱）provider backend。
//!
//! GLM Coding Plan（OpenAI-compatible schema）：
//! - Chat completions: `POST {base_url}/chat/completions`（注意：无 `/v1` 段，
//!   与 DeepSeek backend 的 `/v1/chat/completions` 不同，故独立 backend 而非复用）
//! - 默认端点：Coding Plan `https://open.bigmodel.cn/api/coding/paas/v4`；
//!   通用 API 经 base_url 覆盖为 `https://open.bigmodel.cn/api/paas/v4`
//! - Bearer token 认证；`thinking` 仅发送 `{"type":"enabled"}`——GLM-5.3 不支持
//!   禁用思考，请求中的 `disabled` 值剔除不发送（发送即 400，API 默认始终思考）
//! - `reasoning_effort`: low/high/max（GLM-5.3 官方档位；经 `ReasoningEffort`
//!   统一映射，Medium 省略走 API 默认 max 并保持前缀缓存）
//! - Streaming via SSE（`stream: true`，复用共享 SSE 解析器）

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use super::{BackendError, CompletionRequest, CompletionResponse, LlmBackend, ToolCallResult};

const DEFAULT_BASE_URL: &str = "https://open.bigmodel.cn/api/coding/paas/v4";
const DEFAULT_MODEL: &str = "glm-5.3";
const DEFAULT_FLASH_MODEL: &str = "glm-5.3-flash";

/// GLM 请求体（OpenAI-compatible；reasoning_effort 走 GLM-5.3 官方档位 low/high/max）。
#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct GlmRequest<'a> {
    model: &'a str,
    messages: Vec<GlmMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GlmTool<'a>>>,
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
    response_format: Option<GlmResponseFormat<'a>>,
    /// thinking 开关：{"type":"enabled"|"disabled"}。None = 不发送（API 默认）
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<GlmThinking<'a>>,
    #[serde(default)]
    stream: bool,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct GlmMessage<'a> {
    role: &'a str,
    content: super::MessageContent<'a>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct GlmTool<'a> {
    #[serde(rename = "type")]
    tool_type: &'a str,
    function: GlmFunction<'a>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct GlmFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct GlmResponseFormat<'a> {
    #[serde(rename = "type")]
    format_type: &'a str,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct GlmThinking<'a> {
    #[serde(rename = "type")]
    thinking_type: &'a str,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GlmResponse {
    choices: Vec<GlmChoice>,
    #[serde(default)]
    usage: Option<GlmUsage>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GlmChoice {
    message: GlmAssistantMessage,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GlmAssistantMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<GlmToolCall>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GlmToolCall {
    #[serde(default)]
    id: String,
    function: GlmFunctionCall,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GlmFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GlmUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
}

/// GLM 错误响应体：{"error": {"code": "...", "message": "..."}}
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GlmErrorBody {
    error: Option<GlmErrorDetail>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GlmErrorDetail {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

pub struct GlmBackend {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model: String,
    #[allow(dead_code)]
    flash_model: String,
    timeout_seconds: u64,
}

impl GlmBackend {
    pub fn new(
        api_key: String,
        base_url: Option<String>,
        default_model: String,
        flash_model: String,
        timeout_seconds: u64,
    ) -> Result<Self, BackendError> {
        if api_key.is_empty() {
            return Err(BackendError::Auth("GLM API key is empty".to_string()));
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(timeout_seconds))
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

    /// GLM 端点：`{base}/chat/completions`（无 `/v1` 段）
    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    fn build_headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, "application/json".parse().unwrap());
        h.insert(
            AUTHORIZATION,
            format!("Bearer {}", self.api_key).parse().unwrap(),
        );
        h.insert("User-Agent", "AliothStudio/0.1 (glm)".parse().unwrap());
        h
    }

    fn build_body<'a>(&'a self, req: &'a CompletionRequest, stream: bool) -> GlmRequest<'a> {
        let mut messages = Vec::new();
        if let Some(sys) = &req.system {
            if !sys.is_empty() {
                messages.push(GlmMessage {
                    role: "system",
                    content: super::MessageContent::Text(sys.as_str()),
                });
            }
        }
        for (role, content) in &req.history {
            messages.push(GlmMessage {
                role: role.as_str(),
                content: super::MessageContent::Text(content.as_str()),
            });
        }
        messages.push(GlmMessage {
            role: "user",
            content: super::build_user_content(req.prompt.as_str(), &req.images),
        });

        let tools: Option<Vec<GlmTool>> = if req.tools.is_empty() {
            None
        } else {
            Some(
                req.tools
                    .iter()
                    .map(|t| GlmTool {
                        tool_type: "function",
                        function: GlmFunction {
                            name: t.name.as_str(),
                            description: t.description.as_str(),
                            parameters: &t.parameters,
                        },
                    })
                    .collect(),
            )
        };

        let response_format: Option<GlmResponseFormat> = req
            .response_format
            .as_deref()
            .map(|t| GlmResponseFormat { format_type: t });
        // GLM-5.3 仅支持 thinking enabled（官方文档：disabled 发送即 400）——
        // 剔除 disabled 值走 API 默认（始终思考），不降级为静默错误。
        let thinking = req
            .thinking
            .as_deref()
            .filter(|t| *t == "enabled")
            .map(|t| GlmThinking { thinking_type: t });

        GlmRequest {
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
            reasoning_effort: req.reasoning_effort.as_ref().and_then(|e| e.as_api_value()),
            response_format,
            thinking,
            stream,
        }
    }

    fn parse_error(status: u16, body: &str) -> BackendError {
        if let Ok(parsed) = serde_json::from_str::<GlmErrorBody>(body) {
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
impl LlmBackend for GlmBackend {
    fn provider_name(&self) -> &'static str {
        "glm"
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

        let parsed: GlmResponse = serde_json::from_value(raw.clone())
            .map_err(|e| BackendError::Parse(format!("GLM envelope: {}", e)))?;

        let choice = parsed
            .choices
            .first()
            .ok_or_else(|| BackendError::Parse("no choices in response".to_string()))?;

        let text = choice.message.content.clone().unwrap_or_default();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_appends_chat_completions_without_v1() {
        let backend = GlmBackend::new(
            "k".to_string(),
            None,
            "glm-5.3".to_string(),
            "glm-5.3".to_string(),
            30,
        )
        .expect("backend");
        assert_eq!(
            backend.endpoint(),
            "https://open.bigmodel.cn/api/coding/paas/v4/chat/completions"
        );
    }

    #[test]
    fn endpoint_respects_base_url_override_and_strips_trailing_slash() {
        let backend = GlmBackend::new(
            "k".to_string(),
            Some("https://open.bigmodel.cn/api/paas/v4/".to_string()),
            "glm-5.3".to_string(),
            "glm-5.3".to_string(),
            30,
        )
        .expect("backend");
        assert_eq!(
            backend.endpoint(),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
    }

    #[test]
    fn empty_api_key_rejected() {
        let err = match GlmBackend::new(
            String::new(),
            None,
            "glm-5.3".to_string(),
            "glm-5.3-flash".to_string(),
            30,
        ) {
            Err(e) => e,
            Ok(_) => panic!("empty api key must be rejected"),
        };
        assert!(matches!(err, BackendError::Auth(_)));
    }

    #[test]
    fn request_body_maps_reasoning_effort_and_skips_empty_optionals() {
        let backend = GlmBackend::new(
            "k".to_string(),
            None,
            "glm-5.3".to_string(),
            "glm-5.3".to_string(),
            30,
        )
        .expect("backend");
        let req = CompletionRequest {
            model: "glm-5.3".to_string(),
            system: Some("sys".to_string()),
            history: vec![("user".into(), "hi".to_string())],
            prompt: "ping".to_string(),
            images: vec![],
            tools: vec![],
            temperature: 0.7,
            max_tokens: 128,
            top_p: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            reasoning_effort: Some(crate::types::ReasoningEffort::High),
            response_format: None,
            thinking: None,
            service_tier: None,
            reasoning_split: None,
        };
        let body = backend.build_body(&req, false);
        let json = serde_json::to_value(&body).expect("serialize");
        assert_eq!(json["model"], "glm-5.3");
        assert_eq!(json["messages"].as_array().map(Vec::len), Some(3));
        // GLM-5.3 官方支持 reasoning_effort（low/high/max）：High → "high"
        assert_eq!(json["reasoning_effort"], "high");
        // 空 optional 字段不序列化
        assert!(json.get("tools").is_none());
        assert!(json.get("thinking").is_none());
        assert!(json.get("frequency_penalty").is_none());
        assert_eq!(json["stream"], false);
    }

    #[test]
    fn effort_max_sent_and_medium_omitted_for_prefix_cache() {
        let backend = GlmBackend::new(
            "k".to_string(),
            None,
            "glm-5.3".to_string(),
            "glm-5.3".to_string(),
            30,
        )
        .expect("backend");
        let base = CompletionRequest {
            model: "glm-5.3".to_string(),
            system: None,
            history: vec![],
            prompt: "ping".to_string(),
            images: vec![],
            tools: vec![],
            temperature: 1.0,
            max_tokens: 16,
            top_p: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            reasoning_effort: Some(crate::types::ReasoningEffort::Max),
            response_format: None,
            thinking: None,
            service_tier: None,
            reasoning_split: None,
        };
        let json = serde_json::to_value(backend.build_body(&base, false)).expect("serialize");
        assert_eq!(json["reasoning_effort"], "max");

        let medium = CompletionRequest {
            reasoning_effort: Some(crate::types::ReasoningEffort::Medium),
            ..base.clone()
        };
        let json = serde_json::to_value(backend.build_body(&medium, false)).expect("serialize");
        // Medium 省略——走 API 默认（max）并保持前缀缓存
        assert!(json.get("reasoning_effort").is_none());
    }

    #[test]
    fn thinking_disabled_dropped_enabled_passthrough() {
        let backend = GlmBackend::new(
            "k".to_string(),
            None,
            "glm-5.3".to_string(),
            "glm-5.3".to_string(),
            30,
        )
        .expect("backend");
        let base = CompletionRequest {
            model: "glm-5.3".to_string(),
            system: None,
            history: vec![],
            prompt: "ping".to_string(),
            images: vec![],
            tools: vec![],
            temperature: 1.0,
            max_tokens: 16,
            top_p: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            reasoning_effort: None,
            response_format: None,
            thinking: Some("disabled".to_string()),
            service_tier: None,
            reasoning_split: None,
        };
        let json = serde_json::to_value(backend.build_body(&base, false)).expect("serialize");
        // GLM-5.3 不支持 disabled（发送即 400）——剔除走 API 默认（始终思考）
        assert!(json.get("thinking").is_none());

        let enabled = CompletionRequest {
            thinking: Some("enabled".to_string()),
            ..base
        };
        let json = serde_json::to_value(backend.build_body(&enabled, false)).expect("serialize");
        assert_eq!(json["thinking"]["type"], "enabled");
    }
}
