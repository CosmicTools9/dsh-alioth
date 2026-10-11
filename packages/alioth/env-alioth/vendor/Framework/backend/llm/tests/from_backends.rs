//! 外部 crate 视角的可替身构造验收（finding 2026-10-03 修复面）：
//! 仅经公开 API（`LlmService::from_backends` + `LlmBackend` 端口）构造服务，
//! 无 env、无网络即可驱动生成与视觉分析。

use async_trait::async_trait;
use llm::backends::{BackendError, CompletionRequest, CompletionResponse, LlmBackend};
use llm::{GenerationParams, LlmProvider, LlmService, LlmServiceConfig};
use std::sync::Arc;
use tokio::sync::Mutex;

/// 最小替身后端：固定文本 + 记录收到的请求（含 images）。
struct StubBackend {
    text: String,
    seen: Arc<Mutex<Vec<CompletionRequest>>>,
}

#[async_trait]
impl LlmBackend for StubBackend {
    fn provider_name(&self) -> &'static str {
        "stub"
    }

    fn default_model(&self) -> &str {
        "stub-model"
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, BackendError> {
        self.seen.lock().await.push(req);
        Ok(CompletionResponse {
            text: Some(self.text.clone()),
            tool_calls: Vec::new(),
            usage: None,
            finish_reason: Some("stop".to_string()),
            raw: serde_json::json!({}),
        })
    }
}

fn config() -> LlmServiceConfig {
    LlmServiceConfig {
        provider: LlmProvider::DeepSeek,
        api_key: "test-key".to_string(),
        model: "stub-model".to_string(),
        flash_model: "stub-flash".to_string(),
        base_url: None,
        timeout_seconds: 5,
        max_retries: 0,
        generation_params: GenerationParams::default(),
        roles: std::collections::HashMap::new(),
    }
}

fn stub(text: &str, seen: &Arc<Mutex<Vec<CompletionRequest>>>) -> StubBackend {
    StubBackend {
        text: text.to_string(),
        seen: seen.clone(),
    }
}

#[tokio::test]
async fn generate_routes_through_injected_backend() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let svc = LlmService::from_backends(Box::new(stub("primary-ok", &seen)), config(), None);
    assert_eq!(svc.generate("hi").await.expect("生成应成功"), "primary-ok");
    let reqs = seen.lock().await;
    assert_eq!(reqs.len(), 1, "注入后端应收到请求");
    assert_eq!(reqs[0].prompt, "hi");
}

#[tokio::test]
async fn inspect_image_prefers_injected_vision_backend() {
    let primary_seen = Arc::new(Mutex::new(Vec::new()));
    let vision_seen = Arc::new(Mutex::new(Vec::new()));
    let svc = LlmService::from_backends(
        Box::new(stub("primary", &primary_seen)),
        config(),
        Some(Box::new(stub("vision-ok", &vision_seen))),
    );
    let image = llm::ImageContent {
        mime: "image/png".to_string(),
        data: vec![1, 2, 3],
    };
    let out = svc
        .inspect_image(&image, "描述这张图")
        .await
        .expect("视觉分析应成功");
    assert_eq!(out, "vision-ok");
    let vision_reqs = vision_seen.lock().await;
    assert_eq!(vision_reqs.len(), 1, "视觉后端应收到请求");
    assert_eq!(vision_reqs[0].images.len(), 1, "图像应附加到请求");
    assert!(primary_seen.lock().await.is_empty(), "主后端不应被触碰");
}

#[tokio::test]
async fn inspect_image_falls_back_to_primary_without_vision_backend() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let svc = LlmService::from_backends(Box::new(stub("primary-only", &seen)), config(), None);
    let image = llm::ImageContent {
        mime: "image/jpeg".to_string(),
        data: vec![9],
    };
    let out = svc.inspect_image(&image, "q").await.expect("回落主后端");
    assert_eq!(out, "primary-only");
    assert_eq!(seen.lock().await.len(), 1, "缺省视觉回落主后端");
}
