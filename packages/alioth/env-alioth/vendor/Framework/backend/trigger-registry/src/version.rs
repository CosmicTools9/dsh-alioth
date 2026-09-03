//! Version Trigger Templates
//!

use crate::{
    template::{TriggerMetadata, TriggerOperationDef, TriggerTemplate, TriggerTimingDef},
    utils::*,
    TriggerContext, TriggerError, TriggerResult,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

/// Version head_flag 管理模板
///
/// 当 fk_previous 存在时设置 head_flag。
pub struct VersionHeadFlagTemplate;

#[async_trait]
impl TriggerTemplate for VersionHeadFlagTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ups_75_on_zc_id_version".to_string(),
            applies_to: vec!["zc_id_version".to_string()],
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

        let fk_previous: Option<i64> = get_field(new, "fk_previous");
        if fk_previous.is_none() {
            return Ok(TriggerResult::new());
        }

        Ok(TriggerResult::new().with_modified_field("head_flag", Value::Bool(true)))
    }
}
