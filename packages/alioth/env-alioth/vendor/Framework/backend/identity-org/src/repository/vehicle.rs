// VehicleRepository（split 自 repository.rs 单体，④ 候选）
//! 身份实体 Repository — 标准 CRUD 实现
//!
//! Identity 使用自定义 Repository，其余实体组合 GenericRepository，
//! 仅自定义 create/update 的 INSERT/UPDATE SQL。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::repository::AliothRepository;
use crud::GenericRepository;
use sqlx::{AssertSqlSafe, PgPool};

use crate::models::{CreateVehicleRequest, UpdateVehicleRequest, Vehicle};

use super::ontology_binding;

// ═══════════════════════════════════════════════
// Vehicle — "isahl"."zc_id_stor-ctn-vehicle"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct VehicleRepository {
    generic: GenericRepository<Vehicle>,
    pool: PgPool,
}

impl VehicleRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 载重结构化落标量：capacity_ton → zc_id_scal-weight（模型设计 w_capacity 族），qk_w_capacity 引用
    async fn apply_vehicle_capacity(
        &self,
        vehicle_id: i64,
        capacity_ton: f64,
        user_id: i64,
    ) -> Result<(), ApiError> {
        if capacity_ton <= 0.0 {
            return Ok(());
        }
        let existing: Option<Option<i64>> = sqlx::query_scalar(
            r#"SELECT qk_w_capacity FROM "isahl"."zc_id_stor-ctn-vehicle"
                   WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(vehicle_id)
        .fetch_one(&self.pool)
        .await?;
        let scale_id: i64 =
            match existing.flatten() {
                Some(id) => {
                    sqlx::query(
                        r#"UPDATE "isahl"."zc_id_scal-weight"
                           SET notice = $1, mark = $2, updated_by_id = $3, updated_at = NOW()
                           WHERE id = $4"#,
                    )
                    .bind(format!("{}吨", capacity_ton))
                    .bind(capacity_ton)
                    .bind(user_id)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
                    id
                }
                None => sqlx::query_scalar(
                    r#"INSERT INTO "isahl"."zc_id_scal-weight" (code, notice, mark, created_by_id)
                       VALUES ($1, $2, $3, $4) RETURNING id"#,
                )
                .bind(format!("VEH-CAP-{}", vehicle_id))
                .bind(format!("{}吨", capacity_ton))
                .bind(capacity_ton)
                .bind(user_id)
                .fetch_one(&self.pool)
                .await?,
            };
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_stor-ctn-vehicle"
                   SET qk_w_capacity = $1, updated_by_id = $2, updated_at = NOW()
                   WHERE id = $3 AND deleted_at IS NULL"#,
        )
        .bind(scale_id)
        .bind(user_id)
        .bind(vehicle_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 体积容量结构化落标量：capacity_m3 → zc_id_scal-volume（模型设计 v_capacity 族），qk_v_capacity 引用
    ///（fix-vehicle-unit-binding-add-volume：模型升级后 qk_v_capacity 列已回归）
    async fn apply_vehicle_volume(
        &self,
        vehicle_id: i64,
        capacity_m3: f64,
        user_id: i64,
    ) -> Result<(), ApiError> {
        if capacity_m3 <= 0.0 {
            return Ok(());
        }
        let existing: Option<Option<i64>> = sqlx::query_scalar(
            r#"SELECT qk_v_capacity FROM "isahl"."zc_id_stor-ctn-vehicle"
                   WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(vehicle_id)
        .fetch_one(&self.pool)
        .await?;
        let scale_id: i64 =
            match existing.flatten() {
                Some(id) => {
                    sqlx::query(
                        r#"UPDATE "isahl"."zc_id_scal-volume"
                           SET notice = $1, mark = $2, updated_by_id = $3, updated_at = NOW()
                           WHERE id = $4"#,
                    )
                    .bind(format!("{}立方米", capacity_m3))
                    .bind(capacity_m3)
                    .bind(user_id)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
                    id
                }
                None => sqlx::query_scalar(
                    r#"INSERT INTO "isahl"."zc_id_scal-volume" (code, notice, mark, created_by_id)
                       VALUES ($1, $2, $3, $4) RETURNING id"#,
                )
                .bind(format!("VEH-VOL-{}", vehicle_id))
                .bind(format!("{}立方米", capacity_m3))
                .bind(capacity_m3)
                .bind(user_id)
                .fetch_one(&self.pool)
                .await?,
            };
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_stor-ctn-vehicle"
                   SET qk_v_capacity = $1, updated_by_id = $2, updated_at = NOW()
                   WHERE id = $3 AND deleted_at IS NULL"#,
        )
        .bind(scale_id)
        .bind(user_id)
        .bind(vehicle_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    /// 状态生命周期物化：status_code → zc_id_stus-vehicle 反查 → primary-status 幂等 upsert
    async fn apply_vehicle_status(
        &self,
        vehicle_id: i64,
        status_code: Option<&str>,
        user_id: i64,
    ) -> Result<(), ApiError> {
        let Some(code) = status_code.filter(|s| !s.is_empty()) else {
            return Ok(());
        };
        let status_id: Option<i64> = sqlx::query_scalar(
            r#"SELECT id FROM "isahl"."zc_id_stus-vehicle"
                   WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await?
        .flatten();
        let Some(status_id) = status_id else {
            return Err(ApiError::Validation {
                field: "status_code".into(),
                message: format!("未知车辆状态码: {code}（缺种子先执行 wz-vehicle-dict-seed.sql）"),
            });
        };
        sqlx::query(
                r#"INSERT INTO "isahl"."zc_id_lifecycle_r_primary-status"
                   (id, ref_left, ref_right, status_date, created_by_id, updated_by_id, code)
                   VALUES (isahl.gen_next_zuid(), $1, $2, NOW(), $3, $3, $4)
                   ON CONFLICT (ref_left) DO UPDATE
                   SET ref_right = $2, code = $4, status_date = NOW(), updated_at = NOW(), updated_by_id = $3"#,
            )
            .bind(vehicle_id)
            .bind(status_id)
            .bind(user_id)
            .bind(code)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// 位置点位 upsert：经纬度 → zc_id_geog-point（geometry Point,4326）→ qk_point 引用
    async fn apply_vehicle_point(
        &self,
        vehicle_id: i64,
        lng: Option<f64>,
        lat: Option<f64>,
        user_id: i64,
    ) -> Result<(), ApiError> {
        let (Some(lng), Some(lat)) = (lng, lat) else {
            return Ok(());
        };
        let existing: Option<Option<i64>> = sqlx::query_scalar(
            r#"SELECT qk_point FROM "isahl"."zc_id_stor-ctn-vehicle"
                   WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(vehicle_id)
        .fetch_one(&self.pool)
        .await?;
        match existing.flatten() {
            Some(pid) => {
                sqlx::query(
                        r#"UPDATE "isahl"."zc_id_geog-point"
                           SET point = ST_SetSRID(ST_MakePoint($1, $2), 4326), updated_by_id = $3, updated_at = NOW()
                           WHERE id = $4 AND deleted_at IS NULL"#,
                    )
                    .bind(lng)
                    .bind(lat)
                    .bind(user_id)
                    .bind(pid)
                    .execute(&self.pool)
                    .await?;
            }
            None => {
                let pid: i64 = sqlx::query_scalar(
                    r#"INSERT INTO "isahl"."zc_id_geog-point" (code, sk_unit, point, created_by_id)
                           VALUES ($1, NULL, ST_SetSRID(ST_MakePoint($2, $3), 4326), $4)
                           RETURNING id"#,
                )
                .bind(format!("VEH-PT-{}", vehicle_id))
                .bind(lng)
                .bind(lat)
                .bind(user_id)
                .fetch_one(&self.pool)
                .await?;
                sqlx::query(
                    r#"UPDATE "isahl"."zc_id_stor-ctn-vehicle"
                           SET qk_point = $1, updated_by_id = $2, updated_at = NOW()
                           WHERE id = $3 AND deleted_at IS NULL"#,
                )
                .bind(pid)
                .bind(user_id)
                .bind(vehicle_id)
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }
}

impl From<PgPool> for VehicleRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<Vehicle, CreateVehicleRequest, UpdateVehicleRequest, ApiError>
    for VehicleRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Vehicle>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<Vehicle>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(&self, req: CreateVehicleRequest, user_id: i64) -> Result<Vehicle, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "Vehicle").await?;
        let vehicle = sqlx::query_as::<_, Vehicle>(
                r#"INSERT INTO "isahl"."zc_id_stor-ctn-vehicle" (code, notice, comments, fk_trustee, qk_w_capacity, "ck_r-type", created_by_id, dk_scene, dk_factor, dk_function)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                                      RETURNING id, code, notice, comments, fk_trustee, qk_w_capacity, qk_v_capacity, "ck_r-type", created_at, updated_at, deleted_at"#,
            )
            .bind(&req.code).bind(&req.notice).bind(&req.comments)
            .bind(req.fk_trustee)
            .bind(req.qk_w_capacity)
            .bind(req.ck_r_type)
            .bind(user_id)
            .bind(dk_scene).bind(dk_factor).bind(dk_function)
            .fetch_one(&self.pool)
            .await
            .map_err(ApiError::from)?;
        // fix-vehicle-field-mapping：结构化物化——载重标量 / 生命周期状态 / 位置点位
        // （comments 不再承载业务数据；ck_r_type 由前端直传字典 id）
        if let Some(ton) = req.capacity_ton {
            self.apply_vehicle_capacity(vehicle.id, ton, user_id)
                .await?;
        }
        if let Some(m3) = req.capacity_m3 {
            self.apply_vehicle_volume(vehicle.id, m3, user_id).await?;
        }
        self.apply_vehicle_status(vehicle.id, req.status_code.as_deref(), user_id)
            .await?;
        self.apply_vehicle_point(vehicle.id, req.point_lng, req.point_lat, user_id)
            .await?;
        Ok(vehicle)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateVehicleRequest,
        user_id: i64,
    ) -> Result<Option<Vehicle>, ApiError> {
        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if req.code.is_some() {
            idx += 1;
            sets.push(format!("code = ${}", idx));
        }
        if req.notice.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
        if req.comments.is_some() {
            idx += 1;
            sets.push(format!("comments = ${}", idx));
        }
        if req.fk_trustee.is_some() {
            idx += 1;
            sets.push(format!("fk_trustee = ${}", idx));
        }
        if req.qk_w_capacity.is_some() {
            idx += 1;
            sets.push(format!("qk_w_capacity = ${}", idx));
        }
        if req.ck_r_type.is_some() {
            idx += 1;
            sets.push(format!("\"ck_r-type\" = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_stor-ctn-vehicle" SET {} WHERE id = ${} AND deleted_at IS NULL
                   RETURNING id, code, notice, comments, fk_trustee, qk_w_capacity, qk_v_capacity, "ck_r-type", created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, Vehicle>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_trustee {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_trustee {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_w_capacity {
            q = q.bind(v);
        }
        if let Some(ref v) = req.ck_r_type {
            q = q.bind(v);
        }

        q = q.bind(user_id);
        q = q.bind(id);

        let updated = q.fetch_optional(&self.pool).await.map_err(ApiError::from)?;
        // fix-vehicle-field-mapping：结构化物化——载重标量 / 生命周期状态 / 位置点位
        if updated.is_some() {
            if let Some(ton) = req.capacity_ton {
                self.apply_vehicle_capacity(id, ton, user_id).await?;
            }
            if let Some(m3) = req.capacity_m3 {
                self.apply_vehicle_volume(id, m3, user_id).await?;
            }
            self.apply_vehicle_status(id, req.status_code.as_deref(), user_id)
                .await?;
            self.apply_vehicle_point(id, req.point_lng, req.point_lat, user_id)
                .await?;
        }
        Ok(updated)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
