//! 本体绑定域（DkEntity + 坐标解析）（迁自 transport-dispatch `repositories/ontology.rs`）。
//!
//! BACKEND_FRAMEWORK §7.3.3 API 静态绑定——写链所用实体坐标固定，禁运行时推导/前端传值。

use ontology_binding::DkBinding;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DkEntity {
    /// TSP 凭证出腿——源池扣减（实现·范例·装载标准）
    TspTemplateLeg,
    /// TSP 凭证入腿——目标池落位（实现·实例·装载执行）
    TspInstanceLeg,
    /// 委托创建（交易产品 + com 凭证，实现·实例·装载执行）
    ConsignmentTrade,
    /// 产品管理创建——目录产品（设计·实例·装载方案）
    FreightProduct,
    /// 容量池产品/容量行（实现·范例·装载标准）
    CapacityConfig,
}

impl DkBinding for DkEntity {
    /// 实体 → 固定三元组 code（语义分离时在此处校准，一个点）
    fn coords(&self) -> ontology_binding::Coords {
        match self {
            DkEntity::TspTemplateLeg | DkEntity::CapacityConfig => ("GC", "FJA", "↓.BE"),
            DkEntity::TspInstanceLeg | DkEntity::ConsignmentTrade => ("GC", "FJA", "↓_BE"),
            DkEntity::FreightProduct => ("GC", "FJA", "↑_BE"),
        }
    }
}

impl DkEntity {
    /// 站点态位 = `dk_function.code` 前缀派生（唯一规则：trigger_registry::derive_form_type；
    /// `_f_`/`_t_` 禁止字面量直写——裸 SQL 路径由本方法取值后参数绑定）。
    pub fn form_type(&self) -> (&'static str, &'static str) {
        trigger_registry::lifecycle::derive_form_type(self.coords().2)
            .expect("DkEntity 职能码必须是六象限前缀")
    }
}

pub async fn resolve_ontology_coords(
    conn: &mut sqlx::PgConnection,
    entity: DkEntity,
) -> Result<(Option<i64>, Option<i64>, Option<i64>), sqlx::Error> {
    ontology_binding::resolve_conn(conn, entity.coords()).await
}
