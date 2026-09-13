//! 派车入参模型（迁自 transport-dispatch `repositories/mod.rs`）。

use rust_decimal::Decimal;

pub struct VehicleAllocation {
    pub vehicle_id: i64,
    pub allocated_weight: Decimal,
    /// 司机 ID（可选；派车后写入在途追踪行 fk_operator）
    pub driver_id: Option<i64>,
}

/// 车辆调度入参（dispatch_vehicles_tx / dispatch_vehicles_tx_inner）
pub struct DispatchParams<'a> {
    pub consign_code: &'a str,
    pub consignment_id: i64,
    pub allocations: &'a [VehicleAllocation],
    pub capacity_product_id: Option<i64>,
    pub purchase_price: Option<f64>,
    pub carrier_id: Option<i64>,
    pub user_id: i64,
}
