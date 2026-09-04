//! Contacts HTTP Handler — Gateway thin adapter
//!
//! 业务逻辑在 framework-contacts crate 中。
//! 本层仅负责：调 Framework 服务 → 映射 HTTP 响应。
//! Presence 追踪使用全局 `PresenceTracker` 实例。

use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use framework_contacts::{ContactsService, CreateContactRequest, UpdateContactRequest};
use framework_presence::PresenceTracker;
use serde::Deserialize;
use std::sync::OnceLock;

static PRESENCE: OnceLock<PresenceTracker> = OnceLock::new();

fn presence() -> &'static PresenceTracker {
    PRESENCE.get_or_init(|| {
        let tracker = PresenceTracker::new();
        tracker
            .clone()
            .start_cleanup_task(std::time::Duration::from_secs(60));
        tracker
    })
}

#[derive(Debug, Deserialize)]
pub struct ContactsQuery {
    #[serde(default = "default_page")]
    #[serde(with = "common::serde_zuid")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    #[serde(with = "common::serde_zuid")]
    pub page_size: i64,
}

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    50
}

/// GET /hr/contacts — 返回联系人列表（支持分页 + typed infos + presence）
pub async fn list_contacts(
    pool: web::Data<sqlx::PgPool>,
    query: web::Query<ContactsQuery>,
) -> HttpResponse {
    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 100);

    match ContactsService::list_contacts(pool.get_ref(), page, page_size).await {
        Ok((mut contacts, total)) => {
            let total_pages = (total as f64 / page_size as f64).ceil() as i64;
            // 合并 presence 数据
            let ids: Vec<i64> = contacts.iter().map(|c| c.id).collect();
            let statuses = presence().get_statuses(&ids).await;
            for contact in contacts.iter_mut() {
                if let Some(s) = statuses.iter().find(|s| s.user_id == contact.id) {
                    contact.is_online = Some(s.is_online);
                }
            }
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "data": contacts,
                "total": total,
                "page": page,
                "page_size": page_size,
                "total_pages": total_pages
            }))
        }
        Err(e) => {
            log::error!("contacts: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// POST /hr/contacts — 创建联系人
pub async fn create_contact(
    pool: web::Data<sqlx::PgPool>,
    body: web::Json<CreateContactRequest>,
) -> HttpResponse {
    match ContactsService::create_contact(pool.get_ref(), body.into_inner()).await {
        Ok(contact) => HttpResponse::Created().json(serde_json::json!({
            "success": true,
            "data": contact
        })),
        Err(e) => {
            log::error!("create contact: {}", e);
            HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// PUT /hr/contacts/{id} — 更新联系人
pub async fn update_contact(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    path: web::Path<i64>,
    body: web::Json<UpdateContactRequest>,
) -> HttpResponse {
    let id = path.into_inner();
    // 操作人身份（user_id → 软删/更新归因 deleted_by_id）——与 heartbeat D8 同款提取
    let Some(user_id) = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id)
    else {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "success": false, "message": "无法获取当前用户身份" }));
    };
    match ContactsService::update_contact(pool.get_ref(), id, body.into_inner(), user_id).await {
        Ok(Some(contact)) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "data": contact
        })),
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "success": false,
            "error": "Contact not found"
        })),
        Err(e) => {
            log::error!("update contact: {}", e);
            HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// DELETE /hr/contacts/{id} — 删除联系人（软删除）
pub async fn delete_contact(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    path: web::Path<i64>,
) -> HttpResponse {
    let id = path.into_inner();
    // 操作人身份（deleted_by_id 归因）——与 heartbeat D8 同款提取
    let Some(user_id) = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id)
    else {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "success": false, "message": "无法获取当前用户身份" }));
    };
    match ContactsService::delete_contact(pool.get_ref(), id, user_id).await {
        Ok(true) => HttpResponse::NoContent().finish(),
        Ok(false) => HttpResponse::NotFound().json(serde_json::json!({
            "success": false,
            "error": "Contact not found"
        })),
        Err(e) => {
            log::error!("delete contact: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": e
            }))
        }
    }
}

/// POST /hr/contacts/presence/heartbeat — 发送 heartbeat
/// D8: 当 contact_id = 0 时，从 RequestContext.user_id 反查真实 contact id
pub async fn heartbeat(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    body: web::Json<HeartbeatRequest>,
) -> HttpResponse {
    let contact_id = body.contact_id;

    // D8: 如果前端传 contact_id = 0，从 RequestContext.user_id 经 contact_infos 反查
    let resolved_contact_id =
        if contact_id == 0 {
            let user_id =
                match req
                    .extensions()
                    .get::<common::context::RequestContext>()
                    .map(|ctx| ctx.user_id)
                {
                    Some(id) => id,
                    None => return HttpResponse::BadRequest().json(
                        serde_json::json!({ "success": false, "message": "无法获取当前用户身份" }),
                    ),
                };

            // 路径: auth_users.id (= user_id) → zc_id_entity.id (= entity_id)
            //                         → zc_id_entity_rr_contacts
            //                         → zc_id_contacts.id
            //                         → zc_id_contacts_rr_infos
            //                         → zc_id_contact_infos.id
            //                         → zc_id_info-isahl (isahl_id = user_id)
            let resolved: Option<i64> = sqlx::query_scalar(
                r#"SELECT c.id
               FROM isahl.zc_id_contacts c
               JOIN isahl."zc_id_contacts_rr_infos" cri ON cri.ref_left = c.id
               JOIN isahl.zc_id_contact_infos ci ON ci.id = cri.ref_right
               JOIN isahl."zc_id_info-isahl" ii ON ii.id = ci.id
               JOIN isahl.zc_id_entity_rr_contacts erc ON erc.ref_right = c.id
               JOIN isahl.zc_id_entity e ON e.id = erc.ref_left
               WHERE e.id = $1 AND c.deleted_at IS NULL AND ci.deleted_at IS NULL
               LIMIT 1"#,
            )
            .bind(user_id)
            .fetch_optional(pool.get_ref())
            .await
            .ok()
            .flatten();

            match resolved {
                Some(id) => id,
                None => return HttpResponse::BadRequest().json(
                    serde_json::json!({ "success": false, "message": "找不到对应的联系人记录" }),
                ),
            }
        } else {
            contact_id
        };

    presence().heartbeat(resolved_contact_id).await;
    HttpResponse::Ok().json(serde_json::json!({ "success": true }))
}

/// POST /hr/contacts/presence/heartbeat — 单用户心跳
#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    #[serde(with = "common::serde_zuid")]
    pub contact_id: i64,
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/hr")
            .route("/contacts", web::get().to(list_contacts))
            .route("/contacts", web::post().to(create_contact))
            .route("/contacts/{id}", web::put().to(update_contact))
            .route("/contacts/{id}", web::delete().to(delete_contact))
            .route("/contacts/presence/heartbeat", web::post().to(heartbeat)),
    );
}
