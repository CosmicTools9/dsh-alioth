use actix_web::{web, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use uuid::Uuid;

use super::{AppState, ClientSession};

pub async fn ws_audit_handler(
    req: HttpRequest,
    stream: web::Payload,
    app_state: web::Data<AppState>,
) -> Result<HttpResponse, actix_web::Error> {
    let client_id = Uuid::new_v4();
    log::info!(
        "WebSocket /ws/audit connection established for client {} from {:?}",
        client_id,
        req.peer_addr()
    );

    let session = ClientSession::new(client_id, app_state);
    ws::start(session, &req, stream)
}
