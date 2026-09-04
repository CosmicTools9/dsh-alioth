pub mod handler;

use actix::{Actor, ActorContext, AsyncContext, StreamHandler};
use actix_web::web;
use actix_web_actors::ws;
use dashmap::DashSet;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use uuid::Uuid;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
/// 心跳超时（3× interval）：客户端断网但 TCP 未关闭时，90s 无心跳即强制断开，
/// 避免僵尸连接残留（此前仅记录 last_heartbeat，无超时强制清理）。
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(90);

static WS_SERVER: std::sync::OnceLock<Arc<AuditWsServer>> = std::sync::OnceLock::new();

pub fn init_ws_server() -> &'static Arc<AuditWsServer> {
    WS_SERVER.get_or_init(|| {
        log::info!("Initializing global WebSocket audit server");
        Arc::new(AuditWsServer::new())
    })
}

pub fn get_ws_server() -> Option<&'static Arc<AuditWsServer>> {
    WS_SERVER.get()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditFilters {
    pub user_id: Option<Uuid>,
    pub object_path: Option<String>,
    pub operation: Option<String>,
    pub decision: Option<String>,
}

pub struct AuditWsServer {
    clients: DashSet<Uuid>,
    subscriptions: dashmap::DashMap<Uuid, AuditFilters>,
    sender: broadcast::Sender<WsMessage>,
}

impl AuditWsServer {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(1000);
        Self {
            clients: DashSet::new(),
            subscriptions: dashmap::DashMap::new(),
            sender,
        }
    }

    pub fn register_client(&self, client_id: Uuid) {
        log::info!(
            "WebSocket client {} registered (total={})",
            client_id,
            self.clients.len() + 1
        );
        self.clients.insert(client_id);
    }

    pub fn unregister_client(&self, client_id: Uuid) {
        log::info!("WebSocket client {} unregistered", client_id);
        self.clients.remove(&client_id);
        self.subscriptions.remove(&client_id);
    }

    pub fn subscribe(&self, client_id: Uuid, filters: AuditFilters) {
        self.subscriptions.insert(client_id, filters);
    }

    pub fn unsubscribe(&self, client_id: Uuid) {
        self.subscriptions.remove(&client_id);
    }

    pub fn broadcast_audit_event(&self, event: AuditEvent) {
        let msg = WsMessage {
            msg_type: WsMessageType::AuditEvent,
            payload: serde_json::to_value(&event).unwrap_or(serde_json::Value::Null),
        };
        let _ = self.sender.send(msg);
    }

    /// 新客户端订阅广播流。
    pub fn client_receiver(&self) -> broadcast::Receiver<WsMessage> {
        self.sender.subscribe()
    }

    pub fn client_count(&self) -> usize {
        self.clients.len()
    }
}

impl Default for AuditWsServer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMessage {
    #[serde(rename = "msg_type")]
    pub msg_type: WsMessageType,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WsMessageType {
    AuditEvent,
    PolicyChange,
    Heartbeat,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: Uuid,
    pub event_type: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub subject_id: Uuid,
    pub object_id: Uuid,
    pub object_type: String,
    pub operation: String,
    pub success: bool,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Clone)]
pub struct AppState {
    pub ws_server: Arc<AuditWsServer>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            // 全局共享同一 AuditWsServer（WS_SERVER OnceLock），确保跨 actix worker
            // 的广播一致：每个 worker 各建实例会导致客户端收不到其他 worker 的事件。
            ws_server: init_ws_server().clone(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ClientSession {
    client_id: Uuid,
    last_heartbeat: Instant,
    app_state: web::Data<AppState>,
}

impl ClientSession {
    pub fn new(client_id: Uuid, app_state: web::Data<AppState>) -> Self {
        Self {
            client_id,
            last_heartbeat: Instant::now(),
            app_state,
        }
    }
}

impl Actor for ClientSession {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        log::info!("Client session {} started", self.client_id);
        self.app_state.ws_server.register_client(self.client_id);

        // 订阅全局审计广播流（跨 worker 共享的 AuditWsServer），收到事件后经
        // BroadcastMsg 消息投递到本 actor 上下文，再按订阅过滤器推送。
        let mut rx = self.app_state.ws_server.client_receiver();
        let addr = ctx.address();
        ctx.spawn(actix::fut::wrap_future(async move {
            while let Ok(msg) = rx.recv().await {
                addr.do_send(BroadcastMsg(msg));
            }
        }));

        ctx.run_interval(
            HEARTBEAT_INTERVAL,
            |act: &mut ClientSession, ctx: &mut ws::WebsocketContext<ClientSession>| {
                // 心跳超时：客户端断网（TCP 未关闭）时无 Pong 回报 → 强制断开，
                // stopping 钩子随后 unregister_client 清理注册。
                // last_heartbeat 只在收到 Pong/Ping 时更新（StreamHandler 分支），
                // 此处不重置——否则每次 interval 都刷新心跳，超时检查失效。
                if act.last_heartbeat.elapsed() > HEARTBEAT_TIMEOUT {
                    log::warn!(
                        "Client {} heartbeat timeout (>{:?}), closing",
                        act.client_id,
                        HEARTBEAT_TIMEOUT
                    );
                    ctx.stop();
                    return;
                }
                ctx.ping(b"");
            },
        );
    }

    fn stopping(&mut self, _: &mut Self::Context) -> actix::Running {
        log::info!("Client session {} stopping", self.client_id);
        self.app_state.ws_server.unregister_client(self.client_id);
        actix::Running::Stop
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for ClientSession {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(msg) => match msg {
                ws::Message::Ping(msg) => {
                    self.last_heartbeat = Instant::now();
                    ctx.pong(&msg);
                }
                ws::Message::Pong(_) => {
                    self.last_heartbeat = Instant::now();
                }
                ws::Message::Text(text) => {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(msg_type) = json.get("msg_type").and_then(|v| v.as_str()) {
                            match msg_type {
                                "subscribe" => {
                                    if let Some(filters_json) = json.get("filters") {
                                        if let Ok(filters) = serde_json::from_value::<AuditFilters>(
                                            filters_json.clone(),
                                        ) {
                                            self.app_state
                                                .ws_server
                                                .subscribe(self.client_id, filters);
                                        }
                                    }
                                }
                                "unsubscribe" => {
                                    self.app_state.ws_server.unsubscribe(self.client_id);
                                }
                                "ping" => {
                                    ctx.pong(b"");
                                }
                                _ => {}
                            }
                        }
                    }
                }
                ws::Message::Binary(_) => {}
                ws::Message::Close(reason) => {
                    log::info!("Client {} closing: {:?}", self.client_id, reason);
                    ctx.stop();
                }
                ws::Message::Continuation(_) => {}
                ws::Message::Nop => {}
            },
            Err(e) => {
                log::error!("WebSocket error for client {}: {}", self.client_id, e);
                ctx.stop();
            }
        }
    }
}

/// 广播消息（由全局 AuditWsServer 广播流经 ctx.spawn 转发到本 actor）。
#[derive(actix::Message)]
#[rtype(result = "()")]
struct BroadcastMsg(WsMessage);

impl actix::Handler<BroadcastMsg> for ClientSession {
    type Result = ();

    fn handle(&mut self, msg: BroadcastMsg, ctx: &mut Self::Context) {
        let wm = msg.0;
        // 按订阅过滤器（未订阅 = 全量推送）过滤。
        let filters = self.app_state.ws_server.subscriptions.get(&self.client_id);
        if let Some(f) = filters.as_deref() {
            if let Some(ev) = wm.payload.as_object() {
                if let Some(op) = &f.operation {
                    if ev.get("operation").and_then(|v| v.as_str()) != Some(op.as_str()) {
                        return;
                    }
                }
                if let Some(ot) = &f.object_path {
                    if ev.get("object_type").and_then(|v| v.as_str()) != Some(ot.as_str()) {
                        return;
                    }
                }
                if let Some(dec) = &f.decision {
                    let ev_success = ev.get("success").and_then(|v| v.as_bool());
                    let matched = match dec.as_str() {
                        "permit" => ev_success == Some(true),
                        "deny" => ev_success == Some(false),
                        _ => false,
                    };
                    if !matched {
                        return;
                    }
                }
            }
        }
        if let Ok(json) = serde_json::to_string(&wm) {
            ctx.text(json);
        }
    }
}
