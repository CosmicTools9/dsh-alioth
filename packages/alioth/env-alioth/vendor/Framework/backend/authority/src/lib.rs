//! # service-identity — 身份因子
//!
//! 人员角色管理: 工程师、技能标签、审批岗位、CCB 成员实体管理。
//! 提供标准的 CRUD 操作，支持可选的 NGAC defense-in-depth hook。
//!
//! ## 架构层级
//! - `models/`        — 实体结构体 + AliothDbEntity trait 实现 (L2 DTO)
//! - `repositories/`  — AliothRepository trait 实现 (数据访问)
//! - `services/`      — 业务逻辑层 (字段校验)
//! - `handlers/`      — HTTP handler + 路由注册 (泛型 <G: NgacGuard>)
//! - `ngac/`          — NgacGuard trait + NoopNgacGuard + RlsNgacGuard
//!
//! ## 对应本体表
//! - `isahl.zc_id_subj-employee` — 工程师/员工表
//! - `isahl.zc_id_cate-approve_role` — 审批岗位（2026-09-02 起；此前误用共享字典 zc_id_category）
//! - `isahl.zc_id_subj-position` — CCB 变更委员会成员
use actix_web::web;

use crate::ngac::NgacGuard;

pub mod handlers;
pub mod models;
pub mod ngac;
pub mod repositories;
pub mod services;

/// 注册 identity 因子路由，使用自定义 NGAC guard 和前缀。
///
/// NS 可注入 `RlsNgacGuard`（需 `ngac-rls` feature）并指定自定义 prefix。
pub fn register_service_routes_with_guard<G: NgacGuard + 'static>(
    cfg: &mut web::ServiceConfig,
    prefix: &str,
) {
    cfg.service(
        web::scope(prefix)
            .configure(handlers::employees::register::<G>)
            .configure(handlers::skill_tags::register::<G>)
            .configure(handlers::approval_roles::register::<G>)
            .configure(handlers::approvers::register::<G>)
            .configure(handlers::positions::register::<G>),
    );
}
