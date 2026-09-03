//! Dynamic Trigger Templates

use crate::{
    template::{TriggerMetadata, TriggerOperationDef, TriggerTemplate, TriggerTimingDef},
    utils::*,
    SideEffect, TriggerContext, TriggerError, TriggerResult,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

// ============================================
// Foreign Key Relation Mapper Template
// ============================================

/// 外键 → 关系表映射模板
///
/// 将 `ck_*`/`tk_*`/`qk_*`/`lk_*` 字段映射到显式的多对多关系表。
pub struct ForeignKeyRelationMapperTemplate;

#[async_trait]
impl TriggerTemplate for ForeignKeyRelationMapperTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "gf_gen_tf_af_ups_on_var_foreign_key".to_string(),
            applies_to: crate::lifecycle::ZC_ID_LIFECYCLE_TABLES
                .iter()
                .map(|&s| s.to_string())
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
        _ctx: &TriggerContext,
        old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        // DELETE：业务行删除时 qk_*/lk_* 引用随之移除 → evaluation ref_count -1
        if new_record.is_none() {
            let mut result = TriggerResult::new();
            if let Some(old) = old_record {
                for (key, value) in old.iter() {
                    if key.starts_with("qk_") || key.starts_with("lk_") {
                        if let Some(target) = value.as_i64() {
                            result = result.with_side_effect(SideEffect::RawSql(format!(
                                "UPDATE isahl.zc_id_evaluation SET ref_count = COALESCE(ref_count, 0) - 1 WHERE id = {}",
                                target
                            )));
                        }
                    }
                }
            }
            return Ok(result);
        }

        let new = new_record
            .ok_or_else(|| TriggerError::ExecutionFailed("New record required".to_string()))?;

        let id: i64 = get_field(new, "id").unwrap_or(0);
        let created_by_id: Option<i64> = get_field(new, "created_by_id");
        let created_at: Option<String> = get_field(new, "created_at");
        let updated_by_id: Option<i64> = get_field(new, "updated_by_id");
        let updated_at: Option<String> = get_field(new, "updated_at");

        let mut result = TriggerResult::new();
        let mut has_changes = false;

        for (key, value) in new.iter() {
            if key.starts_with("ck_") && key != "ck_branch" && value.is_number() {
                let old_val = old_record.and_then(|r| r.get(key));
                if value.is_null() && old_val.map(|v| !v.is_null()).unwrap_or(false) {
                    result = result.with_side_effect(SideEffect::RawSql(format!(
                        "DELETE FROM isahl.zc_id_lifecycle_r_category WHERE ref_left = {} AND ref_right = {}",
                        id,
                        old_val.unwrap().as_i64().unwrap_or(0)
                    )));
                    has_changes = true;
                } else if !value.is_null() {
                    result = result.with_side_effect(SideEffect::Insert {
                        table: "zc_id_lifecycle_r_category".to_string(),
                        values: {
                            let mut values = HashMap::new();
                            values.insert("ref_left".to_string(), Value::Number(id.into()));
                            values.insert("ref_right".to_string(), value.clone());
                            values.insert(
                                "created_by_id".to_string(),
                                created_by_id
                                    .map(|v| Value::Number(v.into()))
                                    .unwrap_or(Value::Null),
                            );
                            values.insert(
                                "created_at".to_string(),
                                created_at.clone().map(Value::String).unwrap_or(Value::Null),
                            );
                            values.insert(
                                "updated_by_id".to_string(),
                                updated_by_id
                                    .map(|v| Value::Number(v.into()))
                                    .unwrap_or(Value::Null),
                            );
                            values.insert(
                                "updated_at".to_string(),
                                updated_at.clone().map(Value::String).unwrap_or(Value::Null),
                            );
                            values
                        },
                    });
                    has_changes = true;
                }
            } else if key.starts_with("tk_")
                && key != "tk_version"
                && key != "tk_batch_no"
                && value.is_number()
            {
                let old_val = old_record.and_then(|r| r.get(key));
                if value.is_null() && old_val.map(|v| !v.is_null()).unwrap_or(false) {
                    result = result.with_side_effect(SideEffect::RawSql(format!(
                        "DELETE FROM isahl.zc_id_lifecycle_r_tags WHERE ref_left = {} AND ref_right = {}",
                        id,
                        old_val.unwrap().as_i64().unwrap_or(0)
                    )));
                    has_changes = true;
                } else if !value.is_null() {
                    result = result.with_side_effect(SideEffect::Insert {
                        table: "zc_id_lifecycle_r_tags".to_string(),
                        values: {
                            let mut values = HashMap::new();
                            values.insert("ref_left".to_string(), Value::Number(id.into()));
                            values.insert("ref_right".to_string(), value.clone());
                            values.insert(
                                "created_by_id".to_string(),
                                created_by_id
                                    .map(|v| Value::Number(v.into()))
                                    .unwrap_or(Value::Null),
                            );
                            values.insert(
                                "created_at".to_string(),
                                created_at.clone().map(Value::String).unwrap_or(Value::Null),
                            );
                            values.insert(
                                "updated_by_id".to_string(),
                                updated_by_id
                                    .map(|v| Value::Number(v.into()))
                                    .unwrap_or(Value::Null),
                            );
                            values.insert(
                                "updated_at".to_string(),
                                updated_at.clone().map(Value::String).unwrap_or(Value::Null),
                            );
                            values
                        },
                    });
                    has_changes = true;
                }
            } else if (key.starts_with("qk_") || key.starts_with("lk_"))
                && (value.is_number() || value.is_null())
            {
                let old_val = old_record.and_then(|r| r.get(key));
                let old_id: Option<i64> = old_val.and_then(|v| v.as_i64());
                let new_id: Option<i64> = value.as_i64();

                // ref_count 同步（evaluation 引用计数：建立引用 +1 / 移除引用 -1 / 换值 旧-1 新+1）
                match (old_id, new_id) {
                    (None, Some(n)) => {
                        result = result.with_side_effect(SideEffect::RawSql(format!(
                            "UPDATE isahl.zc_id_evaluation SET ref_count = COALESCE(ref_count, 0) + 1 WHERE id = {}",
                            n
                        )));
                    }
                    (Some(o), None) => {
                        result = result.with_side_effect(SideEffect::RawSql(format!(
                            "UPDATE isahl.zc_id_evaluation SET ref_count = COALESCE(ref_count, 0) - 1 WHERE id = {}",
                            o
                        )));
                    }
                    (Some(o), Some(n)) if o != n => {
                        result = result.with_side_effect(SideEffect::RawSql(format!(
                            "UPDATE isahl.zc_id_evaluation SET ref_count = COALESCE(ref_count, 0) - 1 WHERE id = {}",
                            o
                        )));
                        result = result.with_side_effect(SideEffect::RawSql(format!(
                            "UPDATE isahl.zc_id_evaluation SET ref_count = COALESCE(ref_count, 0) + 1 WHERE id = {}",
                            n
                        )));
                    }
                    _ => {}
                }

                // 既有 r_evaluation 关系表同步（仅非空新值时 INSERT，行为保持不变）
                if let Some(n) = new_id {
                    result = result.with_side_effect(SideEffect::Insert {
                        table: "zc_id_lifecycle_r_evaluation".to_string(),
                        values: {
                            let mut values = HashMap::new();
                            values.insert("ref_left".to_string(), Value::Number(id.into()));
                            values.insert("ref_right".to_string(), Value::Number(n.into()));
                            values.insert(
                                "created_by_id".to_string(),
                                created_by_id
                                    .map(|v| Value::Number(v.into()))
                                    .unwrap_or(Value::Null),
                            );
                            values.insert(
                                "created_at".to_string(),
                                created_at.clone().map(Value::String).unwrap_or(Value::Null),
                            );
                            values.insert(
                                "updated_by_id".to_string(),
                                updated_by_id
                                    .map(|v| Value::Number(v.into()))
                                    .unwrap_or(Value::Null),
                            );
                            values.insert(
                                "updated_at".to_string(),
                                updated_at.clone().map(Value::String).unwrap_or(Value::Null),
                            );
                            values
                        },
                    });
                    has_changes = true;
                }
            }
        }

        if has_changes {
            result = result.with_side_effect(SideEffect::RawSql(format!(
                "INSERT INTO isahl.meta_lifecycle_event (id, event_at) VALUES ({}, NOW()) ON CONFLICT (id) DO UPDATE SET event_at = EXCLUDED.event_at",
                id
            )));
        }

        Ok(result)
    }
}

// ============================================
// Relation Cycle Detect Template
// ============================================

/// 关系循环检测模板
///
/// 检测 M:N 非自引用关系中的自引用循环。
pub struct RelationCycleDetectTemplate;

#[async_trait]
impl TriggerTemplate for RelationCycleDetectTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "gf_gen_tf_bf_ups_relation_cycle_detect".to_string(),
            applies_to: vec!["zc_ad_tensor_rr_non_self-ref".to_string()],
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

        let ref_left: Option<i64> = get_field(new, "ref_left");
        let ref_right: Option<i64> = get_field(new, "ref_right");

        if ref_left.is_some() && ref_left == ref_right {
            return Ok(TriggerResult::blocked(
                "Self-reference detected in non-self-ref relation",
            ));
        }

        Ok(TriggerResult::new())
    }
}

// ============================================
// RR Junction RefCount Template
// ============================================

/// rr 关系表 → evaluation 引用计数维护模板
///
/// rr 表（`zc_id_master_rr_slave` 子树）`ref_right` 变更时维护
/// `zc_id_evaluation.ref_count`（建立 +1 / 移除 -1 / 换值 旧-1 新+1）。
/// 注册在 rr 树根（applies_to 空 = 通配整树），与 `ForeignKeyRelationMapperTemplate`
/// （业务表 qk/lk 引用）共同构成 evaluation 引用计数的**单一 Rust 维护面**
/// 。
pub struct RrRefCountTemplate;

#[async_trait]
impl TriggerTemplate for RrRefCountTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_af_rr_ref_count_junction".to_string(),
            // 通配 = 注册父表（zc_id_master_rr_slave）整棵子树（全部 rr 关系表）
            applies_to: vec![],
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
        _ctx: &TriggerContext,
        old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let mut result = TriggerResult::new();

        let old_ref: Option<i64> = old_record.and_then(|r| get_field(r, "ref_right"));
        let new_ref: Option<i64> = new_record.and_then(|r| get_field(r, "ref_right"));

        match (old_ref, new_ref) {
            (None, Some(n)) => {
                result = result.with_side_effect(SideEffect::RawSql(format!(
                    "UPDATE isahl.zc_id_evaluation SET ref_count = COALESCE(ref_count, 0) + 1 WHERE id = {}",
                    n
                )));
            }
            (Some(o), None) => {
                result = result.with_side_effect(SideEffect::RawSql(format!(
                    "UPDATE isahl.zc_id_evaluation SET ref_count = COALESCE(ref_count, 0) - 1 WHERE id = {}",
                    o
                )));
            }
            (Some(o), Some(n)) if o != n => {
                result = result.with_side_effect(SideEffect::RawSql(format!(
                    "UPDATE isahl.zc_id_evaluation SET ref_count = COALESCE(ref_count, 0) - 1 WHERE id = {}",
                    o
                )));
                result = result.with_side_effect(SideEffect::RawSql(format!(
                    "UPDATE isahl.zc_id_evaluation SET ref_count = COALESCE(ref_count, 0) + 1 WHERE id = {}",
                    n
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
    async fn test_cycle_detection() {
        let tpl = RelationCycleDetectTemplate;
        let mut new = HashMap::new();
        new.insert("ref_left".to_string(), Value::Number(100i64.into()));
        new.insert("ref_right".to_string(), Value::Number(100i64.into()));

        let ctx = TriggerContext::new("zc_ad_tensor_rr_non_self-ref", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();

        assert!(result.blocked);
    }

    #[tokio::test]
    async fn test_foreign_key_mapper() {
        let tpl = ForeignKeyRelationMapperTemplate;
        let mut new = HashMap::new();
        new.insert("id".to_string(), Value::Number(1i64.into()));
        new.insert("ck_category".to_string(), Value::Number(10i64.into()));
        new.insert("tk_tag".to_string(), Value::Number(20i64.into()));
        new.insert("qk_status".to_string(), Value::Number(30i64.into()));

        let ctx = TriggerContext::new("zc_id_bill", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();

        // 3 关系 INSERT + 1 ref_count(+1) + 1 meta_lifecycle_event
        assert_eq!(result.side_effects.len(), 5);
        // qk_status → evaluation 引用建立：emit ref_count +1
        assert!(
            result.side_effects.iter().any(|s| match s {
                SideEffect::RawSql(sql) => {
                    sql.contains("ref_count = COALESCE(ref_count, 0) + 1")
                        && sql.contains("id = 30")
                }
                _ => false,
            }),
            "expected ref_count +1 side effect for qk_status=30, got {:?}",
            result.side_effects
        );
    }

    #[tokio::test]
    async fn test_ref_count_on_clear() {
        let tpl = ForeignKeyRelationMapperTemplate;
        let mut old = HashMap::new();
        old.insert("id".to_string(), Value::Number(1i64.into()));
        old.insert("qk_status".to_string(), Value::Number(30i64.into()));
        let mut new = HashMap::new();
        new.insert("id".to_string(), Value::Number(1i64.into()));
        new.insert("qk_status".to_string(), Value::Null);

        let ctx = TriggerContext::new("zc_id_bill", TriggerOperation::Update);
        let result = tpl.execute(&ctx, Some(&old), Some(&new)).await.unwrap();

        // 引用移除：ref_count -1，且不再 INSERT r_evaluation
        assert_eq!(result.side_effects.len(), 1);
        match &result.side_effects[0] {
            SideEffect::RawSql(sql) => {
                assert!(sql.contains("ref_count = COALESCE(ref_count, 0) - 1"));
                assert!(sql.contains("id = 30"));
            }
            other => panic!("expected RawSql ref_count -1, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ref_count_on_switch() {
        let tpl = ForeignKeyRelationMapperTemplate;
        let mut old = HashMap::new();
        old.insert("id".to_string(), Value::Number(1i64.into()));
        old.insert("qk_status".to_string(), Value::Number(30i64.into()));
        let mut new = HashMap::new();
        new.insert("id".to_string(), Value::Number(1i64.into()));
        new.insert("qk_status".to_string(), Value::Number(40i64.into()));

        let ctx = TriggerContext::new("zc_id_bill", TriggerOperation::Update);
        let result = tpl.execute(&ctx, Some(&old), Some(&new)).await.unwrap();

        // 换值：旧值 -1 + 新值 +1 + r_evaluation INSERT + event
        assert_eq!(result.side_effects.len(), 4);
        assert!(
            result.side_effects.iter().any(|s| match s {
                SideEffect::RawSql(sql) => {
                    sql.contains("ref_count = COALESCE(ref_count, 0) - 1")
                        && sql.contains("id = 30")
                }
                _ => false,
            }),
            "expected ref_count -1 for old value 30, got {:?}",
            result.side_effects
        );
        assert!(
            result.side_effects.iter().any(|s| match s {
                SideEffect::RawSql(sql) => {
                    sql.contains("ref_count = COALESCE(ref_count, 0) + 1")
                        && sql.contains("id = 40")
                }
                _ => false,
            }),
            "expected ref_count +1 for new value 40, got {:?}",
            result.side_effects
        );
    }

    #[tokio::test]
    async fn test_ref_count_on_delete() {
        let tpl = ForeignKeyRelationMapperTemplate;
        let mut old = HashMap::new();
        old.insert("id".to_string(), Value::Number(1i64.into()));
        old.insert("qk_status".to_string(), Value::Number(30i64.into()));
        old.insert("lk_health".to_string(), Value::Number(50i64.into()));
        old.insert("ck_category".to_string(), Value::Number(10i64.into()));

        let ctx = TriggerContext::new("zc_id_bill", TriggerOperation::Delete);
        let result = tpl.execute(&ctx, Some(&old), None).await.unwrap();

        // 业务行删除：qk_*/lk_* 引用全部移除 → 各 -1；ck_* 不处理
        assert_eq!(result.side_effects.len(), 2);
        for target in [30, 50] {
            assert!(
                result.side_effects.iter().any(|s| match s {
                    SideEffect::RawSql(sql) => {
                        sql.contains("ref_count = COALESCE(ref_count, 0) - 1")
                            && sql.contains(&format!("id = {}", target))
                    }
                    _ => false,
                }),
                "expected ref_count -1 for qk/lk target {}, got {:?}",
                target,
                result.side_effects
            );
        }
    }

    #[tokio::test]
    async fn test_rr_ref_count_insert_plus_one() {
        let tpl = RrRefCountTemplate;
        let mut new = HashMap::new();
        new.insert("ref_right".to_string(), Value::Number(30i64.into()));

        let ctx = TriggerContext::new("zc_id_plan_rr_task", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();

        assert_eq!(result.side_effects.len(), 1);
        match &result.side_effects[0] {
            SideEffect::RawSql(sql) => {
                assert!(sql.contains("ref_count = COALESCE(ref_count, 0) + 1"));
                assert!(sql.contains("id = 30"));
            }
            other => panic!("expected +1 RawSql, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_rr_ref_count_delete_minus_one() {
        let tpl = RrRefCountTemplate;
        let mut old = HashMap::new();
        old.insert("ref_right".to_string(), Value::Number(30i64.into()));

        let ctx = TriggerContext::new("zc_id_plan_rr_task", TriggerOperation::Delete);
        let result = tpl.execute(&ctx, Some(&old), None).await.unwrap();

        assert_eq!(result.side_effects.len(), 1);
        match &result.side_effects[0] {
            SideEffect::RawSql(sql) => {
                assert!(sql.contains("ref_count = COALESCE(ref_count, 0) - 1"));
                assert!(sql.contains("id = 30"));
            }
            other => panic!("expected -1 RawSql, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_rr_ref_count_switch_old_minus_new_plus() {
        let tpl = RrRefCountTemplate;
        let mut old = HashMap::new();
        old.insert("ref_right".to_string(), Value::Number(30i64.into()));
        let mut new = HashMap::new();
        new.insert("ref_right".to_string(), Value::Number(40i64.into()));

        let ctx = TriggerContext::new("zc_id_plan_rr_task", TriggerOperation::Update);
        let result = tpl.execute(&ctx, Some(&old), Some(&new)).await.unwrap();

        assert_eq!(result.side_effects.len(), 2);
        assert!(
            result.side_effects.iter().any(|s| match s {
                SideEffect::RawSql(sql) => {
                    sql.contains("ref_count = COALESCE(ref_count, 0) - 1")
                        && sql.contains("id = 30")
                }
                _ => false,
            }),
            "expected -1 for old ref_right 30"
        );
        assert!(
            result.side_effects.iter().any(|s| match s {
                SideEffect::RawSql(sql) => {
                    sql.contains("ref_count = COALESCE(ref_count, 0) + 1")
                        && sql.contains("id = 40")
                }
                _ => false,
            }),
            "expected +1 for new ref_right 40"
        );
    }

    #[tokio::test]
    async fn test_rr_ref_count_unchanged_noop() {
        let tpl = RrRefCountTemplate;
        let mut old = HashMap::new();
        old.insert("ref_right".to_string(), Value::Number(30i64.into()));
        let mut new = HashMap::new();
        new.insert("ref_right".to_string(), Value::Number(30i64.into()));

        let ctx = TriggerContext::new("zc_id_plan_rr_task", TriggerOperation::Update);
        let result = tpl.execute(&ctx, Some(&old), Some(&new)).await.unwrap();

        assert!(result.side_effects.is_empty(), "ref_right 未变不产生副作用");
    }
}
