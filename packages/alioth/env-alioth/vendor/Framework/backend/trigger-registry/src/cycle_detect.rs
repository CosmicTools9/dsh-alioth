//! Hierarchy Cycle Detection Trigger Templates
//!
//! 与 `RelationCycleDetectTemplate`（M:N 自引用关系）不同，
//! 此处检测的是通过 `fk_parent` 实现的 1:N 层级父子关系。

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

const MAX_DEPTH: i32 = 100;
const CYCLE_ERR_TEMPLATE: &str =
    "cycle detected — record id={} would become its own ancestor through fk_parent={}";
const SELF_REF_ERR_TEMPLATE: &str = "fk_parent cannot reference itself (id={})";
const DEPTH_ERR_TEMPLATE: &str =
    "hierarchy depth exceeds {} (possible cycle) starting from fk_parent={}";

// ============================================
// Place Hierarchy Cycle Detection Template
// ============================================

/// 场所层级父子关系防环检测
///
/// 对所有 `zc_id_place` 子表，BEFORE INSERT/UPDATE 时执行：
/// 1. `fk_parent` 为 NULL → 跳过
/// 2. UPDATE 时 `fk_parent = id` → 阻止（自环）
/// 3. 沿 `fk_parent` 链向上追踪 100 层 → 在 UPDATE 中检测是否回到当前记录
pub struct PlaceCycleDetectTemplate;

#[async_trait]
impl TriggerTemplate for PlaceCycleDetectTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ins_place_detect_cycle".to_string(),
            applies_to: vec!["zc_id_place".to_string()],
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

        // fk_parent 为 NULL 时跳过
        let fk_parent: Option<i64> = get_field(new, "fk_parent");
        let fk_parent = match fk_parent {
            Some(p) => p,
            None => return Ok(TriggerResult::new()),
        };

        let new_id: Option<i64> = get_field(new, "id");

        // UPDATE：fk_parent 不能指向自己
        if let Some(id) = new_id {
            if fk_parent == id {
                return Ok(TriggerResult::blocked(
                    SELF_REF_ERR_TEMPLATE.replace("{}", &id.to_string()),
                ));
            }
        }

        if ctx.pool.is_none() {
            return Ok(TriggerResult::new());
        }
        let engine = TemplateEngine::new(ctx.pool.clone());

        // 沿 fk_parent 链向上追踪（查询 zc_id_place 自动包含所有子表记录）
        let mut current_id: i64 = fk_parent;
        let mut depth: i32 = 0;

        while depth < MAX_DEPTH {
            depth += 1;

            // 查询当前记录的 fk_parent
            let parent_id: Option<i64> = engine
                .query_scalar(
                    "SELECT fk_parent FROM isahl.zc_id_place WHERE id = $1",
                    vec![Value::Number(current_id.into())],
                )
                .await?;

            // UPDATE：检查是否回到当前记录
            if let (Some(id), Some(pid)) = (new_id, parent_id) {
                if pid == id {
                    return Ok(TriggerResult::blocked(format!(
                        "zc_id_place: {}",
                        CYCLE_ERR_TEMPLATE
                            .replacen("{}", &id.to_string(), 1)
                            .replacen("{}", &fk_parent.to_string(), 1)
                    )));
                }
            }

            match parent_id {
                Some(pid) => current_id = pid,
                None => return Ok(TriggerResult::new()), // 到根了，无环
            }
        }

        // 超过 MAX_DEPTH
        Ok(TriggerResult::blocked(format!(
            "zc_id_place: {}",
            DEPTH_ERR_TEMPLATE
                .replacen("{}", &MAX_DEPTH.to_string(), 1)
                .replacen("{}", &fk_parent.to_string(), 1)
        )))
    }
}

// ============================================
// Stan Clause Hierarchy Cycle Detection Template
// ============================================

/// 标准条款层级父子关系防环检测
///
/// 对 `zc_id_stan-clause`，BEFORE INSERT/UPDATE 时执行：
/// 1. `fk_parent` 为 NULL → 跳过
/// 2. `fk_parent = fk_standard` → 直接自环
/// 3. `fk_parent` 指向同表（zc_id_stan-clause）→ 同表层级防环
/// 4. `fk_parent` 指向 zc_id_standard → 标准继承环（递归 CTE）
pub struct StanClauseCycleDetectTemplate;

#[async_trait]
impl TriggerTemplate for StanClauseCycleDetectTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ins_stan_clause_detect_cycle".to_string(),
            applies_to: vec!["zc_id_stan-clause".to_string()],
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

        // fk_parent 为 NULL 时跳过
        let fk_parent: Option<i64> = get_field(new, "fk_parent");
        let fk_parent = match fk_parent {
            Some(p) => p,
            None => return Ok(TriggerResult::new()),
        };

        let fk_standard: Option<i64> = get_field(new, "fk_standard");
        let new_id: Option<i64> = get_field(new, "id");

        // 直接自环：fk_parent 不能等于 fk_standard
        if let Some(std) = fk_standard {
            if fk_parent == std {
                return Ok(TriggerResult::blocked(format!(
                    "zc_id_stan-clause: direct cycle detected — fk_parent ({}) cannot equal fk_standard ({})",
                    fk_parent, std
                )));
            }
        }

        if ctx.pool.is_none() {
            return Ok(TriggerResult::new());
        }
        let engine = TemplateEngine::new(ctx.pool.clone());

        // 判断 fk_parent 指向同表还是 zc_id_standard
        let is_self_ref: bool = engine
            .query_scalar::<i64>(
                r#"SELECT 1 FROM isahl."zc_id_stan-clause" WHERE id = $1 LIMIT 1"#,
                vec![Value::Number(fk_parent.into())],
            )
            .await?
            .is_some();

        if is_self_ref {
            // ---- fk_parent 指向 zc_id_stan-clause 自身：同表层级防环 ----

            // UPDATE：fk_parent 不能指向自己
            if let Some(id) = new_id {
                if fk_parent == id {
                    return Ok(TriggerResult::blocked(format!(
                        "zc_id_stan-clause: {}",
                        SELF_REF_ERR_TEMPLATE.replacen("{}", &id.to_string(), 1)
                    )));
                }
            }

            // 沿着 fk_parent 链向上追踪
            let mut current_id: i64 = fk_parent;
            let mut depth: i32 = 0;

            while depth < MAX_DEPTH {
                depth += 1;

                let parent_id: Option<i64> = engine
                    .query_scalar(
                        r#"SELECT fk_parent FROM isahl."zc_id_stan-clause" WHERE id = $1"#,
                        vec![Value::Number(current_id.into())],
                    )
                    .await?;

                // UPDATE：检查是否回到当前记录
                if let (Some(id), Some(pid)) = (new_id, parent_id) {
                    if pid == id {
                        return Ok(TriggerResult::blocked(format!(
                            "zc_id_stan-clause: {}",
                            CYCLE_ERR_TEMPLATE
                                .replacen("{}", &id.to_string(), 1)
                                .replacen("{}", &fk_parent.to_string(), 1)
                        )));
                    }
                }

                match parent_id {
                    Some(pid) => current_id = pid,
                    None => return Ok(TriggerResult::new()),
                }
            }

            Ok(TriggerResult::blocked(format!(
                "zc_id_stan-clause: {}",
                DEPTH_ERR_TEMPLATE
                    .replacen("{}", &MAX_DEPTH.to_string(), 1)
                    .replacen("{}", &fk_parent.to_string(), 1)
            )))
        } else {
            // ---- fk_parent 指向 zc_id_standard：标准继承环检测 ----
            // 使用递归 CTE：从 fk_parent 开始，沿 (fk_standard → fk_parent) 链
            // 检查是否回到当前记录的 fk_standard

            if let Some(std) = fk_standard {
                let cycle_found: Option<i64> = engine
                    .query_scalar(
                        r#"
                        WITH RECURSIVE standard_chain(standard_id, depth) AS (
                            SELECT $1::bigint, 1
                            UNION ALL
                            SELECT sc.fk_parent, sc.depth + 1
                            FROM isahl."zc_id_stan-clause" sc
                            JOIN standard_chain ch ON sc.fk_standard = ch.standard_id
                            WHERE sc.fk_parent IS NOT NULL
                              AND ch.depth < $2
                        )
                        SELECT 1 FROM standard_chain
                        WHERE standard_id = $3
                        LIMIT 1
                        "#,
                        vec![
                            Value::Number(fk_parent.into()),
                            Value::Number(MAX_DEPTH.into()),
                            Value::Number(std.into()),
                        ],
                    )
                    .await?;

                if cycle_found.is_some() {
                    return Ok(TriggerResult::blocked(format!(
                        "zc_id_stan-clause: cycle detected in standard inheritance — \
                         standard {} is an ancestor of itself through fk_parent={}",
                        std, fk_parent
                    )));
                }
            }

            Ok(TriggerResult::new())
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

    #[tokio::test]
    async fn test_place_no_pool_returns_ok() {
        let tpl = PlaceCycleDetectTemplate;
        let mut new = HashMap::new();
        new.insert("fk_parent".to_string(), Value::Number(10i64.into()));

        let ctx = TriggerContext::new("zc_id_place", TriggerOperation::Insert);
        // 无 pool → 返回空结果（不做检测）
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();
        assert!(!result.blocked);
    }

    #[tokio::test]
    async fn test_place_no_parent_skips() {
        let tpl = PlaceCycleDetectTemplate;
        let new = HashMap::new(); // no fk_parent

        let ctx = TriggerContext::new("zc_id_place", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();
        assert!(!result.blocked);
    }

    #[tokio::test]
    async fn test_place_self_ref_blocks() {
        let tpl = PlaceCycleDetectTemplate;
        let mut new = HashMap::new();
        new.insert("id".to_string(), Value::Number(42i64.into()));
        new.insert("fk_parent".to_string(), Value::Number(42i64.into()));

        let ctx = TriggerContext::new("zc_id_place", TriggerOperation::Update);
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();
        assert!(result.blocked);
        assert!(result
            .block_reason
            .unwrap()
            .contains("cannot reference itself"));
    }

    #[tokio::test]
    async fn test_stan_direct_cycle_blocks() {
        let tpl = StanClauseCycleDetectTemplate;
        let mut new = HashMap::new();
        new.insert("fk_parent".to_string(), Value::Number(100i64.into()));
        new.insert("fk_standard".to_string(), Value::Number(100i64.into()));

        let ctx = TriggerContext::new("zc_id_stan-clause", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();
        assert!(result.blocked);
        assert!(result.block_reason.unwrap().contains("direct cycle"));
    }

    #[tokio::test]
    async fn test_stan_no_pool_returns_ok() {
        let tpl = StanClauseCycleDetectTemplate;
        let mut new = HashMap::new();
        new.insert("fk_parent".to_string(), Value::Number(10i64.into()));

        let ctx = TriggerContext::new("zc_id_stan-clause", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();
        assert!(!result.blocked);
    }

    #[tokio::test]
    async fn test_stan_no_parent_skips() {
        let tpl = StanClauseCycleDetectTemplate;
        let new = HashMap::new();

        let ctx = TriggerContext::new("zc_id_stan-clause", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();
        assert!(!result.blocked);
    }
}
