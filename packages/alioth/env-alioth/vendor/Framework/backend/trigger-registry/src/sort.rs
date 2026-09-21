//! Sort Field Trigger Templates
//!
//! 保留 `ConsensusCodeTemplate`（共识/类目表 `code` 自动生成）。
//!
//! 历史遗留的 `v_sort` 自动编码模板（`ConsensusVSortTemplate` / `DimensionVSortTemplate`）已于
//! 2026-09-15 退役：`v_sort` 列在全 schema（含 isahl 以外）**0 张表**存在 ⇒ 两个模板在每张表上都
//! 提前 no-op（纯死码）；维度侧真正的排序列为 `c_sort_`（38 张表），由 `CategoryCSortTemplate` 负责。

use crate::{
    template::{TriggerMetadata, TriggerOperationDef, TriggerTemplate, TriggerTimingDef},
    utils::*,
    TriggerContext, TriggerError, TriggerResult,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

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
