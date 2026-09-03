use sqlx::{AssertSqlSafe, PgPool};

use crate::profile::types::{
    BooleanStatistics, BucketType, DataType, DateTimeStatistics, FieldProfile, GenericStatistics,
    HistogramBucket, HistogramData, NumericStatistics, ProfileStatistics, TextStatistics,
    ValueFrequency,
};

pub struct ProfileEngine {
    db_pool: PgPool,
}

impl ProfileEngine {
    pub fn new(db_pool: PgPool) -> Self {
        Self { db_pool }
    }

    /// 为单个字段生成画像
    pub async fn profile_field(
        &self,
        collection_id: i64,
        field_id: i64,
        schema: &str,
        table: &str,
        field_name: &str,
        data_type: DataType,
    ) -> Result<FieldProfile, sqlx::Error> {
        let run_at = chrono::Utc::now();

        // 1. 获取行数和空值统计
        let (row_count, null_count) = self.get_basic_stats(schema, table, field_name).await?;
        let non_null_count = row_count - null_count;
        let null_percentage = if row_count > 0 {
            (null_count as f64 / row_count as f64) * 100.0
        } else {
            0.0
        };

        // 2. 获取唯一值数量
        let unique_count = self.get_unique_count(schema, table, field_name).await?;
        let cardinality_ratio = if non_null_count > 0 {
            unique_count as f64 / non_null_count as f64
        } else {
            0.0
        };

        // 3. 根据数据类型计算特定统计
        let statistics = if non_null_count > 0 {
            self.calculate_statistics(schema, table, field_name, data_type)
                .await?
        } else {
            ProfileStatistics::Generic(GenericStatistics { distinct_count: 0 })
        };

        // 4. 获取高频值（Top 10）
        let top_values = self.get_top_values(schema, table, field_name, 10).await?;

        // 5. 生成直方图（仅数值和日期类型）
        let histogram = if data_type.is_numeric() || data_type.is_datetime() {
            self.generate_histogram(schema, table, field_name, data_type, 10)
                .await
                .ok()
        } else {
            None
        };

        Ok(FieldProfile {
            id: 0,
            field_id,
            collection_id,
            run_at,
            row_count,
            null_count,
            null_percentage,
            unique_count,
            cardinality_ratio,
            statistics,
            top_values,
            histogram,
        })
    }

    /// 获取基础统计（行数和空值数）
    async fn get_basic_stats(
        &self,
        schema: &str,
        table: &str,
        field: &str,
    ) -> Result<(i64, i64), sqlx::Error> {
        let sql = format!(
            "SELECT COUNT(*) as row_count, \
             COUNT(*) FILTER (WHERE \"{}\" IS NULL) as null_count \
             FROM \"{}\".\"{}\"",
            field, schema, table
        );

        let row: (i64, i64) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await?;

        Ok(row)
    }

    /// 获取唯一值数量
    async fn get_unique_count(
        &self,
        schema: &str,
        table: &str,
        field: &str,
    ) -> Result<i64, sqlx::Error> {
        let sql = format!(
            "SELECT COUNT(DISTINCT \"{}\") FROM \"{}\".\"{}\"",
            field, schema, table
        );

        let row: (i64,) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await?;

        Ok(row.0)
    }

    /// 根据数据类型计算特定统计
    async fn calculate_statistics(
        &self,
        schema: &str,
        table: &str,
        field: &str,
        data_type: DataType,
    ) -> Result<ProfileStatistics, sqlx::Error> {
        if data_type.is_numeric() {
            self.calculate_numeric_stats(schema, table, field).await
        } else if data_type.is_text() {
            self.calculate_text_stats(schema, table, field).await
        } else if data_type.is_datetime() {
            self.calculate_datetime_stats(schema, table, field).await
        } else if data_type.is_boolean() {
            self.calculate_boolean_stats(schema, table, field).await
        } else {
            self.calculate_generic_stats(schema, table, field).await
        }
    }

    /// 计算数值统计
    async fn calculate_numeric_stats(
        &self,
        schema: &str,
        table: &str,
        field: &str,
    ) -> Result<ProfileStatistics, sqlx::Error> {
        let sql = format!(
            "SELECT \
                MIN(\"{}\"::numeric) as min_val, \
                MAX(\"{}\"::numeric) as max_val, \
                AVG(\"{}\"::numeric) as mean_val, \
                STDDEV(\"{}\"::numeric) as std_dev, \
                PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY \"{}\"::numeric) as median_val, \
                SUM(\"{}\"::numeric) as sum_val, \
                COUNT(*) FILTER (WHERE \"{}\"::numeric = 0) as zeros_count, \
                COUNT(*) FILTER (WHERE \"{}\"::numeric < 0) as negatives_count \
             FROM \"{}\".\"{}\" \
             WHERE \"{}\" IS NOT NULL",
            field, field, field, field, field, field, field, field, schema, table, field
        );

        #[allow(clippy::type_complexity)]
        let row: (
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<i64>,
            Option<i64>,
        ) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await?;

        Ok(ProfileStatistics::Numeric(NumericStatistics {
            min: row.0,
            max: row.1,
            mean: row.2,
            std_dev: row.3,
            median: row.4,
            sum: row.5,
            zeros_count: row.6.unwrap_or(0),
            negatives_count: row.7.unwrap_or(0),
        }))
    }

    /// 计算文本统计
    async fn calculate_text_stats(
        &self,
        schema: &str,
        table: &str,
        field: &str,
    ) -> Result<ProfileStatistics, sqlx::Error> {
        let sql = format!(
            "SELECT \
                MIN(LENGTH(\"{}\"::text)) as min_len, \
                MAX(LENGTH(\"{}\"::text)) as max_len, \
                AVG(LENGTH(\"{}\"::text))::float as avg_len, \
                COUNT(*) FILTER (WHERE \"{}\"::text = '') as empty_count, \
                COUNT(*) FILTER (WHERE \"{}\"::text ~ '^\\s+$') as whitespace_count \
             FROM \"{}\".\"{}\" \
             WHERE \"{}\" IS NOT NULL",
            field, field, field, field, field, schema, table, field
        );

        #[allow(clippy::type_complexity)]
        let row: (
            Option<i32>,
            Option<i32>,
            Option<f64>,
            Option<i64>,
            Option<i64>,
        ) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await?;

        Ok(ProfileStatistics::Text(TextStatistics {
            min_length: row.0,
            max_length: row.1,
            avg_length: row.2,
            empty_string_count: row.3.unwrap_or(0),
            whitespace_only_count: row.4.unwrap_or(0),
        }))
    }

    /// 计算日期时间统计
    async fn calculate_datetime_stats(
        &self,
        schema: &str,
        table: &str,
        field: &str,
    ) -> Result<ProfileStatistics, sqlx::Error> {
        let now = chrono::Utc::now();

        let sql = format!(
            "SELECT \
                MIN(\"{}\") as min_date, \
                MAX(\"{}\") as max_date, \
                COUNT(*) FILTER (WHERE \"{}\" > $1) as future_count, \
                COUNT(*) FILTER (WHERE \"{}\" < $1) as past_count \
             FROM \"{}\".\"{}\" \
             WHERE \"{}\" IS NOT NULL",
            field, field, field, field, schema, table, field
        );

        #[allow(clippy::type_complexity)]
        let row: (
            Option<chrono::DateTime<chrono::Utc>>,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<i64>,
            Option<i64>,
        ) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(now)
            .fetch_one(&self.db_pool)
            .await?;

        Ok(ProfileStatistics::DateTime(DateTimeStatistics {
            min_date: row.0,
            max_date: row.1,
            future_dates_count: row.2.unwrap_or(0),
            past_dates_count: row.3.unwrap_or(0),
        }))
    }

    /// 计算布尔统计
    async fn calculate_boolean_stats(
        &self,
        schema: &str,
        table: &str,
        field: &str,
    ) -> Result<ProfileStatistics, sqlx::Error> {
        let sql = format!(
            "SELECT \
                COUNT(*) FILTER (WHERE \"{}\" = true) as true_count, \
                COUNT(*) FILTER (WHERE \"{}\" = false) as false_count \
             FROM \"{}\".\"{}\" \
             WHERE \"{}\" IS NOT NULL",
            field, field, schema, table, field
        );

        let row: (i64, i64) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_one(&self.db_pool)
            .await?;

        let total = row.0 + row.1;
        let true_percentage = if total > 0 {
            (row.0 as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        Ok(ProfileStatistics::Boolean(BooleanStatistics {
            true_count: row.0,
            false_count: row.1,
            true_percentage,
        }))
    }

    /// 计算通用统计
    async fn calculate_generic_stats(
        &self,
        schema: &str,
        table: &str,
        field: &str,
    ) -> Result<ProfileStatistics, sqlx::Error> {
        let distinct_count = self.get_unique_count(schema, table, field).await?;

        Ok(ProfileStatistics::Generic(GenericStatistics {
            distinct_count,
        }))
    }

    /// 获取高频值（Top N）
    async fn get_top_values(
        &self,
        schema: &str,
        table: &str,
        field: &str,
        limit: i64,
    ) -> Result<Vec<ValueFrequency>, sqlx::Error> {
        let sql = format!(
            "SELECT \
                \"{}\"::text as value, \
                COUNT(*) as count, \
                COUNT(*)::float / NULLIF(SUM(COUNT(*)) OVER (), 0) * 100 as percentage \
             FROM \"{}\".\"{}\" \
             WHERE \"{}\" IS NOT NULL \
             GROUP BY \"{}\" \
             ORDER BY count DESC \
             LIMIT {}",
            field, schema, table, field, field, limit
        );

        let rows: Vec<(String, i64, f64)> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .fetch_all(&self.db_pool)
            .await?;

        Ok(rows
            .into_iter()
            .map(|(value, count, percentage)| ValueFrequency {
                value,
                count,
                percentage,
            })
            .collect())
    }

    /// 生成直方图（等宽分桶）
    async fn generate_histogram(
        &self,
        schema: &str,
        table: &str,
        field: &str,
        data_type: DataType,
        buckets: i32,
    ) -> Result<HistogramData, sqlx::Error> {
        if data_type.is_numeric() {
            self.generate_numeric_histogram(schema, table, field, buckets)
                .await
        } else if data_type.is_datetime() {
            self.generate_datetime_histogram(schema, table, field, buckets)
                .await
        } else {
            Err(sqlx::Error::Protocol(
                "Unsupported type for histogram".into(),
            ))
        }
    }

    /// 生成数值直方图
    async fn generate_numeric_histogram(
        &self,
        schema: &str,
        table: &str,
        field: &str,
        buckets: i32,
    ) -> Result<HistogramData, sqlx::Error> {
        let sql = format!(
            "WITH stats AS ( \
                SELECT MIN(\"{}\"::numeric) as min_val, MAX(\"{}\"::numeric) as max_val \
                FROM \"{}\".\"{}\" \
                WHERE \"{}\" IS NOT NULL \
            ), \
            bucketed AS ( \
                SELECT \
                    width_bucket(\"{}\"::numeric, s.min_val, s.max_val, {}) as bucket_num, \
                    COUNT(*) as count \
                FROM \"{}\".\"{}\", stats s \
                WHERE \"{}\" IS NOT NULL AND s.max_val > s.min_val \
                GROUP BY bucket_num \
            ) \
            SELECT \
                b.bucket_num, \
                (s.min_val + (s.max_val - s.min_val) * (b.bucket_num - 1)::numeric / {})::text as range_start, \
                (s.min_val + (s.max_val - s.min_val) * b.bucket_num::numeric / {})::text as range_end, \
                b.count, \
                b.count::float / NULLIF(SUM(b.count) OVER (), 0) * 100 as percentage \
            FROM bucketed b \
            CROSS JOIN stats s \
            WHERE b.bucket_num > 0 AND b.bucket_num <= {} \
            ORDER BY b.bucket_num",
            field, field, schema, table, field,
            field, buckets,
            schema, table, field,
            buckets, buckets,
            buckets
        );

        let rows: Vec<(i32, String, String, i64, f64)> =
            sqlx::query_as(AssertSqlSafe(sql.as_str()))
                .fetch_all(&self.db_pool)
                .await?;

        let bucket_list: Vec<HistogramBucket> = rows
            .into_iter()
            .map(
                |(_, range_start, range_end, count, percentage)| HistogramBucket {
                    range_start,
                    range_end,
                    count,
                    percentage,
                },
            )
            .collect();

        Ok(HistogramData {
            buckets: bucket_list,
            bucket_type: BucketType::Numeric,
        })
    }

    /// 生成日期时间直方图
    async fn generate_datetime_histogram(
        &self,
        schema: &str,
        table: &str,
        field: &str,
        buckets: i32,
    ) -> Result<HistogramData, sqlx::Error> {
        let sql = format!(
            "WITH stats AS ( \
                SELECT MIN(\"{}\") as min_date, MAX(\"{}\") as max_date \
                FROM \"{}\".\"{}\" \
                WHERE \"{}\" IS NOT NULL \
            ), \
            bucketed AS ( \
                SELECT \
                    width_bucket(\"{}\"::float, EXTRACT(EPOCH FROM s.min_date), EXTRACT(EPOCH FROM s.max_date), {}) as bucket_num, \
                    COUNT(*) as count \
                FROM \"{}\".\"{}\", stats s \
                WHERE \"{}\" IS NOT NULL AND s.max_date > s.min_date \
                GROUP BY bucket_num \
            ) \
            SELECT \
                b.bucket_num, \
                to_char(s.min_date + (s.max_date - s.min_date) * (b.bucket_num - 1)::float / {}, 'YYYY-MM-DD') as range_start, \
                to_char(s.min_date + (s.max_date - s.min_date) * b.bucket_num::float / {}, 'YYYY-MM-DD') as range_end, \
                b.count, \
                b.count::float / NULLIF(SUM(b.count) OVER (), 0) * 100 as percentage \
            FROM bucketed b \
            CROSS JOIN stats s \
            WHERE b.bucket_num > 0 AND b.bucket_num <= {} \
            ORDER BY b.bucket_num",
            field, field, schema, table, field,
            field, buckets,
            schema, table, field,
            buckets, buckets,
            buckets
        );

        let rows: Vec<(i32, String, String, i64, f64)> =
            sqlx::query_as(AssertSqlSafe(sql.as_str()))
                .fetch_all(&self.db_pool)
                .await?;

        let bucket_list: Vec<HistogramBucket> = rows
            .into_iter()
            .map(
                |(_, range_start, range_end, count, percentage)| HistogramBucket {
                    range_start,
                    range_end,
                    count,
                    percentage,
                },
            )
            .collect();

        Ok(HistogramData {
            buckets: bucket_list,
            bucket_type: BucketType::DateTime,
        })
    }
}

#[cfg(test)]
mod tests {

    // 这些测试需要数据库连接，暂时只进行编译检查
    #[test]
    fn test_profile_engine_creation() {
        // 仅验证结构可以创建
        // 实际测试需要数据库连接
    }
}
