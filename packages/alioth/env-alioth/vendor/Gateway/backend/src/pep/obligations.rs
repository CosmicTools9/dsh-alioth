//! PEP 内部 seam — 义务算子执行面（评审候选 5 分缝之一：obligations）。
//!
//! 自 `pep/middleware.rs` 内部抽出；`audit:log` 的执行面即审计行本体，
//! 白名单外算子 fail-visible（判决不变）。

use common::telemetry::warn;

/// 义务算子白名单（wire-obligations-into-pep-audit / NGAC_SPEC §2.6）。
///
/// 执行语义（消费方视角）：
/// - `audit:log` —— 由本次审计行落库**等价执行**（`obligations_triggered` 列即回执）；
/// - 白名单外算子 —— **fail-visible**：`warn` 留痕、不静默丢弃（未知算子 = 策略配置
///   领先于执行面——可见性优先，禁止吞掉）；
/// - 判决不变（§2.6 铁律）：执行失败/未知**不**改写已发生的 Permit/Deny。
const OBLIGATION_OP_WHITELIST: &[&str] = &["audit:log"];

/// 执行 PDP 下发的义务算子（在审计写入任务内调用——`audit:log` 的执行面即审计行本体）。
pub(super) fn execute_obligation_ops(user_id: i64, resource: &str, obligations: &[String]) {
    for op in obligations {
        if OBLIGATION_OP_WHITELIST.contains(&op.as_str()) {
            // audit:log —— 审计行落列即执行（本函数调用点在写入任务内）
        } else {
            warn!(
                "ngac obligation op not in executor whitelist (fail-visible): op={op} user={user_id} resource={resource}"
            );
        }
    }
}
