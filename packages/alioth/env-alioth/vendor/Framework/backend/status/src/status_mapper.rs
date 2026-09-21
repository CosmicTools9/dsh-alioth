//! 统一状态映射（全局共享内核）——全 namespace 唯一状态映射来源
//!
//! 输入基准: `isahl.zc_id_stus-trade.code`（ST-* 内部码）或中文业务标识。
//! 输出: 统一前端状态键，供各 service 共用，消灭独立 map_status。
//!
//! 规约依据: `waybill-status-mapping-unified` — 全部 service 的状态展示映射 MUST 收敛到
//! 单一映射函数，MUST 以 `zc_id_stus-trade.code` 为输入基准。

/// 统一状态键（前端枚举基准）
pub const STATUS_PENDING_REVIEW: &str = "pending_review";
pub const STATUS_ACCEPTED: &str = "accepted";
pub const STATUS_PARTIALLY_ALLOCATED: &str = "partially_allocated";
pub const STATUS_IN_TRANSIT: &str = "in_transit";
pub const STATUS_DELAYED: &str = "delayed";
pub const STATUS_ACCIDENT: &str = "accident";
pub const STATUS_ARRIVED: &str = "arrived";
pub const STATUS_DELIVERED: &str = "delivered";
pub const STATUS_CANCELLED: &str = "cancelled";
pub const STATUS_PENDING_PAYMENT: &str = "pending_payment";
pub const STATUS_SETTLED: &str = "settled";
pub const STATUS_COMPLETED: &str = "completed";

/// 将 `zc_id_stus-trade.code`（ST-* 或中文或已是前端键）映射为统一前端状态键。
///
/// 未知 / 缺失值默认降级为 `pending_review`（流程尚未开始的最安全状态）。
pub fn map_status(code: Option<&str>) -> String {
    let raw = match code {
        Some(s) => s.trim(),
        None => return STATUS_PENDING_REVIEW.to_string(),
    };
    if raw.is_empty() {
        return STATUS_PENDING_REVIEW.to_string();
    }

    // 已是合法前端键：原样返回
    if matches!(
        raw,
        "pending_review"
            | "pending"
            | "pending_loading"
            | "wait_load"
            | "accepted"
            | "partially_allocated"
            | "loaded"
            | "in_transit"
            | "delayed"
            | "accident"
            | "arrived"
            | "delivered"
            | "partially_arrived"
            | "cancelled"
            | "pending_payment"
            | "uninvoiced"
            | "settled"
            | "completed"
    ) {
        return raw.to_string();
    }

    // DB code 映射（st.code — 如 ST-DISPATCHED；服务订单状态落 stus-service 叶）
    if raw.starts_with("ST-") {
        return match raw {
            "ST-ORDERED" | "ST-PREPARING" => STATUS_PENDING_REVIEW,
            "ST-ACCEPTED" => STATUS_ACCEPTED,
            // 批注轮 75（60656c5d）：ST-DISPATCHED（刚派车分配车辆）≠ 在途——归待装车（前端 wait_load）；
            // ST-IN_TRANSIT（实际出发）才是在途
            "ST-DISPATCHED" => STATUS_PENDING_REVIEW,
            "ST-IN_TRANSIT" => STATUS_IN_TRANSIT,
            "ST-ACCIDENT" => STATUS_ACCIDENT,
            "ST-ARRIVED" => STATUS_ARRIVED,
            "ST-SIGNED" | "ST-DELIVERED" => STATUS_DELIVERED,
            "ST-COMPLETED" => STATUS_COMPLETED,
            "ST-CANCELLED" => STATUS_CANCELLED,
            "ST-SETTLED" => STATUS_SETTLED,
            "ST-PENDING_PAYMENT" => STATUS_PENDING_PAYMENT,
            _ => STATUS_PENDING_REVIEW,
        }
        .to_string();
    }

    // 中文 / 内部标识映射。覆盖业务常见业务阶段。
    let mapped = match raw {
        "待审核" | "待受理" | "待派车" | "待装车" | "待发运" | "待发" | "待提货" => {
            STATUS_PENDING_REVIEW
        }
        "已受理" => STATUS_ACCEPTED,
        "已装车" | "已发车" | "装车完成" | "提货完成" => STATUS_PARTIALLY_ALLOCATED,
        "运输中" | "在途" | "已发运" => STATUS_IN_TRANSIT,
        "延期" | "已延迟" | "异常" | "延误" => STATUS_DELAYED,
        "已签收" | "已抵达" | "已到达" | "已送达" => STATUS_ARRIVED,
        "待付款" | "待结算" => STATUS_PENDING_PAYMENT,
        "已清算" | "清算完成" | "已结清" | "结清完成" => STATUS_SETTLED,
        "已完成" | "完成" => STATUS_COMPLETED,
        "已取消" => STATUS_CANCELLED,
        _ => STATUS_PENDING_REVIEW,
    };
    mapped.to_string()
}

/// 判断某状态是否处于"执行期"（可用于过渡期校验的辅助谓词）
pub fn is_execution_phase(code: Option<&str>) -> bool {
    matches!(
        map_status(code).as_str(),
        STATUS_IN_TRANSIT | STATUS_DELAYED | STATUS_ACCIDENT
    )
}

/// 委托单上下文状态映射（用户 2026-08-19 状态规则）：
/// 委托单的「已派车(ST-DISPATCHED)」对外展示为「部分分配」（派车成功即有剩余货量）；
/// 「在途/已送达」是运单状态，委托单不使用。运单读侧继续用 `map_status`（ST-DISPATCHED → in_transit）。
pub fn map_consignment_status(code: Option<&str>) -> String {
    match code.map(str::trim) {
        Some("ST-DISPATCHED") => STATUS_PARTIALLY_ALLOCATED.to_string(),
        other => map_status(other),
    }
}

/// 委托是否**尚未受理**（受理动作的前置）——受理守卫的单一来源判定。
///
/// 复用 `map_consignment_status` 的语义（`pending_review` = 流程尚未开始）：
/// 新建/待受理族（`ST-NEW`、缺省、`ST-ORDERED`、`ST-PREPARING`）为 `true`；
/// 已受理及之后（`accepted`/`partially_allocated`/终态/事故）与在途/抵达/签收为 `false`。
///
/// 门禁用例 MUST 走本谓词，**禁止**在守卫里硬编码 `ST-*` code 白名单——委托创建改落
/// 初始状态桥（`ST-NEW`「待受理」）后，常量白名单必然漂移（批注 1862daf2 / 2026-09-14）。
pub fn is_pre_acceptance(code: Option<&str>) -> bool {
    map_consignment_status(code) == STATUS_PENDING_REVIEW
}

/// 委托是否**可派车**——派车守卫的单一来源判定。
///
/// 可派 = 流程未开始（`pending_review` 族：`ST-NEW`/缺省、`ST-ORDERED`、`ST-PREPARING`）
/// ∪ 已受理（`accepted`）∪ 已派车（`partially_allocated`，委托语境即 `ST-DISPATCHED`）；
/// 在途/抵达/签收/结算/完结/取消/事故一律 `false`（守卫 fail-closed 拒之）。
///
/// 同 `is_pre_acceptance`：MUST 复用它而不是在守卫里另立 code 字面量（批注 1862daf2）。
pub fn is_dispatchable_consignment(code: Option<&str>) -> bool {
    let mapped = map_consignment_status(code);
    mapped == STATUS_PENDING_REVIEW
        || mapped == STATUS_ACCEPTED
        || mapped == STATUS_PARTIALLY_ALLOCATED
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 批注 1862daf2 回归：委托创建落初始状态桥 `ST-NEW`「待受理」⇒ 派车守卫 MUST 放行，
    /// 且判定 MUST 与 `map_consignment_status` 同源（禁 code 字面量白名单）。
    #[test]
    fn dispatchable_fleet_matches_mapping() {
        // 放行：新建/待受理族 + 已受理 + 已派车（含缺省 = 无桥「新建」）
        for code in [
            Some("ST-NEW"),
            Some("ST-ORDERED"),
            Some("ST-PREPARING"),
            None,
            Some("ST-ACCEPTED"),
            Some("ST-DISPATCHED"),
        ] {
            assert!(
                is_dispatchable_consignment(code),
                "{code:?} 应可派车（映射 {}）",
                map_consignment_status(code)
            );
        }
        // 拒绝：执行期/终态/事故 fail-closed
        for code in [
            "ST-IN_TRANSIT",
            "ST-ARRIVED",
            "ST-SIGNED",
            "ST-DELIVERED",
            "ST-SETTLED",
            "ST-COMPLETED",
            "ST-CANCELLED",
            "ST-ACCIDENT",
        ] {
            assert!(
                !is_dispatchable_consignment(Some(code)),
                "{code} 不应可派车"
            );
        }
    }

    /// 受理门禁：仅受理前族可受理（ST-NEW 放行；已受理及之后拒，杜绝状态回退）。
    #[test]
    fn pre_acceptance_only_pending_review_family() {
        for code in [
            Some("ST-NEW"),
            Some("ST-ORDERED"),
            Some("ST-PREPARING"),
            None,
        ] {
            assert!(is_pre_acceptance(code), "{code:?} 应可受理");
        }
        for code in [
            "ST-ACCEPTED",
            "ST-DISPATCHED",
            "ST-IN_TRANSIT",
            "ST-ARRIVED",
            "ST-SIGNED",
            "ST-CANCELLED",
            "ST-SETTLED",
        ] {
            assert!(!is_pre_acceptance(Some(code)), "{code} 不应可受理");
        }
    }
}
