//! 数据新鲜度面（NGAC 侧图投影退役后的替代契约；change `retire-ngac-age-graph-projection`）。
//!
//! 背景：`isahl_auth` 的 AGE 图投影（037 引导 + 038 rebuild-only）曾承担「管理面图分析」的
//! 数据面。2026-10-10 用户裁决**退役**该投影：分析面（SoD / 可达性 / explain-paths）直接读
//! **关系单源**（`ngac_association` / `ngac_prohibition` / `effective_ua_ids`），与决策同一真相
//! ⇒ 无投影可漂移、无新鲜度机制需要维护。
//!
//! 本模块保留响应契约形状（`projection` 字段：`graph_ready` / `drifted_entities` /
//! `projection_drift` / `stale`）以稳定前端契约，但取值恒为「关系单源当前态」。
//! 知识域图投影（`isahl_knowledge`，022）**不受影响**，归 Gateway seed 链维护。

/// 投影健康面（fail-visible 契约保留；语义 = 关系单源新鲜度）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct DriftReport {
    /// 数据面就绪（关系单源恒就绪）。
    pub graph_ready: bool,
    /// 漂移非零的实体种数（关系单源直读 ⇒ 恒 0）。
    pub drifted_entities: i64,
    /// 漂移总量（恒 0）。
    pub projection_drift: i64,
}

impl DriftReport {
    /// 关系单源当前态：恒就绪、零漂移（无投影 ⇒ 无陈旧面）。
    pub fn fresh() -> Self {
        Self {
            graph_ready: true,
            drifted_entities: 0,
            projection_drift: 0,
        }
    }

    /// 陈旧判定（契约字段保留；关系单源直读 ⇒ 恒 `false`）
    /// （规约 :: `analysis-projection-health-shape`）。
    pub fn stale(&self) -> bool {
        self.projection_drift != 0
    }
}
