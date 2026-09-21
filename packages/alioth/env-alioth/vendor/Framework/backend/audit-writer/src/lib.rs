//! # audit-writer — 业务审计域（会计审计/合规审计）写链共享事务
//!
//! 单一事实源（change `wire-business-audit-domain` D1/D2，after-sales-writer 同款模式）：
//!
//! - `zc_id_audit`（实现-审计）审计项目主档（`fk_launcher` 发起人）；
//! - `zc_id_audit_rr_auditee`（审计↔受审计对象）——**静态单目标绑定**：仅挂
//!   `zc_id_subjects` 受审主体（`REFERENCE_RESOLVER_SPEC` junction 单目标口径，
//!   多态泛化登记模型中心待裁决）；uid 段 219；
//! - `zc_id_audit_rr_conclusion`（审计↔评估结论）——`ref_right` 指向
//!   `zc_id_prod-conclusion` 叶行（调用方先行创建/解析行 id）；uid 段 220。
//!
//! ## 边界（D2/R4）
//!
//! 本 crate 属**业务审计域**（会计审计/合规审计），与 `isahl_audit` 数据审计基建
//! （audit_outbox/data_change_logs——操作留痕/NGAC 决策审计）正交，MUST NOT 混用表族。
//!
//! ## 设计要点
//!
//! - **id 口径**：`zc_id_audit.id` 列默认 `gen_next_zuid()`——INSERT **省略 id**
//!   由默认值决定（`db-uid-defaults` 首选）；桥行 `gen_next_uid(219/220)`。
//! - **幂等桥**：`attach_auditee_tx` / `set_conclusion_tx` 均 NOT EXISTS 幂等。
//! - **事务语义**：`&mut PgConnection` 由调用方管理（damage-writer/after-sales-writer 同款）。
//! - **坐标三元组参数化**：调用方经 `ontology_binding::resolve` 解析后注入
//!   （首版占位 JC/FTA/↑_NA——管理·审批处理·管理职能，见 change design D3 注）。

pub mod oper;
pub mod read;
pub mod trace;
pub mod writer;

pub use oper::{
    insert_confirm_bill_tx, insert_smtv_review_tx, link_supervision_audit_tx, OperationRowInput,
};
pub use writer::{
    attach_auditee_tx, insert_audit_project_tx, set_conclusion_tx, AuditProjectInput,
};
