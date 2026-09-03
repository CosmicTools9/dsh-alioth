//! Product P-Number Trigger Templates
//!
//! 业务规则差异通过 3 个布尔开关参数化，SQL 查询全部委托 TemplateEngine。

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

// ============================================
// ProdPNumberTemplate
// ============================================

/// 产品 6-branch 编号生成模板
///
/// 覆盖 sales / purchase / request / made 四类产品表的 `p_number` 自动生成。
pub struct ProdPNumberTemplate {
    name: &'static str,
    applies_to: Vec<String>,
    fallback_prefix: &'static str,
    /// 创意/实例分支使用 demand code（true）还是 provider code（false）
    use_demand_in_creative_instance: bool,
    /// 设计/范例分支使用 demand code（true）还是 provider code（false）
    use_demand_in_design_example: bool,
    /// 实现/范例分支使用 demand code（true）还是 provider code（false）
    use_demand_in_implement_example: bool,
}

impl ProdPNumberTemplate {
    pub fn sales() -> Self {
        Self {
            name: "tf_bf_ups_81_on_zc_id_prod-sales",
            applies_to: vec!["zc_id_prod-sales".to_string()],
            fallback_prefix: "PS",
            use_demand_in_creative_instance: false,
            use_demand_in_design_example: false,
            use_demand_in_implement_example: false,
        }
    }
    pub fn purchase() -> Self {
        Self {
            name: "tf_bf_ups_81_on_zc_id_prod-purchase",
            applies_to: vec!["zc_id_prod-purchase".to_string()],
            fallback_prefix: "PP",
            use_demand_in_creative_instance: true,
            use_demand_in_design_example: true,
            use_demand_in_implement_example: true,
        }
    }
    pub fn request() -> Self {
        Self {
            name: "tf_bf_ups_81_on_zc_id_prod-request",
            applies_to: vec!["zc_id_prod-request".to_string()],
            fallback_prefix: "PR",
            use_demand_in_creative_instance: false,
            use_demand_in_design_example: false,
            use_demand_in_implement_example: false,
        }
    }
    pub fn made() -> Self {
        Self {
            name: "tf_bf_ups_81_on_zc_id_prod-made",
            applies_to: vec!["zc_id_prod-made".to_string()],
            fallback_prefix: "PM",
            use_demand_in_creative_instance: true,
            use_demand_in_design_example: false,
            use_demand_in_implement_example: true,
        }
    }
}

#[async_trait]
impl TriggerTemplate for ProdPNumberTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: self.name.to_string(),
            applies_to: self.applies_to.clone(),
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

        let notice: Option<String> = get_field(new, "notice");
        let _f_: Option<String> = get_field(new, "_f_");
        let _t_: Option<String> = get_field(new, "_t_");
        let fk_subj_demand: Option<i64> = get_field(new, "fk_subj-demand");
        let fk_subj_provider: Option<i64> = get_field(new, "fk_subj-provider");

        let engine = TemplateEngine::new(ctx.pool.clone());

        // 无 pool fallback
        if ctx.pool.is_none() {
            let id: i64 = get_field(new, "id").unwrap_or(0);
            let p_number = format!(
                "{}-{}-{}",
                self.fallback_prefix,
                ctx.timestamp.format("%Y%m%d"),
                crc32_hex(&id.to_string())
            );
            return Ok(
                TriggerResult::new().with_modified_field("p_number", Value::String(p_number))
            );
        }

        let dcode = match fk_subj_demand {
            Some(id) => engine.resolve_variable_code_notice(id).await?,
            None => None,
        };
        let pcode = match fk_subj_provider {
            Some(id) => engine.resolve_variable_code_notice(id).await?,
            None => None,
        };

        let form = _f_.as_deref().unwrap_or("");
        let typ = _t_.as_deref().unwrap_or("");
        let base = notice.unwrap_or_default();

        let p_number = match (form, typ) {
            ("创意", "范例") => format!("{}-!.", base),
            ("创意", "实例") => {
                let code = if self.use_demand_in_creative_instance {
                    &dcode
                } else {
                    &pcode
                };
                format!(
                    "{}-{}{}-!_",
                    base,
                    wrap_code(code.as_deref().unwrap_or("")),
                    ""
                )
            }
            ("设计", "范例") => {
                let code = if self.use_demand_in_design_example {
                    &dcode
                } else {
                    &pcode
                };
                format!(
                    "{}-{}{}-↑.",
                    base,
                    wrap_code(code.as_deref().unwrap_or("")),
                    ""
                )
            }
            ("设计", "实例") => {
                let d = dcode.as_deref().unwrap_or("");
                let p = pcode.as_deref().unwrap_or("");
                format!("{}-{}{}{}-↑_", base, demand_arrow(d), provider_close(p), "")
            }
            ("实现", "范例") => {
                let code = if self.use_demand_in_implement_example {
                    &dcode
                } else {
                    &pcode
                };
                format!(
                    "{}-{}{}-↓.",
                    base,
                    wrap_code(code.as_deref().unwrap_or("")),
                    ""
                )
            }
            ("实现", "实例") => {
                let d = dcode.as_deref().unwrap_or("");
                let p = pcode.as_deref().unwrap_or("");
                format!("{}-{}{}{}-↓_", base, demand_arrow(d), provider_close(p), "")
            }
            _ => {
                let id: i64 = get_field(new, "id").unwrap_or(0);
                format!(
                    "{}-{}-{}",
                    self.fallback_prefix,
                    ctx.timestamp.format("%Y%m%d"),
                    crc32_hex(&id.to_string())
                )
            }
        };

        Ok(TriggerResult::new().with_modified_field("p_number", Value::String(p_number)))
    }
}

// ============================================
// 格式化辅助
// ============================================

fn wrap_code(s: &str) -> String {
    if s.is_empty() {
        "".to_string()
    } else {
        format!("{{{}}}", s)
    }
}

fn demand_arrow(d: &str) -> String {
    if d.is_empty() {
        "".to_string()
    } else {
        format!("{{{}→", d)
    }
}

fn provider_close(p: &str) -> String {
    if p.is_empty() {
        "".to_string()
    } else {
        format!("{}}}", p)
    }
}
