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

    /// 载重结构化落标量：capacity_ton → **`zc_id_scal-weight`**（`zc_id_scale` 族**语义叶**）
    /// 并挂 `qk_w_capacity`。模型正本（`crud/fk_index.rs`，容器族声明行）：
    /// `("zc_id_stor-ctn-vehicle", &[… ("w_capacity", "zc_id_scal-weight", "qk_w_capacity") …])`；
    /// 同族 `v_capacity` → `zc_id_scal-volume` / `qk_v_capacity`、`c_capacity` →
    /// `zc_id_scal-amount` / `qk_c_capacity`；无物理种类的 `capacity` 才落
    /// `zc_id_scale` / `qk_capacity`（用户 2026-09-21 裁决「统一落在各种对应的语义表，
    /// 没有再到标量表」）。
    ///
    /// 全链同口径（载体均为语义叶 `zc_id_scal-weight`）：本函数（平台写径）、门户
    /// [`insert_vehicle_measure`]（OpenActivity portal_write.rs）、种子
    /// （`Pre-Proc/WZ/seed/seed-wz-vehicle-load.sql` `WZ-BIZ-VEH-CAP-*`）、平台读径
    /// （transport-operations：`fleet_overview.rs` / `reassign.rs` / `waybill_detail.rs` /
    /// `checkin.rs`）、门户读径（`supplier.rs` 车辆/委托列表）、超载判据
    /// （`portal_write.rs::vehicle_capacity_ton`）。
    ///
    /// 第 10 轮批注 #2 的 `scal-common` 漂移按模型正本回退：那次修复只把**读列**从
    /// `qk_w_capacity` 改成 `qk_capacity`（未纠正标量族叶位）⇒ 写径落通用叶、语义与模型
    /// 冲突。读径既已同批改回 `qk_w_capacity` → `zc_id_scal-weight`，写径必须同叶。
    ///
    /// **既有行策略（本函数不迁数据）**：先探测既有标量行**实际所在叶**（`tableoid`）——
    /// ① 已在 weight 叶 → 经族父表 `zc_id_scale` 按 id **原位**改写（`WHERE id` 命中该行；
    ///    探测已确认落点，故不会误改他叶行，也不依赖叶表名字面量）；
    /// ② 无行 / 既有行落在别的标量叶（历史 `scal-common` 漂移行）→ **重建**：在 weight 叶
    ///    新建标量行并重指 `qk_w_capacity`；旧行不软删（成为未被引用的孤儿，清理属数据迁移，
    ///    见迁移脚本草案——本函数不做，避免在写径里静默删数据）。
    async fn apply_vehicle_capacity(
        &self,
        vehicle_id: i64,
        capacity_ton: f64,
        user_id: i64,
    ) -> Result<(), ApiError> {
        if capacity_ton <= 0.0 {
            return Ok(());
        }
        // 既有引用 + 该引用是否已在 weight 叶（`tableoid` 比对，避免把叶名写进 SQL 字面量）
        let (existing_ref, in_weight_leaf): (Option<i64>, Option<bool>) = sqlx::query_as(
            r#"SELECT v.qk_w_capacity,
                      (SELECT s.tableoid = 'isahl."zc_id_scal-weight"'::regclass
                         FROM "isahl"."zc_id_scale" s
                        WHERE s.id = v.qk_w_capacity)
                 FROM "isahl"."zc_id_stor-ctn-vehicle" v
                WHERE v.id = $1 AND v.deleted_at IS NULL"#,
        )
        .bind(vehicle_id)
        .fetch_one(&self.pool)
        .await?;
        let scale_id: i64 =
            match (existing_ref, in_weight_leaf) {
                (Some(id), Some(true)) => {
                    sqlx::query(
                        r#"UPDATE "isahl"."zc_id_scale"
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
                _ => sqlx::query_scalar(
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

    /// 体积容量结构化落标量：capacity_m3 → `zc_id_scal-volume`（`zc_id_scale` 族语义叶），
    /// `qk_v_capacity` 引用（模型正本：`crud/fk_index.rs` `("v_capacity", "zc_id_scal-volume",
    /// "qk_v_capacity")`）。叶位本就正确，既有行策略同 [`Self::apply_vehicle_capacity`]（原位改写）。
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
                   VALUES (isahl.gen_next_uid(260), $1, $2, NOW(), $3, $3, $4)
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
                           SET point = postgis.ST_SetSRID(postgis.ST_MakePoint($1, $2), 4326), updated_by_id = $3, updated_at = NOW()
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
                           VALUES ($1, NULL, postgis.ST_SetSRID(postgis.ST_MakePoint($2, $3), 4326), $4)
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
        // 号牌先归一 + 形态校验（非法即拒——避免车辆行落地后留下号牌半成品）
        let plates = crate::plates::prepare_plates(&req.plates)?;
        // `fk_trustee` = **发行方**（唯一性来源主体；车辆 = 主机厂；可空 = 发行方未登记）——
        // MUST NOT 承载归属/登记组织/承运商语义（change `align-storage-issuer-and-holding` §D1）；
        // 归属（持有）= 主体↔储元桥叶 `zc_id_subjects_rr_container`，本仓储不解释、不派生该列。
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
        // 号牌（可多牌）：`zc_id_identity`（分类 plate）+ `zc_id_entity_rr_identity` 桥
        // （唯一实现 = `crate::plates`；车辆 `notice`/`code` 不承载号牌）
        if !plates.is_empty() {
            let mut conn = self.pool.acquire().await.map_err(ApiError::from_sqlx)?;
            for input in &plates {
                crate::plates::create_vehicle_plate(&mut conn, vehicle.id, user_id, input).await?;
            }
        }
        Ok(vehicle)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateVehicleRequest,
        user_id: i64,
    ) -> Result<Option<Vehicle>, ApiError> {
        // 号牌先归一 + 形态校验（非法即拒——避免字段已更新而号牌非法）
        let plates = crate::plates::prepare_plates(&req.plates)?;
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
        // `fk_trustee` = 发行方（唯一性来源主体；车辆 = 主机厂）——语义见 `create`，
        // 不承载归属/承运商（§D1）；入参可选，缺省不动该列
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
        if let Some(v) = req.qk_w_capacity {
            q = q.bind(v);
        }
        if let Some(v) = req.ck_r_type {
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
            // 号牌（本次新增；换牌 = 先经号牌端点解除旧牌再新增，历史保留）
            if !plates.is_empty() {
                let mut conn = self.pool.acquire().await.map_err(ApiError::from_sqlx)?;
                for input in &plates {
                    crate::plates::create_vehicle_plate(&mut conn, id, user_id, input).await?;
                }
            }
        }
        Ok(updated)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        // 号牌桥行**随车辆保留**（历史事实：换牌/解绑才软删桥行；车辆软删不等于号牌失效）。
        // 号牌**判重**域已改「同号牌全局 + 生效期不重叠」（change
        // `align-storage-issuer-and-holding` §D3/§D4；见 `crate::plates::create_vehicle_plate`），
        // 已软删车辆的残留绑定不计入占用；**反查**（号牌 → 车辆）仍 MUST 以**车辆存活**为
        // 谓词（见 `crate::plates` 各查询与 damage-writer/门户反查）⇒ 不误命中已删车辆。
        self.generic.delete(id, user_id).await
    }
}
