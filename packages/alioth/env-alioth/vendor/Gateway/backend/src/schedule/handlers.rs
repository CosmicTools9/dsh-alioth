//! Schedule HTTP Handlers
//!
//! 提供 RESTful API 路由，基于 zc_id_plan + zc_id_event 双表模型。
//! 前缀: /api/schedule
//!
//! 迁移自 Framework/backend/schedule/src/handlers.rs

use actix_web::{web, HttpRequest, HttpResponse};
use common::{AliothError, ApiResponse};
use sqlx::PgPool;

use framework_schedule::models::*;
use framework_schedule::service::{ScheduleError, ScheduleService};
use framework_schedule::ScheduleRepository;

fn map_err(e: ScheduleError) -> AliothError {
    match e {
        ScheduleError::NotFound(id) => {
            AliothError::NotFound(format!("Schedule item {} not found", id))
        }
        ScheduleError::Validation(msg) => AliothError::BadRequest(msg),
        ScheduleError::Database(e) => {
            common::telemetry::error!("Schedule database error: {}", e);
            AliothError::Database(e.to_string())
        }
        ScheduleError::Internal(msg) => {
            common::telemetry::error!("Schedule internal error: {}", msg);
            AliothError::Internal(msg)
        }
    }
}

// ============================================
// Schedule Item Handlers
// ============================================

/// GET /schedule/items
/// 列出日程项（Plan + 关联 Event 信息，支持日期范围/类型/完成状态筛选）
pub async fn list_items(
    req: actix_web::HttpRequest,
    pool: web::Data<PgPool>,
    query: web::Query<ScheduleListQuery>,
) -> Result<HttpResponse, AliothError> {
    use actix_web::HttpMessage;
    let repo = ScheduleRepository::from_arc(pool.into_inner());
    let service = ScheduleService::new(repo);
    // RLS（wire-schedule-rls）：PEP 注入 schedule 可见 plan ID 集 → service 行级过滤
    let visible_ids = req
        .extensions()
        .get::<common::context::RequestContext>()
        .and_then(|ctx| ctx.get_visible_resource_ids("schedule").cloned())
        .filter(|ids| !ids.is_empty());

    match service
        .list_items(&query.into_inner(), visible_ids.as_deref())
        .await
    {
        Ok(items) => Ok(HttpResponse::Ok().json(ApiResponse::success(items))),
        Err(e) => Err(map_err(e)),
    }
}

/// GET /schedule/items/{id}
/// 获取单个日程项
pub async fn get_item(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let repo = ScheduleRepository::from_arc(pool.into_inner());
    let service = ScheduleService::new(repo);

    match service.find_item(path.into_inner()).await {
        Ok(Some(item)) => Ok(HttpResponse::Ok().json(ApiResponse::success(item))),
        Ok(None) => Err(AliothError::NotFound("Schedule item not found".into())),
        Err(e) => Err(map_err(e)),
    }
}

/// POST /schedule/items
/// 创建日程计划（自动创建关联 Event 空壳）
pub async fn create_item(
    pool: web::Data<PgPool>,
    req: web::Json<CreatePlanRequest>,
) -> Result<HttpResponse, AliothError> {
    let repo = ScheduleRepository::from_arc(pool.into_inner());
    let service = ScheduleService::new(repo);

    match service.create_plan(req.into_inner()).await {
        Ok(item) => Ok(HttpResponse::Ok().json(ApiResponse::success(item))),
        Err(e) => Err(map_err(e)),
    }
}

/// PUT /schedule/items/{id}
/// 更新日程计划
pub async fn update_item(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    req: web::Json<UpdatePlanRequest>,
) -> Result<HttpResponse, AliothError> {
    let repo = ScheduleRepository::from_arc(pool.into_inner());
    let service = ScheduleService::new(repo);

    match service
        .update_plan(path.into_inner(), req.into_inner())
        .await
    {
        Ok(Some(item)) => Ok(HttpResponse::Ok().json(ApiResponse::success(item))),
        Ok(None) => Err(AliothError::NotFound("Schedule item not found".into())),
        Err(e) => Err(map_err(e)),
    }
}

/// DELETE /schedule/items/{id}
/// 删除日程计划（软删除）
pub async fn delete_item(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let repo = ScheduleRepository::from_arc(pool.into_inner());
    let service = ScheduleService::new(repo);

    match service.delete_plan(path.into_inner()).await {
        Ok(true) => Ok(HttpResponse::NoContent().finish()),
        Ok(false) => Err(AliothError::NotFound("Schedule item not found".into())),
        Err(e) => Err(map_err(e)),
    }
}

/// PATCH /schedule/items/{id}/toggle
/// 切换日程完成状态（经 zc_id_lifecycle_r_primary-status 关系切换，
/// 非 progress_pct 字段——见 Framework/backend/schedule/src/service.rs）
/// id 优先按 plan 解析；待办列表（/schedule/todos）为 event-centric，前端
/// checkbox 传 event id——plan 不存在时按 event 切换，避免 404 误报。
pub async fn toggle_item_done(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let repo = ScheduleRepository::from_arc(pool.into_inner());
    let service = ScheduleService::new(repo);
    let id = path.into_inner();

    match service.toggle_plan_done(id).await {
        Ok(Some(item)) => Ok(HttpResponse::Ok().json(ApiResponse::success(item))),
        Ok(None) => match service.toggle_event_done(id).await {
            Ok(Some(event)) => Ok(HttpResponse::Ok().json(ApiResponse::success(event))),
            Ok(None) => Err(AliothError::NotFound("Schedule item not found".into())),
            Err(e) => Err(map_err(e)),
        },
        Err(e) => Err(map_err(e)),
    }
}

// ============================================
// Event Handlers（为 Plan 补充 Event 信息）
// ============================================

/// POST /schedule/items/{plan_id}/event
/// 为计划创建关联事件（补充地点/参与人等执行侧信息）
pub async fn create_event_for_plan(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    req: web::Json<CreateEventRequest>,
) -> Result<HttpResponse, AliothError> {
    let plan_id = path.into_inner();
    let repo = ScheduleRepository::from_arc(pool.into_inner());
    let service = ScheduleService::new(repo.clone());

    // 先确认 plan 存在
    if service.find_item(plan_id).await.map_err(map_err)?.is_none() {
        return Err(AliothError::NotFound("Plan not found".into()));
    }

    match repo.create_event_for_plan(plan_id, &req.into_inner()).await {
        Ok(event) => Ok(HttpResponse::Ok().json(ApiResponse::success(event))),
        Err(e) => {
            common::telemetry::error!("Schedule database error: {}", e);
            Err(AliothError::Database(e.to_string()))
        }
    }
}

// ============================================
// Event-centric Handlers
// ============================================

/// GET /schedule/todos
/// 列出待办事项（基于 zc_id_event，含客体、主体、状态）
/// 仅返回当前用户创建的事件。
pub async fn list_todos(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> Result<HttpResponse, AliothError> {
    use actix_web::HttpMessage;
    let user_id = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".into()))?;

    let limit: i64 = query
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
        .min(200);
    let offset: i64 = query
        .get("offset")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
        .max(0);

    let repo = ScheduleRepository::from_arc(pool.into_inner());
    let service = ScheduleService::new(repo);
    // RLS（wire-schedule-rls）：PEP 注入 schedule 可见 event ID 集 → service 行级过滤
    let visible_ids = req
        .extensions()
        .get::<common::context::RequestContext>()
        .and_then(|ctx| ctx.get_visible_resource_ids("schedule").cloned())
        .filter(|ids| !ids.is_empty());

    match service
        .list_todos(user_id, limit, offset, visible_ids.as_deref())
        .await
    {
        Ok(todos) => Ok(HttpResponse::Ok().json(ApiResponse::success(todos))),
        Err(e) => Err(map_err(e)),
    }
}

// ============================================
// Overview Handler
// ============================================

pub async fn get_overview(
    req: actix_web::HttpRequest,
    pool: web::Data<PgPool>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> Result<HttpResponse, AliothError> {
    use actix_web::HttpMessage;
    let user_id = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id);

    let repo = ScheduleRepository::from_arc(pool.into_inner());
    let service = ScheduleService::new(repo);

    let now = chrono::Utc::now();
    let date_start = query
        .get("date_start")
        .and_then(|v| v.parse::<chrono::DateTime<chrono::Utc>>().ok())
        .unwrap_or(now);
    let date_end = query
        .get("date_end")
        .and_then(|v| v.parse::<chrono::DateTime<chrono::Utc>>().ok())
        .unwrap_or(now + chrono::Duration::days(7));
    // 类型筛选：?code=meeting → _t_ 过滤 upcoming（fix-workspace-dock-contracts P1-7）
    let code = query.get("code").map(|s| s.as_str());
    // RLS（wire-schedule-rls）：PEP 注入 schedule 可见 ID 集 → service 行级过滤
    let visible_ids = req
        .extensions()
        .get::<common::context::RequestContext>()
        .and_then(|ctx| ctx.get_visible_resource_ids("schedule").cloned())
        .filter(|ids| !ids.is_empty());

    match service
        .get_overview(date_start, date_end, user_id, code, visible_ids.as_deref())
        .await
    {
        Ok(overview) => Ok(HttpResponse::Ok().json(ApiResponse::success(overview))),
        Err(e) => Err(map_err(e)),
    }
}

// ============================================
// Route Configuration
// ============================================

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/schedule")
            // Schedule Items (Plan-centric)
            .route("/items", web::get().to(list_items))
            .route("/items", web::post().to(create_item))
            .route("/items/{id}", web::get().to(get_item))
            .route("/items/{id}", web::put().to(update_item))
            .route("/items/{id}", web::delete().to(delete_item))
            .route("/items/{id}/toggle", web::patch().to(toggle_item_done))
            // Event attachment
            .route(
                "/items/{plan_id}/event",
                web::post().to(create_event_for_plan),
            )
            // Todos (Event-centric)
            .route("/todos", web::get().to(list_todos))
            // Overview
            .route("/overview", web::get().to(get_overview)),
    );
}
