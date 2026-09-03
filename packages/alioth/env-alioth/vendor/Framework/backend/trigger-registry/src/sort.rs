//! Sort Field Trigger Templates
//!
//! 自动计算 `v_sort` 排序字段。
//!
//! `v_sort` 出现在：
//!   - `zc_id_consensus` 继承体系（如 `zc_id_cons-function-cate`）
//!   - `zc_ad_dimension` 继承体系（`zc_id_scene`/`zc_id_factor`/`zc_id_function` 及子表）
//!   - 部分 entity 表

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
// Consensus v_sort Auto-Computation Template
// ============================================

/// 共识表 `v_sort` 自动编码模板
///
/// 对所有 `zc_id_consensus` 子表，BEFORE INSERT 时自动计算 `v_sort`。
/// 对于有 `a_domain_` 的类目表（如 `zc_id_cons-function-cate`）：
///   `v_sort = (domain_code << 6) | slot_position`
/// 其中 domain_code 编码：
///   `!.`=1, `!_`=2, `↑.`=3, `↑_`=4, `↓.`=5, `↓_`=6
///
/// 对于无 `a_domain_` 的表，使用 max(v_sort) + 1 的简单策略。
pub struct ConsensusVSortTemplate;

// 域前缀编码表：a_domain_ 值 → 高位编码
fn domain_encoding(domain: &str) -> i64 {
    match domain {
        "!." => 1,
        "!_" => 2,
        "↑." => 3,
        "↑_" => 4,
        "↓." => 5,
        "↓_" => 6,
        _ => 0, // 未知域
    }
}

#[async_trait]
impl TriggerTemplate for ConsensusVSortTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ins_on_zc_id_consensus_v_sort".to_string(),
            applies_to: vec!["zc_id_consensus".to_string()],
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

        // 已有 v_sort 则跳过
        if get_field::<i64>(new, "v_sort").is_some() {
            return Ok(TriggerResult::new());
        }

        if ctx.pool.is_none() {
            return Ok(TriggerResult::new());
        }
        let engine = TemplateEngine::new(ctx.pool.clone());
        let table = &ctx.table_name;

        // 检查是否有 a_domain_ 字段
        let a_domain: Option<String> = get_field(new, "a_domain_");
        let parent_id: Option<i64> = get_field(new, "parent_id");

        if let Some(domain) = a_domain {
            // ---- 有 a_domain_：按域编码 + 槽位 ----
            let domain_code = domain_encoding(&domain);
            let base_code = domain_code << 6;

            // 同 a_domain_ 内的已用槽位
            let used_slots_sql = format!(
                r#"SELECT v_sort & 63 FROM isahl."{}" WHERE a_domain_ = $1 AND v_sort IS NOT NULL"#,
                table
            );
            let slots: Vec<i64> = engine
                .query_scalar_all(&used_slots_sql, vec![Value::String(domain)])
                .await?;

            let mut sidx: i64 = 1;
            for i in 1..=63i64 {
                if !slots.contains(&i) {
                    sidx = i;
                    break;
                }
            }

            let v_sort = base_code | sidx;
            Ok(TriggerResult::new().with_modified_field("v_sort", Value::Number(v_sort.into())))
        } else if let Some(pid) = parent_id {
            // ---- 有 parent_id：按父记录域+槽位 ----
            let parent_v_sort: Option<i64> = engine
                .query_scalar(
                    &format!(r#"SELECT v_sort FROM isahl."{}" WHERE id = $1"#, table),
                    vec![Value::Number(pid.into())],
                )
                .await?;
            let parent_sort = parent_v_sort.unwrap_or(0);

            let used_slots_sql = format!(
                r#"SELECT v_sort & 63 FROM isahl."{}" WHERE parent_id = $1 AND v_sort IS NOT NULL"#,
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

            let v_sort = (parent_sort << 6) | sidx;
            Ok(TriggerResult::new().with_modified_field("v_sort", Value::Number(v_sort.into())))
        } else {
            // ---- 简单顺序编号 ----
            let max_sort: Option<i64> = engine
                .query_scalar(
                    &format!(r#"SELECT MAX(v_sort) FROM isahl."{}""#, table),
                    vec![],
                )
                .await?;

            let v_sort = max_sort.unwrap_or(0) + 1;
            Ok(TriggerResult::new().with_modified_field("v_sort", Value::Number(v_sort.into())))
        }
    }
}
// ============================================
// Consensus Code Auto-Generation Template
// ============================================

/// 共识/类目表 `code` 自动生成模板
///
/// 对 `zc_id_consensus` / `zc_id_category` 子表，BEFORE INSERT 时从 `notice` 自动生成 `code`。
/// 适用于 `zc_id_cons-industry-cate` 等无 `ck_category` 的 consensus 类目表，
/// 以及 `zc_id_cons-factor-cate` / `zc_id_cons-function-cate` 等共识类目表。
///
/// 生成策略：
///   1. `code` 已存在 → 跳过
///   2. 有 `notice` → 取前 8 位缩写 + CRC32 哈希 4 位后缀
///   3. 有 `notice` 但无其他文本源 → 同上
pub struct ConsensusCodeTemplate;

#[async_trait]
impl TriggerTemplate for ConsensusCodeTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ins_on_zc_id_consensus_code".to_string(),
            applies_to: vec!["zc_id_consensus".to_string()],
            operations: vec![TriggerOperationDef::Insert],
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

        // 已有 code 则跳过
        if get_field::<String>(new, "code").is_some() {
            return Ok(TriggerResult::new());
        }

        // 取文本来源：notice 优先
        let source: String = get_field::<String>(new, "notice").unwrap_or_default();

        if source.is_empty() {
            return Ok(TriggerResult::new());
        }

        // 从 source 生成缩写前缀（取前 8 位有效字符，去除非字母数字）
        let clean: String = source
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .take(8)
            .collect();

        // CRC32 哈希后缀（4 位十六进制）
        let hash = crate::utils::crc32_hex(&source);
        let suffix = &hash[..4]; // 前 4 位 hex

        let code = format!("{}-{}", clean.to_uppercase(), suffix);

        Ok(TriggerResult::new().with_modified_field("code", Value::String(code)))
    }
}

// ============================================
// Dimension v_sort Auto-Computation Template
// ============================================

/// 维度表 `v_sort` 自动编码模板
///
/// 对 `zc_ad_dimension` 子表，BEFORE INSERT 时自动计算 `v_sort`。
/// 使用 `ck_category` 对应的类目 `c_sort` 作为基础编码。
pub struct DimensionVSortTemplate;

#[async_trait]
impl TriggerTemplate for DimensionVSortTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ins_on_zc_ad_dimension_v_sort".to_string(),
            applies_to: vec!["zc_ad_dimension".to_string()],
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

        // 已有 v_sort 则跳过
        if get_field::<i64>(new, "v_sort").is_some() {
            return Ok(TriggerResult::new());
        }

        if ctx.pool.is_none() {
            return Ok(TriggerResult::new());
        }
        let engine = TemplateEngine::new(ctx.pool.clone());
        let table = &ctx.table_name;

        // 如果已有 c_sort（由 DimensionAutoCodeTemplate 设置），直接使用 c_sort 作为 v_sort
        if let Some(c_sort) = get_field::<i64>(new, "c_sort") {
            return Ok(
                TriggerResult::new().with_modified_field("v_sort", Value::Number(c_sort.into()))
            );
        }

        // 查询 ck_category → 类目的 c_sort 作为基底
        let ck_category: Option<i64> = get_field(new, "ck_category");
        if let Some(ck) = ck_category {
            let cat_sort: Option<i64> = engine
                .query_scalar(
                    "SELECT c_sort FROM isahl.zc_id_category WHERE id = $1",
                    vec![Value::Number(ck.into())],
                )
                .await?;
            let base = cat_sort.unwrap_or(0) << 6;

            // 同类目下的已用 v_sort 槽位
            let used_slots_sql = format!(
                r#"SELECT v_sort & 63 FROM isahl."{}" WHERE ck_category = $1 AND v_sort IS NOT NULL"#,
                table
            );
            let slots: Vec<i64> = engine
                .query_scalar_all(&used_slots_sql, vec![Value::Number(ck.into())])
                .await?;

            let mut sidx: i64 = 1;
            for i in 1..=63i64 {
                if !slots.contains(&i) {
                    sidx = i;
                    break;
                }
            }

            let v_sort = base | sidx;
            Ok(TriggerResult::new().with_modified_field("v_sort", Value::Number(v_sort.into())))
        } else {
            // 无 ck_category → max+1
            let max_sort: Option<i64> = engine
                .query_scalar(
                    &format!(r#"SELECT MAX(v_sort) FROM isahl."{}""#, table),
                    vec![],
                )
                .await?;

            let v_sort = max_sort.unwrap_or(0) + 1;
            Ok(TriggerResult::new().with_modified_field("v_sort", Value::Number(v_sort.into())))
        }
    }
}

// ============================================
// Tests
// ============================================

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn test_consensus_code_from_notice() {
        let tpl = ConsensusCodeTemplate;
        let mut new = HashMap::new();
        new.insert(
            "notice".to_string(),
            Value::String("原材料采购".to_string()),
        );
        new.insert(
            "notice_fallback".to_string(),
            Value::String("共识-行业类目".to_string()),
        );

        let ctx = TriggerContext::new("zc_id_cons-industry-cate", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();

        let code = result
            .modified_fields
            .get("code")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        // code 应该包含 notice 的缩写 + 哈希后缀
        assert!(code.contains("原材料采"));
        assert!(code.len() > 4);
    }

    #[tokio::test]
    async fn test_consensus_code_skip_if_exists() {
        let tpl = ConsensusCodeTemplate;
        let mut new = HashMap::new();
        new.insert("code".to_string(), Value::String("EXISTS".to_string()));

        let ctx = TriggerContext::new("zc_id_cons-industry-cate", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();
        // 已有 code → 不修改
        assert!(!result.modified_fields.contains_key("code"));
    }

    #[tokio::test]
    async fn test_consensus_code_empty_source() {
        let tpl = ConsensusCodeTemplate;
        let new = HashMap::new(); // 无 notice

        let ctx = TriggerContext::new("zc_id_cons-industry-cate", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();
        // 无文本来源 → 跳过
        assert!(!result.modified_fields.contains_key("code"));
    }
    use crate::TriggerOperation;

    #[test]
    fn test_domain_encoding() {
        assert_eq!(domain_encoding("!."), 1);
        assert_eq!(domain_encoding("!_"), 2);
        assert_eq!(domain_encoding("↑."), 3);
        assert_eq!(domain_encoding("↑_"), 4);
        assert_eq!(domain_encoding("↓."), 5);
        assert_eq!(domain_encoding("↓_"), 6);
        assert_eq!(domain_encoding("未知"), 0);
    }

    #[test]
    fn test_v_sort_encoding() {
        // base=1 (↓.), slot=5 → (1<<6)|5 = 69
        let v_sort = (1i64 << 6) | 5;
        assert_eq!(v_sort, 69);
        assert_eq!(v_sort >> 6, 1);
        assert_eq!(v_sort & 63, 5);
    }
}
