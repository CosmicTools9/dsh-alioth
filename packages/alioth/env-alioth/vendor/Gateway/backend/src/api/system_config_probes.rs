//! system-config 连通性探测器实现（宿主侧：llm / email / amap）
//!
//! 注入 crate `ProbeRegistry`（change unify-provider-probe）；探测**不产生副作用**
//! （LLM 走 `verify` 探针、SMTP 走 `test_connection`，均不发信不落库）。
//!
//! - `LlmProbe`：复用唯一装配实现 `build_llm_service` + `LlmService::verify`
//!   （探测固定参数 timeout=30 / max_retries=0，与旧 llm/test 及 Meta verify 同语义）；
//! - `EmailProbe`：`common::email::probe_connection`（SMTP `test_connection`）；
//! - `AmapProbe`：高德 restapi geocode 轻探（amap 凭据落 settings 明文，见 schema
//!   `amap_category`；10s 超时单发不重试——取代旧前端直连，change
//!   amap-system-config-category）。
//!
//! 成功时 `message` = 需 FE 本地化承载的数据（llm = 模型名，email 为空串）；
//! 失败时 `error` = 技术诊断串（可含上游错误文本）。

use async_trait::async_trait;
use serde_json::Value;
use system_config::probe::{ProbeOutcome, ProviderProbe};

use crate::api::chat_sessions::adapters::db_llm_config::build_llm_service;

/// LLM 连通性探测器（`POST /system-config/test/llm`）
pub struct LlmProbe;

#[async_trait]
impl ProviderProbe for LlmProbe {
    async fn probe(&self, provider: &str, settings: &Value, credentials: &Value) -> ProbeOutcome {
        let api_key = credentials
            .get("api_key")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        // 探测固定参数：30s 超时 / 0 重试（与 Meta /api/meta/llm-config/verify 同语义）
        let mut probe_settings = settings.clone();
        if let Some(obj) = probe_settings.as_object_mut() {
            obj.insert("timeout".into(), serde_json::json!(30));
            obj.insert("max_retries".into(), serde_json::json!(0));
        }

        let service = match build_llm_service(provider, api_key, Some(&probe_settings), None) {
            Ok(s) => s,
            Err(e) => return ProbeOutcome::fail(format!("[服务构建] {e}")),
        };
        match service.verify().await {
            // 成功 message = 模型名（FE 以 i18n 模板渲染；空模型名时 FE 走通用成功文案）
            Ok(()) => {
                let model = settings.get("model").and_then(|v| v.as_str()).unwrap_or("");
                ProbeOutcome::ok(model)
            }
            Err(e) => {
                common::telemetry::warn!("LLM 连通性探测失败: {e}");
                ProbeOutcome::fail(format!("[连接验证] {e}"))
            }
        }
    }
}

/// SMTP 邮件连通性探测器（`POST /system-config/test/email`）
pub struct EmailProbe;

#[async_trait]
impl ProviderProbe for EmailProbe {
    async fn probe(&self, _provider: &str, settings: &Value, credentials: &Value) -> ProbeOutcome {
        let cfg: common::email::EmailConfig = match serde_json::from_value(settings.clone()) {
            Ok(c) => c,
            Err(e) => return ProbeOutcome::fail(format!("[配置解析] {e}")),
        };
        let password = credentials
            .get("password")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if password.is_empty() {
            return ProbeOutcome::fail(
                "[配置解析] enc_fields.password 缺失（编辑态请留空走已存凭证）",
            );
        }
        match common::email::probe_connection(&cfg, password).await {
            Ok(()) => ProbeOutcome::ok(""),
            Err(e) => {
                common::telemetry::warn!("SMTP 连通性探测失败: {e}");
                ProbeOutcome::fail(format!("[连接验证] {e}"))
            }
        }
    }
}

/// 高德地图连通性探测器（`POST /system-config/test/amap`）
///
/// amap 的 key/secret 落 `settings` 明文（非 `enc_fields`，构建期消费链直读），故从
/// `settings` 取 key。判定口径沿用旧前端直连卡：`status=1` 成功；
/// `USERKEY_PLAT_NOMATCH` = Web 端 JSAPI Key 调 restapi 的平台不匹配——key 本身
/// 可用于地图，按成功计。
pub struct AmapProbe;

#[async_trait]
impl ProviderProbe for AmapProbe {
    async fn probe(&self, _provider: &str, settings: &Value, _credentials: &Value) -> ProbeOutcome {
        let key = settings
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim();
        if key.is_empty() {
            return ProbeOutcome::fail("[配置解析] settings.key 缺失");
        }
        let url = format!(
            "https://restapi.amap.com/v3/geocode/geo?key={}&address={}",
            urlencode(key),
            urlencode("北京市朝阳区"),
        );
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(e) => return ProbeOutcome::fail(format!("[客户端构建] {e}")),
        };
        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                common::telemetry::warn!("amap 连通性探测失败: {e}");
                return ProbeOutcome::fail(format!("[连接验证] {e}"));
            }
        };
        let data: Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => return ProbeOutcome::fail(format!("[响应解析] {e}")),
        };
        let status = data.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let info = data.get("info").and_then(|v| v.as_str()).unwrap_or("");
        let infocode = data.get("infocode").and_then(|v| v.as_str()).unwrap_or("");
        if status == "1" {
            let count = data
                .get("geocodes")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            ProbeOutcome::ok(format!("{count}"))
        } else if infocode == "USERKEY_PLAT_NOMATCH" || info == "USERKEY_PLAT_NOMATCH" {
            // Web 端 JSAPI Key 调 restapi 报平台不匹配——key 本身可用于地图
            ProbeOutcome::ok("jskey")
        } else {
            let reason = if info.is_empty() {
                "高德返回失败"
            } else {
                info
            };
            ProbeOutcome::fail(format!("[连接验证] {reason}"))
        }
    }
}

/// query 键值百分号编码（受控 ASCII/UTF-8 简单值；逐字节编码，零正则）。
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
