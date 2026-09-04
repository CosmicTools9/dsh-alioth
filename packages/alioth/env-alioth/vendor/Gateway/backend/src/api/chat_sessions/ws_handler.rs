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
    message: String,
    /// Optional page/entity context, same semantics as CreateMessageRequest.context.
    context: Option<serde_json::Value>,
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
}

impl Actor for ChatWsSession {
    type Context = ws::WebsocketContext<Self>;

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
                        ctx.text("{\"error\":\"invalid_json\"}");
                        return;
                    }
                };

                let addr = ctx.address();
                let pool = self.pool.clone();
                let i18n = self.i18n.clone();
                let session_id = self.session_id;
                let user_id = match self.user_id {
                    Some(id) => id,
                    None => {
                        addr.do_send(ChatResult(
                            "{\"error\":\"authentication_required\"}".to_string(),
                        ));
                        return;
                    }
                };
                let locale = self.locale.clone();

                tokio::spawn(async move {
                    let orchestrator = build_orchestrator(&pool, i18n);
                    // 流式回调专用 addr（主 addr 后续被 add_message 错误路径 move）
                    let addr_for_chunks = addr.clone();

                    if let Err(e) = orchestrator
                        .add_message(session_id, &body.message, body.context, user_id)
                        .await
                    {
                        let err = serde_json::json!({"error": e});
                        addr.do_send(ChatResult(err.to_string()));
                        return;
                    }

                    let input = TurnInput {
                        session_id,
                        user_id,
                        locale,
                        model: body.model,
                    };

                    match orchestrator
                        .process_turn(
                            input,
                            // 真流式（P1-6）：LLM 逐 chunk → 增量帧（前端 onChunk 渐进渲染）
                            Some(Box::new({
                                let addr = addr_for_chunks.clone();
                                move |chunk: String| {
                                    let frame = serde_json::json!({ "content": chunk }).to_string();
                                    addr.do_send(ChatResult(frame));
                                }
                            })),
                        )
                        .await
                    {
                        Ok(result) => {
                            // 终止帧：完整 ChatMessageResponse（含 id/agent_code，前端 resolve）。
                            // 增量帧已渐进渲染；终止帧 content 与累积文本一致 → 最终一致。
                            let json =
                                serde_json::to_string(&result.message).unwrap_or_else(|_| {
                                    "{\"error\":\"serialization_failed\"}".to_string()
                                });
                            addr.do_send(ChatResult(json));
                        }
                        Err(e) => {
                            let err = serde_json::json!({"error": e});
                            addr.do_send(ChatResult(err.to_string()));
                        }
                    }
                });
            }
            Ok(ws::Message::Ping(data)) => {
                ctx.pong(&data);
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
    };

    ws::start(session, &req, stream)
}

#[cfg(test)]
mod tests {
    // 真流式（LLM SSE）取代了伪流式切块——无纯函数单测。
    // 流式行为经 orchestrator 集成验证。
}
