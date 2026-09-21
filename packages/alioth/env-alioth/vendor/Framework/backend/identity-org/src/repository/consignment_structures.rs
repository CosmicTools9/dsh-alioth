//! 委托编辑结构写路的**薄适配层**（change: `migrate-consignment-fields-to-structures` T10）
//!
//! 读侧已全面切结构（`logi-consignment/repositories/tasks.rs`：货物 ← 明细行、起讫 ← 停靠桥、
//! 时段 ← `segm-date`、体积/货量/金额 ← 明细标量），`comments` 摘要**不再被解析**——编辑保存
//! 的业务字段 MUST 落结构载体，否则即「写进无人读的列」。
//!
//! 写件的**单一实现**在 `consignment-writer`（白名单写件：`zc_id_prod-freight_road-sales`
//! 的写入只允许出现在 `check-wz-product-writer.ts` 的白名单文件内，委托链 =
//! `Framework/backend/consignment-writer/src/writer.rs`）。本模块只做两件事：
//!
//! ① 把 `UpdateConsignmentRequest` 的字段装配为 `consignment_writer::UpdateStructuresInput`；
//! ② 在**单事务**内调用之（结构写入跨多表，全有或全无）。
//!
//! MUST NOT 在本文件复制产品/停靠/明细/标量的 SQL（REUSE_FIRST：同语义第二份写 SQL = 违规）。

use common::AliothError as ApiError;
use sqlx::PgPool;

use crate::models::UpdateConsignmentRequest;

/// 把编辑保存的结构字段落到结构载体（明细 / 停靠桥 / 时段 / 标量 / 承运商）。
///
/// 入参全为空（未编辑任何业务字段）时零写入；委托不存在 → `NotFound`（写件内校验）。
pub async fn apply_structured_update(
    pool: &PgPool,
    consignment_id: i64,
    user_id: i64,
    req: &UpdateConsignmentRequest,
) -> Result<(), ApiError> {
    let input = consignment_writer::UpdateStructuresInput {
        cargo: req.cargo.clone(),
        cbm: req.cbm,
        price: req.price,
        weight_ton: req.volume,
        amount: req.amount,
        origin: req.origin.clone(),
        dest: req.dest.clone(),
        pickup_time: req.pickup_time.clone(),
        eta: req.eta.clone(),
        carrier: req.carrier.clone(),
    };
    if input.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    consignment_writer::update_consignment_structures_tx(&mut tx, consignment_id, &input, user_id)
        .await?;
    tx.commit().await?;
    Ok(())
}
