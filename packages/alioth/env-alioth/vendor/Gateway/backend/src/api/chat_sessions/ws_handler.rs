//! WebSocket handler for real-time AI chat.
//!
//! Accepts a JSON `{ "message": String }` per frame, stores the message,
//! generates an AI response via the orchestrator, and sends the
//! serialized `ChatMessageResponse` JSON back.

use super::orchestrator::{SessionOrchestrator, TurnInput};
use super::{build_orchestrator, extract_user_id};
use crate::i18n::I18nManagerRef;
use actix::{Actor, ActorContext, AsyncContext, Handler, Message as ActixMessage, StreamHandler};
use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use i18n::Locale;
use sqlx::PgPool;

#[derive(serde::Deserialize)]
struct WsIncoming {
    /// 帧类型：缺省 = 消息帧（兼容旧客户端）；"cancel" = 取消当前生成（D2.9）
    #[serde(rename = "type", default)]
    frame_type: Option<String>,
    message: Option<String>,
    /// Optional page/entity context, same semantics as CreateMessageRequest.context.
    context: Option<serde_json::Value>,
    /// 附件 [{type:"image",mime,data_base64}]（D2.6；S2 data_base64 唯一通道）
    attachments: Option<serde_json::Value>,
    /// 命中的知识引用 [{key,title}]（D2.15）
    knowledge_refs: Option<serde_json::Value>,
    /// 模型档位（chat 模型切换）："deep" | "flash"；缺省 = deep（主模型）
    model: Option<String>,
}

#[derive(ActixMessage)]
#[rtype(result = "()")]
struct ChatResult(String);

pub struct ChatWsSession {
    session_id: i64,
    pool: PgPool,
    i18n: I18nManagerRef,
    user_id: Option<i64>,
    locale: String,
    /// 当前轮生成取消信号（每消息帧重建；cancel 帧置位）
    cancel_tx: Option<tokio::sync::watch::Sender<bool>>,
    /// 心跳未收 Pong 计数（连续 2 次 → 断开，D2.10）
    pong_missed: u8,
}

/// 出向 typed 帧（D2.10 v2）：chunk / tool(start|end) / final / error。
/// 旧裸帧格式不再发送（前端 M3 同步升级，兼容逻辑在前端）。
fn error_frame(error: &str) -> String {
    serde_json::json!({ "type": "error", "error": error }).to_string()
}

fn final_frame(message: &super::ChatMessageResponse) -> String {
    let mut v = serde_json::to_value(message).unwrap_or_else(|_| serde_json::json!({}));
    if let serde_json::Value::Object(ref mut map) = v {
        map.insert("type".to_string(), serde_json::json!("final"));
    }
    v.to_string()
}

fn stream_frame(ev: super::orchestrator::TurnStreamEvent) -> String {
    match ev {
        super::orchestrator::TurnStreamEvent::Chunk(content) => {
            serde_json::json!({ "type": "chunk", "content": content }).to_string()
        }
        super::orchestrator::TurnStreamEvent::ToolStart { name, arguments } => serde_json::json!({
            "type": "tool",
            "event": "start",
            "name": name,
            "arguments": arguments
        })
        .to_string(),
        super::orchestrator::TurnStreamEvent::ToolEnd {
            name,
            success,
            output,
        } => serde_json::json!({
            "type": "tool",
            "event": "end",
            "name": name,
            "success": success,
            "output": output
        })
        .to_string(),
    }
}

impl Actor for ChatWsSession {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // D2.10 心跳：30s 间隔 ws Ping；连续 2 次未收 Pong（死连接）→ 断开。
        ctx.run_interval(std::time::Duration::from_secs(30), |act, ctx| {
            if act.pong_missed >= 2 {
                common::telemetry::info!("Gateway WS 心跳超时断开: session={}", act.session_id);
                ctx.stop();
                return;
            }
            act.pong_missed += 1;
            ctx.ping(b"heartbeat");
        });
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        // WS 断连：会话状态已由 orchestrator 每轮 turn 持久化（update_session_state），
        // 此处仅留痕——in-flight turn 的结果无法送达时，前端可重连后经 HTTP 拉取。
        common::telemetry::info!(
            "Gateway WS 断连: session={} user={:?}",
            self.session_id,
            self.user_id
        );
    }
}

impl Handler<ChatResult> for ChatWsSession {
    type Result = ();

    fn handle(&mut self, msg: ChatResult, ctx: &mut Self::Context) {
        ctx.text(msg.0);
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for ChatWsSession {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Text(text)) => {
                let body: WsIncoming = match serde_json::from_str(&text) {
                    Ok(b) => b,
                    Err(_) => {
                        ctx.text(error_frame("invalid_json"));
                        return;
                    }
                };

                // D2.9：cancel 帧 → 置位当前轮取消信号（无 in-flight 轮则空操作）
                if body.frame_type.as_deref() == Some("cancel") {
                    if let Some(tx) = &self.cancel_tx {
                        let _ = tx.send(true);
                    }
                    return;
                }

                let addr = ctx.address();
                let pool = self.pool.clone();
                let i18n = self.i18n.clone();
                let session_id = self.session_id;
                let user_id = match self.user_id {
                    Some(id) => id,
                    None => {
                        addr.do_send(ChatResult(error_frame("authentication_required")));
                        return;
                    }
                };
                let locale = self.locale.clone();
                let Some(message) = body.message else {
                    addr.do_send(ChatResult(error_frame("missing_message")));
                    return;
                };

                // 本轮取消通道：actor 持 sender（cancel 帧置位），turn 持 receiver
                let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
                self.cancel_tx = Some(cancel_tx);

                tokio::spawn(async move {
                    let orchestrator = build_orchestrator(&pool, i18n);
                    // 流式回调专用 addr（主 addr 后续被 add_message 错误路径 move）
                    let addr_for_chunks = addr.clone();

                    if let Err(e) = orchestrator
                        .add_message(
                            session_id,
                            &message,
                            body.context,
                            body.attachments,
                            body.knowledge_refs,
                            user_id,
                        )
                        .await
                    {
                        addr.do_send(ChatResult(error_frame(&e)));
                        return;
                    }

                    let input = TurnInput {
                        session_id,
                        user_id,
                        locale,
                        model: body.model,
                        cancel: Some(cancel_rx),
                    };

                    match orchestrator
                        .process_turn(
                            input,
                            // D2.10 typed 帧：LLM chunk / 工具事件 → 增量帧
                            Some(Box::new({
                                let addr = addr_for_chunks.clone();
                                move |ev: super::orchestrator::TurnStreamEvent| {
                                    let frame = stream_frame(ev);
                                    addr.do_send(ChatResult(frame));
                                }
                            })),
                        )
                        .await
                    {
                        Ok(result) => {
                            // 终止帧：{"type":"final", ...ChatMessageResponse}
                            // （含 id/agent_code，前端 resolve；content 与增量帧累积一致）
                            let frame = final_frame(&result.message);
                            addr.do_send(ChatResult(frame));
                        }
                        Err(e) => {
                            addr.do_send(ChatResult(error_frame(&e)));
                        }
                    }
                });
            }
            Ok(ws::Message::Ping(data)) => {
                ctx.pong(&data);
            }
            Ok(ws::Message::Pong(_)) => {
                // D2.10：Pong 已收 → 清零心跳计数
                self.pong_missed = 0;
            }
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => {}
        }
    }
}

pub async fn ws_connect(
    pool: web::Data<PgPool>,
    i18n_manager: web::Data<I18nManagerRef>,
    req: HttpRequest,
    stream: web::Payload,
    path: web::Path<i64>,
) -> Result<HttpResponse, actix_web::Error> {
    let session_id = path.into_inner();
    let user_id = match extract_user_id(&req) {
        Ok(id) => Some(id),
        Err(e) => return Err(e),
    };
    let locale = req
        .extensions()
        .get::<Locale>()
        .cloned()
        .unwrap_or(Locale::new("zh-CN"))
        .to_string();

    let session = ChatWsSession {
        session_id,
        pool: pool.get_ref().clone(),
        i18n: i18n_manager.get_ref().clone(),
        user_id,
        locale,
        cancel_tx: None,
        pong_missed: 0,
    };

    ws::start(session, &req, stream)
}

#[cfg(test)]
mod tests {
    // 真流式（LLM SSE）取代了伪流式切块——无纯函数单测。
    // 流式行为经 orchestrator 集成验证。
}
