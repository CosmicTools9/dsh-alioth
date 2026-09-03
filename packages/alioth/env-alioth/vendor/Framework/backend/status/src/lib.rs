//! status — 状态/事件共享内核（extract-status-core + unify-status-shells）
//!
//! `isahl.zc_id_status`（id 默认 gen_next_uid(12)）的唯一实现来源：
//! - Status 字段并集（WZ notice/code/flag/enable + Alioth name 别名/comments）
//! - StatusRepository：完整 CRUD（create 列默认 gen_next_uid(12)）+ RLS 读覆盖
//! - DamageReport / EventTracking / EventAccident（isahl 全局表）只读仓库
//! - status_mapper：统一状态映射（zc_id_stus-trade.code → 前端状态键）
//!
//! ns 壳仅注册路由前缀；全部实现本 crate 单一持有。

pub mod models;
pub mod repository;
pub mod status_mapper;

pub use models::{CreateStatusRequest, Status, UpdateStatusRequest};
pub use repository::StatusRepository;

use actix_web::web;
use common::AliothError;

use crate::models::{
    CreateAccidentRequest, CreateDamageRequest, CreateEventRequest, DamageReport, EventAccident,
    EventTracking, UpdateAccidentRequest, UpdateDamageRequest, UpdateEventRequest,
};
use crate::repository::{DamageReportRepository, EventAccidentRepository, EventTrackingRepository};

/// 注册状态因子全部 handler（不含 scope——由调用方自持 scope 路径）
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/statuses",
        web::get().to(crud::crud_list::<
            Status,
            CreateStatusRequest,
            UpdateStatusRequest,
            StatusRepository,
            AliothError,
        >),
    )
    .route(
        "/statuses/{id}",
        web::get().to(crud::crud_get::<
            Status,
            CreateStatusRequest,
            UpdateStatusRequest,
            StatusRepository,
            AliothError,
        >),
    )
    .route(
        "/statuses/{id}",
        web::delete().to(crud::crud_delete::<
            Status,
            CreateStatusRequest,
            UpdateStatusRequest,
            StatusRepository,
            AliothError,
        >),
    )
    .route(
        "/damages",
        web::get().to(crud::crud_list::<
            DamageReport,
            CreateDamageRequest,
            UpdateDamageRequest,
            DamageReportRepository,
            AliothError,
        >),
    )
    .route(
        "/damages/{id}",
        web::get().to(crud::crud_get::<
            DamageReport,
            CreateDamageRequest,
            UpdateDamageRequest,
            DamageReportRepository,
            AliothError,
        >),
    )
    .route(
        "/damages/{id}",
        web::delete().to(crud::crud_delete::<
            DamageReport,
            CreateDamageRequest,
            UpdateDamageRequest,
            DamageReportRepository,
            AliothError,
        >),
    )
    .route(
        "/events",
        web::get().to(crud::crud_list::<
            EventTracking,
            CreateEventRequest,
            UpdateEventRequest,
            EventTrackingRepository,
            AliothError,
        >),
    )
    .route(
        "/events/{id}",
        web::get().to(crud::crud_get::<
            EventTracking,
            CreateEventRequest,
            UpdateEventRequest,
            EventTrackingRepository,
            AliothError,
        >),
    )
    .route(
        "/events/{id}",
        web::delete().to(crud::crud_delete::<
            EventTracking,
            CreateEventRequest,
            UpdateEventRequest,
            EventTrackingRepository,
            AliothError,
        >),
    )
    .route(
        "/accidents",
        web::get().to(crud::crud_list::<
            EventAccident,
            CreateAccidentRequest,
            UpdateAccidentRequest,
            EventAccidentRepository,
            AliothError,
        >),
    )
    .route(
        "/accidents/{id}",
        web::get().to(crud::crud_get::<
            EventAccident,
            CreateAccidentRequest,
            UpdateAccidentRequest,
            EventAccidentRepository,
            AliothError,
        >),
    )
    .route(
        "/accidents/{id}",
        web::delete().to(crud::crud_delete::<
            EventAccident,
            CreateAccidentRequest,
            UpdateAccidentRequest,
            EventAccidentRepository,
            AliothError,
        >),
    );
}
