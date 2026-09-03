//! dk 静态绑定（BACKEND_FRAMEWORK §7.3.3 2026-08-12 裁定）：实体/接口 → 坐标三元组 code，
//! 运行时经 ontology_binding::resolve 解析 code→id。语义校准只改本文件 coords()。
//!
//! ApprovalFlow 坐标 = (JC, FTA, ↑_NA)：
//! - scene JC（系统管理）：审批流程承载于系统管理场景（与 seed FLOW-STD/FLOW-URGENT 一致）
//! - factor FTA（审批内容）：审批内容维度
//! - function ↑_NA（审批方案）：审批方案功能
//!
//! 与 GateTemplate（JE/FUA/↓_NA）共享 zc_id_process 时以坐标区分（ALIOTH_ONTOLOGY_SPEC §4.3）。
//! ⚠️ 历史缺陷：旧实现硬编码 bind(515/522/526) 为悬空 ZUID（AVIC 库无此维度 id），创建行 dk 悬空
//! 违反坐标静态绑定规约——本文件为修复载体。

use ontology_binding::{Coords, DkBinding};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DkEntity {
    /// JC/FTA/↑_NA
    DkJcFtaNa,
}

impl DkBinding for DkEntity {
    fn coords(&self) -> Coords {
        match self {
            DkEntity::DkJcFtaNa => ("JC", "FTA", "↑_NA"),
        }
    }
}

pub(crate) async fn resolve_ontology_coords_pool(
    pool: &sqlx::PgPool,
    entity: DkEntity,
) -> Result<(Option<i64>, Option<i64>, Option<i64>), sqlx::Error> {
    ontology_binding::resolve(pool, entity.coords()).await
}
