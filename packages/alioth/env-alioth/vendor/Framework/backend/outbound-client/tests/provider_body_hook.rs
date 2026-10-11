//! provider `prepare_body` 请求体注入钩子——端到端接线契约测试
//! （change `migrate-image-sender-to-provider-seam`；规约
//! `outbound-provider-abstraction :: provider-abstract-contract`）。
//!
//! 独立进程（provider registry 为进程级 OnceLock，只能在独占进程安装）：安装两类测试
//! provider + 本地 HTTP stub，断言：
//! 1. 覆写 `prepare_body` 的 provider：**实际发送字节 = 变换后体**，且 `authorize` 收到的
//!    `RequestInfo.body_text` 为该最终字节（调用序：prepare_body → 序列化 → authorize）；
//! 2. 默认（未覆写）provider：请求体逐字节不变（既有 provider 零漂移）；
//! 3. `auth_type='none'`：钩子不参与（none 行不要求 provider 装载）。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use async_trait::async_trait;
use common::AliothError as ApiError;
use outbound_client::provider::{self, OutboundProvider, ProviderContext, RequestInfo};
use outbound_client::{OutboundClient, OutboundClientConfig};
use serde_json::{json, Value};
use tokio::sync::Mutex;

/// 本地 HTTP 应答 stub：顺序接收 `expect` 个请求，body 经 channel 送回，回 `{"ok":true}`。
fn spawn_stub(expect: usize) -> (String, std::sync::mpsc::Receiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = std::sync::mpsc::channel::<Value>();
    std::thread::spawn(move || {
        for _ in 0..expect {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf: Vec<u8> = Vec::new();
            let mut tmp = [0u8; 4096];
            let (body_start, content_len) = loop {
                if let Some(pos) = find(&buf, b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    break (pos + 4, len);
                }
                let n = stream.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break (0, 0);
                }
                buf.extend_from_slice(&tmp[..n]);
            };
            while buf.len() < body_start + content_len {
                let n = stream.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let body_end = (body_start + content_len).min(buf.len());
            let body: Value =
                serde_json::from_slice(&buf[body_start..body_end]).unwrap_or(Value::Null);
            let _ = tx.send(body);
            let resp = br#"{"ok":true}"#;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                resp.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(resp);
            let _ = stream.flush();
        }
    });
    (format!("http://{addr}"), rx)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// 覆写 `prepare_body` 的探针 provider：记录 `authorize` 看到的 `body_text`。
struct HookProbe {
    seen_body_text: Arc<Mutex<Option<String>>>,
}

#[async_trait]
impl OutboundProvider for HookProbe {
    fn provider_id(&self) -> &'static str {
        "hook-probe"
    }

    async fn prepare_body(
        &self,
        _ctx: &ProviderContext<'_>,
        _method: &str,
        _url: &str,
        body: Value,
    ) -> Result<Value, ApiError> {
        Ok(json!({"marker": "H", "params": body}))
    }

    async fn authorize(
        &self,
        _ctx: ProviderContext<'_>,
        req: &RequestInfo<'_>,
    ) -> Result<Vec<(String, String)>, ApiError> {
        *self.seen_body_text.lock().await = req.body_text.map(str::to_string);
        Ok(vec![("x-probe".to_string(), "1".to_string())])
    }
}

/// 默认（未覆写 `prepare_body`）的探针 provider。
struct PlainProbe;

#[async_trait]
impl OutboundProvider for PlainProbe {
    fn provider_id(&self) -> &'static str {
        "plain-probe"
    }
}

fn client(provider_id: &str, auth_type: &str, base: &str) -> OutboundClient {
    OutboundClient::new(OutboundClientConfig {
        provider_id: provider_id.to_string(),
        auth_type: auth_type.to_string(),
        base_url: base.to_string(),
        config: Value::Object(Default::default()),
        secret: Some("s".to_string()),
        mock: false,
    })
}

#[tokio::test]
async fn body_hook_transforms_before_send_and_authorize_sees_final_bytes() {
    let seen_body_text = Arc::new(Mutex::new(None));
    provider::install(
        provider::ProviderRegistry::new()
            .with(Arc::new(HookProbe {
                seen_body_text: seen_body_text.clone(),
            }))
            .expect("register hook-probe")
            .with(Arc::new(PlainProbe))
            .expect("register plain-probe"),
    )
    .expect("install registry（本进程独占）");

    let (base, rx) = spawn_stub(3);
    let wait = std::time::Duration::from_secs(10);

    // 1) 覆写钩子：发送体 = 变换体；authorize 见最终字节
    let c = client("hook-probe", "hook-mode", &base);
    let resp = c
        .post_json(&format!("{base}/x"), &[], json!({"a": 1}), false, "t:hook")
        .await
        .expect("hook 请求应成功");
    assert_eq!(resp, json!({"ok": true}));
    let sent: Value = rx.recv_timeout(wait).expect("stub 收到 body");
    assert_eq!(sent, json!({"marker": "H", "params": {"a": 1}}));
    let authorized_text = seen_body_text
        .lock()
        .await
        .clone()
        .expect("authorize 应收到 body_text");
    assert_eq!(
        serde_json::from_str::<Value>(&authorized_text).expect("body_text 为 JSON"),
        json!({"marker": "H", "params": {"a": 1}}),
        "authorize 收到的 body_text MUST = 实际发送字节"
    );

    // 2) 默认钩子：逐字节不变
    let c = client("plain-probe", "plain-mode", &base);
    c.post_json(&format!("{base}/y"), &[], json!({"b": 2}), false, "t:plain")
        .await
        .expect("plain 请求应成功");
    let sent: Value = rx.recv_timeout(wait).expect("stub 收到 body");
    assert_eq!(sent, json!({"b": 2}), "默认 prepare_body MUST 原样透传");

    // 3) none：钩子不参与（provider_id 可为空、无需装载）
    let c = client("", provider::AUTH_TYPE_NONE, &base);
    c.post_json(&format!("{base}/z"), &[], json!({"c": 3}), false, "t:none")
        .await
        .expect("none 请求应成功");
    let sent: Value = rx.recv_timeout(wait).expect("stub 收到 body");
    assert_eq!(sent, json!({"c": 3}));
}
