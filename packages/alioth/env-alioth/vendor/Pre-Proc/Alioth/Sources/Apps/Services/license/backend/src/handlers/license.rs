//! 许可证 Handler — 标准 CRUD 路由,映射 isahl."zc_id_prod-license-purchase"
use crate::models::{CreateLicenseRequest, License, UpdateLicenseRequest};
use crate::repositories::LicenseRepository;
use actix_web::web;
use common::AliothError as ApiError;
use crud::crud_routes;

pub fn register(cfg: &mut web::ServiceConfig) {
    crud_routes::<License, CreateLicenseRequest, UpdateLicenseRequest, LicenseRepository, ApiError>(
        "/licenses",
    )(cfg);
}
