//! identity-org NGAC org 资源 heal 挂接层（cover-org-resources-ngac / B-2；M221 收编）
//!
//! org 写端（org_tree handlers / org_scheme）在业务事务提交后调用本组件的幂等 heal
//! 入口。**单一实现 = common::ngac_org**（B-0 认知派生系收编同款先例）——本模块自
//! M221 起仅薄转发，SQL/层级逻辑一律不在此驻留，禁止复制（NGAC_SPEC §2.2.3
//! 消费同源义务）。实现语义详见 common::ngac_org 模块 doc：行 OA upsert
//! （positions/departments）、部门子集 OA 树 ancestor 闭包、集合 OA 兜底；
//! NGAC 未启用/失败仅 warn、绝不阻断 org 主写（对齐 Gateway ensure_cognition_uas /
//! ngac_seed 运行期自愈容错语义），全部函数可安全重放。
//!
//! 挂接点（转发保持签名，调用点零改动）：
//! create_department / add_org_tree_child（heal_department_scope）/
//! create_position / assign_position_to_department / add_position_employee /
//! remove_position_from_department（heal_position_scope）——事务外。

use sqlx::PgPool;

/// 部门 heal 入口（薄转发，单实现 = common::ngac_org::heal_department_scope）。
pub async fn heal_department_scope(pool: &PgPool, org_id: i64) {
    common::ngac_org::heal_department_scope(pool, org_id).await;
}

/// 岗位 heal 入口（薄转发，单实现 = common::ngac_org::heal_position_scope）。
pub async fn heal_position_scope(pool: &PgPool, position_id: i64) {
    common::ngac_org::heal_position_scope(pool, position_id).await;
}
