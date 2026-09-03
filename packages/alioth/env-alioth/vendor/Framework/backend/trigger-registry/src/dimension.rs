//! zc_ad_dimension Level Trigger Templates

use crate::{
    template::{TriggerMetadata, TriggerOperationDef, TriggerTemplate, TriggerTimingDef},
    utils::*,
    TriggerContext, TriggerError, TriggerResult,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

// ============================================
// Lifecycle Injective Template
// ============================================

/// 单射内容预处理模板
///
/// 从 `notice` + 引用字段计算 `number` 哈希。
pub struct LifecycleInjectiveTemplate;

#[async_trait]
impl TriggerTemplate for LifecycleInjectiveTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "gf_gen_tf_bf_ups_on_var_injective".to_string(),
            applies_to: crate::lifecycle::ZC_ID_LIFECYCLE_TABLES
                .iter()
                .map(|&s| s.to_string())
                .collect(),
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
        let new = new_record
            .ok_or_else(|| TriggerError::ExecutionFailed("New record required".to_string()))?;

        let notice: Option<String> = get_field(new, "notice");
        if notice.is_none() {
            return Ok(TriggerResult::new());
        }

        let mut keys: Vec<String> = Vec::new();
        for (key, value) in new.iter() {
            if (key.starts_with("qk_")
                || key.starts_with("tk_")
                || key.starts_with("ck_")
                || key.starts_with("sk_")
                || key.starts_with("lk_"))
                && value.is_number()
            {
                keys.push(key.clone());
            }
        }

        let notice_str = notice.unwrap();
        let hash = if keys.is_empty() {
            crc32_hex(&notice_str)
        } else {
            crc32_hex(&format!("{}-{}", notice_str, keys.join(",")))
        };

        Ok(TriggerResult::new()
            .with_modified_field("projection", Value::String(format!("DIM-{}", hash))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TriggerOperation;

    #[tokio::test]
    async fn test_injective_trigger() {
        let tpl = LifecycleInjectiveTemplate;
        let mut new_record = HashMap::new();
        new_record.insert("notice".to_string(), Value::String("Test".to_string()));
        new_record.insert("qk_qty".to_string(), Value::Number(50i64.into()));

        let ctx = TriggerContext::new("zc_id_invoice", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new_record)).await.unwrap();

        assert!(result.modified_fields.contains_key("projection"));
    }
}
