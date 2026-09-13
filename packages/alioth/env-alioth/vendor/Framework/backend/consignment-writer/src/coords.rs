//! 本体坐标解析（迁移自 transport-dispatch repositories/mod.rs 的 DkEntity 子集）：
//! BACKEND_FRAMEWORK §7.3.3 API 静态绑定——写链所用两个实体坐标固定，禁运行时推导/前端传值。

use ontology_binding::DkBinding;

/// 写链使用的坐标实体（dispatch 本地枚举的其余变体留在 dispatch）
///
/// `TradeOrderDetail` 为声明即预留的明细行坐标（`deta-trade_order` 写点当前经
/// SQL 内联坐标，尚未切换到本枚举）——保留声明以固化坐标口径，消 dead_code 警告。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriterDk {
    /// TSP 凭证出腿——源池扣减（实现·范例·装载标准）
    TspTemplateLeg,
    /// 委托创建（交易产品 + com 凭证，实现·实例·装载执行）
    ConsignmentTrade,
    /// 委托/运单订单行（`zc_id_orde-land`，交易域）
    OrderDocument,
    /// 装载包装产品（`zc_id_prod-loading`，交易域装载标准）
    LoadingPackage,
    /// 交易明细行（`zc_id_deta-trade_order`）
    TradeOrderDetail,
}

impl DkBinding for WriterDk {
    fn coords(&self) -> ontology_binding::Coords {
        match self {
            WriterDk::TspTemplateLeg => ("GC", "FJA", "↓.BE"),
            WriterDk::ConsignmentTrade => ("GC", "FJA", "↓_BE"),
            WriterDk::OrderDocument => ("TX", "FJA", "↓_EV"),
            WriterDk::LoadingPackage => ("GC", "FJA", "↓_GG"),
            WriterDk::TradeOrderDetail => ("TX", "FJA", "↓_GG"),
        }
    }
}

impl WriterDk {
    /// 站点态位 = dk_function.code 前缀派生（trigger_registry::derive_form_type；
    /// `_f_`/`_t_` 禁止字面量直写——由本方法取值后参数绑定）
    pub(crate) fn form_type(&self) -> (&'static str, &'static str) {
        trigger_registry::lifecycle::derive_form_type(self.coords().2)
            .expect("WriterDk 职能码必须是六象限前缀")
    }
}

pub(crate) async fn resolve_coords(
    conn: &mut sqlx::PgConnection,
    entity: WriterDk,
) -> Result<(Option<i64>, Option<i64>, Option<i64>), sqlx::Error> {
    ontology_binding::resolve_conn(conn, entity.coords()).await
}
