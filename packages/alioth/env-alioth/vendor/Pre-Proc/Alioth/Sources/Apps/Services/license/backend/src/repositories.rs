//! 许可证 Repository — 标准 CRUD 实现
//!
//! Repository 持有 PgPool。其他 namespace 应参照此实现。

use crate::models;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::data::{ListQuery, PaginatedResponse};
use common::error::AliothError;
use crud::AliothRepository;
use rust_decimal::Decimal;
use sqlx::{AssertSqlSafe, FromRow, PgPool, Postgres, Transaction};

// ── 许可证 License Repository ────────────────────────────────────────────
// 完整实现 ontology 映射：
// - seats  -> qk_capacity -> zc_id_scal-common.mark
// - expires-> zc_id_deta-trade_order.fk_delivery = license.id
//            + trade_order.qk_date (zc_id_scal-date)
//            + license.qk_duration (zc_id_scal-duration)
// - status -> zc_id_lifecycle_r_primary-status (ref_left=license.id, ref_right=status.id)
#[derive(Clone)]
pub struct LicenseRepository {
    pool: PgPool,
}

impl LicenseRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl From<PgPool> for LicenseRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

/// 用于 list/get 的 JOIN 查询字段列表。
const LICENSE_SELECT_FIELDS: &str = r#"
l.id, l.notice AS name, l.code AS key,
l."fk_subj-provider" AS vendor, l.ck_category AS kind,
sc.mark AS seats,
sd.date + (sdu.mark * interval '1 second') AS expires,
rps.ref_right AS status,
0::bigint AS used,
jsonb_build_object(
    'vendor', jsonb_build_object('notice', subj.notice, 'code', subj.code),
    'type', jsonb_build_object('notice', cate.notice, 'code', cate.code),
    'status', jsonb_build_object('notice', st.notice, 'code', st.code)
) AS _refs,
l.created_at, l.updated_at, l.deleted_at"#;
/// 插入或更新 scalar-common，返回 ID。
async fn ensure_common_scalar(
    tx: &mut Transaction<'_, Postgres>,
    value: i64,
) -> Result<i64, AliothError> {
    let mark = Decimal::from(value);
    let id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_scal-common" WHERE mark = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(mark)
    .fetch_optional(&mut **tx)
    .await
    .map_err(AliothError::from)?;
    if let Some(id) = id {
        return Ok(id);
    }
    let notice = format!("common: {}", mark);
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-common" (notice, mark, created_by_id)
           VALUES ($1, $2, 1) RETURNING id"#,
    )
    .bind(&notice)
    .bind(mark)
    .fetch_one(&mut **tx)
    .await
    .map_err(AliothError::from)?;
    Ok(id)
}

/// 插入或更新 scalar-date（到天），返回 ID。
async fn ensure_date_scalar(
    tx: &mut Transaction<'_, Postgres>,
    date: DateTime<Utc>,
) -> Result<i64, AliothError> {
    let id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_scal-date" WHERE date = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(date)
    .fetch_optional(&mut **tx)
    .await
    .map_err(AliothError::from)?;
    if let Some(id) = id {
        return Ok(id);
    }
    let notice = date.format("%Y-%m-%d").to_string();
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-date" (notice, date, created_by_id)
           VALUES ($1, $2, 1) RETURNING id"#,
    )
    .bind(&notice)
    .bind(date)
    .fetch_one(&mut **tx)
    .await
    .map_err(AliothError::from)?;
    Ok(id)
}

/// 插入或更新 scalar-duration（秒数），返回 ID。
async fn ensure_duration_scalar(
    tx: &mut Transaction<'_, Postgres>,
    seconds: i64,
) -> Result<i64, AliothError> {
    let mark = Decimal::from(seconds);
    let id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_scal-duration" WHERE mark = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(mark)
    .fetch_optional(&mut **tx)
    .await
    .map_err(AliothError::from)?;
    if let Some(id) = id {
        return Ok(id);
    }
    let notice = format!("duration: {}s", seconds);
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-duration" (notice, mark, created_by_id)
           VALUES ($1, $2, 1) RETURNING id"#,
    )
    .bind(&notice)
    .bind(mark)
    .fetch_one(&mut **tx)
    .await
    .map_err(AliothError::from)?;
    Ok(id)
}

/// 计算生效日期与 duration（秒数）。
fn compute_duration_from_expires(
    expires: DateTime<Utc>,
) -> Result<(DateTime<Utc>, i64), AliothError> {
    let effective = Utc::now();
    let dur = expires.signed_duration_since(effective);
    if dur.num_seconds() < 0 {
        return Err(AliothError::BadRequest(
            "expires must be in the future".into(),
        ));
    }
    Ok((effective, dur.num_seconds()))
}

/// 将供应商名称（notice）解析为 `zc_id_subjects` 的 ID。
async fn resolve_subject_id(
    pool: &PgPool,
    name: &Option<String>,
) -> Result<Option<i64>, AliothError> {
    let Some(name) = name else {
        return Ok(None);
    };
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM isahl.zc_id_subjects WHERE notice = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(AliothError::from)
}

/// 将类型名称（notice）解析为 `zc_id_category` 的 ID。
async fn resolve_category_id(
    pool: &PgPool,
    name: &Option<String>,
) -> Result<Option<i64>, AliothError> {
    let Some(name) = name else {
        return Ok(None);
    };
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM isahl.zc_id_category WHERE notice = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(AliothError::from)
}

/// update 时用于读取当前 license 原始列的内部结构。
#[derive(Debug, FromRow)]
#[expect(dead_code)]
struct LicenseRaw {
    id: i64,
    qk_capacity: Option<i64>,
    qk_duration: Option<i64>,
}

#[async_trait]
impl
    AliothRepository<
        models::License,
        models::CreateLicenseRequest,
        models::UpdateLicenseRequest,
        AliothError,
    > for LicenseRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<models::License>, AliothError> {
        let page = query.page.max(1);
        let page_size = query.page_size.max(1);
        let offset = (page - 1) * page_size;

        let items_sql = format!(
            r#"SELECT {} FROM isahl."zc_id_prod-license-purchase" l
               LEFT JOIN isahl."zc_id_scal-common" sc ON sc.id = l.qk_capacity AND sc.deleted_at IS NULL
               LEFT JOIN LATERAL (
                   SELECT dto.qk_date FROM isahl."zc_id_deta-trade_order" dto
                   WHERE dto.fk_delivery = l.id AND dto.deleted_at IS NULL
                   ORDER BY dto.id LIMIT 1
               ) dto ON true
               LEFT JOIN isahl."zc_id_scal-date" sd ON sd.id = dto.qk_date AND sd.deleted_at IS NULL
               LEFT JOIN isahl."zc_id_scal-duration" sdu ON sdu.id = l.qk_duration AND sdu.deleted_at IS NULL
               LEFT JOIN LATERAL (
                   SELECT rps.ref_right FROM isahl."zc_id_lifecycle_r_primary-status" rps
                   WHERE rps.ref_left = l.id AND rps.deleted_at IS NULL LIMIT 1
               ) rps ON true
               LEFT JOIN isahl.zc_id_subjects subj ON subj.id = l."fk_subj-provider" AND subj.deleted_at IS NULL
               LEFT JOIN isahl.zc_id_category cate ON cate.id = l.ck_category AND cate.deleted_at IS NULL
               LEFT JOIN isahl.zc_id_status st ON st.id = rps.ref_right AND st.deleted_at IS NULL
               WHERE l.deleted_at IS NULL
               ORDER BY l.id DESC LIMIT $1 OFFSET $2"#,
            LICENSE_SELECT_FIELDS
        );
        let items: Vec<models::License> =
            sqlx::query_as::<_, models::License>(AssertSqlSafe(items_sql))
                .bind(page_size)
                .bind(offset)
                .fetch_all(&self.pool)
                .await
                .map_err(AliothError::from)?;

        let count_sql = r#"SELECT COUNT(*) FROM isahl."zc_id_prod-license-purchase" l WHERE l.deleted_at IS NULL"#;
        let (total,) = sqlx::query_as::<_, (i64,)>(count_sql)
            .fetch_one(&self.pool)
            .await
            .map_err(AliothError::from)?;

        Ok(PaginatedResponse {
            items,
            total,
            page,
            page_size,
        })
    }

    async fn get(&self, id: i64) -> Result<Option<models::License>, AliothError> {
        let sql = format!(
            r#"SELECT {} FROM isahl."zc_id_prod-license-purchase" l
               LEFT JOIN isahl."zc_id_scal-common" sc ON sc.id = l.qk_capacity AND sc.deleted_at IS NULL
               LEFT JOIN LATERAL (
                   SELECT dto.qk_date FROM isahl."zc_id_deta-trade_order" dto
                   WHERE dto.fk_delivery = l.id AND dto.deleted_at IS NULL
                   ORDER BY dto.id LIMIT 1
               ) dto ON true
               LEFT JOIN isahl."zc_id_scal-date" sd ON sd.id = dto.qk_date AND sd.deleted_at IS NULL
               LEFT JOIN isahl."zc_id_scal-duration" sdu ON sdu.id = l.qk_duration AND sdu.deleted_at IS NULL
               LEFT JOIN LATERAL (
                   SELECT rps.ref_right FROM isahl."zc_id_lifecycle_r_primary-status" rps
                   WHERE rps.ref_left = l.id AND rps.deleted_at IS NULL LIMIT 1
               ) rps ON true
               LEFT JOIN isahl.zc_id_subjects subj ON subj.id = l."fk_subj-provider" AND subj.deleted_at IS NULL
               LEFT JOIN isahl.zc_id_category cate ON cate.id = l.ck_category AND cate.deleted_at IS NULL
               LEFT JOIN isahl.zc_id_status st ON st.id = rps.ref_right AND st.deleted_at IS NULL
               WHERE l.id = $1 AND l.deleted_at IS NULL"#,
            LICENSE_SELECT_FIELDS
        );
        sqlx::query_as::<_, models::License>(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(AliothError::from)
    }

    async fn create(
        &self,
        req: models::CreateLicenseRequest,
        user_id: i64,
    ) -> Result<models::License, AliothError> {
        let mut tx = self.pool.begin().await.map_err(AliothError::from)?;

        let vendor_id = match req.vendor {
            Some(id) => Some(id),
            None => resolve_subject_id(&self.pool, &req.vendor_name).await?,
        };
        let kind_id = match req.kind {
            Some(id) => Some(id),
            None => resolve_category_id(&self.pool, &req.kind_name).await?,
        };

        let seats_id = match req.seats {
            Some(v) => Some(ensure_common_scalar(&mut tx, v).await?),
            None => None,
        };

        let (date_id, duration_id) = match req.expires {
            Some(expires) => {
                let (effective, seconds) = compute_duration_from_expires(expires)?;
                let date_id = ensure_date_scalar(&mut tx, effective).await?;
                let duration_id = ensure_duration_scalar(&mut tx, seconds).await?;
                (Some(date_id), Some(duration_id))
            }
            None => (None, None),
        };

        let license_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_prod-license-purchase"
               (notice, code, "fk_subj-provider", ck_category, qk_capacity, qk_duration, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               RETURNING id"#,
        )
        .bind(&req.name)
        .bind(&req.key)
        .bind(vendor_id)
        .bind(kind_id)
        .bind(seats_id)
        .bind(duration_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(AliothError::from)?;

        if let Some(date_id) = date_id {
            sqlx::query(
                r#"INSERT INTO isahl."zc_id_deta-trade_order" (notice, fk_delivery, qk_date, created_by_id)
                   VALUES ($1, $2, $3, $4)"#,
            )
            .bind(format!("delivery for license {}", license_id))
            .bind(license_id)
            .bind(date_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
        }

        if let Some(status_id) = req.status {
            sqlx::query(
                r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (ref_left, ref_right, created_by_id)
                   VALUES ($1, $2, $3)"#,
            )
            .bind(license_id)
            .bind(status_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
        }

        tx.commit().await.map_err(AliothError::from)?;
        self.get(license_id).await?.ok_or_else(|| {
            AliothError::NotFound(format!("License {} not found after create", license_id))
        })
    }

    async fn update(
        &self,
        id: i64,
        req: models::UpdateLicenseRequest,
        user_id: i64,
    ) -> Result<Option<models::License>, AliothError> {
        let mut tx = self.pool.begin().await.map_err(AliothError::from)?;

        let current = sqlx::query_as::<_, LicenseRaw>(
            r#"SELECT id, qk_capacity, qk_duration FROM isahl."zc_id_prod-license-purchase"
               WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(AliothError::from)?;
        let current = match current {
            Some(c) => c,
            None => return Ok(None),
        };

        let vendor_id = match req.vendor {
            Some(id) => Some(id),
            None => resolve_subject_id(&self.pool, &req.vendor_name).await?,
        };
        let kind_id = match req.kind {
            Some(id) => Some(id),
            None => resolve_category_id(&self.pool, &req.kind_name).await?,
        };

        let seats_id = match req.seats {
            Some(v) => Some(ensure_common_scalar(&mut tx, v).await?),
            None => current.qk_capacity,
        };

        let duration_id = if let Some(expires) = req.expires {
            let (effective, seconds) = compute_duration_from_expires(expires)?;
            let date_id = ensure_date_scalar(&mut tx, effective).await?;
            let duration_id = ensure_duration_scalar(&mut tx, seconds).await?;

            // 软删除旧 trade_order，插入新的。
            sqlx::query(
                r#"UPDATE isahl."zc_id_deta-trade_order" SET deleted_at = NOW(), updated_by_id = $3
                   WHERE fk_delivery = $1 AND deleted_at IS NULL"#,
            )
            .bind(id)
            .bind(user_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
            sqlx::query(
                r#"INSERT INTO isahl."zc_id_deta-trade_order" (notice, fk_delivery, qk_date, created_by_id)
                   VALUES ($1, $2, $3, $4)"#,
            )
            .bind(format!("delivery for license {}", id))
            .bind(id)
            .bind(date_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
            Some(duration_id)
        } else {
            current.qk_duration
        };

        sqlx::query(
            r#"UPDATE isahl."zc_id_prod-license-purchase"
               SET notice = COALESCE($1, notice), code = COALESCE($2, code),
                   "fk_subj-provider" = COALESCE($3, "fk_subj-provider"),
                   ck_category = COALESCE($4, ck_category),
                   qk_capacity = COALESCE($5, qk_capacity),
                   qk_duration = COALESCE($6, qk_duration),
                   updated_at = NOW(), updated_by_id = $7
               WHERE id = $8 AND deleted_at IS NULL"#,
        )
        .bind(&req.name)
        .bind(&req.key)
        .bind(vendor_id)
        .bind(kind_id)
        .bind(seats_id)
        .bind(duration_id)
        .bind(user_id)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(AliothError::from)?;

        if let Some(status_id) = req.status {
            sqlx::query(
                r#"UPDATE isahl."zc_id_lifecycle_r_primary-status" SET deleted_at = NOW(), updated_by_id = $3
                   WHERE ref_left = $1 AND deleted_at IS NULL"#,
            )
            .bind(id)
            .bind(user_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
            sqlx::query(
                r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (ref_left, ref_right, created_by_id)
                   VALUES ($1, $2, $3)"#,
            )
            .bind(id)
            .bind(status_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
        }

        tx.commit().await.map_err(AliothError::from)?;
        self.get(id).await
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        let mut tx = self.pool.begin().await.map_err(AliothError::from)?;
        let r = sqlx::query(r#"UPDATE isahl."zc_id_prod-license-purchase" SET deleted_at = NOW(), updated_by_id = $2 WHERE id = $1 AND deleted_at IS NULL"#)
            .bind(id).bind(user_id).execute(&mut *tx).await.map_err(AliothError::from)?;
        if r.rows_affected() == 0 {
            return Err(AliothError::NotFound(format!("License {} not found", id)));
        }
        tx.commit().await.map_err(AliothError::from)?;
        Ok(())
    }

    async fn batch_delete(&self, ids: Vec<i64>, user_id: i64) -> Result<(), AliothError> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await.map_err(AliothError::from)?;
        sqlx::query(
            r#"UPDATE isahl."zc_id_prod-license-purchase" SET deleted_at = NOW(), updated_by_id = $2 WHERE id = ANY($1) AND deleted_at IS NULL"#,
        )
        .bind(&ids)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(AliothError::from)?;
        tx.commit().await.map_err(AliothError::from)?;
        Ok(())
    }
}
