//! BOM 编号同步模板：`b_number`（结构化规范输入）→ `code`（最终承载列）。
//!
//! 域主裁定（2026-09-16）：`b_number` 是结构化规范输入，由**前端**负责自动生成；
//! `code` 是 BOM 编号的最终承载列。写入 `b_number`（非空）时 `code` 复制其值；
//! 未写入 `b_number` 时 `code` 由应用直写（本模板不动它）。
//! 后端只做同步、不再自动计算编号（原 `BomBNumberTemplate` 已随
//! `remove-bom-bnumber-autocompute-trigger` 移除）。

use crate::{
    template::{TriggerMetadata, TriggerOperationDef, TriggerTemplate, TriggerTimingDef},
    utils::*,
    TriggerContext, TriggerError, TriggerResult,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

/// bom 族 `b_number → code` 同步。
///
/// 适用表：`zc_id_bom`、`zc_id_bom-assemble`（父表注册，经 pg_catalog 继承图覆盖全部 bom 子孙表）。
pub struct BomCodeSyncTemplate;

#[async_trait]
impl TriggerTemplate for BomCodeSyncTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ups_82_on_zc_id_bom_code_sync".to_string(),
            applies_to: vec!["zc_id_bom".to_string(), "zc_id_bom-assemble".to_string()],
            operations: vec![TriggerOperationDef::Insert, TriggerOperationDef::Update],
            timing: TriggerTimingDef::Before,
        }
    }

    async fn execute(
        &self,
        _ctx: &TriggerContext,
        _old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let Some(new) = new_record else {
            return Ok(TriggerResult::new());
        };
        // 写入 `b_number`（非空）⇒ `code` 复制其值；`b_number` 缺省/为空 ⇒ 不动 `code`（应用直写）。
        match get_field::<String>(new, "b_number") {
            Some(bn) if !bn.is_empty() => {
                Ok(TriggerResult::new().with_modified_field("code", Value::String(bn)))
            }
            _ => Ok(TriggerResult::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TriggerOperation;

    #[tokio::test]
    async fn copies_b_number_to_code_when_present() {
        let tpl = BomCodeSyncTemplate;
        let mut new_record = HashMap::new();
        new_record.insert(
            "b_number".to_string(),
            Value::String("BOM-01-02".to_string()),
        );
        let ctx = TriggerContext::new("zc_id_bom", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new_record)).await.unwrap();
        assert_eq!(
            result.modified_fields.get("code"),
            Some(&Value::String("BOM-01-02".to_string())),
            "写入 b_number ⇒ code MUST 复制其值"
        );
    }

    #[tokio::test]
    async fn leaves_code_untouched_when_b_number_absent() {
        let tpl = BomCodeSyncTemplate;
        let mut new_record = HashMap::new();
        new_record.insert("code".to_string(), Value::String("直写码".to_string()));
        let ctx = TriggerContext::new("zc_id_bom", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new_record)).await.unwrap();
        assert!(
            !result.modified_fields.contains_key("code"),
            "未写 b_number ⇒ code MUST NOT 被同步模板改写（由应用直写）"
        );
    }

    #[tokio::test]
    async fn leaves_code_untouched_when_b_number_empty() {
        let tpl = BomCodeSyncTemplate;
        let mut new_record = HashMap::new();
        new_record.insert("b_number".to_string(), Value::String(String::new()));
        new_record.insert("code".to_string(), Value::String("直写码".to_string()));
        let ctx = TriggerContext::new("zc_id_bom", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new_record)).await.unwrap();
        assert!(
            !result.modified_fields.contains_key("code"),
            "b_number 为空串 ⇒ 视为未写入 ⇒ code MUST NOT 被覆盖"
        );
    }
}
