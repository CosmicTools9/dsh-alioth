//! Operation / Process Number Trigger Templates
//!
//! Operation 与 Process 的 6-branch 格式不同：
//! - Operation 用 `-` 连接 version，`#` 连接 batch，空值保留占位符
//! - Process 用 `[]` 包裹 version，`#` 连接 batch，空值省略

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
// OperationOpNumberTemplate
// ============================================

/// 操作编号生成模板
///
/// BEFORE INSERT/UPDATE 时按 6-branch 矩阵组合 `o_number`。
pub struct OperationOpNumberTemplate;

#[async_trait]
impl TriggerTemplate for OperationOpNumberTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ups_81_on_zc_id_operation".to_string(),
            applies_to: vec!["zc_id_operation".to_string()],
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

        let notice: Option<String> = get_field(new, "notice");

        // No pool fallback
        if ctx.pool.is_none() {
            let id = get_id_field(new, "id").unwrap_or(0);
            let o_number = generate_o_number(id, notice.as_deref(), &ctx.timestamp);
            return Ok(
                TriggerResult::new().with_modified_field("o_number", Value::String(o_number))
            );
        }

        let _f_: Option<String> = get_field(new, "_f_");
        let _t_: Option<String> = get_field(new, "_t_");
        let tk_batch_no: Option<i64> = get_field(new, "tk_batch_no");
        let tk_version: Option<i64> = get_field(new, "tk_version");
        let fk_operator: Option<i64> = get_field(new, "fk_operator");
        let fk_subject: Option<i64> = get_field(new, "fk_subject");

        let batch = match tk_batch_no {
            Some(id) => engine.resolve_variable_notice(id).await?,
            None => None,
        };
        let version = match tk_version {
            Some(id) => engine.resolve_variable_notice(id).await?,
            None => None,
        };
        let dcode = match fk_operator {
            Some(id) => engine.resolve_variable_code_notice(id).await?,
            None => None,
        };
        let scode = match fk_subject {
            Some(id) => engine.resolve_variable_code_notice(id).await?,
            None => None,
        };

        let form = _f_.as_deref().unwrap_or("");
        let typ = _t_.as_deref().unwrap_or("");
        let base = notice.as_deref().unwrap_or("");
        let ver = version.as_deref().unwrap_or("");
        let bat = batch.as_deref().unwrap_or("");
        let dc = dcode.as_deref().unwrap_or("");
        let sc = scode.as_deref().unwrap_or("");

        let o_number = match (form, typ) {
            ("创意", "范例") => format!("{}-!.", base),
            ("创意", "实例") => format!(
                "{}{}{}{}-!_",
                base,
                op_subject_wrap(dc),
                op_ver_sep(ver),
                op_batch_hash(bat)
            ),
            ("设计", "范例") => format!(
                "{}{}{}{}-↑.",
                base,
                op_subject_wrap(dc),
                op_ver_sep(ver),
                op_batch_hash(bat)
            ),
            ("设计", "实例") => format!(
                "{}{}{}{}-↑_",
                base,
                op_subject_colon(sc, dc),
                op_ver_sep(ver),
                op_batch_hash(bat)
            ),
            ("实现", "范例") => format!(
                "{}{}{}{}-↓.",
                base,
                op_subject_wrap(dc),
                op_ver_sep(ver),
                op_batch_hash(bat)
            ),
            ("实现", "实例") => format!(
                "{}{}{}{}-↓_",
                base,
                op_subject_colon(dc, sc),
                op_ver_sep(ver),
                op_batch_hash(bat)
            ),
            _ => {
                let id = get_id_field(new, "id").unwrap_or(0);
                generate_o_number(id, notice.as_deref(), &ctx.timestamp)
            }
        };

        Ok(TriggerResult::new().with_modified_field("o_number", Value::String(o_number)))
    }
}

// ============================================
// ProcessPNumberTemplate
// ============================================

/// 流程编号生成模板
///
/// BEFORE INSERT/UPDATE 时按 6-branch 矩阵组合 `p_number`。
pub struct ProcessPNumberTemplate;

#[async_trait]
impl TriggerTemplate for ProcessPNumberTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ups_81_on_zc_id_process".to_string(),
            applies_to: vec!["zc_id_process".to_string()],
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
            let p_number = format!(
                "PROC-{}-{}",
                ctx.timestamp.format("%Y%m%d"),
                crc32_hex(&id.to_string())
            );
            return Ok(
                TriggerResult::new().with_modified_field("p_number", Value::String(p_number))
            );
        }

        let notice: Option<String> = get_field(new, "notice");
        let _f_: Option<String> = get_field(new, "_f_");
        let _t_: Option<String> = get_field(new, "_t_");
        let tk_batch_no: Option<i64> = get_field(new, "tk_batch_no");
        let tk_version: Option<i64> = get_field(new, "tk_version");
        let fk_subj_define: Option<i64> = get_field(new, "fk_subj-define");
        let fk_subject: Option<i64> = get_field(new, "fk_subject");

        let batch = match tk_batch_no {
            Some(id) => engine.resolve_variable_notice(id).await?,
            None => None,
        };
        let version = match tk_version {
            Some(id) => engine.resolve_variable_notice(id).await?,
            None => None,
        };
        let dcode = match fk_subj_define {
            Some(id) => engine.resolve_variable_code_notice(id).await?,
            None => None,
        };
        let scode = match fk_subject {
            Some(id) => engine.resolve_variable_code_notice(id).await?,
            None => None,
        };

        let form = _f_.as_deref().unwrap_or("");
        let typ = _t_.as_deref().unwrap_or("");
        let base = notice.unwrap_or_default();
        let ver = version.as_deref().unwrap_or("");
        let bat = batch.as_deref().unwrap_or("");
        let dc = dcode.as_deref().unwrap_or("");
        let sc = scode.as_deref().unwrap_or("");

        let p_number = match (form, typ) {
            ("创意", "范例") => format!("{}-!.", base),
            ("创意", "实例") => {
                format!(
                    "{}{}{}{}-!_",
                    base,
                    proc_wrap(dc),
                    proc_bracket(ver),
                    proc_hash(bat)
                )
            }
            ("设计", "范例") => {
                format!(
                    "{}{}{}{}-↑.",
                    base,
                    proc_wrap(dc),
                    proc_bracket(ver),
                    proc_hash(bat)
                )
            }
            ("设计", "实例") => {
                format!(
                    "{}{}{}{}-↑_",
                    base,
                    proc_colon_wrap(sc, dc),
                    proc_bracket(ver),
                    proc_hash(bat)
                )
            }
            ("实现", "范例") => {
                format!(
                    "{}{}{}{}-↓.",
                    base,
                    proc_wrap(dc),
                    proc_bracket(ver),
                    proc_hash(bat)
                )
            }
            ("实现", "实例") => {
                format!(
                    "{}{}{}{}-↓_",
                    base,
                    proc_colon_wrap(dc, sc),
                    proc_bracket(ver),
                    proc_hash(bat)
                )
            }
            _ => {
                let id: i64 = get_field(new, "id").unwrap_or(0);
                format!(
                    "PROC-{}-{}",
                    ctx.timestamp.format("%Y%m%d"),
                    crc32_hex(&id.to_string())
                )
            }
        };

        Ok(TriggerResult::new().with_modified_field("p_number", Value::String(p_number)))
    }
}

// ============================================
// Operation 格式辅助
// ============================================

fn op_ver_sep(v: &str) -> String {
    if v.is_empty() {
        "-".to_string()
    } else {
        format!("-{}", v)
    }
}

fn op_batch_hash(b: &str) -> String {
    if b.is_empty() {
        "#".to_string()
    } else {
        format!("#{}", b)
    }
}

fn op_subject_wrap(s: &str) -> String {
    if s.is_empty() {
        "{}".to_string()
    } else {
        format!("{{{}}}", s)
    }
}

fn op_subject_colon(s: &str, d: &str) -> String {
    match (s.is_empty(), d.is_empty()) {
        (true, true) => "{:}".to_string(),
        (true, false) => format!("{{:{}}}", d),
        (false, true) => format!("{{{}}}", s),
        (false, false) => format!("{{{}}}:{}}}", s, d),
    }
}

// ============================================
// Process 格式辅助
// ============================================

fn proc_wrap(s: &str) -> String {
    if s.is_empty() {
        "".to_string()
    } else {
        format!("{{{}}}", s)
    }
}

fn proc_bracket(v: &str) -> String {
    if v.is_empty() {
        "".to_string()
    } else {
        format!("[{v}]")
    }
}

fn proc_hash(b: &str) -> String {
    if b.is_empty() {
        "".to_string()
    } else {
        format!("#{b}")
    }
}

fn proc_colon_wrap(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => "".to_string(),
        (true, false) => format!("{{{}}}", b),
        (false, true) => format!("{{{}}}", a),
        (false, false) => format!("{{{a}:{b}}}", a = a, b = b),
    }
}
