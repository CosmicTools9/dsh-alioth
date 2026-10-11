//! zc_id_lifecycle Level Trigger Templates
//!

use crate::{
    template::{
        TemplateEngine, TriggerMetadata, TriggerOperationDef, TriggerTemplate, TriggerTimingDef,
    },
    utils::*,
    TriggerContext, TriggerError, TriggerOperation, TriggerResult,
};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::AssertSqlSafe;
use std::collections::HashMap;

// ============================================
// Lifecycle Table List
// ============================================

pub const ZC_ID_LIFECYCLE_TABLES: &[&str] = &[
    "zc_id_lifecycle",
    "zc_id_bill",
    "zc_id_bill-check",
    "zc_id_event",
    "zc_id_even-approve",
    "zc_id_appr-purchase",
    "zc_id_entity",
    "zc_id_agreement",
    "zc_id_statement",
    "zc_id_version",
    "zc_id_vers-context",
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
    "zc_id_bom-assemble",
    "zc_id_document",
    "zc_id_law",
    "zc_id_file-manual",
    "zc_id_operation",
    "zc_id_project",
    "zc_id_process",
    "zc_id_production",
    "zc_id_standard",
    "zc_id_task",
    "zc_id_prod-sales",
    "zc_id_prod-request",
    "zc_id_prod-made",
    "zc_id_prod-purchase",
    "zc_id_stat-sto-voucher",
    "zc_id_stat-trade_order",
    "zc_id_deta-trade_order",
];

fn lifecycle_tables_vec() -> Vec<String> {
    ZC_ID_LIFECYCLE_TABLES
        .iter()
        .map(|&s| s.to_string())
        .collect()
}

// ============================================
// Helper: lifecycle transfer check
// ============================================

fn lifecycle_transfer_check(src_form: &str, src_type: &str, t_form: &str, t_type: &str) -> bool {
    match t_form {
        "设计" => match t_type {
            "实例" => matches!(
                (src_form, src_type),
                ("创意", "范例") | ("创意", "实例") | ("设计", "范例")
            ),
            _ => matches!((src_form, src_type), ("创意", "范例") | ("创意", "实例")),
        },
        "实现" => match t_type {
            "实例" => matches!(
                (src_form, src_type),
                ("创意", "范例")
                    | ("创意", "实例")
                    | ("设计", "范例")
                    | ("设计", "实例")
                    | ("实现", "范例")
            ),
            _ => matches!(
                (src_form, src_type),
                ("创意", "范例") | ("创意", "实例") | ("设计", "范例") | ("设计", "实例")
            ),
        },
        _ => match t_type {
            "实例" => matches!((src_form, src_type), ("创意", "范例")),
            _ => false,
        },
    }
}

// ============================================
// 1. Lifecycle BizSet Template
// ============================================

pub struct LifecycleBizSetTemplate;

#[async_trait]
impl TriggerTemplate for LifecycleBizSetTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ins_09_on_zc_id_lifecycle".to_string(),
            applies_to: lifecycle_tables_vec(),
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

        let dk_function: Option<i64> = get_field(new, "dk_function");
        if dk_function.is_none() {
            return Ok(TriggerResult::new());
        }

        let engine = TemplateEngine::new(ctx.pool.clone());
        let func_id = dk_function.unwrap();

        let func_code: Option<String> = engine.resolve_variable_code_notice(func_id).await?;

        let code = func_code.ok_or_else(|| {
            TriggerError::ExecutionFailed(format!(
                "zc_id_function not found for dk_function={}",
                func_id
            ))
        })?;

        let (_f_, _t_) = if code.starts_with("!.") && code.len() > 2 {
            ("创意", "范例")
        } else if code.starts_with("!_") && code.len() > 2 {
            ("创意", "实例")
        } else if code.starts_with("↑.") && code.len() > 2 {
            ("设计", "范例")
        } else if code.starts_with("↑_") && code.len() > 2 {
            ("设计", "实例")
        } else if code.starts_with("↓.") && code.len() > 2 {
            ("实现", "范例")
        } else if code.starts_with("↓_") && code.len() > 2 {
            ("实现", "实例")
        } else {
            return Err(TriggerError::ExecutionFailed(format!(
                "职能编码无法识别: {}",
                code
            )));
        };

        // Lifecycle transfer check + cycle detection on ak_source
        let ak_source: Option<Vec<i64>> = get_field(new, "ak_source");
        if let Some(ref src_ids) = ak_source {
            let self_id: Option<i64> = get_field(new, "id");

            for &src_id in src_ids {
                if Some(src_id) == self_id {
                    return Err(TriggerError::ExecutionFailed(format!(
                        "循环引用! ak_source 包含自身 id={}",
                        src_id
                    )));
                }

                // Indirect cycle check via recursive CTE
                if let Some(my_id) = self_id {
                    let cycle_detected: Option<bool> = engine
                        .query_scalar(
                            r#"
                            WITH RECURSIVE source_chain(id) AS (
                                SELECT unnest(l.ak_source)
                                FROM isahl.zc_id_lifecycle l
                                WHERE l.id = $1
                                  AND l.ak_source IS NOT NULL
                                  AND array_length(l.ak_source, 1) > 0
                                UNION ALL
                                SELECT unnest(l.ak_source)
                                FROM isahl.zc_id_lifecycle l
                                JOIN source_chain sc ON l.id = sc.id
                                WHERE l.ak_source IS NOT NULL
                                  AND array_length(l.ak_source, 1) > 0
                            )
                            SELECT EXISTS(SELECT 1 FROM source_chain WHERE id = $2 LIMIT 1)
                            "#,
                            vec![Value::Number(src_id.into()), Value::Number(my_id.into())],
                        )
                        .await?;

                    if cycle_detected == Some(true) {
                        return Err(TriggerError::ExecutionFailed(format!(
                            "ak_source 间接循环引用 detected! src={} → ... → self={}",
                            src_id, my_id
                        )));
                    }
                }

                // Lifecycle transfer form/type check
                let src_row: Option<(String, String)> = engine
                    .query_scalar(
                        r#"SELECT "_f_", "_t_" FROM isahl.zc_id_lifecycle WHERE id = $1"#,
                        vec![Value::Number(src_id.into())],
                    )
                    .await?;

                if let Some((src_form, src_type)) = src_row {
                    if !lifecycle_transfer_check(&src_form, &src_type, _f_, _t_) {
                        return Err(TriggerError::ExecutionFailed(format!(
                            "源({},{}) → {},{}, 变换失败, ak_source 包含 id={}",
                            src_form, src_type, _f_, _t_, src_id
                        )));
                    }
                } else {
                    return Err(TriggerError::ExecutionFailed(format!(
                        "Source lifecycle not found: {}",
                        src_id
                    )));
                }
            }
        }

        let mut result = TriggerResult::new();
        result
            .modified_fields
            .insert("_f_".to_string(), Value::String(_f_.to_string()));
        result
            .modified_fields
            .insert("_t_".to_string(), Value::String(_t_.to_string()));
        Ok(result)
    }
}

// ============================================
// 2. Lifecycle Number Template
// ============================================

pub struct LifecycleNumberTemplate;

#[async_trait]
impl TriggerTemplate for LifecycleNumberTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ups_91_on_zc_id_lifecycle".to_string(),
            applies_to: lifecycle_tables_vec(),
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

        let mut result = TriggerResult::new();

        let tpl_id: Option<i64> = get_field(new, "tpl_id");
        let id: Option<i64> = get_field(new, "id");
        if tpl_id.is_none() {
            if let Some(id_val) = id {
                result = result.with_modified_field("tpl_id", Value::Number(id_val.into()));
            }
        }

        let notice: Option<String> = get_field(new, "notice");
        if let Some(notice_str) = notice {
            let mut keys: Vec<String> = Vec::new();
            for (key, value) in new.iter() {
                if (key.starts_with("qk_")
                    || key.starts_with("tk_")
                    || key.starts_with("ck_")
                    || key.starts_with("sk_")
                    || key.starts_with("lk_"))
                    && value.is_number()
                {
                    keys.push(key.clone());
                }
            }
            let hash = if keys.is_empty() {
                crc32_hex(&notice_str)
            } else {
                crc32_hex(&format!("{}-{}", notice_str, keys.join(",")))
            };

            let dk_scene: Option<i64> = get_field(new, "dk_scene");
            let dk_factor: Option<i64> = get_field(new, "dk_factor");
            let dk_function: Option<i64> = get_field(new, "dk_function");

            let prefix = if let Some(ref _pool) = ctx.pool {
                let engine = TemplateEngine::new(ctx.pool.clone());
                let mut parts = Vec::new();
                for id in [dk_scene, dk_factor, dk_function].into_iter().flatten() {
                    if let Ok(Some(code)) = engine.resolve_variable_code_notice(id).await {
                        parts.push(code);
                    }
                }
                parts.join("")
            } else {
                format!(
                    "{}{}{}",
                    dk_scene.map(|_| "S").unwrap_or(""),
                    dk_factor.map(|_| "F").unwrap_or(""),
                    dk_function.map(|_| "N").unwrap_or("")
                )
            };

            let number = if prefix.is_empty() {
                hash
            } else {
                format!("{}-{}", prefix, hash)
            };

            result = result.with_modified_field("projection", Value::String(number));
        }

        Ok(result)
    }
}

// ============================================
// 2.5 Notice Deduplication Template
// ============================================
///
/// On INSERT and UPDATE, checks if the same `notice` already exists in the same table.
/// If a conflict is detected, appends `@N` (sequential number) to make the notice unique.
///
/// Example: "华东区销售订单" → "华东区销售订单@2" (if "华东区销售订单" already exists)
///
pub struct NoticeDedupTemplate;

#[async_trait]
impl TriggerTemplate for NoticeDedupTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ups_notice_dedup".to_string(),
            applies_to: lifecycle_tables_vec(),
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
        let notice = match notice {
            Some(n) if !n.is_empty() => n,
            _ => return Ok(TriggerResult::new()),
        };

        // On UPDATE, skip dedup if notice hasn't changed
        if let Some(old) = _old_record {
            let old_notice: Option<String> = get_field(old, "notice");
            if old_notice.as_deref() == Some(&notice) {
                return Ok(TriggerResult::new());
            }
        }

        let self_id: Option<i64> = get_field(new, "id");
        let table_name = &ctx.table_name;

        let pool = ctx.pool.as_ref().ok_or_else(|| {
            TriggerError::ExecutionFailed("Pool required for dedup check".to_string())
        })?;

        let query = format!(
            "SELECT COUNT(*) as cnt FROM isahl.\"{}\" WHERE notice = $1 AND id IS DISTINCT FROM $2 AND deleted_at IS NULL",
            table_name
        );

        let count: i64 = sqlx::query_scalar(AssertSqlSafe(&query[..]))
            .bind(&notice)
            .bind(self_id)
            .fetch_one(pool)
            .await
            .map_err(|e| TriggerError::ExecutionFailed(format!("Dedup query failed: {}", e)))?;

        if count == 0 {
            return Ok(TriggerResult::new());
        }

        // Conflict exists — also count @N variants to determine next suffix
        let like_pattern = format!("{}@%", notice);
        let suffix_query = format!(
            "SELECT COUNT(*) as cnt FROM isahl.\"{}\" WHERE (notice = $1 OR notice LIKE $2) AND id IS DISTINCT FROM $3 AND deleted_at IS NULL",
            table_name
        );

        let total_similar: i64 = sqlx::query_scalar(AssertSqlSafe(&suffix_query[..]))
            .bind(&notice)
            .bind(&like_pattern)
            .bind(self_id)
            .fetch_one(pool)
            .await
            .map_err(|e| TriggerError::ExecutionFailed(format!("Suffix query failed: {}", e)))?;

        let suffix = total_similar + 1;
        let deduped = format!("{}@{}", notice, suffix);

        let mut result = TriggerResult::new();
        result
            .modified_fields
            .insert("notice".to_string(), Value::String(deduped));
        Ok(result)
    }
}

// ============================================
// 3. Lifecycle Delete Template
// ============================================

pub struct LifecycleDeleteTemplate;

#[async_trait]
impl TriggerTemplate for LifecycleDeleteTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_af_del_99_on_zc_id_lifecycle".to_string(),
            applies_to: lifecycle_tables_vec(),
            operations: vec![TriggerOperationDef::Delete],
            timing: TriggerTimingDef::After,
        }
    }

    async fn execute(
        &self,
        _ctx: &TriggerContext,
        old_record: Option<&HashMap<String, Value>>,
        _new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let old = old_record.ok_or_else(|| {
            TriggerError::ExecutionFailed("Old record required for DELETE trigger".to_string())
        })?;

        let id: i64 = get_field(old, "id").unwrap_or(0);

        Ok(
            TriggerResult::new().with_side_effect(crate::SideEffect::RawSql(format!(
                "DELETE FROM isahl.meta_lifecycle_event WHERE id = {}",
                id
            ))),
        )
    }
}

// ============================================
// 4. Lifecycle Non-Self Delete Template
// ============================================

pub struct LifecycleNonSelfDeleteTemplate;

#[async_trait]
impl TriggerTemplate for LifecycleNonSelfDeleteTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_af_del_90_on_zc_id_lifecycle".to_string(),
            applies_to: lifecycle_tables_vec(),
            operations: vec![TriggerOperationDef::Delete],
            timing: TriggerTimingDef::After,
        }
    }

    async fn execute(
        &self,
        _ctx: &TriggerContext,
        old_record: Option<&HashMap<String, Value>>,
        _new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let old = old_record.ok_or_else(|| {
            TriggerError::ExecutionFailed("Old record required for DELETE trigger".to_string())
        })?;

        let id: i64 = get_field(old, "id").unwrap_or(0);
        let mut result = TriggerResult::new();

        for table in [
            "zc_id_lifecycle_r_evaluation",
            "zc_id_lifecycle_r_category",
            "zc_id_lifecycle_r_tags",
        ] {
            result = result.with_side_effect(crate::SideEffect::RawSql(format!(
                "DELETE FROM isahl.{} WHERE ref_left = {} OR ref_right = {}",
                table, id, id
            )));
        }

        Ok(result)
    }
}

// ============================================
// 5. Lifecycle Relation Update Template
// ============================================

pub struct LifecycleRelationUpdateTemplate;

#[async_trait]
impl TriggerTemplate for LifecycleRelationUpdateTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_af_ups_on_zc_id_lifecycle_r_".to_string(),
            applies_to: vec![
                "zc_id_lifecycle_r_evaluation".to_string(),
                "zc_id_lifecycle_r_category".to_string(),
                "zc_id_lifecycle_r_tags".to_string(),
            ],
            operations: vec![TriggerOperationDef::Insert, TriggerOperationDef::Update],
            timing: TriggerTimingDef::After,
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

        let id: i64 = get_field(new, "id").unwrap_or(0);

        Ok(TriggerResult::new().with_side_effect(crate::SideEffect::RawSql(format!(
            "UPDATE isahl.zc_id_lifecycle SET number = isahl.gf_crc32(notice), updated_at = NOW() WHERE id = {}",
            id
        ))))
    }
}

// ============================================
// 6. Lifecycle _f_ / _t_ Auto-Derivation Template
// ============================================

/// 自动派生 `_f_` 与 `_t_` 字段。
///
/// 适用于所有 `zc_id_lifecycle` 子表。
/// 规则：当 INSERT/UPDATE 时 `_f_` 或 `_t_` 为空，
/// 根据 `dk_function` 查询 `zc_id_function.code` 并按前缀计算：
///
/// | 前缀  | _f_ | _t_ |
/// |-------|----------|----------|
/// | `!.`  | 创意     | 范例     |
/// | `!_`  | 创意     | 实例     |
/// | `↑.`  | 设计     | 范例     |
/// | `↑_`  | 设计     | 实例     |
/// | `↓.`  | 实现     | 范例     |
/// | `↓_`  | 实现     | 实例     |
///
/// 若显式传入了非空值则保留，不覆盖。
///
///
/// `LifecycleBizTemplate` 与裸 SQL 写路径（WZ Service 落库）共用本函数——
/// `_f_`/`_t_` 禁止字面量直写，一律由 `dk_function.code` 前缀派生。
/// 用 `starts_with` 链而非字节切片：`↑`/`↓` 为 3 字节 UTF-8，`&code[..2]` 会 panic。
pub fn derive_form_type(function_code: &str) -> Option<(&'static str, &'static str)> {
    if function_code.starts_with("!.") {
        Some(("创意", "范例"))
    } else if function_code.starts_with("!_") {
        Some(("创意", "实例"))
    } else if function_code.starts_with("↑.") {
        Some(("设计", "范例"))
    } else if function_code.starts_with("↑_") {
        Some(("设计", "实例"))
    } else if function_code.starts_with("↓.") {
        Some(("实现", "范例"))
    } else if function_code.starts_with("↓_") {
        Some(("实现", "实例"))
    } else {
        None
    }
}

pub struct LifecycleBizTemplate;

#[async_trait]
impl TriggerTemplate for LifecycleBizTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_lifecycle__f__type".to_string(),
            applies_to: vec!["zc_id_lifecycle".to_string()],
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

        // 获取当前 _f_ / _t_
        let _f_: Option<String> = get_field(new, "_f_");
        let _t_: Option<String> = get_field(new, "_t_");

        // 如果两者都已显式提供，跳过自动派生
        let form_empty = _f_.as_deref().is_none_or(|s| s.trim().is_empty());
        let type_empty = _t_.as_deref().is_none_or(|s| s.trim().is_empty());
        if !form_empty && !type_empty {
            return Ok(TriggerResult::new());
        }

        // 获取 dk_function
        let dk_function: Option<i64> = get_field(new, "dk_function");
        let dk = match dk_function {
            Some(d) if d > 0 => d,
            _ => return Ok(TriggerResult::new()),
        };

        // 查询 zc_id_function.code
        let engine = TemplateEngine::new(ctx.pool.clone());
        let func_code: Option<String> = engine
            .query_scalar(
                "SELECT COALESCE(code, notice) FROM isahl.zc_id_function WHERE id = $1",
                vec![Value::Number(dk.into())],
            )
            .await?;

        let v_code = match func_code {
            Some(c) => c,
            None => return Ok(TriggerResult::new()),
        };
        // 按前缀派生 _f_ / _t_（共享实现；字节切片对 ↑/↓ 这类 3 字节前缀会 panic）
        let Some((form, typ)) = derive_form_type(&v_code) else {
            return Ok(TriggerResult::new());
        };

        let mut result = TriggerResult::new();
        if form_empty {
            result = result.with_modified_field("_f_", Value::String(form.to_string()));
        }
        if type_empty {
            result = result.with_modified_field("_t_", Value::String(typ.to_string()));
        }
        Ok(result)
    }
}

// ============================================
// 7. Lifecycle NGAC Sync Template
// ============================================

/// 在 lifecycle 子表的 INSERT/UPDATE/DELETE 后自动同步到 NGAC 对象属性表。
///
/// 这是数据层整合的核心组件：将业务数据实例映射为 NGAC ObjectAttribute，
/// 使 Gateway NgacEnforcer 能够对具体资源实例进行权限判定。
pub struct LifecycleNgacSyncTemplate;

#[async_trait]
impl TriggerTemplate for LifecycleNgacSyncTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_ngac_sync_lifecycle".to_string(),
            applies_to: lifecycle_tables_vec(),
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
        match ctx.operation {
            TriggerOperation::Insert => {
                let new = new_record.ok_or_else(|| {
                    TriggerError::ExecutionFailed("New record required for NGAC sync".to_string())
                })?;
                self.handle_insert(ctx, new).await
            }
            TriggerOperation::Update => {
                let new = new_record.ok_or_else(|| {
                    TriggerError::ExecutionFailed("New record required for NGAC sync".to_string())
                })?;
                self.handle_update(ctx, old_record, new).await
            }
            TriggerOperation::Delete => {
                let old = old_record.ok_or_else(|| {
                    TriggerError::ExecutionFailed("Old record required for NGAC sync".to_string())
                })?;
                self.handle_delete(ctx, old).await
            }
        }
    }
}

/// NGAC 可读标识（`resource_identifier`）取值优先级：`notice → code → 回退 {resource_type}-{id}`
/// （NGAC_SPEC §2.2.1；通用 CRUD 路径 `crud::handler::register_created_resource_ngac` 同式）。
fn ngac_readable_identifier(
    record: &HashMap<String, Value>,
    resource_type: &str,
    id: i64,
) -> String {
    get_field::<String>(record, "notice")
        .or_else(|| get_field::<String>(record, "code"))
        .unwrap_or_else(|| format!("{resource_type}-{id}"))
}

impl LifecycleNgacSyncTemplate {
    async fn handle_insert(
        &self,
        ctx: &TriggerContext,
        new: &HashMap<String, Value>,
    ) -> Result<TriggerResult, TriggerError> {
        let id: i64 = get_id_field(new, "id")
            .ok_or_else(|| TriggerError::ExecutionFailed("Missing id for NGAC sync".to_string()))?;

        let resource_type = ctx.table_name.clone();
        // NGAC：行级 OA 注册走**授权写入单源**（`common::ngac_grants::ensure_resource_oa`，
        // NGAC_SPEC §2.2.5）——本模板只产数据面变种 `SideEffect::EnsureResourceOa`：
        // 幂等（`(resource_type, fk_resource)` 唯一 + `ON CONFLICT DO NOTHING`）、
        // 软删占位行不复活、命名（`o_name` = `{resource_type}-{id}`）与可读标识
        // （notice → code → 回退）均由单源决定，触发器不再自拼 OA SQL。
        let resource_identifier = Some(ngac_readable_identifier(new, &resource_type, id));

        Ok(
            TriggerResult::new().with_side_effect(crate::SideEffect::EnsureResourceOa {
                resource_type,
                resource_id: id,
                resource_identifier,
            }),
        )
    }

    async fn handle_update(
        &self,
        ctx: &TriggerContext,
        old: Option<&HashMap<String, Value>>,
        new: &HashMap<String, Value>,
    ) -> Result<TriggerResult, TriggerError> {
        let id: i64 = get_id_field(new, "id")
            .ok_or_else(|| TriggerError::ExecutionFailed("Missing id for NGAC sync".to_string()))?;

        // Detect soft delete: deleted_at transitions from null to non-null
        let was_soft_deleted = if let Some(old) = old {
            let old_deleted_at: Option<String> = get_field(old, "deleted_at");
            let new_deleted_at: Option<String> = get_field(new, "deleted_at");
            old_deleted_at.is_none() && new_deleted_at.is_some()
        } else {
            false
        };

        if was_soft_deleted {
            return self.handle_delete(ctx, old.unwrap_or(new)).await;
        }

        let resource_type = ctx.table_name.clone();
        let readable = ngac_readable_identifier(new, &resource_type, id);

        // 已存在行的标识同步保留为内联 SQL（**非授权写入**）：单源 `ngac_grants` 只提供
        // OA 幂等注册（insert-only）与主体 right 授权，没有「改写已存在行 o_name / 标识」的
        // 入口（NGAC_SPEC §2.2.5 的收口面 = 资源**创建路径**的行级授权写入）。
        // 取值与 insert 路径同式（`o_name` = `{resource_type}-{id}`、标识 notice → code →
        // 回退），使同一行经 insert / update 两条路径得到同一命名。
        let sql = r#"
            UPDATE isahl_auth.ngac_object_attribute
            SET o_name = $1,
                resource_identifier = $2,
                updated_at = NOW()
            WHERE resource_type = $3 AND fk_resource = $4
        "#
        .to_string();

        let params = vec![
            Value::String(format!("{resource_type}-{id}")),
            Value::String(readable),
            Value::String(resource_type),
            Value::Number(id.into()),
        ];

        Ok(TriggerResult::new()
            .with_side_effect(crate::SideEffect::RawSqlWithParams { sql, params }))
    }

    async fn handle_delete(
        &self,
        ctx: &TriggerContext,
        old: &HashMap<String, Value>,
    ) -> Result<TriggerResult, TriggerError> {
        let id: i64 = get_id_field(old, "id")
            .ok_or_else(|| TriggerError::ExecutionFailed("Missing id for NGAC sync".to_string()))?;

        let resource_type = ctx.table_name.clone();

        // 撤销保留为内联 SQL（**非授权写入**）：单源 `ngac_grants` 无 OA 撤销入口
        // （NGAC_SPEC §2.2.5 的收口面 = 资源**创建路径**的行级授权写入），且本语句非 INSERT
        // ⇒ 不在 `check-ngac-grant-single-source` 的语句级判据面内。
        let sql = r#"
            DELETE FROM isahl_auth.ngac_object_attribute
            WHERE resource_type = $1 AND fk_resource = $2
        "#
        .to_string();

        let params = vec![Value::String(resource_type), Value::Number(id.into())];

        Ok(TriggerResult::new()
            .with_side_effect(crate::SideEffect::RawSqlWithParams { sql, params }))
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
    async fn test_lifecycle_number_tpl_id_init() {
        let tpl = LifecycleNumberTemplate;
        let mut new = HashMap::new();
        new.insert("id".to_string(), Value::Number(123i64.into()));
        new.insert("notice".to_string(), Value::String("test".to_string()));

        let ctx = TriggerContext::new("zc_id_bill", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();

        assert_eq!(
            result.modified_fields.get("tpl_id"),
            Some(&Value::Number(123i64.into()))
        );
        assert!(result.modified_fields.contains_key("projection"));
        let projection = result
            .modified_fields
            .get("projection")
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(projection.len(), 8); // CRC32 hex
    }

    /// 数据面单源：insert 只产 `SideEffect::EnsureResourceOa`（不再自拼 OA SQL）。
    #[tokio::test]
    async fn test_lifecycle_ngac_sync_insert_emits_ensure_resource_oa() {
        let tpl = LifecycleNgacSyncTemplate;
        let mut new = HashMap::new();
        new.insert("id".to_string(), Value::Number(42i64.into()));
        new.insert("notice".to_string(), Value::String("公告".to_string()));

        let ctx = TriggerContext::new("zc_id_bill", TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();

        match result.side_effects.as_slice() {
            [crate::SideEffect::EnsureResourceOa {
                resource_type,
                resource_id,
                resource_identifier,
            }] => {
                assert_eq!(resource_type.as_str(), "zc_id_bill");
                assert_eq!(*resource_id, 42);
                assert_eq!(resource_identifier.as_deref(), Some("公告"));
            }
            other => panic!("期望单个 EnsureResourceOa，实得 {:?}", other),
        }
    }

    /// 标识优先级：notice → code → 回退 `{resource_type}-{id}`（NGAC_SPEC §2.2.1）。
    #[tokio::test]
    async fn test_lifecycle_ngac_sync_identifier_priority() {
        let tpl = LifecycleNgacSyncTemplate;
        let ctx = TriggerContext::new("zc_id_bill", TriggerOperation::Insert);
        let cases: [(&str, Option<&str>, Option<&str>, &str); 3] = [
            ("notice 优先", Some("公告7"), Some("BILL-7"), "公告7"),
            ("无 notice 取 code", None, Some("BILL-7"), "BILL-7"),
            ("两者皆无回退编号", None, None, "zc_id_bill-7"),
        ];
        for (why, notice, code, expected) in cases {
            let mut new = HashMap::new();
            new.insert("id".to_string(), Value::Number(7i64.into()));
            if let Some(n) = notice {
                new.insert("notice".to_string(), Value::String(n.to_string()));
            }
            if let Some(c) = code {
                new.insert("code".to_string(), Value::String(c.to_string()));
            }
            let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();
            let ident = match result.side_effects.as_slice() {
                [crate::SideEffect::EnsureResourceOa {
                    resource_identifier,
                    ..
                }] => resource_identifier.clone(),
                other => panic!("{why}: 期望单个 EnsureResourceOa，实得 {:?}", other),
            };
            assert_eq!(ident.as_deref(), Some(expected), "{why}");
        }
    }
}
