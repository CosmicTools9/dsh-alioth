//! 共享 SSE 流式解析（OpenAI 兼容后端：DeepSeek / Kimi / MiniMax）。
//!
//! 三个后端请求/响应结构同构（`choices[].delta.content`），SSE 解析逻辑
//! 抽为共享模块，避免三份重复实现（REUSE_FIRST_SPEC §3）。
//!
//! 空产出守卫（fix-chat-ai-empty-reply）：thinking 模型（DeepSeek v4 默认
//! 开启、effort 实际 high）思考耗尽 max_tokens 预算时，流仅含
//! `reasoning_content` 帧并以正常 finish 结束——content 零字节。静默成功
//! 会沿 orchestrator → WS/HTTP → 前端传播成空白气泡，因此流结束时从未
//! 产出非空 content → 以 `Err(BackendError::NoContent(finish_reason))` 终止。

use super::BackendError;
use futures_util::StreamExt;
use serde::Deserialize;

/// SSE 流式帧（OpenAI 兼容：`data: {choices:[{delta:{content}}]}`）。
#[derive(Debug, Deserialize)]
struct SseChunk {
    choices: Vec<SseChoice>,
}

#[derive(Debug, Deserialize)]
struct SseChoice {
    delta: SseDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SseDelta {
    #[serde(default)]
    content: Option<String>,
}

/// 单行 SSE 数据的解析动作（`parse_sse_line` 返回值）。
#[derive(Debug, PartialEq, Eq)]
enum SseAction {
    /// 非数据行 / 无法解析的帧——忽略
    None,
    /// `data: [DONE]`——正常结束
    Done,
    /// 需要下发消费者的 content chunk（保序，通常单元素）
    Chunks(Vec<String>),
}

/// 解析单行 SSE 数据，更新 `got_content` / `finish_reason` 追踪状态。
///
/// 纯函数（无 IO）——便于对空产出守卫做穷举单测。
fn parse_sse_line(
    line: &str,
    got_content: &mut bool,
    finish_reason: &mut Option<String>,
) -> SseAction {
    let line = line.trim();
    if line.is_empty() || !line.starts_with("data:") {
        return SseAction::None;
    }
    let data = line.trim_start_matches("data:").trim();
    if data == "[DONE]" {
        return SseAction::Done;
    }
    let Ok(chunk) = serde_json::from_str::<SseChunk>(data) else {
        return SseAction::None;
    };
    let mut chunks = Vec::new();
    for choice in chunk.choices {
        if choice.finish_reason.is_some() {
            *finish_reason = choice.finish_reason;
        }
        if let Some(content) = choice.delta.content {
            if !content.is_empty() {
                *got_content = true;
                chunks.push(content);
            }
        }
    }
    if chunks.is_empty() {
        SseAction::None
    } else {
        SseAction::Chunks(chunks)
    }
}

/// 将 reqwest Response（已确认 2xx）的 bytes stream 解析为逐 content chunk 的
/// tokio channel。
///
/// 生产者：按 `\n` 行处理，`data: {...}` 解析 delta.content 逐条 `tx.send(Ok(..))`；
/// `data: [DONE]` 或流结束 → 若从未产出非空 content，以
/// `Err(BackendError::NoContent)` 终止（空产出守卫）；否则关闭 channel（正常结束）。
/// 传输错误以 `Err` 项发出。HTTP 非 2xx 由调用方在调用本函数前处理。
pub fn spawn_sse_parser(
    response: reqwest::Response,
) -> tokio::sync::mpsc::Receiver<Result<String, BackendError>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, BackendError>>(64);
    tokio::spawn(async move {
        let mut bytes_stream = response.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut closed = false;
        let mut got_content = false;
        let mut finish_reason: Option<String> = None;

        while let Some(byte_chunk) = bytes_stream.next().await {
            match byte_chunk {
                Ok(bytes) => {
                    buf.extend_from_slice(&bytes);
                    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        let line: Vec<u8> = buf.drain(..=pos).collect();
                        let line_str = String::from_utf8_lossy(&line);
                        match parse_sse_line(&line_str, &mut got_content, &mut finish_reason) {
                            SseAction::None => {}
                            SseAction::Done => {
                                closed = true;
                                break;
                            }
                            SseAction::Chunks(chunks) => {
                                for content in chunks {
                                    if tx.send(Ok(content)).await.is_err() {
                                        closed = true;
                                        break;
                                    }
                                }
                            }
                        }
                        if closed {
                            break;
                        }
                    }
                    if closed {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(BackendError::Transport(e.to_string()))).await;
                    return;
                }
            }
        }
        // 正常结束（[DONE] 或流耗尽）：从未产出非空 content → 空产出守卫
        if !got_content {
            let _ = tx
                .send(Err(BackendError::NoContent(
                    finish_reason.unwrap_or_else(|| "unknown".to_string()),
                )))
                .await;
        }
    });
    rx
}

/// 供 `complete_stream` 覆写复用的 channel→BoxStream 转换。
pub fn channel_to_stream(
    rx: tokio::sync::mpsc::Receiver<Result<String, BackendError>>,
) -> futures_util::stream::BoxStream<'static, Result<String, BackendError>> {
    futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    })
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> (SseAction, bool, Option<String>) {
        let mut got_content = false;
        let mut finish_reason = None;
        let action = parse_sse_line(line, &mut got_content, &mut finish_reason);
        (action, got_content, finish_reason)
    }

    #[test]
    fn content_chunk_updates_got_content() {
        let (action, got, fr) =
            parse(r#"data: {"choices":[{"delta":{"content":"你好"},"finish_reason":null}]}"#);
        assert_eq!(
            action,
            SseAction::Chunks(vec!["你好".to_string()]),
            "content chunk 必须原样下发"
        );
        assert!(got, "收到 content 后 got_content 必须置位");
        assert_eq!(fr, None);
    }

    #[test]
    fn reasoning_only_frame_is_ignored_but_captures_finish_reason() {
        // thinking 模型：思考帧只有 reasoning_content，无 content
        let (action, got, fr) = parse(
            r#"data: {"choices":[{"delta":{"reasoning_content":"thinking..."},"finish_reason":null}]}"#,
        );
        assert_eq!(action, SseAction::None, "reasoning 帧不得下发 content");
        assert!(!got, "reasoning 帧不得置位 got_content");
        assert_eq!(fr, None);

        // 结束帧：无 content、finish_reason=length（思考耗尽预算截断）
        let (action, got, fr) =
            parse(r#"data: {"choices":[{"delta":{},"finish_reason":"length"}]}"#);
        assert_eq!(action, SseAction::None);
        assert!(!got);
        assert_eq!(fr.as_deref(), Some("length"), "finish_reason 必须被捕获");
    }

    #[test]
    fn done_and_noise_lines() {
        assert_eq!(parse("data: [DONE]").0, SseAction::Done);
        assert_eq!(parse("").0, SseAction::None);
        assert_eq!(parse(": keep-alive").0, SseAction::None);
        assert_eq!(parse("data: not-json").0, SseAction::None, "坏帧忽略不致命");
    }

    #[test]
    fn empty_string_content_does_not_count_as_content() {
        // content="" 的帧（tool_calls 结束帧常见形态）不算产出
        let (action, got, _) =
            parse(r#"data: {"choices":[{"delta":{"content":""},"finish_reason":null}]}"#);
        assert_eq!(action, SseAction::None);
        assert!(!got, "空字符串 content 不得置位 got_content");
    }
}
