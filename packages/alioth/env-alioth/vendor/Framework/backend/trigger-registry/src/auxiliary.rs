//! Auxiliary Trigger Templates

use crate::{
    template::{TriggerMetadata, TriggerOperationDef, TriggerTemplate, TriggerTimingDef},
    utils::*,
    TriggerContext, TriggerError, TriggerResult,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

// ============================================
// Production After Template
// ============================================

/// 生产记录级联更新模板
///
/// 当生产记录的 `fk_previous` 被设置时，将新的生产 id 级联到相关贸易订单明细。
pub struct ProductionAfterTemplate;

#[async_trait]
impl TriggerTemplate for ProductionAfterTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_af_ups_55_on_zc_id_production".to_string(),
            applies_to: vec!["zc_id_production".to_string()],
            operations: vec![TriggerOperationDef::Insert, TriggerOperationDef::Update],
            timing: TriggerTimingDef::After,
        }
    }

    async fn execute(
        &self,
        _ctx: &TriggerContext,
        _old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let new = new_record
            .ok_or_else(|| TriggerError::ExecutionFailed("New record required".to_string()))?;

        let new_id: i64 = get_field(new, "id").unwrap_or(0);
        let previous: Option<i64> = get_field(new, "fk_previous");

        if new_id == 0 {
            return Ok(TriggerResult::new());
        }
        let Some(prev_id) = previous else {
            return Ok(TriggerResult::new());
        };

        let mut result = TriggerResult::new();
        for col in ["fk_demand", "fk_goods", "fk_deal", "fk_delivery"] {
            result = result.with_side_effect(crate::SideEffect::RawSql(format!(
                r#"UPDATE isahl."zc_id_deta-trade_order" SET {} = {}, updated_at = NOW() WHERE {} = {}"#,
                col, new_id, col, prev_id
            )));
        }
        Ok(result)
    }
}

// ============================================
// Production Delete Template
// ============================================

/// 生产记录删除清理模板
///
/// 删除生产记录时清理关联的计数、库存和凭证记录。
pub struct ProductionDeleteTemplate;

#[async_trait]
impl TriggerTemplate for ProductionDeleteTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_af_del_60_on_zc_id_production".to_string(),
            applies_to: vec!["zc_id_production".to_string()],
            operations: vec![TriggerOperationDef::Delete],
            timing: TriggerTimingDef::After,
        }
    }

    async fn execute(
        &self,
        _ctx: &TriggerContext,
        old_record: Option<&HashMap<String, Value>>,
        _new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let old = old_record
            .ok_or_else(|| TriggerError::ExecutionFailed("Old record required".to_string()))?;

        let id: i64 = get_field(old, "id").unwrap_or(0);
        if id == 0 {
            return Ok(TriggerResult::new());
        }

        let mut result = TriggerResult::new();
        // 库存 = 统计关系（用户 2026-08-07 定稿）：删除 production 时清理
        // zc_id_production_rr_storage（ref_left=production 的统计关系行）与
        // zc_id_relation-inventory_r_status（指向这些行的状态关系）。
        // zc_id_counting/zc_id_inventory 旧对象模型表已从 dev DB 删除（不引用）。
        result = result.with_side_effect(crate::SideEffect::RawSql(format!(
            r#"DELETE FROM isahl."zc_id_production_rr_storage" WHERE ref_left = {}"#,
            id
        )));
        // 状态关系经 rr_storage 行 id 级联（ref_left 指向 production_rr_storage 行）
        result = result.with_side_effect(crate::SideEffect::RawSql(format!(
            r#"DELETE FROM isahl."zc_id_relation-inventory_r_status"
               WHERE ref_left IN (SELECT id FROM isahl."zc_id_production_rr_storage" WHERE ref_left = {})"#,
            id
        )));
        result = result.with_side_effect(crate::SideEffect::RawSql(format!(
            r#"DELETE FROM isahl."zc_id_stat-sto-voucher" WHERE fk_production = {}"#,
            id
        )));
        Ok(result)
    }
}
