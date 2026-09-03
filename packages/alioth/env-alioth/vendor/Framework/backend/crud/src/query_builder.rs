//! QueryBuilder — 深度 SQL 组合模块
//!
//! 从 AliothDbEntity 元数据生成标准 CRUD SQL，处理：
//! - 显式字段列表
//! - 软删除过滤
//! - 审计列注入
//! - 参数索引管理
//! - 分页 + 计数
//! - 引用解析（当 E: HasReferenceJoins 时可用 `fetch_refs` / `get_refs`）

use std::collections::HashMap;
use std::marker::PhantomData;

use sqlx::{AssertSqlSafe, PgPool, Postgres, Row};

use crate::cascade::CascadeConfig;
use crate::column_types;
use crate::entity::AliothDbEntity;
use crate::filter::Filter;
use crate::pagination::{ListQuery, ListQueryExt, PaginatedResponse};
use crate::reference::{build_refs_select_suffix, build_sensitive_suffix, HasReferenceJoins};
use crate::sort::Sort;
use common::AliothError;

/// 查询构建器
///
/// 泛型参数 `E` 为实体类型，必须实现 `AliothDbEntity`。
pub struct QueryBuilder<'a, E: AliothDbEntity> {
    pool: &'a PgPool,
    filters: Vec<Filter>,
    raw_filters: Vec<String>,
    sorts: Vec<Sort>,
    /// RLS visible_ids — 列表查询仅返回 id 在此数组内的行
    visible_ids: Option<Vec<i64>>,
    /// 列级授权（NGAC X-Authorized-Columns）——敏感列投影裁剪；None=未启用列控
    authorized_columns: Option<Vec<String>>,
    /// COORDINATE_FILTER code 子查询预解析的 dk 三元组 id（参数化，避免每查询子查询开销）
    dk_binds: Vec<i64>,
    _phantom: PhantomData<E>,
}

/// 从 COORDINATE_FILTER 提取 code 子查询形态的三元组 code（scene/factor/function）。
/// 形态：`dk_scene = (SELECT id FROM isahl."zc_id_scene" WHERE code = 'JE' ...) AND dk_factor = ... AND dk_function = ...`
/// 非此形态（任意自定义 SQL）→ None（fallback 原样）。
fn extract_dk_codes(filter: &str) -> Option<(String, String, String)> {
    if !(filter.contains("zc_id_scene")
        && filter.contains("zc_id_factor")
        && filter.contains("zc_id_function"))
    {
        return None;
    }
    let marker = "WHERE code = '";
    let mut codes: Vec<String> = Vec::new();
    let mut rest = filter;
    for _ in 0..3 {
        let idx = rest.find(marker)?;
        let start = idx + marker.len();
        let end_rel = rest[start..].find('\'')?;
        codes.push(rest[start..start + end_rel].to_string());
        rest = &rest[start + end_rel..];
    }
    Some((codes[0].clone(), codes[1].clone(), codes[2].clone()))
}

impl<'a, E: AliothDbEntity> QueryBuilder<'a, E> {
    /// 构建 WHERE 子句（SOFT_DELETE + COORDINATE_FILTER + filters + raw_filters + visible_ids）。
    /// 递增 `param_idx`；返回 (以 " WHERE " 开头的 SQL 片段, 列类型映射)。
    async fn build_where_sql(
        &mut self,
        param_idx: &mut usize,
    ) -> (String, std::collections::HashMap<String, String>) {
        let mut sql = if E::SOFT_DELETE {
            String::from(" WHERE deleted_at IS NULL")
        } else {
            String::from(" WHERE 1 = 1")
        };

        // Append coordinate discriminator filter for shared-table entities
        // COORDINATE_FILTER code 子查询形态（BACKEND_FRAMEWORK §7.3.3 生成的
        // `dk_scene = (SELECT id FROM isahl."zc_id_scene" WHERE code = 'X' ...)`）：
        // 预解析 code → 参数化 bind（避免每查询 3 次子查询的 NFR 退化，NFR-001 实测
        // 983ms vs 阈值 500ms）；解析失败（维度缺失）→ fallback 原样子查询（行为一致）。
        if !E::COORDINATE_FILTER.is_empty() {
            let codes = extract_dk_codes(E::COORDINATE_FILTER);
            if let Some((s, f, fn_)) = codes {
                // code → id 内联解析（Coords 为 &'static str，不适用运行时 String——直接查维度表）
                let resolved = sqlx::query(
                    r#"SELECT
                        (SELECT id FROM "isahl"."zc_id_scene" WHERE code = $1 AND deleted_at IS NULL LIMIT 1),
                        (SELECT id FROM "isahl"."zc_id_factor" WHERE code = $2 AND deleted_at IS NULL LIMIT 1),
                        (SELECT id FROM "isahl"."zc_id_function" WHERE code = $3 AND deleted_at IS NULL LIMIT 1)"#,
                )
                .bind(&s)
                .bind(&f)
                .bind(&fn_)
                .fetch_one(self.pool)
                .await;
                if let Ok(row) = resolved {
                    let si: Option<i64> = row.try_get(0).ok().flatten();
                    let fi: Option<i64> = row.try_get(1).ok().flatten();
                    let ki: Option<i64> = row.try_get(2).ok().flatten();
                    if let (Some(si), Some(fi), Some(ki)) = (si, fi, ki) {
                        sql.push_str(&format!(
                            " AND dk_scene = ${} AND dk_factor = ${} AND dk_function = ${}",
                            *param_idx,
                            *param_idx + 1,
                            *param_idx + 2
                        ));
                        self.dk_binds.extend([si, fi, ki]);
                        *param_idx += 3;
                    } else {
                        sql.push_str(" AND ");
                        sql.push_str(E::COORDINATE_FILTER);
                    }
                } else {
                    sql.push_str(" AND ");
                    sql.push_str(E::COORDINATE_FILTER);
                }
            } else {
                sql.push_str(" AND ");
                sql.push_str(E::COORDINATE_FILTER);
            }
        }

        let col_types = self.column_types().await;
        for filter in &self.filters {
            if let Ok(()) = filter.validate() {
                if let Some(condition) = filter
                    .to_sql_with_type(*param_idx, col_types.get(&filter.field).map(String::as_str))
                {
                    sql.push_str(" AND ");
                    sql.push_str(&condition);
                    *param_idx += 1;
                }
            }
        }

        for raw in &self.raw_filters {
            sql.push_str(" AND ");
            sql.push_str(raw);
        }

        // RLS: visible_ids 行级安全过滤
        // Some(空集) = 显式无授权（`none` header）→ 恒假谓词（fail-closed 零行）；
        // Some(非空) = 仅返回授权 id 行；None = 无 RLS 约束（兼容非 RLS 调用方）。
        if let Some(ref ids) = self.visible_ids {
            if ids.is_empty() {
                sql.push_str(" AND false");
            } else {
                sql.push_str(&format!(" AND id = ANY(${})", param_idx));
                *param_idx += 1;
            }
        }

        (sql, col_types)
    }

    /// 构建 ORDER BY 子句（无排序时按 id DESC）。
    fn build_order_sql(&self) -> String {
        if !self.sorts.is_empty() {
            let sort_clauses: Vec<String> = self
                .sorts
                .iter()
                .filter_map(|s| {
                    if s.validate().is_ok() {
                        Some(s.to_sql())
                    } else {
                        None
                    }
                })
                .collect();
            if sort_clauses.is_empty() {
                String::from(" ORDER BY id DESC")
            } else {
                format!(" ORDER BY {}", sort_clauses.join(", "))
            }
        } else {
            String::from(" ORDER BY id DESC")
        }
    }

    /// 解析实体表列 → data_type 映射（共享 column_types 模块的进程级缓存）。
    ///
    /// 失败时返回空表，过滤逻辑退化为列 `::text`（历史行为，不报错）。
    async fn column_types(&self) -> HashMap<String, String> {
        column_types::resolve(self.pool, E::table_name()).await
    }

    pub fn new(pool: &'a PgPool) -> Self {
        Self {
            pool,
            filters: Vec::new(),
            raw_filters: Vec::new(),
            sorts: Vec::new(),
            visible_ids: None,
            authorized_columns: None,
            dk_binds: Vec::new(),
            _phantom: PhantomData,
        }
    }

    /// 从 ListQuery 初始化（便捷构造）
    pub fn from_list_query(pool: &'a PgPool, query: &ListQuery) -> Self {
        let mut builder = Self::new(pool);
        if let Some(mut f) = query.to_filter() {
            for (from, to) in E::column_renames() {
                if f.field == from {
                    f.field = to.to_string();
                    break;
                }
            }
            builder.filters.push(f);
        }
        if let Some(s) = query.to_sort() {
            builder.sorts.push(s);
        }
        builder
    }

    pub fn filter(mut self, filter: Filter) -> Self {
        self.filters.push(filter);
        self
    }

    /// 设置 RLS 可见 ID 列表（行级安全过滤）
    pub fn with_visible_ids(mut self, ids: Vec<i64>) -> Self {
        self.visible_ids = Some(ids);
        self
    }

    /// 设置列级授权（NGAC X-Authorized-Columns）——敏感列投影裁剪
    pub fn with_authorized_columns(mut self, cols: Vec<String>) -> Self {
        self.authorized_columns = Some(cols);
        self
    }

    /// 添加原始 SQL WHERE 条件（不经过 Filter 验证，由调用方确保安全）
    pub fn raw_filter(mut self, condition: String) -> Self {
        self.raw_filters.push(condition);
        self
    }

    pub fn sort(mut self, sort: Sort) -> Self {
        self.sorts.push(sort);
        self
    }

    // ===================================================================
    // 列表查询（不含引用解析）
    // ===================================================================

    /// 执行分页列表查询（不含引用解析）
    pub async fn fetch(
        mut self,
        page: i64,
        page_size: i64,
    ) -> Result<PaginatedResponse<E>, AliothError> {
        let items = self.fetch_items(page, page_size).await?;
        let total = self.fetch_count().await?;
        Ok(PaginatedResponse::new(items, total, page, page_size))
    }

    async fn fetch_items(&mut self, page: i64, page_size: i64) -> Result<Vec<E>, AliothError> {
        let mut fields = E::SELECT_FIELDS.to_string();
        fields.push_str(&build_sensitive_suffix::<E>(
            self.authorized_columns.as_deref(),
            "e.",
        ));
        let mut sql = format!("SELECT {} FROM {} AS e", fields, E::table_name());
        let mut param_idx = 1usize;
        let (where_sql, _) = self.build_where_sql(&mut param_idx).await;
        sql.push_str(&where_sql);
        sql.push_str(&self.build_order_sql());
        sql.push_str(&format!(" LIMIT ${} OFFSET ${}", param_idx, param_idx + 1));

        let mut q = sqlx::query_as::<Postgres, E>(AssertSqlSafe(sql.as_str()));
        for dk in &self.dk_binds {
            q = q.bind(*dk);
        }
        for filter in &self.filters {
            if filter.validate().is_ok() {
                q = q.bind(&filter.value);
            }
        }
        if let Some(ref ids) = self.visible_ids {
            if !ids.is_empty() {
                q = q.bind(ids.as_slice());
            }
        }
        q = q.bind(page_size).bind((page - 1) * page_size);

        Ok(q.fetch_all(self.pool).await?)
    }

    async fn fetch_count(&mut self) -> Result<i64, AliothError> {
        // items 查询已填充 dk_binds——count 复用同一实例需重置（参数序从 $1 重新分配）
        self.dk_binds.clear();
        let mut sql = format!("SELECT COUNT(*) FROM {}", E::table_name());
        let mut param_idx = 1usize;
        let (where_sql, _) = self.build_where_sql(&mut param_idx).await;
        sql.push_str(&where_sql);

        let mut q = sqlx::query_as::<_, (i64,)>(AssertSqlSafe(sql.as_str()));
        for dk in &self.dk_binds {
            q = q.bind(*dk);
        }
        for filter in &self.filters {
            if filter.validate().is_ok() {
                q = q.bind(&filter.value);
            }
        }
        if let Some(ref ids) = self.visible_ids {
            if !ids.is_empty() {
                q = q.bind(ids.as_slice());
            }
        }
        let (count,) = q.fetch_one(self.pool).await?;
        Ok(count)
    }

    // ===================================================================
    // 单条查询（不含引用解析）
    // ===================================================================

    /// 根据 ID 查询单条记录（不含引用解析）
    pub async fn get(
        pool: &PgPool,
        id: i64,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<Option<E>, AliothError> {
        let deleted_cond = if E::SOFT_DELETE {
            " AND deleted_at IS NULL"
        } else {
            ""
        };
        let coord_filter = if !E::COORDINATE_FILTER.is_empty() {
            format!(" AND {}", E::COORDINATE_FILTER)
        } else {
            String::new()
        };
        let rls_clause = if let Some(ids) = visible_ids {
            if !ids.is_empty() {
                " AND e.id = ANY($2::bigint[])".to_string()
            } else {
                // Some(空集) = 显式无授权 → 恒假谓词（fail-closed，与列表路径一致）
                " AND false".to_string()
            }
        } else {
            String::new()
        };
        let mut fields = E::SELECT_FIELDS.to_string();
        fields.push_str(&build_sensitive_suffix::<E>(authorized_columns, "e."));
        let sql = format!(
            "SELECT {} FROM {} AS e WHERE e.id = ${}{}{}{}",
            fields,
            E::table_name(),
            1,
            deleted_cond,
            coord_filter,
            rls_clause
        );
        let mut q = sqlx::query_as::<Postgres, E>(AssertSqlSafe(sql.as_str())).bind(id);
        if let Some(ids) = visible_ids {
            if !ids.is_empty() {
                q = q.bind(ids);
            }
        }
        Ok(q.fetch_optional(pool).await?)
    }

    // ===================================================================
    // 删除
    // ===================================================================

    /// 软删除单条记录（默认级联配置：关系表 + 子实体级联，业务引用不级联）。
    ///
    /// 主实体与全部级联目标在同一数据库事务内完成；任一 UPDATE 失败整体回滚。
    /// 当 `E::HAS_AUDIT` 为 `true` 时，自动注入 `updated_by_id`。
    pub async fn soft_delete(pool: &PgPool, id: i64, user_id: i64) -> Result<u64, AliothError> {
        Self::soft_delete_with_cascade(pool, id, user_id, CascadeConfig::default()).await
    }

    /// 软删除单条记录（显式级联配置，见 [`crate::cascade::CascadeConfig`]）。
    ///
    /// 级联目标按 fk_index 注册拓扑推导（`lookup_reverse_fk`），同一事务多表
    /// UPDATE `deleted_at`；任一目标失败整体回滚（REQ-NFR-005 级联原子性）。
    pub async fn soft_delete_with_cascade(
        pool: &PgPool,
        id: i64,
        user_id: i64,
        cascade: CascadeConfig,
    ) -> Result<u64, AliothError> {
        let table = E::table_name();
        let bare_table = crate::cascade::bare_table_name(table);

        let mut tx = pool.begin().await.map_err(AliothError::from)?;
        let outcome = async {
            let deleted_cond = if E::SOFT_DELETE {
                " AND deleted_at IS NULL"
            } else {
                ""
            };

            let sql = if E::HAS_AUDIT {
                format!(
                    "UPDATE {} SET deleted_at = NOW(), updated_at = NOW(), updated_by_id = ${} WHERE id = ${}{}",
                    table,
                    2,
                    1,
                    deleted_cond
                )
            } else {
                format!(
                    "UPDATE {} SET deleted_at = NOW(), updated_at = NOW() WHERE id = ${}{}",
                    table,
                    1,
                    deleted_cond
                )
            };

            let mut q = sqlx::query(AssertSqlSafe(sql.as_str())).bind(id);
            if E::HAS_AUDIT {
                q = q.bind(user_id);
            }

            let result = q.execute(&mut *tx).await?;
            let rows = result.rows_affected();
            if rows > 0 {
                crate::cascade::cascade_soft_delete(&mut tx, bare_table, &[id], &cascade).await?;
            }
            Ok::<u64, sqlx::Error>(rows)
        }
        .await;

        match outcome {
            Ok(rows) => {
                tx.commit().await.map_err(AliothError::from)?;
                Ok(rows)
            }
            Err(e) => {
                tx.rollback().await.map_err(AliothError::from)?;
                Err(e.into())
            }
        }
    }

    /// 批量软删除（默认级联配置：关系表 + 子实体级联，业务引用不级联）。
    ///
    /// 主实体与全部级联目标在同一数据库事务内完成；任一 UPDATE 失败整体回滚。
    pub async fn batch_soft_delete(
        pool: &PgPool,
        ids: &[i64],
        user_id: i64,
    ) -> Result<u64, AliothError> {
        Self::batch_soft_delete_with_cascade(pool, ids, user_id, CascadeConfig::default()).await
    }

    /// 批量软删除（显式级联配置，见 [`crate::cascade::CascadeConfig`]）。
    ///
    /// 级联目标按 fk_index 注册拓扑推导（`lookup_reverse_fk`），同一事务多表
    /// UPDATE `deleted_at`；任一目标失败整体回滚（REQ-NFR-005 级联原子性）。
    pub async fn batch_soft_delete_with_cascade(
        pool: &PgPool,
        ids: &[i64],
        user_id: i64,
        cascade: CascadeConfig,
    ) -> Result<u64, AliothError> {
        if ids.is_empty() {
            return Ok(0);
        }

        let table = E::table_name();
        let bare_table = crate::cascade::bare_table_name(table);
        let ids_owned = ids.to_vec();

        let mut tx = pool.begin().await.map_err(AliothError::from)?;
        let outcome = async {
            let placeholders: Vec<String> =
                (1..=ids_owned.len()).map(|i| format!("${}", i)).collect();
            let deleted_cond = if E::SOFT_DELETE {
                " AND deleted_at IS NULL"
            } else {
                ""
            };

            let sql = if E::HAS_AUDIT {
                let user_idx = ids_owned.len() + 1;
                format!(
                    "UPDATE {} SET deleted_at = NOW(), updated_at = NOW(), updated_by_id = ${} WHERE id IN ({}){}",
                    table,
                    user_idx,
                    placeholders.join(", "),
                    deleted_cond
                )
            } else {
                format!(
                    "UPDATE {} SET deleted_at = NOW(), updated_at = NOW() WHERE id IN ({}){}",
                    table,
                    placeholders.join(", "),
                    deleted_cond
                )
            };

            let mut q = sqlx::query(AssertSqlSafe(sql.as_str()));
            for id in &ids_owned {
                q = q.bind(*id);
            }
            if E::HAS_AUDIT {
                q = q.bind(user_id);
            }

            let result = q.execute(&mut *tx).await?;
            let rows = result.rows_affected();
            if rows > 0 {
                crate::cascade::cascade_soft_delete(&mut tx, bare_table, &ids_owned, &cascade)
                    .await?;
            }
            Ok::<u64, sqlx::Error>(rows)
        }
        .await;

        match outcome {
            Ok(rows) => {
                tx.commit().await.map_err(AliothError::from)?;
                Ok(rows)
            }
            Err(e) => {
                tx.rollback().await.map_err(AliothError::from)?;
                Err(e.into())
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 关键词搜索——仅当 E: KeywordSearchable 时可用
// ═══════════════════════════════════════════════════════════════════════════════

impl<'a, E: AliothDbEntity + crate::search::KeywordSearchable> QueryBuilder<'a, E> {
    /// 添加关键词搜索条件（通过 raw ILIKE，列名由 KeywordSearchable 声明）
    pub fn with_keyword(mut self, keyword: &str) -> Self {
        let escaped = keyword
            .replace('\'', "''")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{}%", escaped);
        let conditions: Vec<String> = E::SEARCH_COLUMNS
            .iter()
            .map(|col| format!("{}::text ILIKE '{}'", col, pattern))
            .collect();
        if !conditions.is_empty() {
            self.raw_filters
                .push(format!("({})", conditions.join(" OR ")));
        }
        self
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 引用解析方法——仅当 E: HasReferenceJoins 时可用
// ═══════════════════════════════════════════════════════════════════════════════

impl<'a, E: AliothDbEntity + HasReferenceJoins> QueryBuilder<'a, E> {
    /// 分页列表查询（含引用解析，输出 `_refs` 嵌入 JSONB）
    pub async fn fetch_refs(
        mut self,
        page: i64,
        page_size: i64,
    ) -> Result<PaginatedResponse<E>, AliothError> {
        let items = self.fetch_items_refs(page, page_size).await?;
        let total = self.fetch_count().await?;
        Ok(PaginatedResponse::new(items, total, page, page_size))
    }

    async fn fetch_items_refs(&mut self, page: i64, page_size: i64) -> Result<Vec<E>, AliothError> {
        let mut fields = E::SELECT_FIELDS.to_string();
        fields.push_str(&build_sensitive_suffix::<E>(
            self.authorized_columns.as_deref(),
            "e.",
        ));
        fields.push_str(&build_refs_select_suffix::<E>());

        let mut sql = format!("SELECT {} FROM {} AS e", fields, E::table_name());
        let mut param_idx = 1usize;
        let (where_sql, _) = self.build_where_sql(&mut param_idx).await;
        sql.push_str(&where_sql);
        sql.push_str(&self.build_order_sql());
        sql.push_str(&format!(" LIMIT ${} OFFSET ${}", param_idx, param_idx + 1));

        let mut q = sqlx::query_as::<Postgres, E>(AssertSqlSafe(sql.as_str()));
        for dk in &self.dk_binds {
            q = q.bind(*dk);
        }
        for filter in &self.filters {
            if filter.validate().is_ok() {
                q = q.bind(&filter.value);
            }
        }
        if let Some(ref ids) = self.visible_ids {
            if !ids.is_empty() {
                q = q.bind(ids.as_slice());
            }
        }
        q = q.bind(page_size).bind((page - 1) * page_size);

        Ok(q.fetch_all(self.pool).await?)
    }

    /// 根据 ID 查询单条记录（含引用解析）
    pub async fn get_refs(
        pool: &PgPool,
        id: i64,
        authorized_columns: Option<&[String]>,
    ) -> Result<Option<E>, AliothError> {
        let mut fields = E::SELECT_FIELDS.to_string();
        fields.push_str(&build_sensitive_suffix::<E>(authorized_columns, "e."));
        fields.push_str(&build_refs_select_suffix::<E>());

        let deleted_cond = if E::SOFT_DELETE {
            " AND deleted_at IS NULL"
        } else {
            ""
        };
        let coord_filter = if !E::COORDINATE_FILTER.is_empty() {
            format!(" AND {}", E::COORDINATE_FILTER)
        } else {
            String::new()
        };
        let sql = format!(
            "SELECT {} FROM {} AS e WHERE e.id = ${}{}{}",
            fields,
            E::table_name(),
            1,
            deleted_cond,
            coord_filter
        );
        Ok(sqlx::query_as::<Postgres, E>(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_optional(pool)
            .await?)
    }
}
