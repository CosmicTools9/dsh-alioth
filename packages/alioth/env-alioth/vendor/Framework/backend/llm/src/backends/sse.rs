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

use super::{BackendError, StreamToolCallOutcome, TokenUsage, ToolCallResult};
use futures_util::StreamExt;
use serde::Deserialize;
use std::collections::BTreeMap;

/// SSE 流式帧（OpenAI 兼容：`data: {choices:[{delta:{content}}]}`）。
#[derive(Debug, Deserialize)]
struct SseChunk {
    choices: Vec<SseChoice>,
    /// 顶层 usage（多数 provider 在 `stream_options.include_usage` 的最终帧下发；
    /// 缺省不解析）
    #[serde(default)]
    usage: Option<SseUsage>,
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
    /// 工具调用增量分片（OpenAI 兼容：index 路由、arguments 跨 chunk 追加）
    #[serde(default)]
    tool_calls: Option<Vec<SseToolDelta>>,
}

/// `delta.tool_calls[]` 单元素增量
#[derive(Debug, Deserialize)]
struct SseToolDelta {
    /// 并发工具序号；缺省 0（单工具形态）
    index: Option<u32>,
    /// 首个分片携带调用 id
    id: Option<String>,
    function: Option<SseToolFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct SseToolFunctionDelta {
    /// 首个分片携带完整函数名（规范按整值下发，不追加）
    name: Option<String>,
    /// JSON 字符串分片——跨 chunk 追加拼接
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SseUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
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
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    spawn_sse_parser_with_finish_slot(response, slot)
}

/// `spawn_sse_parser` 的 finish_reason 槽位变体：provider 停止原因
/// （"length" = 输出达 max_tokens 上限被截断）在流结束/空产出守卫路径写回
/// `finish_slot`——供调用方在消费完整条流后判定截断（内容流正常结束时不额外
/// 发信号，避免污染文本通道）。
pub fn spawn_sse_parser_with_finish_slot(
    response: reqwest::Response,
    finish_slot: std::sync::Arc<std::sync::Mutex<Option<String>>>,
) -> tokio::sync::mpsc::Receiver<Result<String, BackendError>> {
    let slot_for_task = finish_slot.clone();
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
                    if let Some(fr) = finish_reason.take() {
                        if let Ok(mut slot) = slot_for_task.lock() {
                            *slot = Some(fr);
                        }
                    }
                    return;
                }
            }
        }
        if let Some(fr) = finish_reason.take() {
            if let Ok(mut slot) = slot_for_task.lock() {
                *slot = Some(fr);
            }
        }
        // 正常结束（[DONE] 或流耗尽）：从未产出非空 content → 空产出守卫
        if !got_content {
            let fr = slot_for_task
                .lock()
                .ok()
                .and_then(|s| s.clone())
                .unwrap_or_else(|| "unknown".to_string());
            let _ = tx.send(Err(BackendError::NoContent(fr))).await;
        }
    });
    rx
}

/// 单工具调用的分片缓冲（按 OpenAI `index` 路由）
#[derive(Debug, Default, Clone)]
struct ToolDeltaAccum {
    id: String,
    name: String,
    arguments: String,
}

/// 工具流式行的解析状态（parse_sse_tools_line 的增量载体）
#[derive(Debug, Default)]
struct SseToolsState {
    /// 是否产出过内容（content 或工具分片）——空产出守卫依据
    got_content: bool,
    finish_reason: Option<String>,
    /// index → 分片缓冲（BTreeMap：乱序 index 到达亦可最终按序收束）
    calls: BTreeMap<u32, ToolDeltaAccum>,
    /// 顶层 usage 帧（最终帧常见；缺失为 None）
    usage: Option<TokenUsage>,
}

/// 解析工具流式单行 SSE 数据（OpenAI `delta.tool_calls` 分片语义）：
/// - `index` 区分并发工具（缺省 0 = 单工具形态）；`function.arguments` 跨 chunk 追加；
/// - `id`/`function.name` 按首个分片落位（name 规范整值下发，不追加）；
/// - 顶层 `usage` 帧解析进状态；
/// - 任何工具分片均计为产出（纯工具调用流不触发空产出守卫）。
fn parse_sse_tools_line(line: &str, state: &mut SseToolsState) -> SseAction {
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
            state.finish_reason = choice.finish_reason;
        }
        if let Some(content) = choice.delta.content {
            if !content.is_empty() {
                state.got_content = true;
                chunks.push(content);
            }
        }
        if let Some(tool_calls) = choice.delta.tool_calls {
            for delta in tool_calls {
                let index = delta.index.unwrap_or(0);
                let entry = state.calls.entry(index).or_default();
                if let Some(id) = delta.id {
                    if !id.is_empty() && entry.id.is_empty() {
                        entry.id = id;
                    }
                }
                if let Some(fn_delta) = delta.function {
                    if let Some(name) = fn_delta.name {
                        if !name.is_empty() && entry.name.is_empty() {
                            entry.name = name;
                        }
                    }
                    if let Some(arguments) = fn_delta.arguments {
                        if !arguments.is_empty() {
                            entry.arguments.push_str(&arguments);
                        }
                    }
                }
                if entry.id.is_empty() && entry.name.is_empty() && entry.arguments.is_empty() {
                    // 空分片（tool_calls 结束帧常见形态）不计数
                } else {
                    state.got_content = true;
                }
            }
        }
    }
    if let Some(usage) = chunk.usage {
        state.usage = super::map_usage(usage.prompt_tokens, usage.completion_tokens);
    }
    if chunks.is_empty() {
        SseAction::None
    } else {
        SseAction::Chunks(chunks)
    }
}

/// 流结束/终止时把累积状态收束为 [`StreamToolCallOutcome`]。
/// index 序（BTreeMap）输出；id 缺失回退 `call_{index}`；arguments 按非流式
/// complete 路径同款约定解析（JSON 失败保留原始字符串）。
fn finalize_tool_state(state: &SseToolsState) -> StreamToolCallOutcome {
    let tool_calls = state
        .calls
        .iter()
        .map(|(index, accum)| ToolCallResult {
            id: if accum.id.is_empty() {
                format!("call_{}", index)
            } else {
                accum.id.clone()
            },
            name: accum.name.clone(),
            arguments: serde_json::from_str(&accum.arguments)
                .unwrap_or_else(|_| serde_json::Value::String(accum.arguments.clone())),
        })
        .collect();
    StreamToolCallOutcome {
        tool_calls,
        usage: state.usage,
        finish_reason: state.finish_reason.clone(),
    }
}

/// `spawn_sse_parser_with_finish_slot` 的工具调用变体：同一 OpenAI 兼容 SSE 流，
/// 除逐 content chunk 下发外，把 `delta.tool_calls` 分片就地累积、顶层 usage
/// 解析进 `outcome_slot`（流结束/终止后读取）。空产出守卫同文本变体——
/// 工具分片亦计为产出，纯工具调用流不误报。
pub fn spawn_sse_tools_parser(
    response: reqwest::Response,
    outcome_slot: std::sync::Arc<std::sync::Mutex<Option<StreamToolCallOutcome>>>,
) -> tokio::sync::mpsc::Receiver<Result<String, BackendError>> {
    let slot_for_task = outcome_slot.clone();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, BackendError>>(64);
    tokio::spawn(async move {
        let mut bytes_stream = response.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut closed = false;
        let mut state = SseToolsState::default();

        while let Some(byte_chunk) = bytes_stream.next().await {
            match byte_chunk {
                Ok(bytes) => {
                    buf.extend_from_slice(&bytes);
                    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        let line: Vec<u8> = buf.drain(..=pos).collect();
                        let line_str = String::from_utf8_lossy(&line);
                        match parse_sse_tools_line(&line_str, &mut state) {
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
                    if let Ok(mut slot) = slot_for_task.lock() {
                        *slot = Some(finalize_tool_state(&state));
                    }
                    return;
                }
            }
        }
        if let Ok(mut slot) = slot_for_task.lock() {
            *slot = Some(finalize_tool_state(&state));
        }
        // 正常结束（[DONE] 或流耗尽）：从未产出非空内容/工具分片 → 空产出守卫
        if !state.got_content {
            let fr = state
                .finish_reason
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let _ = tx.send(Err(BackendError::NoContent(fr))).await;
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

    // --- 工具调用流式解析（delta.tool_calls 分片累积） ---

    fn tools_state() -> SseToolsState {
        SseToolsState::default()
    }

    #[test]
    fn tool_fragmented_arguments_are_concatenated() {
        // OpenAI 标准工具流：id+name 首帧，arguments 按 JSON 字符串跨帧切片
        let mut st = tools_state();
        assert_eq!(
            parse_sse_tools_line(
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"query_document","arguments":""}}]},"finish_reason":null}]}"#,
                &mut st
            ),
            SseAction::None
        );
        parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"document_type\":"}}]},"finish_reason":null}]}"#,
            &mut st,
        );
        parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"PO\"}"}}]},"finish_reason":null}]}"#,
            &mut st,
        );
        parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            &mut st,
        );
        assert!(
            st.got_content,
            "工具分片必须计为产出（纯工具流不触发空产出守卫）"
        );
        assert_eq!(st.finish_reason.as_deref(), Some("tool_calls"));
        let outcome = finalize_tool_state(&st);
        assert_eq!(outcome.tool_calls.len(), 1);
        let call = &outcome.tool_calls[0];
        assert_eq!(call.id, "call_1");
        assert_eq!(call.name, "query_document");
        assert_eq!(call.arguments["document_type"], "PO");
    }

    #[test]
    fn tool_multi_index_routes_fragments_and_sorts_out_of_order() {
        // 并发双工具：index 0/1 交错到达（乱序 index 亦按最终 index 序收束）
        let mut st = tools_state();
        parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_b","function":{"name":"query_schema","arguments":"{\"table_name\":"}}]},"finish_reason":null}]}"#,
            &mut st,
        );
        parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"query_sql","arguments":"{\"sql\":"}}]},"finish_reason":null}]}"#,
            &mut st,
        );
        parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"\"orders\"}"}}]},"finish_reason":null}]}"#,
            &mut st,
        );
        parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"SELECT 1\"}"}}]},"finish_reason":null}]}"#,
            &mut st,
        );
        let outcome = finalize_tool_state(&st);
        assert_eq!(outcome.tool_calls.len(), 2, "并发工具各自独立累积");
        assert_eq!(outcome.tool_calls[0].id, "call_a");
        assert_eq!(outcome.tool_calls[0].name, "query_sql");
        assert_eq!(outcome.tool_calls[0].arguments["sql"], "SELECT 1");
        assert_eq!(outcome.tool_calls[1].id, "call_b");
        assert_eq!(outcome.tool_calls[1].name, "query_schema");
        assert_eq!(outcome.tool_calls[1].arguments["table_name"], "orders");
    }

    #[test]
    fn tool_without_index_treats_as_single_call() {
        // 无 index 分片 → 路由 index 0（单工具形态）
        let mut st = tools_state();
        parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"id":"call_1","function":{"name":"validate_form","arguments":"{\"x\":1"}}]},"finish_reason":null}]}"#,
            &mut st,
        );
        parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"}"}}]},"finish_reason":null}]}"#,
            &mut st,
        );
        let outcome = finalize_tool_state(&st);
        assert_eq!(outcome.tool_calls.len(), 1);
        assert_eq!(outcome.tool_calls[0].id, "call_1");
        assert_eq!(outcome.tool_calls[0].arguments["x"], 1);
    }

    #[test]
    fn tool_name_in_later_frame_and_duplicate_id_idempotent() {
        // 分片形态：id 帧与 name 帧分离；重复 id/name 帧幂等（不追加不重复）
        let mut st = tools_state();
        parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_9"}]},"finish_reason":null}]}"#,
            &mut st,
        );
        parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_9","function":{"name":"send_notification"}}]},"finish_reason":null}]}"#,
            &mut st,
        );
        parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"send_notification"}}]},"finish_reason":null}]}"#,
            &mut st,
        );
        let outcome = finalize_tool_state(&st);
        assert_eq!(outcome.tool_calls.len(), 1);
        assert_eq!(outcome.tool_calls[0].id, "call_9");
        assert_eq!(
            outcome.tool_calls[0].name, "send_notification",
            "重复 name 帧不得拼接"
        );
    }

    #[test]
    fn tool_pure_text_stream_yields_zero_tool_calls() {
        // 纯文本流（含思考帧）：零 tool_calls、usage 帧解析、finish 捕获
        let mut st = tools_state();
        let action = parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{"content":"你好"},"finish_reason":null}]}"#,
            &mut st,
        );
        assert_eq!(action, SseAction::Chunks(vec!["你好".to_string()]));
        parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            &mut st,
        );
        parse_sse_tools_line(
            r#"data: {"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":34,"total_tokens":46}}"#,
            &mut st,
        );
        let outcome = finalize_tool_state(&st);
        assert!(outcome.tool_calls.is_empty(), "纯文本流必须零 tool_calls");
        assert_eq!(outcome.finish_reason.as_deref(), Some("stop"));
        let usage = outcome.usage.expect("usage 帧必须解析");
        assert_eq!(usage.input_tokens, 12);
        assert_eq!(usage.output_tokens, 34);
    }

    #[test]
    fn tool_bad_arguments_fallback_keeps_raw_string() {
        // arguments 非 JSON（空串/截断）：与非流式 complete 路径同款回退原串
        let mut st = tools_state();
        parse_sse_tools_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"t"}}]},"finish_reason":null}]}"#,
            &mut st,
        );
        let outcome = finalize_tool_state(&st);
        assert_eq!(outcome.tool_calls.len(), 1);
        assert_eq!(outcome.tool_calls[0].id, "c1");
        assert_eq!(
            outcome.tool_calls[0].arguments,
            serde_json::Value::String(String::new()),
            "空 arguments 回退为原字符串"
        );
    }
}
