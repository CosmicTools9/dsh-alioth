//! Provider 连通性探测 seam
//!
//! 探测实现（各类别消费者侧 builder：llm / email / …）**由宿主注入**——crate 不感知
//! 消费者 crate（依赖方向：`system-config` → `common`，MUST NOT 反向）。HTTP 面 =
//! scope 内单一路由 `POST /system-config/test/{category}`（MUST NOT 依赖宿主级注册顺序）。
//!
//! 能力位与注册表双向一致：`ConfigCategory.supports_probe`（schema）为真 ⇔ 宿主注册了探测器；
//! 一致性由测试钉住（`framework` 面：schema 单测 + 宿主装配测试）。

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// 探测结果（统一响应形状；`latency_ms` 由 handler 统一测时）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProbeOutcome {
    pub success: bool,
    pub latency_ms: u64,
    /// 成功信息（如「模型 X 连接成功」）；失败时可为空
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ProbeOutcome {
    /// 成功（latency 由 handler 回填）
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            success: true,
            latency_ms: 0,
            message: message.into(),
            error: None,
        }
    }

    /// 失败（latency 由 handler 回填）
    pub fn fail(error: impl Into<String>) -> Self {
        Self {
            success: false,
            latency_ms: 0,
            message: String::new(),
            error: Some(error.into()),
        }
    }
}

/// 单类别探测器（宿主实现；`credentials` 已解密或为请求期草稿明文）
#[async_trait]
pub trait ProviderProbe: Send + Sync + 'static {
    /// 探测给定 provider 配置的连通性（MUST NOT 产生副作用——不发信、不落库）
    async fn probe(&self, provider: &str, settings: &Value, credentials: &Value) -> ProbeOutcome;
}

/// 类别 → 探测器（宿主装配；clone 廉价——Arc 共享）
#[derive(Clone, Default)]
pub struct ProbeRegistry {
    probes: Arc<HashMap<String, Arc<dyn ProviderProbe>>>,
}

impl ProbeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册探测器（builder 风格；重复类别后者覆盖）
    pub fn with(self, category: impl Into<String>, probe: impl ProviderProbe) -> Self {
        let mut map = (*self.probes).clone();
        map.insert(category.into(), Arc::new(probe));
        Self {
            probes: Arc::new(map),
        }
    }

    pub fn get(&self, category: &str) -> Option<Arc<dyn ProviderProbe>> {
        self.probes.get(category).cloned()
    }

    pub fn supports(&self, category: &str) -> bool {
        self.probes.contains_key(category)
    }

    pub fn is_empty(&self) -> bool {
        self.probes.is_empty()
    }
}

impl std::fmt::Debug for ProbeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProbeRegistry")
            .field("categories", &self.probes.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AlwaysOk;

    #[async_trait]
    impl ProviderProbe for AlwaysOk {
        async fn probe(
            &self,
            _provider: &str,
            _settings: &Value,
            _credentials: &Value,
        ) -> ProbeOutcome {
            ProbeOutcome::ok("ok")
        }
    }

    struct AlwaysFail;

    #[async_trait]
    impl ProviderProbe for AlwaysFail {
        async fn probe(
            &self,
            _provider: &str,
            _settings: &Value,
            _credentials: &Value,
        ) -> ProbeOutcome {
            ProbeOutcome::fail("boom")
        }
    }

    #[test]
    fn registry_lookup_and_capability() {
        let registry = ProbeRegistry::new()
            .with("llm", AlwaysOk)
            .with("email", AlwaysFail);
        assert!(registry.supports("llm"));
        assert!(registry.supports("email"));
        assert!(!registry.supports("sms"));
        assert!(registry.get("sms").is_none());
    }

    #[tokio::test]
    async fn probe_outcomes_shape() {
        let ok = AlwaysOk.probe("p", &Value::Null, &Value::Null).await;
        assert!(ok.success && ok.error.is_none() && ok.latency_ms == 0);
        let fail = AlwaysFail.probe("p", &Value::Null, &Value::Null).await;
        assert!(!fail.success && fail.error.as_deref() == Some("boom"));
    }
}
