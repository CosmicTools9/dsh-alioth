//! zc_id_category Level Trigger Templates
//!
//! 自动计算 `c_sort_` 字段，使类目按位编码排序。
//!
//! `c_sort_` 编码方式：
//!   - 对有 parent_id 的类目（如科目类目）：
//!     `c_sort_ = (parent.c_sort_ << 6) | slot_position` (1-63)
//!   - 对无 parent_id 的根级类目：
//!     `c_sort_ = 顺序编号`

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
// Category c_sort_ Auto-Computation Template
// ============================================

/// 类目排序自动编码模板
///
/// 对所有 `zc_id_category` 子表，BEFORE INSERT 时自动计算 `c_sort_`。
pub struct CategoryCSortTemplate;

#[async_trait]
impl TriggerTemplate for CategoryCSortTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ins_on_zc_id_category".to_string(),
            applies_to: vec!["zc_id_category".to_string()],
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
        let new = new_record
            .ok_or_else(|| TriggerError::ExecutionFailed("New record required".to_string()))?;

        // 已有 c_sort_ 则跳过（手动设置优先）
        if get_field::<i64>(new, "c_sort_").is_some() {
            return Ok(TriggerResult::new());
        }

        if ctx.pool.is_none() {
            return Ok(TriggerResult::new());
        }
        let engine = TemplateEngine::new(ctx.pool.clone());

        // 使用 ctx.table_name 获取当前执行的具体表名（SmartTriggerRegistry 传入）
        let table = &ctx.table_name;

        let parent_id: Option<i64> = get_field(new, "parent_id");

        if let Some(pid) = parent_id {
            // ---- 有父类目：从父类目的 c_sort_ 通过位移编码 ----
            let parent_sort: Option<i64> = engine
                .query_scalar(
                    "SELECT c_sort_ FROM isahl.zc_id_category WHERE id = $1",
                    vec![Value::Number(pid.into())],
                )
                .await?;
            let parent_sort = parent_sort.unwrap_or(0);

            // 此父类目下已用的槽位
            let used_slots_sql = format!(
                r#"SELECT c_sort_ & 63 FROM isahl."{}" WHERE parent_id = $1 AND c_sort_ IS NOT NULL"#,
                table
            );
            let slots: Vec<i64> = engine
                .query_scalar_all(&used_slots_sql, vec![Value::Number(pid.into())])
                .await?;

            let mut sidx: i64 = 1;
            for i in 1..=63i64 {
                if !slots.contains(&i) {
                    sidx = i;
                    break;
                }
            }

            let c_sort = (parent_sort << 6) | sidx;
            Ok(TriggerResult::new().with_modified_field("c_sort_", Value::Number(c_sort.into())))
        } else {
            // ---- 根级类目：查询当前表 max(c_sort_) + 1 ----
            let max_sort: Option<i64> = engine
                .query_scalar(
                    &format!(r#"SELECT MAX(c_sort_) FROM isahl."{}""#, table),
                    vec![],
                )
                .await?;

            let c_sort = max_sort.unwrap_or(0) + 1;
            Ok(TriggerResult::new().with_modified_field("c_sort_", Value::Number(c_sort.into())))
        }
    }
}

// ============================================
// Tests
// ============================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TriggerOperation;

    #[test]
    fn test_c_sort_encoding() {
        // 编码验证：父类目 c_sort_=10, slot=3 → (10<<6)|3 = 643
        let parent_sort: i64 = 10;
        let slot: i64 = 3;
        let c_sort = (parent_sort << 6) | slot;
        assert_eq!(c_sort, 643);
        // 解码验证
        assert_eq!(c_sort >> 6, 10);
        assert_eq!(c_sort & 63, 3);
    }

    #[tokio::test]
    async fn test_category_trigger_no_pool() {
        let tpl = CategoryCSortTemplate;
        let mut new_record = HashMap::new();
        new_record.insert("id".to_string(), Value::Number(100i64.into()));

        let ctx = TriggerContext::new("zc_id_category", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new_record)).await.unwrap();
        // 无 pool → 空结果
        assert!(!result.modified_fields.contains_key("c_sort_"));
    }

    #[tokio::test]
    async fn test_skip_if_c_sort_exists() {
        let tpl = CategoryCSortTemplate;
        let mut new_record = HashMap::new();
        new_record.insert("c_sort_".to_string(), Value::Number(42i64.into()));

        let ctx = TriggerContext::new("zc_id_category", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new_record)).await.unwrap();
        // 已有 c_sort_ → 跳过
        assert!(!result.modified_fields.contains_key("c_sort_"));
    }
}
