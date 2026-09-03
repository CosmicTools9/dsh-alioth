//! Alioth 标量引用转换服务
//!
//! 提供 `qk_*` 标量引用 ID 与实际值之间的查找/创建/转换能力。
//!
//! ## 设计原则
//!
//! - 数据层存储标量引用 ID（`bigint`），不存储实际值
//! - DTO 可接收实际值（`Decimal`/`String`），由服务层通过 `ScalarService` 转换为标量 ID
//! - 标量实体采用「查找存在则复用，不存在则创建」的 UPSERT 策略
//!
//! ## 标量表映射
//!
//! | 业务含义 | 标量表 | 实际值字段 | 类型 |
//! |---------|--------|-----------|------|
//! | 日期 | `zc_id_scal-date` | `date` | `timestamptz` |
//! | 金额 | `zc_id_scal-amount` | `mark` | `numeric(30,10)` |
//! | 价格 | `zc_id_scal-price` | `mark` | `numeric(30,10)` |
//! | 通用数量 | `zc_id_scal-common` | `mark` | `numeric(30,10)` |
//! | 其他刻度 | `zc_id_scale` | `mark` | `numeric(30,10)` |

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use rust_decimal::Decimal;
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};

use crate::error::AliothError;

// ---------------------------------------------------------------------------
// 标量值对象类型 — DTO 层统一使用，由 ScalarService 转换为标量引用 ID
// 所有模块从此处导入，禁止本地重复定义
// ---------------------------------------------------------------------------

/// 标量日期值对象（前端传 "YYYY-MM-DD" 字符串）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScalarDateValue {
    pub value: String,
}

/// 标量起止日期段值对象（前端传 { valueSt, valueEd }）。
/// 对应 `zc_id_segm-date`（date_st/date_ed 起止窗口，如保质期/寿命段）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScalarSegmDateValue {
    pub value_st: String,
    pub value_ed: String,
}

/// 标量通用数值对象（前端传 { value: 123.45 }）。
/// 对应 `zc_id_scal-common`，用于数量、容量等通用数值标量。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScalarCommonValue {
    pub value: Decimal,
}

/// `ScalarQtyValue` 是 `ScalarCommonValue` 的别名，语义等价。
pub type ScalarQtyValue = ScalarCommonValue;

/// 标量价格值对象（前端传 { value: 999.99 }）。
/// 对应 `zc_id_scal-price`。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScalarPriceValue {
    pub value: Decimal,
}

/// 标量金额值对象（前端传 { value: 999.99 }）。
/// 对应 `zc_id_scal-amount`。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScalarAmountValue {
    pub value: Decimal,
}
/// 标量转换服务
///
/// 持有一个数据库连接池，提供标量查找/创建/查询方法。
#[derive(Clone)]
pub struct ScalarService {
    pool: PgPool,
}

impl ScalarService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // ------------------------------------------------------------------
    // 查找或创建：日期标量
    // ------------------------------------------------------------------

    /// 根据日期文本查找或创建 `zc_id_scal-date` 记录，返回标量 ID。
    ///
    /// `date_text` 格式应为 `YYYY-MM-DD`，内部解析为 `timestamptz`（当天 00:00:00 UTC）。
    pub async fn find_or_create_date(&self, date_text: &str) -> Result<i64, AliothError> {
        let naive = NaiveDate::parse_from_str(date_text, "%Y-%m-%d").map_err(|e| {
            AliothError::BadRequest(format!("Invalid date format '{}': {}", date_text, e))
        })?;
        let date =
            DateTime::<Utc>::from_naive_utc_and_offset(naive.and_hms_opt(0, 0, 0).unwrap(), Utc);

        // 先尝试查找已存在的记录
        if let Some(id) = self.find_date_id(date).await? {
            return Ok(id);
        }

        // 不存在则创建
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_scal-date" (notice, date, created_by_id)
               VALUES ($1, $2, 1)
               RETURNING id"#,
        )
        .bind(date_text)
        .bind(date)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AliothError::Database(format!("Failed to create scalar date: {}", e)))?;

        Ok(id)
    }

    /// 根据 `DateTime<Utc>` 查找日期标量 ID。
    async fn find_date_id(&self, date: DateTime<Utc>) -> Result<Option<i64>, AliothError> {
        let id: Option<i64> =
            sqlx::query_scalar(r#"SELECT id FROM isahl."zc_id_scal-date" WHERE date = $1 LIMIT 1"#)
                .bind(date)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| AliothError::Database(format!("Failed to find scalar date: {}", e)))?;
        Ok(id)
    }

    /// 通过日期标量 ID 查询实际日期值。
    pub async fn get_date(&self, scale_id: i64) -> Result<Option<DateTime<Utc>>, AliothError> {
        let date: Option<DateTime<Utc>> =
            sqlx::query_scalar(r#"SELECT date FROM isahl."zc_id_scal-date" WHERE id = $1"#)
                .bind(scale_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| AliothError::Database(format!("Failed to get scalar date: {}", e)))?;
        Ok(date)
    }

    // ------------------------------------------------------------------
    // ref_count 感知：日期标量引用更新（COW 语义）
    // ------------------------------------------------------------------

    /// 读取 `zc_id_scal-date` 行的引用计数（ref_count 由 DB 触发器维护，见 ONTOLOGY_SPEC）。
    /// 行不存在（如已软删）或 ref_count 为 NULL 时按 0（独占）处理。
    async fn date_ref_count(&self, id: i64) -> Result<i64, AliothError> {
        let rc: Option<i64> = sqlx::query_scalar(
            r#"SELECT COALESCE(ref_count, 0) FROM isahl."zc_id_scal-date" WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            AliothError::Database(format!("Failed to read scalar date ref_count: {}", e))
        })?;
        Ok(rc.unwrap_or(0))
    }

    /// 读取 `zc_id_segm-date` 行的引用计数（ref_count 由 DB 触发器维护，见 ONTOLOGY_SPEC）。
    /// 行不存在（如已软删）或 ref_count 为 NULL 时按 0（独占）处理。
    async fn segm_date_ref_count(&self, id: i64) -> Result<i64, AliothError> {
        let rc: Option<i64> = sqlx::query_scalar(
            r#"SELECT COALESCE(ref_count, 0) FROM isahl."zc_id_segm-date" WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            AliothError::Database(format!("Failed to read scalar segment ref_count: {}", e))
        })?;
        Ok(rc.unwrap_or(0))
    }

    /// 更新业务实体对日期标量 `zc_id_scal-date` 的引用（ref_count 感知，COW 语义）。
    ///
    /// 日期/时间是**客观唯一计量尺**：同 `date` 值的所有引用必须指向同一
    /// `zc_id_scal-date.id`（与金额/数量等 o2o 隔离标量不同）。因此判定顺序：
    /// 1. **先全局复用**——新值已有行（含当前行自身，即无变化）→ 直接返回该行 ID，
    ///    保证「同日期同 id」，绝不产生同值双行；
    /// 2. 当前引用的标量行 `ref_count < 2`（独占，仅当前业务实体引用）→ **原位修订**
    ///    `date` / `"time"` 两列，返回原 ID，业务实体外键保持不变；
    /// 3. 否则（`ref_count >= 2`，被多个业务实体共享）→ **重新 INSERT** 新行，
    ///    返回新 ID，调用方应将业务实体外键改指新行。
    ///
    /// **ref_count 自维护**（方案 1，不依赖 DB 触发器）：发生换绑（返回值 != `current_id`）时
    /// 本方法自行 `旧行 -1 / 新行 +1`；原位修订（引用关系不变）不维护。仅当目标行
    /// `ref_count` 已被维护（非 NULL）时调整生效，未启用维护的环境保持 NULL、行为不变。
    /// 业务实体 create 引用 +1 / delete -1 由 trigger-registry 在业务 CRUD 时统一维护。
    ///
    /// `current_id` 为业务实体当前的外键值（未引用任何标量时传 `None`）。
    /// `date_text` 支持 `YYYY-MM-DD` 或 `YYYY-MM-DD HH:MM[:SS]`。
    pub async fn update_date_ref(
        &self,
        current_id: Option<i64>,
        date_text: &str,
    ) -> Result<i64, AliothError> {
        let (date, time) = parse_date_time(date_text)?;

        // 客观唯一：新值已有行 → 复用（含当前行，即 no-op）
        if let Some(existing_id) = self.find_date_id(date).await? {
            // 换绑到另一行：旧引用 -1、新引用 +1
            if let Some(cur) = current_id {
                if cur != existing_id {
                    self.adjust_date_ref_count(cur, -1).await?;
                    self.adjust_date_ref_count(existing_id, 1).await?;
                }
            }
            return Ok(existing_id);
        }

        // 独占（ref_count < 2）→ 原位修订 date / "time"，外键 ID 不变
        if let Some(current_id) = current_id {
            if self.date_ref_count(current_id).await? < 2 {
                sqlx::query(
                    r#"UPDATE isahl."zc_id_scal-date"
                       SET date = $1, "time" = COALESCE($2, "time"), updated_at = NOW()
                       WHERE id = $3 AND deleted_at IS NULL"#,
                )
                .bind(date)
                .bind(time)
                .bind(current_id)
                .execute(&self.pool)
                .await
                .map_err(|e| {
                    AliothError::Database(format!("Failed to update scalar date in place: {}", e))
                })?;
                return Ok(current_id);
            }
        }

        // 共享（ref_count >= 2）或无旧引用 → 重新 INSERT 新行
        let new_id = self.insert_date_row(date, time, date_text).await?;
        // 共享换绑：旧引用 -1、新引用 +1（无旧引用场景由业务 create 路径维护）
        if let Some(current_id) = current_id {
            self.adjust_date_ref_count(current_id, -1).await?;
            self.adjust_date_ref_count(new_id, 1).await?;
        }
        Ok(new_id)
    }

    /// 调整 `zc_id_scal-date` 行引用计数（±1）。
    ///
    /// 仅当目标行 `ref_count` 已被维护（非 NULL）时生效——未启用 ref_count 维护的环境
    /// 保持 NULL 不变，避免产生半吊子计数；`GREATEST(0, …)` 防止 -1 溢出为负。
    async fn adjust_date_ref_count(&self, id: i64, delta: i64) -> Result<(), AliothError> {
        sqlx::query(
            r#"UPDATE isahl."zc_id_scal-date"
               SET ref_count = GREATEST(0, ref_count + $1), updated_at = NOW()
               WHERE id = $2 AND ref_count IS NOT NULL"#,
        )
        .bind(delta)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AliothError::Database(format!("Failed to adjust scalar date ref_count: {}", e))
        })?;
        Ok(())
    }

    /// 插入新的 `zc_id_scal-date` 行（含 `"time"` 列写入）。调用方须已确认无同值行。
    async fn insert_date_row(
        &self,
        date: DateTime<Utc>,
        time: Option<NaiveTime>,
        notice: &str,
    ) -> Result<i64, AliothError> {
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_scal-date" (notice, date, "time", created_by_id)
               VALUES ($1, $2, $3, 1) RETURNING id"#,
        )
        .bind(notice)
        .bind(date)
        .bind(time)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AliothError::Database(format!("Failed to create scalar date: {}", e)))?;
        Ok(id)
    }

    /// 根据起止日期文本查找或创建 `zc_id_segm-date` 记录，返回标量 ID。
    ///
    /// `date_st_text` / `date_ed_text` 支持 `YYYY-MM-DD` 或 `YYYY-MM-DD HH:MM[:SS]`。
    /// 按 `(date_st, date_ed)` 匹配复用；不存在则创建新行（写入 `date_st` / `date_ed` /
    /// `time_st` / `time_ed`）。
    pub async fn find_or_create_segm_date(
        &self,
        date_st_text: &str,
        date_ed_text: &str,
    ) -> Result<i64, AliothError> {
        let (date_st, time_st) = parse_date_time(date_st_text)?;
        let (date_ed, time_ed) = parse_date_time(date_ed_text)?;
        if let Some(id) = self.find_segm_date_id(date_st, date_ed).await? {
            return Ok(id);
        }
        self.insert_segm_date_row(
            date_st,
            date_ed,
            time_st,
            time_ed,
            date_st_text,
            date_ed_text,
        )
        .await
    }

    /// 更新业务实体对时段标量 `zc_id_segm-date` 的引用（ref_count 感知，COW 语义）。
    ///
    /// 判定规则同 [`Self::update_date_ref`]（时段按 `(date_st, date_ed)` 客观唯一）：
    /// 1. 新值已有行（含当前行自身）→ 复用，保证同起止日期同 id；
    /// 2. `ref_count < 2` → **原位修订** `date_st` / `date_ed` / `time_st` / `time_ed`，
    ///    返回原 ID；
    /// 3. `ref_count >= 2` 或无旧引用 → **重新 INSERT** 新行，返回新 ID。
    ///
    /// **ref_count 自维护**：换绑（返回值 != `current_id`）时 `旧行 -1 / 新行 +1`，
    /// 原位修订不维护；仅目标行 `ref_count` 非 NULL 时生效（见 [`Self::adjust_date_ref_count`]）。
    pub async fn update_segm_date_ref(
        &self,
        current_id: Option<i64>,
        date_st_text: &str,
        date_ed_text: &str,
    ) -> Result<i64, AliothError> {
        let (date_st, time_st) = parse_date_time(date_st_text)?;
        let (date_ed, time_ed) = parse_date_time(date_ed_text)?;

        // 客观唯一：新值已有行 → 复用（含当前行，即 no-op）
        if let Some(existing_id) = self.find_segm_date_id(date_st, date_ed).await? {
            // 换绑到另一行：旧引用 -1、新引用 +1
            if let Some(cur) = current_id {
                if cur != existing_id {
                    self.adjust_segm_date_ref_count(cur, -1).await?;
                    self.adjust_segm_date_ref_count(existing_id, 1).await?;
                }
            }
            return Ok(existing_id);
        }

        // 独占（ref_count < 2）→ 原位修订四个字段，外键 ID 不变
        if let Some(current_id) = current_id {
            if self.segm_date_ref_count(current_id).await? < 2 {
                sqlx::query(
                    r#"UPDATE isahl."zc_id_segm-date"
                       SET date_st = $1, date_ed = $2,
                           time_st = COALESCE($3, time_st), time_ed = COALESCE($4, time_ed),
                           updated_at = NOW()
                       WHERE id = $5 AND deleted_at IS NULL"#,
                )
                .bind(date_st)
                .bind(date_ed)
                .bind(time_st)
                .bind(time_ed)
                .bind(current_id)
                .execute(&self.pool)
                .await
                .map_err(|e| {
                    AliothError::Database(format!(
                        "Failed to update scalar segment in place: {}",
                        e
                    ))
                })?;
                return Ok(current_id);
            }
        }

        // 共享（ref_count >= 2）或无旧引用 → 重新 INSERT 新行
        let new_id = self
            .insert_segm_date_row(
                date_st,
                date_ed,
                time_st,
                time_ed,
                date_st_text,
                date_ed_text,
            )
            .await?;
        // 共享换绑：旧引用 -1、新引用 +1（无旧引用场景由业务 create 路径维护）
        if let Some(current_id) = current_id {
            self.adjust_segm_date_ref_count(current_id, -1).await?;
            self.adjust_segm_date_ref_count(new_id, 1).await?;
        }
        Ok(new_id)
    }

    /// 调整 `zc_id_segm-date` 行引用计数（±1）。
    ///
    /// 仅当目标行 `ref_count` 已被维护（非 NULL）时生效；`GREATEST(0, …)` 防负。
    async fn adjust_segm_date_ref_count(&self, id: i64, delta: i64) -> Result<(), AliothError> {
        sqlx::query(
            r#"UPDATE isahl."zc_id_segm-date"
               SET ref_count = GREATEST(0, ref_count + $1), updated_at = NOW()
               WHERE id = $2 AND ref_count IS NOT NULL"#,
        )
        .bind(delta)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AliothError::Database(format!("Failed to adjust scalar segment ref_count: {}", e))
        })?;
        Ok(())
    }

    /// 插入新的 `zc_id_segm-date` 行。调用方须已确认无同值行。
    #[allow(clippy::too_many_arguments)]
    async fn insert_segm_date_row(
        &self,
        date_st: DateTime<Utc>,
        date_ed: DateTime<Utc>,
        time_st: Option<NaiveTime>,
        time_ed: Option<NaiveTime>,
        date_st_text: &str,
        date_ed_text: &str,
    ) -> Result<i64, AliothError> {
        let notice = format!("{} ~ {}", date_st_text, date_ed_text);
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_segm-date"
               (notice, date_st, date_ed, time_st, time_ed, created_by_id)
               VALUES ($1, $2, $3, $4, $5, 1) RETURNING id"#,
        )
        .bind(&notice)
        .bind(date_st)
        .bind(date_ed)
        .bind(time_st)
        .bind(time_ed)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            AliothError::Database(format!("Failed to create scalar segment date: {}", e))
        })?;
        Ok(id)
    }

    /// 根据 `(date_st, date_ed)` 查找时段标量 ID。
    async fn find_segm_date_id(
        &self,
        date_st: DateTime<Utc>,
        date_ed: DateTime<Utc>,
    ) -> Result<Option<i64>, AliothError> {
        let id: Option<i64> = sqlx::query_scalar(
            r#"SELECT id FROM isahl."zc_id_segm-date"
               WHERE date_st = $1 AND date_ed = $2 LIMIT 1"#,
        )
        .bind(date_st)
        .bind(date_ed)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AliothError::Database(format!("Failed to find scalar segment: {}", e)))?;
        Ok(id)
    }

    // ------------------------------------------------------------------
    // 查找或创建：金额标量
    // ------------------------------------------------------------------

    /// 根据金额值查找或创建 `zc_id_scal-amount` 记录，返回标量 ID。
    pub async fn find_or_create_amount(&self, mark: Decimal) -> Result<i64, AliothError> {
        self.find_or_create_scalar_mark(mark, r#"isahl."zc_id_scal-amount""#, "amount")
            .await
    }

    /// 通过金额标量 ID 查询实际金额值。
    pub async fn get_amount(&self, scale_id: i64) -> Result<Option<Decimal>, AliothError> {
        self.get_mark(scale_id, r#"isahl."zc_id_scal-amount""#)
            .await
    }

    // ------------------------------------------------------------------
    // 查找或创建：价格标量
    // ------------------------------------------------------------------

    /// 根据价格值查找或创建 `zc_id_scal-price` 记录，返回标量 ID。
    pub async fn find_or_create_price(&self, mark: Decimal) -> Result<i64, AliothError> {
        self.find_or_create_scalar_mark(mark, r#"isahl."zc_id_scal-price""#, "price")
            .await
    }

    /// 通过价格标量 ID 查询实际价格值。
    pub async fn get_price(&self, scale_id: i64) -> Result<Option<Decimal>, AliothError> {
        self.get_mark(scale_id, r#"isahl."zc_id_scal-price""#).await
    }

    // ------------------------------------------------------------------
    // 查找或创建：通用数量标量
    // ------------------------------------------------------------------

    /// 根据通用数值查找或创建 `zc_id_scal-common` 记录，返回标量 ID。
    pub async fn find_or_create_common(&self, mark: Decimal) -> Result<i64, AliothError> {
        self.find_or_create_scalar_mark(mark, r#"isahl."zc_id_scal-common""#, "common")
            .await
    }

    /// 通过通用标量 ID 查询实际数值。
    pub async fn get_common(&self, scale_id: i64) -> Result<Option<Decimal>, AliothError> {
        self.get_mark(scale_id, r#"isahl."zc_id_scal-common""#)
            .await
    }

    // ------------------------------------------------------------------
    // 通用标量操作（基于 mark）
    // ------------------------------------------------------------------

    /// 在任意标量表中根据 `mark` 查找或创建记录。
    ///
    /// `table` 应为完全限定表名（如 `isahl."zc_id_scal-amount"`）。
    /// `notice_prefix` 用于生成默认 notice（如 `"amount: 100.50"`）。
    pub async fn find_or_create_scalar_mark(
        &self,
        mark: Decimal,
        table: &str,
        notice_prefix: &str,
    ) -> Result<i64, AliothError> {
        // 先查找
        if let Some(id) = self.find_mark_id(mark, table).await? {
            return Ok(id);
        }

        // 不存在则创建（使用动态 SQL，表名已做基本校验）
        let notice = format!("{}: {}", notice_prefix, mark);
        let sql = format!(
            r#"INSERT INTO {} (notice, mark, created_by_id) VALUES ($1, $2, 1) RETURNING id"#,
            table
        );
        let id: i64 = sqlx::query_scalar(AssertSqlSafe(sql.as_str()))
            .bind(&notice)
            .bind(mark)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                AliothError::Database(format!("Failed to create scalar in {}: {}", table, e))
            })?;

        Ok(id)
    }

    /// 在指定标量表中根据 `mark` 查找 ID。
    async fn find_mark_id(&self, mark: Decimal, table: &str) -> Result<Option<i64>, AliothError> {
        let sql = format!(r#"SELECT id FROM {} WHERE mark = $1 LIMIT 1"#, table);
        let id: Option<i64> = sqlx::query_scalar(AssertSqlSafe(sql.as_str()))
            .bind(mark)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                AliothError::Database(format!("Failed to find scalar in {}: {}", table, e))
            })?;
        Ok(id)
    }

    /// 通过标量 ID 在任意标量表中查询 `mark` 值。
    pub async fn get_mark(
        &self,
        scale_id: i64,
        table: &str,
    ) -> Result<Option<Decimal>, AliothError> {
        let sql = format!(r#"SELECT mark FROM {} WHERE id = $1"#, table);
        let mark: Option<Decimal> = sqlx::query_scalar(AssertSqlSafe(sql.as_str()))
            .bind(scale_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                AliothError::Database(format!("Failed to get scalar mark from {}: {}", table, e))
            })?;
        Ok(mark)
    }

    // ------------------------------------------------------------------
    // 事务安全版本
    // ------------------------------------------------------------------

    /// 在事务内查找或创建日期标量。
    pub async fn find_or_create_date_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        date_text: &str,
    ) -> Result<i64, AliothError> {
        let naive = NaiveDate::parse_from_str(date_text, "%Y-%m-%d").map_err(|e| {
            AliothError::BadRequest(format!("Invalid date format '{}': {}", date_text, e))
        })?;
        let date =
            DateTime::<Utc>::from_naive_utc_and_offset(naive.and_hms_opt(0, 0, 0).unwrap(), Utc);

        let id: Option<i64> =
            sqlx::query_scalar(r#"SELECT id FROM isahl."zc_id_scal-date" WHERE date = $1 LIMIT 1"#)
                .bind(date)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| AliothError::Database(format!("Failed to find scalar date: {}", e)))?;

        if let Some(id) = id {
            return Ok(id);
        }

        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_scal-date" (notice, date, created_by_id)
               VALUES ($1, $2, 1) RETURNING id"#,
        )
        .bind(date_text)
        .bind(date)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| AliothError::Database(format!("Failed to create scalar date: {}", e)))?;

        Ok(id)
    }

    /// 在事务内查找或创建 mark 标量。
    pub async fn find_or_create_mark_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        mark: Decimal,
        table: &str,
        notice_prefix: &str,
    ) -> Result<i64, AliothError> {
        let find_sql = format!(r#"SELECT id FROM {} WHERE mark = $1 LIMIT 1"#, table);
        let id: Option<i64> = sqlx::query_scalar(AssertSqlSafe(find_sql.as_str()))
            .bind(mark)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| {
                AliothError::Database(format!("Failed to find scalar in {}: {}", table, e))
            })?;

        if let Some(id) = id {
            return Ok(id);
        }

        let notice = format!("{}: {}", notice_prefix, mark);
        let insert_sql = format!(
            r#"INSERT INTO {} (notice, mark, created_by_id) VALUES ($1, $2, 1) RETURNING id"#,
            table
        );
        let id: i64 = sqlx::query_scalar(AssertSqlSafe(insert_sql.as_str()))
            .bind(&notice)
            .bind(mark)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| {
                AliothError::Database(format!("Failed to create scalar in {}: {}", table, e))
            })?;

        Ok(id)
    }

    /// 在事务内查找或创建金额标量。
    pub async fn find_or_create_amount_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        mark: Decimal,
    ) -> Result<i64, AliothError> {
        self.find_or_create_mark_tx(tx, mark, r#"isahl."zc_id_scal-amount""#, "amount")
            .await
    }

    /// 在事务内查找或创建价格标量。
    pub async fn find_or_create_price_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        mark: Decimal,
    ) -> Result<i64, AliothError> {
        self.find_or_create_mark_tx(tx, mark, r#"isahl."zc_id_scal-price""#, "price")
            .await
    }

    /// 在事务内查找或创建通用数量标量。
    pub async fn find_or_create_common_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        mark: Decimal,
    ) -> Result<i64, AliothError> {
        self.find_or_create_mark_tx(tx, mark, r#"isahl."zc_id_scal-common""#, "common")
            .await
    }
}

/// 解析日期文本，支持 `YYYY-MM-DD` 或 `YYYY-MM-DD HH:MM[:SS]`。
///
/// 返回 `(date, time)`：`date` 为当天 00:00:00 UTC 的 `timestamptz`（对应 `date` 列）；
/// `time` 为可选的时间部分（对应 `zc_id_scal-date."time"` / `zc_id_segm-date.time_*` 列）。
fn parse_date_time(text: &str) -> Result<(DateTime<Utc>, Option<NaiveTime>), AliothError> {
    let text = text.trim();
    let (date_part, time_part) = match text.split_once(' ') {
        Some((d, t)) => (d, Some(t)),
        None => (text, None),
    };
    let naive = NaiveDate::parse_from_str(date_part, "%Y-%m-%d")
        .map_err(|e| AliothError::BadRequest(format!("Invalid date format '{}': {}", text, e)))?;
    let date = DateTime::<Utc>::from_naive_utc_and_offset(naive.and_hms_opt(0, 0, 0).unwrap(), Utc);
    let time = match time_part {
        Some(t) => Some(
            NaiveTime::parse_from_str(t, "%H:%M:%S")
                .or_else(|_| NaiveTime::parse_from_str(t, "%H:%M"))
                .map_err(|e| {
                    AliothError::BadRequest(format!("Invalid time format '{}': {}", t, e))
                })?,
        ),
        None => None,
    };
    Ok((date, time))
}
