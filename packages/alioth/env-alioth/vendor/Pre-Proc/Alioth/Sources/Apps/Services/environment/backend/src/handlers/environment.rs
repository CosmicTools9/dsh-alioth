//! 运行环境 Handler — 标准 CRUD 路由,映射 isahl."zc_id_prot-env_config"
use crate::models::{CreateEnvironmentRequest, Environment, UpdateEnvironmentRequest};
use crate::repositories::EnvironmentRepository;
use actix_web::{web, HttpResponse};
use common::AliothError as ApiError;
use crud::crud_routes;
use serde::Serialize;
use sqlx::PgPool;

/// 环境级别统计
#[derive(Debug, Serialize)]
pub struct EnvironmentStats {
    #[serde(with = "common::serde_zuid")]
    pub info: i64,
    #[serde(with = "common::serde_zuid")]
    pub warn: i64,
    #[serde(with = "common::serde_zuid")]
    pub error: i64,
    #[serde(with = "common::serde_zuid")]
    pub debug: i64,
    #[serde(with = "common::serde_zuid")]
    pub notice: i64,
    #[serde(with = "common::serde_zuid")]
    pub fetal: i64,
}

async fn stats(pool: web::Data<PgPool>) -> Result<HttpResponse, ApiError> {
    let rows = EnvironmentRepository::new(pool.get_ref().clone())
        .stats()
        .await?;

    let mut stats = EnvironmentStats {
        info: 0,
        warn: 0,
        error: 0,
        debug: 0,
        notice: 0,
        fetal: 0,
    };
    for (level, cnt) in rows {
        match level.as_str() {
            "info" => stats.info = cnt,
            "warn" => stats.warn = cnt,
            "error" => stats.error = cnt,
            "debug" => stats.debug = cnt,
            "notice" => stats.notice = cnt,
            "fetal" => stats.fetal = cnt,
            _ => {}
        }
    }
    Ok(HttpResponse::Ok().json(stats))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    crud_routes::<
        Environment,
        CreateEnvironmentRequest,
        UpdateEnvironmentRequest,
        EnvironmentRepository,
        ApiError,
    >("/environments")(cfg);
    cfg.route("/environments/stats", web::get().to(stats));
}
