//! BOM B-Number Trigger Template
//!
//! BEFORE INSERT/UPDATE 时生成 BOM 编号。

use crate::{
    template::{
        TemplateEngine, TriggerMetadata, TriggerOperationDef, TriggerTemplate, TriggerTimingDef,
    },
    utils::*,
    TriggerContext, TriggerError, TriggerResult,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

/// BOM 编号生成模板
///
/// 适用表：`zc_id_bom`、`zc_id_bom-assemble`
pub struct BomBNumberTemplate;

#[async_trait]
impl TriggerTemplate for BomBNumberTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ups_81_on_zc_id_bom".to_string(),
            applies_to: vec!["zc_id_bom".to_string(), "zc_id_bom-assemble".to_string()],
            operations: vec![TriggerOperationDef::Insert, TriggerOperationDef::Update],
            timing: TriggerTimingDef::Before,
        }
    }

    async fn execute(
        &self,
        ctx: &TriggerContext,
        _old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let new = new_record
            .ok_or_else(|| TriggerError::ExecutionFailed("New record required".to_string()))?;

        let engine = TemplateEngine::new(ctx.pool.clone());

        if ctx.pool.is_none() {
            let id: i64 = get_field(new, "id").unwrap_or(0);
            let notice: Option<String> = get_field(new, "notice");
            let b_number = format!(
                "BOM-{}-{}",
                ctx.timestamp.format("%Y%m%d"),
                crc32_hex(&format!("{}-{}", id, notice.unwrap_or_default()))
            );
            return Ok(
                TriggerResult::new().with_modified_field("b_number", Value::String(b_number))
            );
        }

        let notice: Option<String> = get_field(new, "notice");
        let _f_: Option<String> = get_field(new, "_f_");
        let _t_: Option<String> = get_field(new, "_t_");
        let typ: Option<String> = get_field(new, "type");
        let tk_version: Option<i64> = get_field(new, "tk_version");
        let tk_batch_no: Option<i64> = get_field(new, "tk_batch_no");
        let fk_editor: Option<i64> = get_field(new, "fk_editor");

        let version = match tk_version {
            Some(id) => engine.resolve_variable_notice(id).await?,
            None => None,
        };
        let batch = match tk_batch_no {
            Some(id) => engine.resolve_variable_notice(id).await?,
            None => None,
        };
        let ecode = match fk_editor {
            Some(id) => engine.resolve_variable_code_notice(id).await?,
            None => None,
        };

        let form = _f_.as_deref().unwrap_or("");
        let btyp = _t_.as_deref().unwrap_or("");
        let base = notice.unwrap_or_default();
        let type_ = typ.as_deref().unwrap_or("");
        let ver = version.as_deref().unwrap_or("");
        let bat = batch.as_deref().unwrap_or("");
        let ec = ecode.as_deref().unwrap_or("");

        let b_number = match (form, btyp) {
            ("创意", "范例") if !type_.is_empty() => {
                format!("{}{}", base, type_)
            }
            ("创意", "实例") if !type_.is_empty() && !ec.is_empty() && !ver.is_empty() => {
                format!("{}{}-{{{}}}[{}]", base, type_, ec, ver)
            }
            ("设计", "范例") if !type_.is_empty() && !ec.is_empty() && !ver.is_empty() => {
                format!("{}{}-{{{}}}[{}]", base, type_, ec, ver)
            }
            ("设计", "实例") if !type_.is_empty() && !ec.is_empty() && !ver.is_empty() => {
                format!("{}{}-{{{}}}[{}]", base, type_, ec, ver)
            }
            ("实现", "范例") if !type_.is_empty() && !ec.is_empty() && !ver.is_empty() => {
                format!("{}{}-{{{}}}[{}]", base, type_, ec, ver)
            }
            ("实现", "实例")
                if !type_.is_empty() && !ec.is_empty() && !ver.is_empty() && !bat.is_empty() =>
            {
                format!("{}{}-{{{}}}[{}]#{}", base, type_, ec, ver, bat)
            }
            _ => {
                return Err(TriggerError::ValidationFailed(format!(
                    "BOM编号生成失败, _f_={}, _t_={}, notice={}, type={}, fk_editor={}, tk_version={}, tk_batch_no={}",
                    form, btyp, base, type_,
                    fk_editor.map(|v| v.to_string()).unwrap_or_else(|| "NULL".to_string()),
                    tk_version.map(|v| v.to_string()).unwrap_or_else(|| "NULL".to_string()),
                    tk_batch_no.map(|v| v.to_string()).unwrap_or_else(|| "NULL".to_string())
                )));
            }
        };

        Ok(TriggerResult::new().with_modified_field("b_number", Value::String(b_number)))
    }
}
