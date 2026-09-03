//! # factor-commitment — 审批承诺因子
//!
//! Framework 审批承诺因子（跨 namespace 通用）: 审批流程、节点、实例、动作、委托规则实体管理。
//! 提供标准的 CRUD 操作。
//!
//! ## 对应本体表
//! - `isahl."zc_id_proc-approve"`   — 审批流程定义（zc_id_process 审批族子类；读经基表继承并集）
//! - `isahl.zc_id_process`          — 流程基表（与工艺路线共享流程系统）
//! - `isahl.zc_id_oper-approve`     — 发起审批事项（实例↔审批事件经 `zc_id_operation_rr_event` 桥）
//! - `isahl.zc_id_deta-opinion`     — 审批意见明细
//! - `isahl.zc_id_operation`        — 委托规则（公式驱动）
use actix_web::web;

pub mod advance;
pub mod context_domain;
pub mod context_meta;
mod dk;
pub mod handlers;
pub mod mermaid;
pub mod models;
pub mod ngac_ensure;
pub mod node_meta;
pub mod repositories;
pub mod services;
pub mod sla_timeout;

/// 注册审批因子的所有 handler（不含 scope——由调用方自持 scope 路径）
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.configure(handlers::analytics::register)
        .configure(handlers::enriched_instance::register)
        // 所有 /approval-flows/{id}/* 子路径必须在 approval_flow CRUD scope 之前注册，
        // 避免被 web::scope 前缀匹配吞掉。
        .configure(handlers::publish::register)
        .configure(handlers::version::register)
        .configure(handlers::flow_node::register)
        .configure(handlers::flow_nodes::register)
        // /approval-flows/scope-options 必须在 approval_flow CRUD scope 之前注册，
        // 避免 "scope-options" 被当作 {id} 段吞掉
        .configure(handlers::scope_options::register)
        // /approval-flows/context-fields 同理，必须在 CRUD scope 之前
        .configure(handlers::context_fields::register)
        .configure(handlers::context_field_domain::register)
        .configure(handlers::context_objects::register)
        // /approval-flows/validate 与 {id}/initiate 同理，必须在 CRUD scope 之前
        .configure(handlers::validate::register)
        .configure(handlers::initiate::register)
        // /approval-flows/lifecycle/{class} 与 /{id}/generate-template 须在 CRUD scope 之前
        .configure(handlers::flow_lifecycle::register)
        .configure(handlers::approval_flow::register)
        // 所有 /approval-instances/{id}/* 子路径必须在 approval_instance CRUD scope 之前注册；
        // 字面路径（batch/approve、batch/reject）必须先于任何参数段（{id}/approve）注册——
        // actix-router 同层 param 先占位会遮蔽后注册字面路由（曾致 POST batch/approve
        // 落入 {id} 解析报 can not parse "batch" to a i64）
        .configure(handlers::batch_approve_reject::register)
        .configure(handlers::timeline::register)
        .configure(handlers::remind::register)
        .configure(handlers::approve_reject::register)
        .configure(handlers::cc_inbox::register)
        .configure(handlers::transfer_cc::register)
        .configure(handlers::withdraw::register)
        .configure(handlers::approval_instance::register)
        .configure(handlers::approval_action::register)
        .configure(handlers::delegation_rule::register);
}
