use sqlx::PgPool;

use crate::profile::types::{FieldProfile, ProfileTrendPoint, RunStatus};

pub struct ProfileRepository {
    db_pool: PgPool,
}

impl ProfileRepository {
    pub fn new(db_pool: PgPool) -> Self {
        Self { db_pool }
    }

    async fn generate_id(&self) -> Result<i64, sqlx::Error> {
        let id: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) + 1 FROM isahl_meta.field_profiles")
                .fetch_one(&self.db_pool)
                .await?;
        Ok(id)
    }

    /// 保存字段画像
    pub async fn save(&self, profile: &FieldProfile) -> Result<i64, sqlx::Error> {
        let id = self.generate_id().await?;
        let sql = r#"
            INSERT INTO isahl_meta.field_profiles (
                id, field_id, collection_id, run_at, row_count, null_count, 
                null_percentage, unique_count, cardinality_ratio, 
                statistics, top_values, histogram
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ON CONFLICT (id) DO UPDATE SET
                run_at = EXCLUDED.run_at,
                row_count = EXCLUDED.row_count,
                null_count = EXCLUDED.null_count,
                null_percentage = EXCLUDED.null_percentage,
                unique_count = EXCLUDED.unique_count,
                cardinality_ratio = EXCLUDED.cardinality_ratio,
                statistics = EXCLUDED.statistics,
                top_values = EXCLUDED.top_values,
                histogram = EXCLUDED.histogram,
                updated_at = NOW()
            RETURNING id
        "#;

        let result_id = sqlx::query_scalar::<_, i64>(sql)
            .bind(id)
            .bind(profile.field_id)
            .bind(profile.collection_id)
            .bind(profile.run_at)
            .bind(profile.row_count)
            .bind(profile.null_count)
            .bind(profile.null_percentage)
            .bind(profile.unique_count)
            .bind(profile.cardinality_ratio)
            .bind(serde_json::to_value(&profile.statistics).unwrap_or_default())
            .bind(serde_json::to_value(&profile.top_values).unwrap_or_default())
            .bind(
                profile
                    .histogram
                    .as_ref()
                    .map(|h| serde_json::to_value(h).unwrap_or_default()),
            )
            .fetch_one(&self.db_pool)
            .await?;

        Ok(result_id)
    }

    /// 获取字段最新画像
    pub async fn get_latest_by_field(
        &self,
        field_id: i64,
    ) -> Result<Option<FieldProfile>, sqlx::Error> {
        let sql = r#"
            SELECT 
                id, field_id, collection_id, run_at, row_count, null_count,
                null_percentage, unique_count, cardinality_ratio,
                statistics, top_values, histogram
            FROM isahl_meta.field_profiles
            WHERE field_id = $1
            ORDER BY run_at DESC
            LIMIT 1
        "#;

        let row = sqlx::query_as::<_, FieldProfileRow>(sql)
            .bind(field_id)
            .fetch_optional(&self.db_pool)
            .await?;

        Ok(row.map(|r| r.into()))
    }

    /// 获取字段画像历史
    pub async fn get_history_by_field(
        &self,
        field_id: i64,
        limit: i64,
    ) -> Result<Vec<FieldProfile>, sqlx::Error> {
        let sql = r#"
            SELECT 
                id, field_id, collection_id, run_at, row_count, null_count,
                null_percentage, unique_count, cardinality_ratio,
                statistics, top_values, histogram
            FROM isahl_meta.field_profiles
            WHERE field_id = $1
            ORDER BY run_at DESC
            LIMIT $2
        "#;

        let rows: Vec<FieldProfileRow> = sqlx::query_as(sql)
            .bind(field_id)
            .bind(limit)
            .fetch_all(&self.db_pool)
            .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// 获取集合所有字段的最新画像
    pub async fn get_by_collection(
        &self,
        collection_id: i64,
    ) -> Result<Vec<FieldProfile>, sqlx::Error> {
        let sql = r#"
            SELECT DISTINCT ON (field_id)
                id, field_id, collection_id, run_at, row_count, null_count,
                null_percentage, unique_count, cardinality_ratio,
                statistics, top_values, histogram
            FROM isahl_meta.field_profiles
            WHERE collection_id = $1
            ORDER BY field_id, run_at DESC
        "#;

        let rows: Vec<FieldProfileRow> = sqlx::query_as(sql)
            .bind(collection_id)
            .fetch_all(&self.db_pool)
            .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// 删除旧画像（保留最近 N 次）
    pub async fn cleanup_old_profiles(
        &self,
        field_id: i64,
        keep_count: i64,
    ) -> Result<u64, sqlx::Error> {
        let sql = r#"
            DELETE FROM isahl_meta.field_profiles
            WHERE field_id = $1
            AND id NOT IN (
                SELECT id FROM isahl_meta.field_profiles
                WHERE field_id = $1
                ORDER BY run_at DESC
                LIMIT $2
            )
        "#;

        let result = sqlx::query(sql)
            .bind(field_id)
            .bind(keep_count)
            .execute(&self.db_pool)
            .await?;

        Ok(result.rows_affected())
    }

    /// 记录集合画像运行开始
    pub async fn record_run_start(&self, collection_id: i64) -> Result<i64, sqlx::Error> {
        let id: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) + 1 FROM collection_profile_runs")
                .fetch_one(&self.db_pool)
                .await?;

        let sql = r#"
            INSERT INTO collection_profile_runs (id, collection_id, started_at, status, fields_count)
            VALUES ($1, $2, NOW(), 'RUNNING', 0)
            RETURNING id
        "#;

        let result_id: i64 = sqlx::query_scalar(sql)
            .bind(id)
            .bind(collection_id)
            .fetch_one(&self.db_pool)
            .await?;

        Ok(result_id)
    }

    /// 更新集合画像运行状态
    pub async fn update_run_status(
        &self,
        run_id: i64,
        status: RunStatus,
        fields_count: Option<i32>,
        error: Option<String>,
    ) -> Result<(), sqlx::Error> {
        let sql = r#"
            UPDATE collection_profile_runs
            SET status = $1, 
                completed_at = CASE WHEN $1 IN ('COMPLETED', 'FAILED') THEN NOW() ELSE NULL END,
                fields_count = COALESCE($2, fields_count),
                error_message = $3
            WHERE id = $4
        "#;

        sqlx::query(sql)
            .bind(match status {
                RunStatus::Running => "RUNNING",
                RunStatus::Completed => "COMPLETED",
                RunStatus::Failed => "FAILED",
            })
            .bind(fields_count)
            .bind(error)
            .bind(run_id)
            .execute(&self.db_pool)
            .await?;

        Ok(())
    }

    /// 获取字段画像趋势
    pub async fn get_profile_trend(
        &self,
        field_id: i64,
        days: i32,
    ) -> Result<Vec<ProfileTrendPoint>, sqlx::Error> {
        let sql = r#"
            SELECT run_at, null_percentage, unique_count, row_count
            FROM isahl_meta.field_profiles
            WHERE field_id = $1
            AND run_at >= NOW() - INTERVAL '$2 days'
            ORDER BY run_at ASC
        "#;

        let rows: Vec<ProfileTrendRow> = sqlx::query_as(sql)
            .bind(field_id)
            .bind(days as i64)
            .fetch_all(&self.db_pool)
            .await?;

        Ok(rows
            .into_iter()
            .map(|r| ProfileTrendPoint {
                run_at: r.run_at,
                null_percentage: r.null_percentage,
                unique_count: r.unique_count,
                row_count: r.row_count,
            })
            .collect())
    }
}

// 数据库行结构
#[derive(sqlx::FromRow)]
struct FieldProfileRow {
    id: i64,
    field_id: i64,
    collection_id: i64,
    run_at: chrono::DateTime<chrono::Utc>,
    row_count: i64,
    null_count: i64,
    null_percentage: f64,
    unique_count: i64,
    cardinality_ratio: f64,
    statistics: serde_json::Value,
    top_values: serde_json::Value,
    histogram: Option<serde_json::Value>,
}

impl From<FieldProfileRow> for FieldProfile {
    fn from(row: FieldProfileRow) -> Self {
        Self {
            id: row.id,
            field_id: row.field_id,
            collection_id: row.collection_id,
            run_at: row.run_at,
            row_count: row.row_count,
            null_count: row.null_count,
            null_percentage: row.null_percentage,
            unique_count: row.unique_count,
            cardinality_ratio: row.cardinality_ratio,
            statistics: serde_json::from_value(row.statistics).unwrap_or(
                crate::profile::types::ProfileStatistics::Generic(
                    crate::profile::types::GenericStatistics { distinct_count: 0 },
                ),
            ),
            top_values: serde_json::from_value(row.top_values).unwrap_or_default(),
            histogram: row.histogram.and_then(|h| serde_json::from_value(h).ok()),
        }
    }
}

#[derive(sqlx::FromRow)]
struct ProfileTrendRow {
    run_at: chrono::DateTime<chrono::Utc>,
    null_percentage: f64,
    unique_count: i64,
    row_count: i64,
}
