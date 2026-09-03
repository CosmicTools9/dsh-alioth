//! Schema Repository — 运行时感知表结构的完整 CRUD 引擎
//!
//! 替代 `OntologyDispatcher` 的扁平 JSON CRUD，提供完整的 C/R/U/D 能力，
//! 并利用编译期生成的 `fk_index` 实现零运行时开销的 FK 感知。
//!
//! # 设计原则
//!
//! - **运行时列发现**：通过 `information_schema.columns` 获取可写列（排除保护列）
//! - **编译期 FK 索引**：正向/反向 FK 关系来自 `fk_index.rs`，零 DB 往返
//! - **_refs 解析**：根据 FK 索引自动生成子查询
//! - **更新支持**：只更新前端提供的字段（不发全量）
//! - **created_by_id 自动注入**：create 时自动绑定 `created_by_id = user_id`

use crate::fk_index;
use common::AliothError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool, Postgres, Row, Transaction};

/// 叶子表插入请求载荷——通用的 JSON 文档（原 ontology_dispatcher 迁入）。
///
/// 任何 zc_id_lifecycle 叶子的 INSERT 都可以表达为：必填列 + 通用文本列
/// + 关联 FK + 标量 qk_* 引用。SQL 层把它们按列顺序展开。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AliothLeaf {
    /// 业务可见字段（notice / code / comments / o_number / etc.）
    #[serde(flatten)]
    pub fields: serde_json::Map<String, Value>,
}

impl AliothLeaf {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }
}

/// 已知的本体维度绑定：(scene, factor, function) 三元组。
pub type Binding = (Option<i64>, Option<i64>, Option<i64>);

/// 禁止用户直接写入的列——基础清单（两套写路径共享，单一来源）。
///
/// 通用写路径（`create`/`create_tx`/`update`）在此基础上附加 `SCHEMA_EXTRA_COLUMNS`；
/// 叶写路径（`create_in_leaf` 系）附加 `LEAF_EXTRA_COLUMNS`。差异是有意为之：
/// 通用 create 自动注入 `created_by_id`，故其保护清单不得包含该列；
/// 叶写路径的审计列由显式逻辑写入。
const PROTECTED_COLUMNS_BASE: &[&str] = &[
    "id",
    "created_at",
    "updated_at",
    "deleted_at",
    "deleted_by_id",
    "notice",
    "d_count",
    "x_version",
    "tk_version",
    "tk_batch_no",
    "reversion",
    "ak_dimensions",
    "ak_source",
    "fk_previous",
    "_f_",
    "_t_",
    "dk_scene",
    "dk_factor",
    "dk_function",
];

/// 通用写路径附加保护列（模板/集合列）。
const SCHEMA_EXTRA_COLUMNS: &[&str] = &["tpl_id", "majority", "sprint"];

/// 叶写路径附加保护列（审计列，由 create_in_leaf 显式写入）。
const LEAF_EXTRA_COLUMNS: &[&str] = &["created_by_id", "updated_by_id"];

/// 组装指定路径的保护列清单（BASE + 附加）。
fn protected_columns(extra: &[&'static str]) -> Vec<&'static str> {
    let mut cols: Vec<&str> = PROTECTED_COLUMNS_BASE.to_vec();
    cols.extend_from_slice(extra);
    cols
}

/// 运行时 CRUD 引擎。
#[derive(Clone)]
pub struct SchemaRepository {
    pool: PgPool,
}

impl SchemaRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 获取表的用户可写列（通用写路径保护清单）。
    pub async fn writable_columns(&self, table: &str) -> Result<Vec<String>, AliothError> {
        self.writable_columns_with(table, SCHEMA_EXTRA_COLUMNS)
            .await
    }

    /// 获取表的用户可写列——按指定保护清单（叶写路径传 `LEAF_EXTRA_COLUMNS`）。
    async fn writable_columns_with(
        &self,
        table: &str,
        protected: &[&'static str],
    ) -> Result<Vec<String>, AliothError> {
        let rows = sqlx::query(
            r#"SELECT column_name FROM information_schema.columns
               WHERE table_schema='isahl' AND table_name=$1
                 AND is_nullable='YES'
                 AND column_name <> ALL($2)
               ORDER BY ordinal_position"#,
        )
        .bind(table)
        .bind(protected_columns(protected))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(rows.iter().map(|r| r.get::<String, _>(0)).collect())
    }

    // ═══════════════════════════════════════════════════════════════
    // 创建（自动注入 created_by_id）
    // ═══════════════════════════════════════════════════════════════

    pub async fn create(&self, table: &str, data: Value, user_id: i64) -> Result<i64, AliothError> {
        let cols = self.writable_columns(table).await?;
        let (mut use_cols, mut use_vals) = Self::extract_fields(&data, &cols);
        // 自动注入 created_by_id
        if cols.contains(&"created_by_id".to_string()) {
            use_cols.push("created_by_id".to_string());
            use_vals.push(Value::from(user_id));
        }
        if use_cols.is_empty() {
            return Err(AliothError::BadRequest("no writable columns".into()));
        }
        let sql = Self::build_insert_sql(table, &use_cols);
        let col_types = crate::column_types::resolve(&self.pool, table).await;
        let mut q = sqlx::query_as::<_, (i64,)>(AssertSqlSafe(sql.as_str()));
        for (col, v) in use_cols.iter().zip(&use_vals) {
            q = bind_json(q, v, col_types.get(col).map(String::as_str));
        }
        let row = q
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(row.0)
    }

    pub async fn create_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        table: &str,
        data: Value,
        user_id: i64,
    ) -> Result<i64, AliothError> {
        let cols = self.writable_columns(table).await?;
        let (mut use_cols, mut use_vals) = Self::extract_fields(&data, &cols);
        if cols.contains(&"created_by_id".to_string()) {
            use_cols.push("created_by_id".to_string());
            use_vals.push(Value::from(user_id));
        }
        if use_cols.is_empty() {
            return Err(AliothError::BadRequest("no writable columns".into()));
        }
        let sql = Self::build_insert_sql(table, &use_cols);
        let col_types = crate::column_types::resolve(&self.pool, table).await;
        let mut q = sqlx::query_as::<_, (i64,)>(AssertSqlSafe(sql.as_str()));
        for (col, v) in use_cols.iter().zip(&use_vals) {
            q = bind_json(q, v, col_types.get(col).map(String::as_str));
        }
        let row = q
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(row.0)
    }

    // ═══════════════════════════════════════════════════════════════
    // 读取（含 _refs 自动解析）
    // ═══════════════════════════════════════════════════════════════

    pub async fn get(&self, table: &str, id: i64) -> Result<Option<Value>, AliothError> {
        let refs_sql = self.build_refs_sql(table);
        let sql = format!(
            r#"SELECT to_jsonb(t) || COALESCE(jsonb_build_object('_refs', jsonb_build_object({refs})), '{{}}'::jsonb)
               FROM isahl."{table}" t
               WHERE t.id = $1 AND t.deleted_at IS NULL"#,
            refs = refs_sql,
            table = table,
        );
        let row: Option<(Value,)> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(row.map(|(v,)| v))
    }

    pub async fn list(
        &self,
        table: &str,
        page: i64,
        page_size: i64,
    ) -> Result<Vec<Value>, AliothError> {
        let limit = page_size.clamp(1, 500);
        let offset = (page.max(1) - 1) * limit;
        let sql = format!(
            r#"SELECT to_jsonb(t) FROM (
               SELECT * FROM isahl."{table}"
               WHERE deleted_at IS NULL
               ORDER BY id DESC
               LIMIT $1 OFFSET $2
             ) t"#,
            table = table,
        );
        let rows: Vec<(Value,)> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(|(v,)| v).collect())
    }

    pub async fn list_with_refs(
        &self,
        table: &str,
        page: i64,
        page_size: i64,
    ) -> Result<Vec<Value>, AliothError> {
        let limit = page_size.clamp(1, 500);
        let offset = (page.max(1) - 1) * limit;
        let refs_sql = self.build_refs_sql(table);
        let sql = format!(
            r#"SELECT to_jsonb(t) || COALESCE(jsonb_build_object('_refs', jsonb_build_object({refs})), '{{}}'::jsonb)
               FROM isahl."{table}" t
               WHERE t.deleted_at IS NULL
               ORDER BY t.id DESC
               LIMIT $1 OFFSET $2"#,
            refs = refs_sql,
            table = table,
        );
        let rows: Vec<(Value,)> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(|(v,)| v).collect())
    }

    /// List with optional row-level security filter.
    ///
    /// When `visible_ids` is `Some(ids)`, only resources whose `id` is in
    /// the set are returned. When `None`, all non-deleted resources are returned
    /// (admin/unfiltered path).
    pub async fn list_filtered(
        &self,
        table: &str,
        page: i64,
        page_size: i64,
        visible_ids: Option<&[i64]>,
    ) -> Result<Vec<Value>, AliothError> {
        let limit = page_size.clamp(1, 500);
        let offset = (page.max(1) - 1) * limit;

        let (sql, _param_count): (String, usize) = match visible_ids {
            Some(ids) if !ids.is_empty() => {
                let placeholders: Vec<String> =
                    (0..ids.len()).map(|i| format!("${}", i + 3)).collect();
                (
                    format!(
                        r#"SELECT to_jsonb(t) FROM (
                           SELECT * FROM isahl."{table}"
                           WHERE deleted_at IS NULL
                             AND id = ANY(ARRAY[{ph}]::BIGINT[])
                           ORDER BY id DESC
                           LIMIT $1 OFFSET $2
                         ) t"#,
                        table = table,
                        ph = placeholders.join(","),
                    ),
                    ids.len(),
                )
            }
            Some(_) => {
                // Some(空集) = 显式无授权 → 恒假谓词零行（NGAC_SPEC：Some([])=无权限）
                (
                    format!(
                        r#"SELECT to_jsonb(t) FROM (
                           SELECT * FROM isahl."{table}"
                           WHERE deleted_at IS NULL
                             AND false
                           ORDER BY id DESC
                           LIMIT $1 OFFSET $2
                         ) t"#,
                        table = table,
                    ),
                    0,
                )
            }
            None => {
                // None = admin / 非 RLS 调用方，不过滤
                (
                    format!(
                        r#"SELECT to_jsonb(t) FROM (
                           SELECT * FROM isahl."{table}"
                           WHERE deleted_at IS NULL
                           ORDER BY id DESC
                           LIMIT $1 OFFSET $2
                         ) t"#,
                        table = table,
                    ),
                    0,
                )
            }
        };

        let mut q = sqlx::query_as::<_, (Value,)>(AssertSqlSafe(sql.as_str()))
            .bind(limit)
            .bind(offset);
        if let Some(ids) = visible_ids {
            for id in ids {
                q = q.bind(*id);
            }
        }
        let rows: Vec<(Value,)> = q
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(|(v,)| v).collect())
    }

    // ═══════════════════════════════════════════════════════════════
    // 更新（自动注入 updated_by_id）
    // ═══════════════════════════════════════════════════════════════

    pub async fn update(
        &self,
        table: &str,
        id: i64,
        data: Value,
        user_id: i64,
    ) -> Result<Option<Value>, AliothError> {
        let cols = self.writable_columns(table).await?;
        let (use_cols, use_vals) = Self::extract_fields(&data, &cols);
        if use_cols.is_empty() {
            return Err(AliothError::BadRequest("no writable columns".into()));
        }
        let n = use_cols.len();
        let set_clause = use_cols
            .iter()
            .enumerate()
            .map(|(i, c)| format!("\"{}\" = ${}", c, i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            r#"UPDATE isahl."{table}" SET {set}, updated_at = NOW(), updated_by_id = ${uid}
               WHERE id = ${id} AND deleted_at IS NULL RETURNING id"#,
            table = table,
            set = set_clause,
            uid = n + 1,
            id = n + 2,
        );
        let col_types = crate::column_types::resolve(&self.pool, table).await;
        let mut q = sqlx::query_as::<_, (i64,)>(AssertSqlSafe(sql.as_str()));
        for (col, v) in use_cols.iter().zip(&use_vals) {
            q = bind_json(q, v, col_types.get(col).map(String::as_str));
        }
        q = q.bind(user_id).bind(id);
        let _ = q
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        self.get(table, id).await
    }

    // ═══════════════════════════════════════════════════════════════
    // 删除（支持级联策略）
    // ═══════════════════════════════════════════════════════════════

    /// 软删除记录。根据 `fk_index::resolve_cascade()` 决定的策略处理反向 FK：
    ///
    /// - `Restrict` — 有子记录则拒绝删除
    /// - `Cascade` — 级联软删除所有子记录
    /// - `SetNull` — 将子记录的 FK 置为 NULL
    /// - `SetDefault` — 将子记录的 FK 设为默认值
    pub async fn delete(&self, table: &str, id: i64, user_id: i64) -> Result<(), AliothError> {
        use fk_index::CascadeStrategy;
        let back_refs = fk_index::lookup_reverse_fk(table);
        for (source_table, field_name, _local_key) in back_refs {
            let strategy = fk_index::resolve_cascade(source_table, field_name);
            match strategy {
                CascadeStrategy::Restrict => {
                    let exists = self.has_children(source_table, field_name, id).await?;
                    if exists {
                        return Err(AliothError::BadRequest(format!(
                            "Cannot delete {}: {} still has related records in {} via {}",
                            table, table, source_table, field_name
                        )));
                    }
                }
                CascadeStrategy::Cascade => {
                    self.cascade_delete(source_table, field_name, id, user_id)
                        .await?;
                }
                CascadeStrategy::SetNull => {
                    self.set_null_fk(source_table, field_name, id).await?;
                }
                CascadeStrategy::SetDefault => {
                    self.set_default_fk(source_table, field_name, id).await?;
                }
            }
        }
        let sql = format!(
            r#"UPDATE isahl."{table}" SET deleted_at = NOW(), deleted_by_id = $1
               WHERE id = $2 AND deleted_at IS NULL"#,
            table = table,
        );
        let rows = sqlx::query(AssertSqlSafe(sql.as_str()))
            .bind(user_id)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        if rows.rows_affected() == 0 {
            return Err(AliothError::NotFound("not_found_or_already_deleted".into()));
        }
        Ok(())
    }

    // ─── 级联策略辅助 ─────────────────────────────────────────────

    async fn has_children(
        &self,
        source: &str,
        field: &str,
        parent_id: i64,
    ) -> Result<bool, AliothError> {
        let sql = format!(
            r#"SELECT EXISTS (SELECT 1 FROM isahl."{source}" WHERE "{field}" = $1 AND deleted_at IS NULL)"#,
            source = source,
            field = field,
        );
        let (exists,): (bool,) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(parent_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(exists)
    }

    async fn cascade_delete(
        &self,
        source: &str,
        field: &str,
        parent_id: i64,
        user_id: i64,
    ) -> Result<(), AliothError> {
        let sql = format!(
            r#"UPDATE isahl."{source}" SET deleted_at = NOW(), deleted_by_id = $1
               WHERE "{field}" = $2 AND deleted_at IS NULL"#,
            source = source,
            field = field,
        );
        sqlx::query(AssertSqlSafe(sql.as_str()))
            .bind(user_id)
            .bind(parent_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(())
    }

    async fn set_null_fk(
        &self,
        source: &str,
        field: &str,
        parent_id: i64,
    ) -> Result<(), AliothError> {
        let sql = format!(
            r#"UPDATE isahl."{source}" SET "{field}" = NULL, updated_at = NOW()
               WHERE "{field}" = $1 AND deleted_at IS NULL"#,
            source = source,
            field = field,
        );
        sqlx::query(AssertSqlSafe(sql.as_str()))
            .bind(parent_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(())
    }

    async fn set_default_fk(
        &self,
        source: &str,
        field: &str,
        parent_id: i64,
    ) -> Result<(), AliothError> {
        // SET_DEFAULT 需要知道每列的默认值。information_schema.columns 中有 column_default。
        // 对有默认值的列：重置为 DEFAULT
        let sql = format!(
            r#"UPDATE isahl."{source}" SET "{field}" = DEFAULT, updated_at = NOW()
               WHERE "{field}" = $1 AND deleted_at IS NULL"#,
            source = source,
            field = field,
        );
        sqlx::query(AssertSqlSafe(sql.as_str()))
            .bind(parent_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // 内部辅助
    // ═══════════════════════════════════════════════════════════════

    fn extract_fields(data: &Value, cols: &[String]) -> (Vec<String>, Vec<Value>) {
        let fields = match data {
            Value::Object(m) => m,
            _ => return (vec![], vec![]),
        };
        let mut use_cols = Vec::new();
        let mut use_vals = Vec::new();
        for c in cols {
            if let Some(v) = fields.get(c) {
                if !v.is_null() {
                    use_cols.push(c.clone());
                    use_vals.push(v.clone());
                }
            }
        }
        (use_cols, use_vals)
    }

    fn build_insert_sql(table: &str, use_cols: &[String]) -> String {
        let col_list = use_cols
            .iter()
            .map(|c| format!("\"{}\"", c))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = (1..=use_cols.len())
            .map(|i| format!("${}", i))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            r#"INSERT INTO isahl."{table}" ({cols}) VALUES ({ph}) RETURNING id"#,
            table = table,
            cols = col_list,
            ph = placeholders,
        )
    }

    /// 构建 `_refs` 子查询的逗号分隔片段。
    ///
    /// 返回形如：`'fk_parent', (SELECT jsonb_build_object('notice', r0.notice) FROM isahl."target" r0 WHERE r0.id = t.fk_parent)`
    fn build_refs_sql(&self, table: &str) -> String {
        let refs = fk_index::lookup_forward_fk(table);
        if refs.is_empty() {
            return String::new();
        }
        let mut parts = Vec::new();
        for (i, (field_name, target_table, local_key)) in refs.iter().enumerate() {
            let alias = format!("r{}", i);
            let lk = if local_key.is_empty() {
                field_name
            } else {
                local_key
            };
            let sub = format!(
                "'{field}', (SELECT jsonb_build_object('notice', {alias}.notice) \
                 FROM isahl.\"{target}\" {alias} WHERE {alias}.id = t.\"{lk}\")",
                field = field_name,
                alias = alias,
                target = target_table,
                lk = lk,
            );
            parts.push(sub);
        }
        parts.join(", ")
    }

    // ═══════════════════════════════════════════════════════════════
    // 叶写路径（原 OntologyDispatcher，已并入）——zc_id_lifecycle 继承链
    // ═══════════════════════════════════════════════════════════════

    /// 有子表的根/中间表（如 zc_id_lifecycle）返回 false，不可作为写入目标。
    ///
    /// 复用 `isahl_meta.gf_query_leafs`（权威继承视图 `devv_inherits_view` 的
    /// 叶查询函数，与 schema-info `leafs-of`、ontology baseline 刷新同源），
    /// 不手写继承链。受管根：zc_id_lifecycle（业务实体）/ zc_id_scale（qk 标量）/
    /// zc_id_object + zc_ad_scalar（sk/ck/tk 标量引用）/ zc_id_eval-comparable（lk 等级）。
    /// FUNC-017: `vw_` 前缀视图（只读投影，如 isahl.vw_requirement_flat）放行列表/详情读取；
    /// 写入路径（create_in_leaf / delete_leaf）仍拒绝视图。
    pub async fn is_leaf_table(&self, table: &str) -> Result<bool, AliothError> {
        if table.starts_with("vw_") {
            return Ok(true);
        }
        let (is_leaf,): (bool,) = sqlx::query_as(
            r#"SELECT $1 = ANY(isahl_meta.gf_query_leafs('zc_id_lifecycle'))
                    OR $1 = ANY(isahl_meta.gf_query_leafs('zc_id_scale'))
                    OR $1 = ANY(isahl_meta.gf_query_leafs('zc_id_object'))
                    OR $1 = ANY(isahl_meta.gf_query_leafs('zc_ad_scalar'))
                    OR $1 = ANY(isahl_meta.gf_query_leafs('zc_id_eval-comparable'))"#,
        )
        .bind(table)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(is_leaf)
    }

    /// 视图只读守卫：vw_ 前缀视图禁止写入（FUNC-017 投影只读铁则）。
    fn ensure_writable_table(table: &str) -> Result<(), AliothError> {
        if table.starts_with("vw_") {
            return Err(AliothError::BadRequest(format!(
                "{} 是只读投影视图，禁止写入（写路径走基础表）",
                table
            )));
        }
        Ok(())
    }

    /// 创建一条记录到指定叶子表，自动应用 ontology_bindings。
    /// 返回新记录的 `id`。
    pub async fn create_in_leaf(
        &self,
        table: &str,
        binding: Binding,
        leaf: AliothLeaf,
        user_id: i64,
    ) -> Result<i64, AliothError> {
        Self::ensure_writable_table(table)?;
        if !self.is_leaf_table(table).await? {
            return Err(AliothError::BadRequest(format!(
                "{} 不是 zc_id_lifecycle 的叶子表",
                table
            )));
        }
        let cols = self
            .writable_columns_with(table, LEAF_EXTRA_COLUMNS)
            .await?;
        // 业务字段：仅接受现有列名
        let mut use_cols: Vec<String> = Vec::new();
        let mut use_vals: Vec<Value> = Vec::new();
        for c in &cols {
            if let Some(v) = leaf.fields.get(c) {
                if !v.is_null() {
                    use_cols.push(c.clone());
                    use_vals.push(v.clone());
                }
            }
        }
        // 本体绑定列（如果存在）：dk_* 在保护清单中被排除，须用全列映射判断存在性
        // （回归：4e6b6ab31 把 dk_* 加入保护清单后，旧 cols.contains 逻辑恒为 false）
        let all_cols = crate::column_types::resolve(&self.pool, table).await;
        let (scene, factor, function) = binding;
        if scene.is_some() && all_cols.contains_key("dk_scene") {
            use_cols.push("dk_scene".to_string());
            use_vals.push(serde_json::json!(scene));
        }
        if factor.is_some() && all_cols.contains_key("dk_factor") {
            use_cols.push("dk_factor".to_string());
            use_vals.push(serde_json::json!(factor));
        }
        if function.is_some() && all_cols.contains_key("dk_function") {
            use_cols.push("dk_function".to_string());
            use_vals.push(serde_json::json!(function));
        }
        if use_cols.is_empty() {
            return Err(AliothError::BadRequest("no writable columns".into()));
        }
        // 审计列：全列映射判断（与 create_in_leaf_tx 一致；修复原实现
        // cols.contains("created_by_id") 因保护清单含该列而恒 false、永不注入的问题）
        if all_cols.contains_key("created_by_id") {
            use_cols.push("created_by_id".to_string());
            use_vals.push(serde_json::json!(user_id));
        }
        let sql = Self::build_insert_sql(table, &use_cols);
        let mut q = sqlx::query_as::<_, (i64,)>(AssertSqlSafe(sql.as_str()));
        for (col, v) in use_cols.iter().zip(&use_vals) {
            q = bind_json(q, v, all_cols.get(col).map(String::as_str));
        }
        let row = q
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(row.0)
    }

    /// 事务内创建（用于与其他写操作一同提交）。
    pub async fn create_in_leaf_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        table: &str,
        binding: Binding,
        leaf: AliothLeaf,
        user_id: i64,
    ) -> Result<i64, AliothError> {
        Self::ensure_writable_table(table)?;
        // 叶子表验证仅查一次池
        if !self.is_leaf_table(table).await? {
            return Err(AliothError::BadRequest(format!(
                "{} 不是 zc_id_lifecycle 的叶子表",
                table
            )));
        }
        let cols = self
            .writable_columns_with(table, LEAF_EXTRA_COLUMNS)
            .await?;
        let mut use_cols: Vec<String> = Vec::new();
        let mut use_vals: Vec<Value> = Vec::new();
        for c in &cols {
            if let Some(v) = leaf.fields.get(c) {
                if !v.is_null() {
                    use_cols.push(c.clone());
                    use_vals.push(v.clone());
                }
            }
        }
        // dk_* 在保护清单中被排除，须用全列映射判断存在性
        let all_cols = crate::column_types::resolve(&self.pool, table).await;
        let (scene, factor, function) = binding;
        if scene.is_some() && all_cols.contains_key("dk_scene") {
            use_cols.push("dk_scene".to_string());
            use_vals.push(serde_json::json!(scene));
        }
        if factor.is_some() && all_cols.contains_key("dk_factor") {
            use_cols.push("dk_factor".to_string());
            use_vals.push(serde_json::json!(factor));
        }
        if function.is_some() && all_cols.contains_key("dk_function") {
            use_cols.push("dk_function".to_string());
            use_vals.push(serde_json::json!(function));
        }
        if use_cols.is_empty() {
            return Err(AliothError::BadRequest("no writable columns".into()));
        }
        if all_cols.contains_key("created_by_id") {
            use_cols.push("created_by_id".to_string());
            use_vals.push(serde_json::json!(user_id));
        }
        let sql = Self::build_insert_sql(table, &use_cols);
        let mut q = sqlx::query_as::<_, (i64,)>(AssertSqlSafe(sql.as_str()));
        for (col, v) in use_cols.iter().zip(&use_vals) {
            q = bind_json(q, v, all_cols.get(col).map(String::as_str));
        }
        let row = q
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(row.0)
    }

    /// 列表分页查询任意叶子表（基于 QueryBuilder 思路但按 JSON 形态返回）。
    pub async fn list_leaf(
        &self,
        table: &str,
        page: i64,
        page_size: i64,
    ) -> Result<Vec<Value>, AliothError> {
        // 读取不要求 leaf 表判定（leaf 端点可读任意业务表；判定仅约束写路径），
        // 但保留表存在性校验（防 SQL 标识符注入/不存在表）
        self.ensure_reference_table_exists(table).await?;
        let limit = page_size.clamp(1, 500);
        let offset = (page.max(1) - 1) * limit;
        let sql = format!(
            r#"SELECT to_jsonb(t) FROM (
               SELECT * FROM isahl."{}"
               WHERE deleted_at IS NULL
               ORDER BY id DESC
               LIMIT {} OFFSET {}
             ) t"#,
            table, limit, offset
        );
        let rows: Vec<(Value,)> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(|(v,)| v).collect())
    }

    /// 按 id 获取单条记录。
    pub async fn get_leaf(&self, table: &str, id: i64) -> Result<Option<Value>, AliothError> {
        // 读取不要求 leaf 表判定（同 list_leaf：判定仅约束写路径），保留表存在性校验
        self.ensure_reference_table_exists(table).await?;
        let sql = format!(
            r#"SELECT to_jsonb(t) FROM isahl."{}" t WHERE id = $1 AND deleted_at IS NULL"#,
            table
        );
        let row: Option<(Value,)> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(row.map(|(v,)| v))
    }

    /// 引用表存在性校验（DB 侧兜底，授权在 handler 层完成）。
    /// 仅确认表存在于 `isahl` schema；授权（fk_index 覆盖 或 service 注入
    /// allowlist）由 handler 判定，安全策略不散落于此。
    pub async fn ensure_reference_table_exists(&self, table: &str) -> Result<(), AliothError> {
        let (exists,): (bool,) = sqlx::query_as(
            r#"SELECT EXISTS(
                 SELECT 1 FROM information_schema.tables
                 WHERE table_schema='isahl' AND table_name=$1
               )"#,
        )
        .bind(table)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AliothError::Database(e.to_string()))?;
        if exists {
            Ok(())
        } else {
            Err(AliothError::BadRequest(format!(
                "{} 不是 isahl 下的受管表",
                table
            )))
        }
    }

    /// 只读引用列表（枚举下拉场景）：读取任意受管表（含非叶表）。
    /// 与 `list_leaf` 的区别：不走 `is_leaf_table` 契约（leaf 语义保留给写路径）。
    pub async fn list_reference(
        &self,
        table: &str,
        page: i64,
        page_size: i64,
    ) -> Result<Vec<Value>, AliothError> {
        self.ensure_reference_table_exists(table).await?;
        let limit = page_size.clamp(1, 500);
        let offset = (page.max(1) - 1) * limit;
        let sql = format!(
            r#"SELECT to_jsonb(t) FROM (
               SELECT * FROM isahl."{}"
               WHERE deleted_at IS NULL
               ORDER BY id DESC
               LIMIT {} OFFSET {}
             ) t"#,
            table, limit, offset
        );
        let rows: Vec<(Value,)> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(|(v,)| v).collect())
    }

    /// 只读引用详情（枚举下拉场景，按 id 取单条）。
    pub async fn get_reference(&self, table: &str, id: i64) -> Result<Option<Value>, AliothError> {
        self.ensure_reference_table_exists(table).await?;
        let sql = format!(
            r#"SELECT to_jsonb(t) FROM isahl."{}" t WHERE id = $1 AND deleted_at IS NULL"#,
            table
        );
        let row: Option<(Value,)> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;
        Ok(row.map(|(v,)| v))
    }
}

// ═══════════════════════════════════════════════════════════════
// Value → SQL 绑定（按列类型强转，见 crate::bind_json）
// ═══════════════════════════════════════════════════════════════

fn bind_json<'q>(
    q: sqlx::query::QueryAs<'q, Postgres, (i64,), sqlx::postgres::PgArguments>,
    v: &'q Value,
    data_type: Option<&str>,
) -> sqlx::query::QueryAs<'q, Postgres, (i64,), sqlx::postgres::PgArguments> {
    crate::bind_json::apply_query_as(q, crate::bind_json::coerce(v, data_type))
}
