//! `LlmService` is the high-level façade that the rest of the project uses.
//!
//! Internally it holds a `Box<dyn LlmBackend>` selected by `LlmProvider` and
//! applies per-request timeouts and parameter overrides.
use super::backends::{
    build_backend, BackendError, CompletionRequest, CompletionResponse, LlmBackend,
};
use super::types::{
    GenerationParams, LlmProvider, LlmResponse, LlmServiceConfig, ModelRole, ToolCall,
    ToolDefinition,
};
use futures_util::StreamExt;
use std::time::Duration;
/// Wrapper error used by the public API.
///
/// Backends produce `BackendError`; we expose it as `LlmError` so the
/// rest of the codebase doesn't have to import the backend module.
pub type LlmError = BackendError;

pub struct LlmService {
    backend: Box<dyn LlmBackend>,
    config: LlmServiceConfig,
    /// 独立视觉后端（LLM_VISION_* env；未配置时为 None → inspect_image 走主 backend）
    vision_backend: Option<Box<dyn LlmBackend>>,
}

impl LlmService {
    pub fn new(config: LlmServiceConfig) -> Result<Self, LlmError> {
        // 保留真实 provider（原 mem::replace 把 config.provider 替换为 DeepSeek，
        // 导致 build_request 的 per-provider 参数修正（kimi top_p=0.95）永远不生效——
        // 会话 553/554/555 实测 kimi fallback 因 top_p=1.0 被拒）。
        let provider = config.provider.clone();
        let backend = build_backend(
            provider,
            config.api_key.clone(),
            config.base_url.clone(),
            config.model.clone(),
            config.flash_model.clone(),
            config.timeout_seconds,
        )?;
        let vision_backend = Self::build_vision_backend(config.timeout_seconds);
        Ok(Self {
            backend,
            config,
            vision_backend,
        })
    }

    /// 独立视觉后端：LLM_VISION_PROVIDER / LLM_VISION_API_KEY / LLM_VISION_BASE_URL /
    /// LLM_MODEL_VISION（缺省跟随主 provider / 主 key / provider 默认地址 / flash 档）。
    /// API key 全缺时返回 None（inspect_image 回退主 backend）。
    fn build_vision_backend(timeout_seconds: u64) -> Option<Box<dyn LlmBackend>> {
        let provider = std::env::var("LLM_VISION_PROVIDER")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(LlmProvider::from_env);
        let api_key = std::env::var("LLM_VISION_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .or_else(|| std::env::var("LLM_API_KEY").ok().filter(|k| !k.is_empty()))?;
        let base_url = std::env::var("LLM_VISION_BASE_URL")
            .ok()
            .or_else(|| Some(provider.default_base_url().to_string()));
        let model = std::env::var("LLM_MODEL_VISION")
            .ok()
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| provider.default_flash_model().to_string());
        match build_backend(
            provider,
            api_key,
            base_url,
            model.clone(),
            model,
            timeout_seconds,
        ) {
            Ok(b) => Some(b),
            Err(e) => {
                log::warn!(
                    "vision backend 构造失败，inspect_image 回退主 backend: {}",
                    e
                );
                None
            }
        }
    }

    /// 图像理解（vision 链路）：按 vision 角色模型 + 参数构造请求，附加图像后发送。
    /// backend 选择：独立视觉后端优先，未配置时主 backend（provider 不支持图像时由上游报错）。
    pub async fn inspect_image(
        &self,
        image: &super::backends::ImageContent,
        prompt: &str,
    ) -> Result<String, LlmError> {
        let (model, base_params) = self.role_binding(ModelRole::Vision);
        let req = self.build_request_base(
            None,
            prompt,
            &[],
            None,
            None,
            None,
            None,
            Some(&model),
            &base_params,
        )?;
        let mut req = req;
        req.images.push(image.clone());
        let backend: &dyn LlmBackend = self
            .vision_backend
            .as_deref()
            .unwrap_or(self.backend.as_ref());
        let resp = self.complete_with_timeout_via(backend, req).await?;
        Ok(resp.text.unwrap_or_default())
    }

    /// Plain text generation (no system prompt, no overrides).
    pub async fn generate(&self, prompt: &str) -> Result<String, LlmError> {
        self.generate_with_overrides(prompt, None, None, None, None)
            .await
    }

    /// 带参数覆盖的生成。适用于 Harness 层按 TaskType 动态调节。
    ///
    /// 未设置的参数（None）使用构造时的默认值。
    pub async fn generate_with_overrides(
        &self,
        prompt: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
        reasoning_effort: Option<&str>,
        response_format: Option<&str>,
    ) -> Result<String, LlmError> {
        let req = self.build_request(
            None,
            prompt,
            &[],
            temperature,
            max_tokens,
            reasoning_effort,
            response_format,
            None,
        )?;
        let resp = self.complete_with_timeout(req).await?;
        Ok(resp.text.unwrap_or_default())
    }

    /// 带 system prompt + 模型切换的生成。
    /// 将 system prompt 作为 API system message 发送。
    #[allow(clippy::too_many_arguments)]
    pub async fn generate_with_system_preamble(
        &self,
        system: &str,
        prompt: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
        reasoning_effort: Option<&str>,
        response_format: Option<&str>,
        model_override: Option<&str>,
    ) -> Result<String, LlmError> {
        let req = self.build_request(
            Some(system),
            prompt,
            &[],
            temperature,
            max_tokens,
            reasoning_effort,
            response_format,
            model_override,
        )?;
        let resp = self.complete_with_timeout(req).await?;
        Ok(resp.text.unwrap_or_default())
    }

    /// 流式生成（LLM SSE 真流式，P1-6 完成态）。
    ///
    /// 返回逐 chunk 文本流；超时以 `Err(LlmError::Timeout)` 项终止。
    /// 不支持流式的后端（Kimi/MiniMax）经 trait 默认实现退化为单 chunk。
    /// 注意：流式不适用 `complete_with_timeout` 的重试语义（重试会重复消费上游），
    /// 此处仅做整体超时包裹，重试留给调用方决策。
    pub fn generate_stream<'a>(
        &'a self,
        system: Option<&str>,
        prompt: &str,
    ) -> futures_util::stream::BoxStream<'a, Result<String, LlmError>> {
        self.generate_stream_detailed(system, prompt, None, None, None, None, None)
    }

    /// 参数化流式生成（LLM SSE 真流式 + overrides）。
    ///
    /// 与 `generate_stream` 同语义（逐 chunk 文本流；超时以 `Err(LlmError::Timeout)`
    /// 项终止；不做重试——流式重试会重复消费上游生成），额外透传
    /// temperature / max_tokens / reasoning_effort / response_format / model_override。
    /// 供需要流式 + 参数覆盖的调用方（如 DbDrivenLlmAdapter）使用。
    #[allow(clippy::too_many_arguments)]
    pub fn generate_stream_detailed<'a>(
        &'a self,
        system: Option<&str>,
        prompt: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
        reasoning_effort: Option<&str>,
        response_format: Option<&str>,
        model_override: Option<&str>,
    ) -> futures_util::stream::BoxStream<'a, Result<String, LlmError>> {
        let req = match self.build_request(
            system,
            prompt,
            &[],
            temperature,
            max_tokens,
            reasoning_effort,
            response_format,
            model_override,
        ) {
            Ok(r) => r,
            Err(e) => {
                return futures_util::stream::once(async move { Err(e) }).boxed();
            }
        };
        let timeout = self.config.timeout_seconds;
        let backend = &self.backend;

        // 从后端流式 + 超时包裹
        let inner = backend.complete_stream(req);
        futures_util::stream::unfold((inner, timeout), |(mut inner, timeout)| async move {
            match tokio::time::timeout(std::time::Duration::from_secs(timeout), inner.next()).await
            {
                Ok(Some(item)) => Some((item, (inner, timeout))),
                Ok(None) => None,
                Err(_) => Some((
                    Err(LlmError::Timeout(timeout)),
                    (futures_util::stream::empty().boxed(), timeout),
                )),
            }
        })
        .boxed()
    }

    /// 带历史消息对 + 参数覆盖的流式生成（optimize-appagent-token-efficiency D3）：
    /// 复用 generate_stream_detailed 的 stream 包装（build_request + complete_stream +
    /// 超时），history 构造后填入 req——messages 为 [system, ...history, user]，
    /// 流式输出逐 chunk，history 轮次前缀缓存可命中。
    #[allow(clippy::too_many_arguments)]
    pub fn generate_stream_detailed_with_history<'a>(
        &'a self,
        system: Option<&str>,
        history: &[(String, String)],
        prompt: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
        reasoning_effort: Option<&str>,
        response_format: Option<&str>,
        model_override: Option<&str>,
    ) -> futures_util::stream::BoxStream<'a, Result<String, LlmError>> {
        let mut req = match self.build_request(
            system,
            prompt,
            &[],
            temperature,
            max_tokens,
            reasoning_effort,
            response_format,
            model_override,
        ) {
            Ok(r) => r,
            Err(e) => {
                return futures_util::stream::once(async move { Err(e) }).boxed();
            }
        };
        req.history = history.to_vec();
        let timeout = self.config.timeout_seconds;
        let backend = &self.backend;

        let inner = backend.complete_stream(req);
        futures_util::stream::unfold((inner, timeout), |(mut inner, timeout)| async move {
            match tokio::time::timeout(std::time::Duration::from_secs(timeout), inner.next()).await
            {
                Ok(Some(item)) => Some((item, (inner, timeout))),
                Ok(None) => None,
                Err(_) => Some((
                    Err(LlmError::Timeout(timeout)),
                    (futures_util::stream::empty().boxed(), timeout),
                )),
            }
        })
        .boxed()
    }

    /// 带 system prompt + 模型切换的生成，同时返回 token 用量。
    /// 与 `generate_with_system_preamble` 参数一致，供需要用量统计的调用方（如预算治理）使用。
    #[allow(clippy::too_many_arguments)]
    pub async fn generate_detailed(
        &self,
        system: &str,
        prompt: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
        reasoning_effort: Option<&str>,
        response_format: Option<&str>,
        model_override: Option<&str>,
    ) -> Result<(String, Option<crate::types::TokenUsage>), LlmError> {
        let req = self.build_request(
            Some(system),
            prompt,
            &[],
            temperature,
            max_tokens,
            reasoning_effort,
            response_format,
            model_override,
        )?;
        let resp = self.complete_with_timeout(req).await?;
        Ok((resp.text.unwrap_or_default(), resp.usage))
    }
    /// 带历史消息对的多轮生成（fix-appagent-prefix-cache）：messages 拼装为
    /// [system, ...history, user]——历史追加式传递，provider 前缀缓存可命中
    /// 既有轮次。history 为空时与 `generate_detailed` 行为完全一致。
    #[allow(clippy::too_many_arguments)]
    pub async fn generate_detailed_with_history(
        &self,
        system: &str,
        history: &[(String, String)],
        prompt: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
        reasoning_effort: Option<&str>,
        response_format: Option<&str>,
        model_override: Option<&str>,
    ) -> Result<(String, Option<crate::types::TokenUsage>), LlmError> {
        let mut req = self.build_request(
            Some(system),
            prompt,
            &[],
            temperature,
            max_tokens,
            reasoning_effort,
            response_format,
            model_override,
        )?;
        req.history = history.to_vec();
        let resp = self.complete_with_timeout(req).await?;
        Ok((resp.text.unwrap_or_default(), resp.usage))
    }
    /// 带工具调用的生成
    pub async fn generate_with_tools(
        &self,
        prompt: &str,
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse, LlmError> {
        let req = self.build_request(None, prompt, tools, None, None, None, None, None)?;
        let resp = self.complete_with_timeout(req).await?;
        Ok(to_llm_response(resp))
    }

    /// 带 system prompt + 参数覆盖 + 工具调用的生成，同时返回 token 用量。
    /// 与 `generate_detailed` 参数一致 + tools；响应为 `LlmResponse`（Text 或 ToolCalls）。
    /// 供 AppAgent harness 工具型任务使用（原生 function calling 优先）。
    #[allow(clippy::too_many_arguments)]
    pub async fn generate_detailed_with_tools(
        &self,
        system: &str,
        prompt: &str,
        tools: &[ToolDefinition],
        temperature: Option<f64>,
        max_tokens: Option<u64>,
        reasoning_effort: Option<&str>,
        response_format: Option<&str>,
        model_override: Option<&str>,
    ) -> Result<(LlmResponse, Option<crate::types::TokenUsage>), LlmError> {
        let req = self.build_request(
            Some(system),
            prompt,
            tools,
            temperature,
            max_tokens,
            reasoning_effort,
            response_format,
            model_override,
        )?;
        let resp = self.complete_with_timeout(req).await?;
        let usage = resp.usage;
        Ok((to_llm_response(resp), usage))
    }

    pub fn provider_name(&self) -> &str {
        self.backend.provider_name()
    }

    pub fn config(&self) -> &LlmServiceConfig {
        &self.config
    }

    pub async fn verify(&self) -> Result<(), LlmError> {
        self.backend.verify().await
    }

    /// 按角色生成（角色模型 + 角色参数；显式参数优先）。
    /// 角色见 [`ModelRole`]；roles 表缺失时回退 default（config.model + 全局参数）。
    pub async fn generate_for_role(
        &self,
        role: ModelRole,
        prompt: &str,
    ) -> Result<String, LlmError> {
        self.generate_with_system_preamble_for_role(role, None, prompt, None, None, None, None)
            .await
    }

    /// 带 system prompt + 角色选型的生成。
    #[allow(clippy::too_many_arguments)]
    pub async fn generate_with_system_preamble_for_role(
        &self,
        role: ModelRole,
        system: Option<&str>,
        prompt: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
        reasoning_effort: Option<&str>,
        response_format: Option<&str>,
    ) -> Result<String, LlmError> {
        let (model, base_params) = self.role_binding(role);
        let req = self.build_request_base(
            system,
            prompt,
            &[],
            temperature,
            max_tokens,
            reasoning_effort,
            response_format,
            Some(&model),
            &base_params,
        )?;
        let resp = self.complete_with_timeout(req).await?;
        Ok(resp.text.unwrap_or_default())
    }

    /// 角色绑定解析：roles 表 → (模型, 参数)；缺失角色回退 default。
    fn role_binding(&self, role: ModelRole) -> (String, GenerationParams) {
        match self.config.roles.get(&role) {
            Some(r) => (r.model.clone(), r.generation_params.clone()),
            None => (
                self.config.model.clone(),
                self.config.generation_params.clone(),
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_request(
        &self,
        system: Option<&str>,
        prompt: &str,
        tools: &[ToolDefinition],
        temperature: Option<f64>,
        max_tokens: Option<u64>,
        reasoning_effort: Option<&str>,
        response_format: Option<&str>,
        model_override: Option<&str>,
    ) -> Result<CompletionRequest, LlmError> {
        let model = model_override
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.config.model.clone());
        self.build_request_base(
            system,
            prompt,
            tools,
            temperature,
            max_tokens,
            reasoning_effort,
            response_format,
            Some(&model),
            &self.config.generation_params,
        )
    }

    /// base 参数版构造：角色调用传角色参数，常规调用传全局 generation_params。
    #[allow(clippy::too_many_arguments)]
    fn build_request_base(
        &self,
        system: Option<&str>,
        prompt: &str,
        tools: &[ToolDefinition],
        temperature: Option<f64>,
        max_tokens: Option<u64>,
        reasoning_effort: Option<&str>,
        response_format: Option<&str>,
        model: Option<&str>,
        params: &GenerationParams,
    ) -> Result<CompletionRequest, LlmError> {
        let reasoning_effort = match reasoning_effort.and_then(super::types::ReasoningEffort::parse)
        {
            // 显式参数优先
            Some(e) => Some(e),
            // 未显式传时应用 base 参数（角色/全局）；Medium 语义 = 不发送（API 默认）
            None => match &params.reasoning_effort {
                super::types::ReasoningEffort::Medium => None,
                other => other
                    .as_api_value()
                    .and_then(super::types::ReasoningEffort::parse),
            },
        };

        let temperature = temperature.unwrap_or(params.temperature);
        let max_tokens = max_tokens.unwrap_or(params.max_tokens);
        let response_format = response_format
            .map(|s| s.to_string())
            .or_else(|| params.response_format.clone());

        // 思考模式/服务层级/输出拆分取自全局配置（无 per-call override）
        let thinking = params.thinking.clone();
        let service_tier = params.service_tier.clone();
        let reasoning_split = params.reasoning_split;

        let model = model
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.config.model.clone());

        // top_p 按 provider 约束修正：kimi API 仅接受 0.95（1.0 报 400
        // 「only 0.95 is allowed」）——AppAgent provider fallback 到 kimi 时
        // 若仍发 1.0 会被拒，导致 deepseek 挂起后 fallback 也失败（实测 552）。
        let effective_top_p = match self.config.provider {
            LlmProvider::Kimi => 0.95,
            _ => params.top_p,
        };
        Ok(CompletionRequest {
            model,
            history: vec![],
            system: system.map(|s| s.to_string()),
            prompt: prompt.to_string(),
            tools: tools.to_vec(),
            temperature,
            max_tokens,
            top_p: effective_top_p,
            frequency_penalty: params.frequency_penalty,
            presence_penalty: params.presence_penalty,
            reasoning_effort,
            response_format,
            thinking,
            service_tier,
            reasoning_split,
            images: vec![],
        })
    }

    async fn complete_with_timeout(
        &self,
        req: CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        self.complete_with_timeout_via(self.backend.as_ref(), req)
            .await
    }

    /// 超时 + 重试主体，backend 可注入（vision 链路走独立后端）
    async fn complete_with_timeout_via(
        &self,
        backend: &dyn LlmBackend,
        req: CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        // L1 预算（超时机制重设计）：单次请求 200s（覆盖 connect+TTFB+body），重试 2 次
        // （共 3 次尝试，总预算 ≤600s）。实测校准（usage log）：deepseek 调用
        // 8.2%（170/2080）超 120s、实测上限 194s——120s 会误杀这些正常调用 →
        // 重试浪费（残留项修复）。200s 覆盖实测上限，真正挂起（无响应 >200s）快速失败。
        // Auth/Parse/4xx 不重试。
        let timeout = self.config.timeout_seconds.min(200);
        let max_retries = self.config.max_retries.clamp(1, 2);
        let mut last_err: LlmError = BackendError::Timeout(timeout);
        for attempt in 0..=max_retries {
            let outcome =
                tokio::time::timeout(Duration::from_secs(timeout), backend.complete(req.clone()))
                    .await;
            match outcome {
                Ok(Ok(resp)) => return Ok(resp),
                Ok(Err(e)) => last_err = e,
                Err(_) => last_err = BackendError::Timeout(timeout),
            }
            if attempt >= max_retries {
                break;
            }
            // 分类判断：仅瞬态错误等待重试
            let should_retry = match &last_err {
                BackendError::Transport(_) | BackendError::Timeout(_) => true,
                BackendError::RateLimit(reset) if *reset < 20 => true,
                _ => false,
            };
            if !should_retry {
                break;
            }
            match &last_err {
                BackendError::RateLimit(reset) => {
                    tokio::time::sleep(Duration::from_secs((*reset).min(15))).await;
                }
                _ => backoff_wait(attempt).await,
            }
        }
        Err(last_err)
    }
}

/// 指数退避（0.5s / 1s / 2s / 4s / 8s，封顶 8s）
async fn backoff_wait(attempt: u32) {
    let secs = 0.5f64 * 2f64.powi(attempt.min(4) as i32);
    tokio::time::sleep(Duration::from_secs_f64(secs.min(8.0))).await;
}

fn to_llm_response(resp: CompletionResponse) -> LlmResponse {
    if resp.tool_calls.is_empty() {
        LlmResponse::Text(resp.text.unwrap_or_default())
    } else {
        let tool_calls: Vec<ToolCall> = resp
            .tool_calls
            .into_iter()
            .map(|tc| ToolCall {
                id: tc.id,
                name: tc.name,
                arguments: tc.arguments,
            })
            .collect();
        LlmResponse::ToolCalls(tool_calls)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::{BackendError, CompletionRequest, CompletionResponse};
    use crate::types::{LlmResponse, LlmServiceConfig, TokenUsage, ToolDefinition};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    /// 进程内 mock backend：按序返回预设响应，并捕获请求供断言。
    struct MockBackend {
        responses: Mutex<Vec<CompletionResponse>>,
        captured: Arc<Mutex<Vec<CompletionRequest>>>,
    }

    #[async_trait]
    impl LlmBackend for MockBackend {
        fn provider_name(&self) -> &'static str {
            "mock"
        }
        async fn complete(
            &self,
            req: CompletionRequest,
        ) -> Result<CompletionResponse, BackendError> {
            self.captured.lock().unwrap().push(req.clone());
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(BackendError::Config("no mock response queued".into()));
            }
            Ok(responses.remove(0))
        }
        fn default_model(&self) -> &str {
            "mock-model"
        }
    }

    fn service_with(backend: MockBackend) -> LlmService {
        let config = LlmServiceConfig {
            provider: LlmProvider::DeepSeek,
            api_key: "k".to_string(),
            model: "mock-model".to_string(),
            flash_model: "mock-flash".to_string(),
            base_url: Some("http://localhost:9".to_string()),
            timeout_seconds: 5,
            max_retries: 0,
            generation_params: crate::types::GenerationParams::default(),
            roles: std::collections::HashMap::new(),
        };
        LlmService {
            backend: Box::new(backend),
            config,
            vision_backend: None,
        }
    }

    fn tool_def() -> ToolDefinition {
        serde_json::from_str(
            r#"{"name":"write_file","description":"write a file","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}"#,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn generate_detailed_with_tools_returns_tool_calls() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let backend = MockBackend {
            responses: Mutex::new(vec![CompletionResponse {
                text: None,
                tool_calls: vec![crate::backends::ToolCallResult {
                    id: "call_1".into(),
                    name: "write_file".into(),
                    arguments: serde_json::json!({"path": "a/b.ts"}),
                }],
                usage: Some(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 20,
                }),
                raw: serde_json::json!({}),
            }]),
            captured: captured.clone(),
        };
        let svc = service_with(backend);
        let (resp, usage) = svc
            .generate_detailed_with_tools(
                "sys",
                "do it",
                &[tool_def()],
                Some(0.2),
                Some(8192),
                Some("low"),
                None,
                Some("mock-model"),
            )
            .await
            .unwrap();
        match resp {
            LlmResponse::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "write_file");
                assert_eq!(calls[0].arguments["path"], "a/b.ts");
            }
            other => panic!("expected ToolCalls, got {:?}", other),
        }
        let u = usage.unwrap();
        assert_eq!(u.input_tokens, 10);
        assert_eq!(u.output_tokens, 20);
        let captured = captured.lock().unwrap();
        assert_eq!(captured[0].system.as_deref(), Some("sys"));
        assert_eq!(captured[0].tools.len(), 1);
        assert_eq!(captured[0].tools[0].name, "write_file");
        assert_eq!(captured[0].max_tokens, 8192);
        assert!(captured[0].response_format.is_none());
    }

    #[tokio::test]
    async fn generate_detailed_with_tools_returns_text_when_no_calls() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let backend = MockBackend {
            responses: Mutex::new(vec![CompletionResponse {
                text: Some("plain answer".into()),
                tool_calls: vec![],
                usage: Some(TokenUsage {
                    input_tokens: 5,
                    output_tokens: 7,
                }),
                raw: serde_json::json!({}),
            }]),
            captured: captured.clone(),
        };
        let svc = service_with(backend);
        let (resp, usage) = svc
            .generate_detailed_with_tools("sys", "hi", &[tool_def()], None, None, None, None, None)
            .await
            .unwrap();
        assert!(matches!(resp, LlmResponse::Text(ref t) if t == "plain answer"));
        assert_eq!(usage.unwrap().output_tokens, 7);
    }

    #[tokio::test]
    async fn generate_detailed_without_tools_stays_text_path() {
        // 未传 tools 的既有路径：请求体 tools 为空、响应解析为 Text，行为不变。
        let captured = Arc::new(Mutex::new(Vec::new()));
        let backend = MockBackend {
            responses: Mutex::new(vec![CompletionResponse {
                text: Some("ok".into()),
                tool_calls: vec![],
                usage: None,
                raw: serde_json::json!({}),
            }]),
            captured: captured.clone(),
        };
        let svc = service_with(backend);
        let (text, usage) = svc
            .generate_detailed("sys", "hi", None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(text, "ok");
        assert!(usage.is_none());
        let captured = captured.lock().unwrap();
        assert!(captured[0].tools.is_empty());
    }

    #[tokio::test]
    async fn generate_stream_default_backend_yields_full_text() {
        // MockBackend 未覆写 complete_stream → 走 trait 默认实现（单 chunk 完整文本）
        let backend = MockBackend {
            responses: Mutex::new(vec![CompletionResponse {
                text: Some("hello world".to_string()),
                tool_calls: vec![],
                usage: None,
                raw: serde_json::json!({}),
            }]),
            captured: Arc::new(Mutex::new(Vec::new())),
        };
        let svc = service_with(backend);

        let mut stream = svc.generate_stream(None, "ping");
        let mut collected = String::new();
        while let Some(chunk) = stream.next().await {
            collected.push_str(&chunk.expect("stream chunk"));
        }
        assert_eq!(collected, "hello world");
    }

    #[tokio::test]
    async fn generate_stream_empty_text_empty_stream() {
        let backend = MockBackend {
            responses: Mutex::new(vec![CompletionResponse {
                text: None,
                tool_calls: vec![],
                usage: None,
                raw: serde_json::json!({}),
            }]),
            captured: Arc::new(Mutex::new(Vec::new())),
        };
        let svc = service_with(backend);

        let mut stream = svc.generate_stream(None, "ping");
        assert!(
            stream.next().await.is_none(),
            "empty text should yield empty stream"
        );
    }

    /// 挂起 backend：complete 永不返回（complete_stream 走默认实现 → 超时以 Err 终止）
    struct HangingBackend;

    #[async_trait]
    impl LlmBackend for HangingBackend {
        fn provider_name(&self) -> &'static str {
            "hanging"
        }

        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, BackendError> {
            std::future::pending().await
        }

        fn default_model(&self) -> &str {
            "hanging-model"
        }
    }

    #[tokio::test]
    async fn generate_stream_detailed_respects_overrides() {
        // 参数化流式：overrides 必须透传到请求（build_request 全参数生效）
        let captured = Arc::new(Mutex::new(Vec::new()));
        let backend = MockBackend {
            responses: Mutex::new(vec![CompletionResponse {
                text: Some("chunked".into()),
                tool_calls: vec![],
                usage: None,
                raw: serde_json::json!({}),
            }]),
            captured: captured.clone(),
        };
        let svc = service_with(backend);

        let mut stream = svc.generate_stream_detailed(
            Some("sys"),
            "prompt",
            Some(0.3),
            Some(128),
            Some("high"),
            Some("json_object"),
            Some("override-model"),
        );
        let mut collected = String::new();
        while let Some(chunk) = stream.next().await {
            collected.push_str(&chunk.expect("stream chunk"));
        }
        assert_eq!(collected, "chunked");

        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].temperature, 0.3);
        assert_eq!(captured[0].max_tokens, 128);
        assert_eq!(captured[0].model, "override-model");
        assert_eq!(captured[0].response_format.as_deref(), Some("json_object"));
        assert!(captured[0].reasoning_effort.is_some());
        assert_eq!(captured[0].system.as_deref(), Some("sys"));
        assert_eq!(captured[0].prompt, "prompt");
    }

    #[tokio::test]
    async fn generate_stream_detailed_none_uses_defaults() {
        // 全 None：回退 generation_params 默认（temperature=1.0）+ config.model
        let captured = Arc::new(Mutex::new(Vec::new()));
        let backend = MockBackend {
            responses: Mutex::new(vec![CompletionResponse {
                text: Some("ok".into()),
                tool_calls: vec![],
                usage: None,
                raw: serde_json::json!({}),
            }]),
            captured: captured.clone(),
        };
        let svc = service_with(backend);

        let mut stream = svc.generate_stream_detailed(None, "ping", None, None, None, None, None);
        while let Some(chunk) = stream.next().await {
            chunk.expect("stream chunk");
        }

        let captured = captured.lock().unwrap();
        assert_eq!(captured[0].temperature, 1.0);
        assert_eq!(captured[0].model, "mock-model");
        assert!(captured[0].system.is_none());
    }

    #[tokio::test]
    async fn generate_stream_detailed_timeout_emits_err() {
        // 挂起后端 + 1s 超时 → 流以 Err(Timeout) 项终止（与 generate_stream 同一 unfold 包裹）
        let config = LlmServiceConfig {
            provider: LlmProvider::DeepSeek,
            api_key: "k".to_string(),
            model: "mock-model".to_string(),
            flash_model: "mock-flash".to_string(),
            base_url: Some("http://localhost:9".to_string()),
            timeout_seconds: 1,
            max_retries: 0,
            generation_params: crate::types::GenerationParams::default(),
            roles: std::collections::HashMap::new(),
        };
        let svc = LlmService {
            backend: Box::new(HangingBackend),
            config,
            vision_backend: None,
        };

        let mut stream = svc.generate_stream_detailed(None, "ping", None, None, None, None, None);
        let first = stream.next().await.expect("stream yields one item");
        assert!(matches!(first, Err(BackendError::Timeout(1))));
        assert!(stream.next().await.is_none(), "timeout stream terminates");
    }

    fn service_with_roles(
        roles: std::collections::HashMap<ModelRole, crate::types::RoleModel>,
    ) -> (LlmService, Arc<Mutex<Vec<CompletionRequest>>>) {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let config = LlmServiceConfig {
            provider: LlmProvider::DeepSeek,
            api_key: "k".to_string(),
            model: "mock-model".to_string(),
            flash_model: "mock-flash".to_string(),
            base_url: Some("http://localhost:9".to_string()),
            timeout_seconds: 5,
            max_retries: 0,
            generation_params: crate::types::GenerationParams::default(),
            roles,
        };
        (
            LlmService {
                vision_backend: None,
                backend: Box::new(MockBackend {
                    responses: Mutex::new(vec![
                        CompletionResponse {
                            text: Some("ok".to_string()),
                            tool_calls: vec![],
                            usage: None,
                            raw: serde_json::json!({}),
                        },
                        CompletionResponse {
                            text: Some("ok".to_string()),
                            tool_calls: vec![],
                            usage: None,
                            raw: serde_json::json!({}),
                        },
                    ]),
                    captured: captured.clone(),
                }),
                config,
            },
            captured,
        )
    }

    #[tokio::test]
    async fn generate_for_role_uses_role_model_and_params() {
        use crate::types::{ModelRole, ReasoningEffort, RoleModel};
        let mut roles = std::collections::HashMap::new();
        roles.insert(
            ModelRole::Task,
            RoleModel {
                model: "role-task-model".to_string(),
                generation_params: crate::types::GenerationParams {
                    reasoning_effort: ReasoningEffort::Low,
                    max_tokens: 2048,
                    ..Default::default()
                },
            },
        );
        let (svc, captured) = service_with_roles(roles);
        svc.generate_for_role(ModelRole::Task, "hello")
            .await
            .unwrap();
        let captured = captured.lock().unwrap();
        assert_eq!(captured[0].model, "role-task-model");
        assert_eq!(
            captured[0].reasoning_effort,
            Some(crate::types::ReasoningEffort::Low)
        );
        assert_eq!(captured[0].max_tokens, 2048);
    }

    #[tokio::test]
    async fn role_api_explicit_overrides_win() {
        use crate::types::{ModelRole, ReasoningEffort, RoleModel};
        let mut roles = std::collections::HashMap::new();
        roles.insert(
            ModelRole::Task,
            RoleModel {
                model: "role-task-model".to_string(),
                generation_params: crate::types::GenerationParams {
                    reasoning_effort: ReasoningEffort::Low,
                    ..Default::default()
                },
            },
        );
        let (svc, captured) = service_with_roles(roles);
        svc.generate_with_system_preamble_for_role(
            ModelRole::Task,
            Some("sys"),
            "hello",
            Some(0.9),
            Some(4096),
            Some("high"),
            Some("json_object"),
        )
        .await
        .unwrap();
        let captured = captured.lock().unwrap();
        assert_eq!(captured[0].model, "role-task-model");
        assert_eq!(captured[0].temperature, 0.9);
        assert_eq!(captured[0].max_tokens, 4096);
        assert_eq!(
            captured[0].reasoning_effort,
            Some(crate::types::ReasoningEffort::High)
        );
        assert_eq!(captured[0].response_format.as_deref(), Some("json_object"));
        assert_eq!(captured[0].system.as_deref(), Some("sys"));
    }

    #[tokio::test]
    async fn inspect_image_uses_vision_role_and_attaches_image() {
        use crate::backends::ImageContent;
        use crate::types::{ModelRole, RoleModel};
        let mut roles = std::collections::HashMap::new();
        roles.insert(
            ModelRole::Vision,
            RoleModel {
                model: "vision-model".to_string(),
                generation_params: crate::types::GenerationParams::default(),
            },
        );
        let (svc, captured) = service_with_roles(roles);
        let img = ImageContent {
            mime: "image/png".to_string(),
            data: vec![1, 2, 3],
        };
        svc.inspect_image(&img, "看到什么?").await.unwrap();
        let captured = captured.lock().unwrap();
        assert_eq!(captured[0].model, "vision-model");
        assert_eq!(captured[0].prompt, "看到什么?");
        assert_eq!(captured[0].images.len(), 1);
        assert_eq!(captured[0].images[0].mime, "image/png");
        assert_eq!(captured[0].images[0].data, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn inspect_image_without_vision_backend_uses_main_backend() {
        use crate::backends::ImageContent;
        use crate::types::{ModelRole, RoleModel};
        let mut roles = std::collections::HashMap::new();
        roles.insert(
            ModelRole::Vision,
            RoleModel {
                model: "vision-model".to_string(),
                generation_params: crate::types::GenerationParams::default(),
            },
        );
        let (svc, captured) = service_with_roles(roles);
        // vision_backend = None（service_with_roles 构造）→ 回退主 backend
        assert!(svc.vision_backend.is_none());
        svc.inspect_image(
            &ImageContent {
                mime: "image/jpeg".to_string(),
                data: vec![9],
            },
            "q",
        )
        .await
        .unwrap();
        let captured = captured.lock().unwrap();
        assert_eq!(captured[0].model, "vision-model");
        assert_eq!(captured[0].images.len(), 1);
    }

    #[tokio::test]
    async fn role_api_falls_back_to_default_when_role_missing() {
        use crate::types::ModelRole;
        let (svc, captured) = service_with_roles(std::collections::HashMap::new());
        svc.generate_for_role(ModelRole::Slow, "hello")
            .await
            .unwrap();
        svc.generate_for_role(ModelRole::Default, "hello")
            .await
            .unwrap();
        let captured = captured.lock().unwrap();
        assert_eq!(captured[0].model, "mock-model");
        assert_eq!(captured[1].model, "mock-model");
    }
}
