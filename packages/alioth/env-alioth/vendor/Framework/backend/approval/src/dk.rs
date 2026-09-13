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
//!
//! 2026-09-11 扩充（check-leaf-insert 规约 3 归零）：生命周期叶表写路径按 AVIC 参考库
//! 声明值逐表补坐标实体（每实体 = 一张叶表的固定写入语义；code 取自参考库，勿改注释）。

use ontology_binding::{Coords, DkBinding};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DkEntity {
    /// JC/FTA/↑_NA — 审批流程定义（zc_id_process 族）
    DkJcFtaNa,
    /// JC/FTA/↓_NC — 审批意见 zc_id_deta-opinion
    DkJcFtaNc,
    /// JE/FTA/↓_EZ — 审批实例 zc_id_oper-approve
    DkJeFtaEz,
    /// JE/FBB/↓_EZ — 门禁/自动节点操作 zc_id_oper-gate
    DkJeFbbEz,
    /// TX/FJA/↓_GG — 自然人主数据 zc_id_empl-natural
    DkTxFjaGg,
    /// JE/FBA/↓_AB — 标准/规章模板 zc_id_standard（声明+avic 库现存行）
    DkJeFbaAb,
    /// JE/GEC/↑_DA — prot 配置族 zc_id_prot-profile_config（同族先例 prot-env_config /
    /// prot-oss_config：seed-demo-surfaces.sql:59 / seed-storage-config.sql:17，均 JE·GEC·↑_DA）
    DkJeGecDa,
}

impl DkBinding for DkEntity {
    fn coords(&self) -> Coords {
        match self {
            DkEntity::DkJcFtaNa => ("JC", "FTA", "↑_NA"),
            DkEntity::DkJcFtaNc => ("JC", "FTA", "↓_NC"),
            DkEntity::DkJeFtaEz => ("JE", "FTA", "↓_EZ"),
            DkEntity::DkJeFbbEz => ("JE", "FBB", "↓_EZ"),
            DkEntity::DkTxFjaGg => ("TX", "FJA", "↓_GG"),
            DkEntity::DkJeFbaAb => ("JE", "FBA", "↓_AB"),
            DkEntity::DkJeGecDa => ("JE", "GEC", "↑_DA"),
        }
    }
}

pub(crate) async fn resolve_ontology_coords_pool(
    pool: &sqlx::PgPool,
    entity: DkEntity,
) -> Result<(Option<i64>, Option<i64>, Option<i64>), sqlx::Error> {
    ontology_binding::resolve(pool, entity.coords()).await
}

/// 事务内解析（`&mut *tx` 连接）：先解析后执行 INSERT，避免与执行借用相撞。
pub(crate) async fn resolve_ontology_coords_conn(
    conn: &mut sqlx::PgConnection,
    entity: DkEntity,
) -> Result<(Option<i64>, Option<i64>, Option<i64>), sqlx::Error> {
    ontology_binding::resolve_conn(conn, entity.coords()).await
}
