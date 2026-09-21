//! operator_org — 运营组织主体解析（薄封装，唯一实现已上移 [`crate::actor_identity`]）。
//!
//! 写操作门禁：当前用户 → 单据「我」的主体（`zc_id_subjects`），供各 Service 写链注入
//! 「我方主体」（产品属权 `fk_subj-provider`/`fk_subj-demand`、订单双层双方等）。
//!
//! **历史包袱已摘除**（2026-09-14 裁决）：旧实现对特权 UA（admin/auditor/operator/enterprise）
//! 未绑定时回退 `SUBJ-SYSTEM` 兜底——该行为违反 `wz-trade-chain-ownership::dual-layer-order-parties`
//! 的 `unbound-operator-rejected`，且使「我」的身份不可回溯。现一律按三段链解析，
//! 未绑定 → `OPERATOR_ORG_UNBOUND`。
//!
//! 需要岗位视角（如平台侧 `VIEW-BIZ` 校验）的调用方 MUST 直接用
//! [`crate::actor_identity::resolve_actor_identity`]；本函数只返回主体 id。
use crate::actor_identity::resolve_actor_identity;
use crate::AliothError;

pub async fn resolve_operator_org(
    conn: &mut sqlx::PgConnection,
    user_id: i64,
) -> Result<i64, AliothError> {
    Ok(resolve_actor_identity(conn, user_id).await?.subject_id)
}
