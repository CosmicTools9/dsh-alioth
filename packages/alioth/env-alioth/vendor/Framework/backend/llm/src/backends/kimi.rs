//! Kimi (Moonshot) provider backend.
//!
//! Kimi offers OpenAI-compatible chat completions. Default access is the
//! Kimi Code subscription endpoint `https://api.kimi.com/coding` with model
//! ids `k3-256k` / `kimi-for-coding[-highspeed]` (defaults below). Platform
//! keys (platform.kimi.com) instead use `https://api.moonshot.cn` with model
//! ids `kimi-k3` / `kimi-k2.6` — override `base_url` + models in config for
//! those (the `/v1` suffix is appended by `endpoint()`).
//! Differences from generic OpenAI:
//! - `Authorization: Bearer <key>` (same as OpenAI)
//! - Native `tools` schema with `type: "function"`
//! - Kimi K3 supports `reasoning_effort` as a top-level field (`low`/`high`/`max`).
//! - Kimi K3 uses `max_completion_tokens` (not `max_tokens`).
//! - Temperature is fixed per-model and MUST NOT be sent (will error).
//! - Does NOT support `response_format.type = "json_object"` directly —
//!   instead we use `tools` with a JSON schema or rely on prompt instructions.
//! - Returns tool calls with `arguments` as a JSON string (parses identically
//!   to DeepSeek).

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use super::{BackendError, CompletionRequest, CompletionResponse, LlmBackend, ToolCallResult};

const DEFAULT_BASE_URL: &str = "https://api.kimi.com/coding";
const DEFAULT_MODEL: &str = "k3-256k";
const DEFAULT_FLASH_MODEL: &str = "kimi-for-coding";

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct KimiRequest<'a> {
    model: &'a str,
    messages: Vec<KimiMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<KimiTool<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a str>,
    /// Temperature is fixed per-model. Skipped entirely (Kimi errors if sent).
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    /// Used for K3 (deprecated max_tokens). Non-K3 uses max_tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u64>,
    /// Used for non-K3 models (k2.6, k2.7-code). K3 uses max_completion_tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    top_p: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<u8>,
    /// Kimi K3 supports reasoning_effort (low/high/max).
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
    /// thinking 控制:非 K3 模型（K2.7 固定 enabled+keep:all；K2.6 enabled/disabled）。
    /// K3 始终思考，不发送。
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<KimiThinking<'a>>,
    #[serde(default)]
    stream: bool,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct KimiThinking<'a> {
    #[serde(rename = "type")]
    thinking_type: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    keep: Option<&'a str>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct KimiMessage<'a> {
    role: &'a str,
    content: super::MessageContent<'a>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct KimiTool<'a> {
    #[serde(rename = "type")]
    tool_type: &'static str,
    function: KimiFunction<'a>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct KimiFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct KimiResponse {
    id: Option<String>,
    model: Option<String>,
    choices: Vec<KimiChoice>,
    usage: Option<KimiUsage>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct KimiChoice {
    index: u32,
    message: KimiAssistantMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct KimiAssistantMessage {
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<KimiToolCall>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct KimiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: KimiFunctionCall,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct KimiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct KimiUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct KimiErrorBody {
    error: Option<KimiErrorDetail>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct KimiErrorDetail {
    message: String,
    #[serde(rename = "type")]
    err_type: Option<String>,
    code: Option<String>,
}

pub struct KimiBackend {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    default_model: String,
    #[allow(dead_code)]
    flash_model: String,
    timeout_seconds: u64,
}

impl KimiBackend {
    pub fn new(
        api_key: String,
        base_url: Option<String>,
        default_model: String,
        flash_model: String,
        timeout_seconds: u64,
    ) -> Result<Self, BackendError> {
        if api_key.is_empty() {
            return Err(BackendError::Auth("Kimi API key is empty".to_string()));
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(timeout_seconds))
                // 防御性优化：同 deepseek——reqwest .timeout() 已是总请求超时，
                // connect_timeout 仅额外限制 TCP 连接建立等待上限，非根因修复。
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
        // Kimi's chat completions endpoint is at /v1/chat/completions.
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
        h.insert("User-Agent", "KimiCLI".parse().unwrap());
        h
    }

    fn build_body<'a>(&'a self, req: &'a CompletionRequest, stream: bool) -> KimiRequest<'a> {
        let mut messages = Vec::new();
        if let Some(sys) = &req.system {
            if !sys.is_empty() {
                messages.push(KimiMessage {
                    role: "system",
                    content: super::MessageContent::Text(sys.as_str()),
                });
            }
        }
        for (role, content) in &req.history {
            messages.push(KimiMessage {
                role: role.as_str(),
                content: super::MessageContent::Text(content.as_str()),
            });
        }
        messages.push(KimiMessage {
            role: "user",
            content: super::build_user_content(req.prompt.as_str(), &req.images),
        });

        let tools: Option<Vec<KimiTool>> = if req.tools.is_empty() {
            None
        } else {
            Some(
                req.tools
                    .iter()
                    .map(|t| KimiTool {
                        tool_type: "function",
                        function: KimiFunction {
                            name: t.name.as_str(),
                            description: t.description.as_str(),
                            parameters: &t.parameters,
                        },
                    })
                    .collect(),
            )
        };

        // Model capability branching: K3 uses new fields, legacy models use old.
        // Coding-endpoint ids (`k3`, `k3-256k`) are K3 variants without the `kimi-` prefix.
        let is_k3 =
            req.model.contains("kimi-k3") || req.model == "k3" || req.model.starts_with("k3-");

        // Kimi K3 reasoning_effort: low/high/max. Only for K3.
        let reasoning_effort: Option<&'static str> = if is_k3 {
            match req.reasoning_effort {
                Some(crate::types::ReasoningEffort::Minimal) => Some("low"),
                Some(crate::types::ReasoningEffort::Low) => Some("low"),
                Some(crate::types::ReasoningEffort::Medium) => Some("high"),
                Some(crate::types::ReasoningEffort::High) => Some("high"),
                Some(crate::types::ReasoningEffort::XHigh) => Some("max"),
                Some(crate::types::ReasoningEffort::Max) => Some("max"),
                None => None,
            }
        } else {
            None
        };

        // Temperature is fixed per-model (all Kimi); skip entirely to avoid 400.
        let temperature: Option<f64> = None;

        // thinking 控制（仅非 K3）：
        // - K2.7-code 固定 enabled + keep:all（官方唯一合法值；disabled 请求会被纠正）
        // - K2.6 按配置 enabled/disabled（None = 不发送，官方默认 enabled）
        let thinking: Option<KimiThinking> = if is_k3 {
            None
        } else {
            let is_k27 = req.model.contains("k2.7") || req.model.contains("kimi-for-coding");
            match req.thinking.as_deref() {
                Some("disabled") if !is_k27 => Some(KimiThinking {
                    thinking_type: "disabled",
                    keep: None,
                }),
                Some("enabled") if !is_k27 => Some(KimiThinking {
                    thinking_type: "enabled",
                    keep: None,
                }),
                Some(_) if is_k27 => Some(KimiThinking {
                    thinking_type: "enabled",
                    keep: Some("all"),
                }),
                _ => None,
            }
        };

        KimiRequest {
            model: req.model.as_str(),
            messages,
            tools,
            tool_choice: if req.tools.is_empty() {
                None
            } else {
                Some("auto")
            },
            temperature,
            max_completion_tokens: if is_k3 { Some(req.max_tokens) } else { None },
            max_tokens: if is_k3 { None } else { Some(req.max_tokens) },
            top_p: req.top_p,
            // Kimi supports multiple completions per request; default to 1.
            n: Some(1),
            reasoning_effort,
            thinking,
            stream,
        }
    }

    fn parse_error(status: u16, body: &str) -> BackendError {
        if let Ok(parsed) = serde_json::from_str::<KimiErrorBody>(body) {
            let msg = parsed
                .error
                .as_ref()
                .map(|e| e.message.clone())
                .or(parsed.message)
                .unwrap_or_else(|| body.to_string());
            return match status {
                401 | 403 => BackendError::Auth(msg),
                429 => BackendError::RateLimit(60),
                _ => BackendError::ProviderStatus { status, body: msg },
            };
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
impl LlmBackend for KimiBackend {
    fn provider_name(&self) -> &'static str {
        "kimi"
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    /// kimi API 仅接受 top_p=0.95（1.0 报 400）——覆盖 trait 默认 verify（硬编码 1.0），
    /// 否则连通性验证误报失败（AppAgent 主链路经 build_request 已修正）。
    async fn verify(&self) -> Result<(), BackendError> {
        let req = CompletionRequest {
            model: self.default_model().to_string(),
            system: None,
            prompt: "ping".to_string(),
            tools: vec![],
            history: vec![],
            temperature: 0.0,
            max_tokens: 1,
            top_p: 0.95,
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

        let parsed: KimiResponse = serde_json::from_value(raw.clone())
            .map_err(|e| BackendError::Parse(format!("Kimi envelope: {}", e)))?;

        let choice = parsed
            .choices
            .first()
            .ok_or_else(|| BackendError::Parse("no choices in response".to_string()))?;

        let text = choice.message.content.clone();
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
            text: if text.as_ref().map(|s| s.is_empty()).unwrap_or(true) {
                None
            } else {
                text
            },
            tool_calls,
            usage,
            raw,
        })
    }

    /// 真流式（SSE，OpenAI 兼容）：`stream: true` + 共享 SSE 解析。
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
            // owned body（stream:true）——build_body 返回 borrow req 结构无法跨 spawn
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
    use crate::types::ReasoningEffort;

    fn make_backend() -> KimiBackend {
        KimiBackend::new(
            "test-key".to_string(),
            None,
            "k3-256k".to_string(),
            "kimi-for-coding".to_string(),
            30,
        )
        .unwrap()
    }

    fn make_request(model: &str, effort: Option<ReasoningEffort>) -> CompletionRequest {
        CompletionRequest {
            model: model.to_string(),
            system: None,
            prompt: "hello".to_string(),
            tools: vec![],
            images: vec![],
            history: vec![],
            temperature: 1.0,
            max_tokens: 4096,
            top_p: 1.0,
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            reasoning_effort: effort,
            response_format: None,

            thinking: None,
            service_tier: None,
            reasoning_split: None,
        }
    }

    #[test]
    fn test_k3_serializes_max_completion_tokens() {
        let backend = make_backend();
        let req = make_request("k3-256k", None);
        let body = backend.build_body(&req, false);
        let json = serde_json::to_string(&body).unwrap();
        assert!(
            json.contains("max_completion_tokens"),
            "K3 should serialize max_completion_tokens"
        );
        assert!(
            !json.contains("max_tokens"),
            "K3 should NOT serialize max_tokens"
        );
        assert!(
            !json.contains("temperature"),
            "Kimi should NOT serialize temperature"
        );
    }

    #[test]
    fn test_k26_serializes_max_tokens() {
        let backend = make_backend();
        let req = make_request("kimi-k2.6", None);
        let body = backend.build_body(&req, false);
        let json = serde_json::to_string(&body).unwrap();
        assert!(
            json.contains("max_tokens"),
            "k2.6 should serialize max_tokens"
        );
        assert!(
            !json.contains("max_completion_tokens"),
            "k2.6 should NOT serialize max_completion_tokens"
        );
        assert!(
            !json.contains("temperature"),
            "Kimi should NOT serialize temperature"
        );
    }

    #[test]
    fn test_k3_reasoning_effort_included() {
        let backend = make_backend();
        let req = make_request("k3-256k", Some(ReasoningEffort::XHigh));
        let body = backend.build_body(&req, false);
        let json = serde_json::to_string(&body).unwrap();
        assert!(
            json.contains("reasoning_effort"),
            "K3 should include reasoning_effort"
        );
        assert!(json.contains("\"max\""), "K3 XHigh maps to max");
    }

    #[test]
    fn test_k26_reasoning_effort_omitted() {
        let backend = make_backend();
        let req = make_request("kimi-k2.6", Some(ReasoningEffort::High));
        let body = backend.build_body(&req, false);
        let json = serde_json::to_string(&body).unwrap();
        assert!(
            !json.contains("reasoning_effort"),
            "k2.6 should NOT include reasoning_effort"
        );
    }
}
