//! 单位换算率 Handler — 标准 CRUD 路由（biz 共享内核类型）

use crate::biz_models::{
    CreateUnitConversionRateRequest, UnitConversionRate, UpdateUnitConversionRateRequest,
};
use crate::biz_repositories::UnitConversionRateRepository;
use actix_web::web;
use common::error::AliothError;
use crud::crud_routes;

pub fn register(cfg: &mut web::ServiceConfig) {
    crud_routes::<
        UnitConversionRate,
        CreateUnitConversionRateRequest,
        UpdateUnitConversionRateRequest,
        UnitConversionRateRepository,
        AliothError,
    >("/conversion-rates")(cfg);
}
