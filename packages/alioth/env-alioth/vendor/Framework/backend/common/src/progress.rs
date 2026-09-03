//! 进度事件基础设施 — 为长任务提供 SSE 流式进度推送

use actix_web::{web::Bytes, HttpResponse};
use futures::stream::Stream;
use serde::Serialize;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;

/// 进度事件
#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub step: String,
    pub label: String,
    pub detail: String,
    pub progress: u8,
    #[serde(rename = "severity")]
    pub severity: String,
}

impl ProgressEvent {
    pub fn info(
        step: impl Into<String>,
        label: impl Into<String>,
        detail: impl Into<String>,
        progress: u8,
    ) -> Self {
        Self {
            step: step.into(),
            label: label.into(),
            detail: detail.into(),
            progress,
            severity: "info".into(),
        }
    }
    pub fn success(
        step: impl Into<String>,
        label: impl Into<String>,
        detail: impl Into<String>,
        progress: u8,
    ) -> Self {
        Self {
            step: step.into(),
            label: label.into(),
            detail: detail.into(),
            progress,
            severity: "success".into(),
        }
    }
    pub fn error(
        step: impl Into<String>,
        label: impl Into<String>,
        detail: impl Into<String>,
        progress: u8,
    ) -> Self {
        Self {
            step: step.into(),
            label: label.into(),
            detail: detail.into(),
            progress,
            severity: "error".into(),
        }
    }
}

/// 创建进度通道
pub fn progress_channel(
    buffer: usize,
) -> (mpsc::Sender<ProgressEvent>, mpsc::Receiver<ProgressEvent>) {
    mpsc::channel(buffer)
}

/// SSE 事件序列化
fn format_sse(event_type: &str, data: &str) -> String {
    format!("event: {}\ndata: {}\n\n", event_type, data)
}

/// 将 ProgressEvent 流转换为 SSE 字节流
pub fn sse_stream(rx: mpsc::Receiver<ProgressEvent>) -> SseStream {
    SseStream {
        inner: rx,
        done: false,
    }
}

pub struct SseStream {
    inner: mpsc::Receiver<ProgressEvent>,
    done: bool,
}

impl Stream for SseStream {
    type Item = Result<Bytes, actix_web::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }
        match self.inner.poll_recv(cx) {
            Poll::Ready(Some(event)) => {
                let json = serde_json::to_string(&event).unwrap_or_default();
                let chunk = format_sse("progress", &json);
                Poll::Ready(Some(Ok(Bytes::from(chunk))))
            }
            Poll::Ready(None) => {
                self.done = true;
                Poll::Ready(Some(Ok(Bytes::from(format_sse(
                    "complete",
                    r#"{"status":"done"}"#,
                )))))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// 构建 SSE HttpResponse（长任务流式响应）
pub fn sse_response(rx: mpsc::Receiver<ProgressEvent>) -> HttpResponse {
    HttpResponse::Ok()
        .insert_header(("content-type", "text/event-stream"))
        .insert_header(("cache-control", "no-cache"))
        .insert_header(("x-accel-buffering", "no"))
        .streaming(sse_stream(rx))
}
