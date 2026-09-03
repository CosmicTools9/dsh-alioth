//! zc_id_object Level Trigger Templates

use crate::{
    template::{TriggerMetadata, TriggerOperationDef, TriggerTemplate, TriggerTimingDef},
    utils::*,
    TriggerContext, TriggerError, TriggerResult,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

// ============================================
// Object O-Number Template
// ============================================

/// 对象编号生成模板
///
/// BEFORE INSERT 时生成 `o_number`。
pub struct ObjectONumberTemplate;

#[async_trait]
impl TriggerTemplate for ObjectONumberTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ins_93_on_zc_id_object".to_string(),
            applies_to: vec![
                "zc_id_object",
                "zc_id_evaluation",
                "zc_id_factor",
                "zc_id_function",
                "zc_id_scene",
                "zc_id_category",
                "zc_id_status",
                "zc_id_tags",
                "zc_id_unit",
                "zc_id_consensus",
                "zc_id_lifecycle",
                "zc_id_bill",
                "zc_id_event",
                "zc_id_entity",
                "zc_id_agreement",
                "zc_id_statement",
                "zc_id_version",
                "zc_id_contract",
                "zc_id_contacts",
                "zc_id_detail",
                "zc_id_identity",
                "zc_id_invoice",
                "zc_id_prod-license",
                "zc_id_message",
                "zc_id_place",
                "zc_id_plan",
                "zc_id_protocol",
                "zc_id_storage",
                "zc_id_threads",
                "zc_id_bom",
                "zc_id_document",
                "zc_id_law",
                "zc_id_file-manual",
                "zc_id_operation",
                "zc_id_project",
                "zc_id_process",
                "zc_id_production",
                "zc_id_standard",
                "zc_id_task",
                "zc_id_even-approve",
                "zc_id_appr-purchase",
                "zc_id_bill-check",
                "zc_id_bom-assemble",
                "zc_id_vers-context",
                "zc_id_subj-hierarchy",
                "zc_id_subj-position",
                "zc_id_subjects",
                "zc_id_stor-account",
                "zc_id_scale",
                "zc_id_scal-date",
                "zc_id_segment",
                "zc_id_segm-date",
                "zc_id_formula",
                "zc_id_rate",
                "zc_id_ratio",
                "zc_id_stat-sto-voucher",
                "zc_id_stat-trade_order",
                "zc_id_deta-trade_order",
                "zc_id_prod-sales",
                "zc_id_prod-request",
                "zc_id_prod-made",
                "zc_id_prod-purchase",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            operations: vec![TriggerOperationDef::Insert],
            timing: TriggerTimingDef::Before,
        }
    }

    async fn execute(
        &self,
        ctx: &TriggerContext,
        _old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let new = new_record.ok_or_else(|| {
            TriggerError::ExecutionFailed("New record required for INSERT trigger".to_string())
        })?;

        let id: i64 = get_field(new, "id").unwrap_or(0);
        let name: Option<String> = get_field(new, "notice");
        let o_number = generate_o_number(id, name.as_deref(), &ctx.timestamp);

        Ok(TriggerResult::new().with_modified_field("o_number", Value::String(o_number)))
    }
}

// ============================================
// Object I18n Template
// ============================================

/// i18n 元数据同步模板
///
/// 在 notice 变更后同步 `isahl_meta.meta_fields` 的 enum 值。
pub struct ObjectI18nTemplate;

#[async_trait]
impl TriggerTemplate for ObjectI18nTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_af_ups_89_on_zc_id_object".to_string(),
            applies_to: vec![
                "zc_id_object",
                "zc_id_evaluation",
                "zc_id_factor",
                "zc_id_function",
                "zc_id_scene",
                "zc_id_category",
                "zc_id_status",
                "zc_id_tags",
                "zc_id_unit",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            operations: vec![
                TriggerOperationDef::Insert,
                TriggerOperationDef::Update,
                TriggerOperationDef::Delete,
            ],
            timing: TriggerTimingDef::After,
        }
    }

    async fn execute(
        &self,
        ctx: &TriggerContext,
        old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let mut result = TriggerResult::new();

        // Gateway 禁止访问 isahl_meta，跳过 meta_fields 同步副作用
        if ctx.app_container == crate::AppContainer::Gateway {
            return Ok(result);
        }

        let old_notice: Option<String> = old_record.and_then(|r| get_field(r, "notice"));
        let new_notice: Option<String> = new_record.and_then(|r| get_field(r, "notice"));

        match (old_notice, new_notice) {
            (Some(old), None) => {
                result.side_effects.push(crate::SideEffect::RawSql(format!(
                    "UPDATE isahl_meta.meta_fields SET options = jsonb_set(options::jsonb, '{{uiSchema, enum}}', (options->'uiSchema'->'enum') - (SELECT index - 1 FROM jsonb_array_elements(options->'uiSchema'->'enum') WITH ORDINALITY arr(element, index) WHERE element->>'value' = '{}')::int, false)::json WHERE fk_collection = (SELECT table_name FROM isahl_meta.meta_collections WHERE name = '{}') AND name = 'notice' AND interface = 'select'",
                    old, ctx.table_name
                )));
            }
            (None, Some(new)) => {
                result.side_effects.push(crate::SideEffect::RawSql(format!(
                    "UPDATE isahl_meta.meta_fields SET options = jsonb_insert(options::jsonb, '{{uiSchema, enum, 0}}', jsonb_build_object('value', '{}', 'label', '{}', 'color', 'default'), false)::json WHERE fk_collection = (SELECT table_name FROM isahl_meta.meta_collections WHERE name = '{}') AND name = 'notice' AND interface = 'select'",
                    new, new, ctx.table_name
                )));
            }
            (Some(old), Some(new)) if old != new => {
                result.side_effects.push(crate::SideEffect::RawSql(format!(
                    "UPDATE isahl_meta.meta_fields SET options = jsonb_insert(jsonb_set(options::jsonb, '{{uiSchema, enum}}', (options->'uiSchema'->'enum') - (SELECT index - 1 FROM jsonb_array_elements(options->'uiSchema'->'enum') WITH ORDINALITY arr(element, index) WHERE element->>'value' = '{}')::int, false), '{{uiSchema, enum, 0}}', jsonb_build_object('value', '{}', 'label', '{}', 'color', 'default'), false)::json WHERE fk_collection = (SELECT table_name FROM isahl_meta.meta_collections WHERE name = '{}') AND name = 'notice' AND interface = 'select'",
                    old, new, new, ctx.table_name
                )));
            }
            _ => {}
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TriggerOperation;

    #[tokio::test]
    async fn test_o_number_generation() {
        let tpl = ObjectONumberTemplate;
        let mut new_record = HashMap::new();
        new_record.insert("id".to_string(), Value::Number(123i64.into()));
        new_record.insert("notice".to_string(), Value::String("entity".to_string()));

        let ctx = TriggerContext::new("zc_id_scene", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new_record)).await.unwrap();

        assert!(result.modified_fields.contains_key("o_number"));
        let o_number = result
            .modified_fields
            .get("o_number")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(o_number.len() > 20);
        assert!(o_number.contains("_"));
    }
}
