//! 身份实体 Repository — 标准 CRUD 实现
//!
//! Identity 使用自定义 Repository，其余实体组合 GenericRepository，
//! 仅自定义 create/update 的 INSERT/UPDATE SQL。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::query_builder::QueryBuilder;
use crud::repository::AliothRepository;
use crud::GenericRepository;
use crud::SubtableRouter;
use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool};

use crate::models::{
    BillCheck, Consignment, Contract, CreateBillCheckRequest, CreateConsignmentRequest,
    CreateContractRequest, CreateDetaBillCheckRequest, CreateEnvironmentRequest,
    CreateFenceRequest, CreateFreightProductRequest, CreateIdentityRequest,
    CreateInventorySalesRequest, CreateInvoiceDetailRequest, CreateInvoiceRequest,
    CreateLicenseRequest, CreatePaymentRequest, CreatePricingAgreementRequest,
    CreateSealBatchRequest, CreateSealRequest, CreateSettlementBankRequest,
    CreateSettlementCashRequest, CreateSettlementChannelRequest, CreateTradeOrderRequest,
    CreateTrafficLineRequest, CreateTransitRouteRequest, CreateTransportTrackingRequest,
    CreateVehicleRequest, DetaBillCheck, Environment, Fence, FreightProduct, Identity,
    InventorySales, Invoice, InvoiceDetail, License, Payment, PricingAgreement, Seal,
    SettlementBank, SettlementCash, SettlementChannel, TradeOrder, TrafficLine, TransitRoute,
    TransportTracking, UpdateBillCheckRequest, UpdateConsignmentRequest, UpdateContractRequest,
    UpdateDetaBillCheckRequest, UpdateEnvironmentRequest, UpdateFenceRequest,
    UpdateFreightProductRequest, UpdateIdentityRequest, UpdateInventorySalesRequest,
    UpdateInvoiceDetailRequest, UpdateInvoiceRequest, UpdateLicenseRequest, UpdatePaymentRequest,
    UpdatePricingAgreementRequest, UpdateSealRequest, UpdateSettlementBankRequest,
    UpdateSettlementCashRequest, UpdateSettlementChannelRequest, UpdateTradeOrderRequest,
    UpdateTrafficLineRequest, UpdateTransitRouteRequest, UpdateTransportTrackingRequest,
    UpdateVehicleRequest, Vehicle,
};

// ═══════════════════════════════════════════════════════════════════════════════
// 本体维度绑定辅助（Ontology Binding）
//
// 坐标 code 来自 factor.json 的 ontology.entities[].coordinates，运行时通过 DB
// 维度表解析为 ZUID，再注入 create 语句的 dk_scene/dk_factor/dk_function。
// 注意：IdentityRepository 实际写入的是 zc_id_subjects 叶表，仍复用 Identity
// 实体的坐标（scene=JE, factor=FJA, function=↑_DA）。
// ═══════════════════════════════════════════════
// Environment — isahl.zc_id_prot-env_config
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct EnvironmentRepository {
    generic: GenericRepository<Environment>,
    pool: PgPool,
}

impl EnvironmentRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for EnvironmentRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<Environment, CreateEnvironmentRequest, UpdateEnvironmentRequest, ApiError>
    for EnvironmentRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Environment>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<Environment>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateEnvironmentRequest,
        user_id: i64,
    ) -> Result<Environment, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "Environment").await?;
        sqlx::query_as::<_, Environment>(
            r#"INSERT INTO "isahl"."zc_id_prot-env_config" (notice, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id, notice AS name, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateEnvironmentRequest,
        user_id: i64,
    ) -> Result<Option<Environment>, ApiError> {
        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if req.name.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl.zc_id_prot-env_config" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, notice AS name, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, Environment>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.name {
            q = q.bind(v);
        }

        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// License — isahl.zc_id_prod-license-purchase
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct LicenseRepository {
    generic: GenericRepository<License>,
    pool: PgPool,
}

impl LicenseRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for LicenseRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<License, CreateLicenseRequest, UpdateLicenseRequest, ApiError>
    for LicenseRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<License>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<License>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(&self, req: CreateLicenseRequest, user_id: i64) -> Result<License, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "License").await?;
        sqlx::query_as::<_, License>(
            r#"INSERT INTO "isahl"."zc_id_prod-license-purchase" (notice, qk_capacity, qk_period, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               RETURNING id, notice AS name, qk_capacity AS qk_qty, qk_period, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(req.qk_qty)
        .bind(req.qk_period)
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateLicenseRequest,
        user_id: i64,
    ) -> Result<Option<License>, ApiError> {
        let mut sets = Vec::new();

        let mut idx: usize = 0;

        if req.name.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
        if req.qk_qty.is_some() {
            idx += 1;
            sets.push(format!("qk_capacity = ${}", idx));
        }
        if req.qk_period.is_some() {
            idx += 1;
            sets.push(format!("qk_period = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;
        let sql = format!(
            r#"UPDATE "isahl.zc_id_prod-license-purchase" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, notice AS name, qk_capacity AS qk_qty, qk_period, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );
        let mut q = sqlx::query_as::<_, License>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.name {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_qty {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_period {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// Consignment — "isahl"."zc_id_orde-land"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct ConsignmentRepository {
    generic: GenericRepository<Consignment>,
    pool: PgPool,
}

impl ConsignmentRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// traffic_line_id 此前寄生于产品 comments JSON，已随 comments 文本化失效
    /// （模型无承载列）——绑定能力停用，显式拒绝而非静默丢数据。
    async fn apply_traffic_line(
        &self,
        _consignment_id: i64,
        traffic_line_id: Option<i64>,
    ) -> Result<(), ApiError> {
        if traffic_line_id.unwrap_or(0) > 0 {
            return Err(ApiError::Validation {
                field: "traffic_line_id".into(),
                message: "运输线路绑定已停用（comments 已文本化，模型无承载列）".into(),
            });
        }
        Ok(())
    }
}

impl From<PgPool> for ConsignmentRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<Consignment, CreateConsignmentRequest, UpdateConsignmentRequest, ApiError>
    for ConsignmentRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Consignment>, ApiError> {
        self.generic.list_refs(query).await
    }
    async fn get(&self, id: i64) -> Result<Option<Consignment>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateConsignmentRequest,
        user_id: i64,
    ) -> Result<Consignment, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "Consignment").await?;
        sqlx::query_as::<_, Consignment>(
            r#"INSERT INTO "isahl"."zc_id_orde-land" (code, notice, comments, fk_subject, fk_object, fk_contract, qk_date, sk_currency, ck_category, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               RETURNING id, code, notice, comments, fk_subject, fk_object, fk_contract, qk_date, sk_currency, ck_category, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_subject).bind(req.fk_object).bind(req.fk_contract)
        .bind(req.qk_date)
        .bind(req.sk_currency).bind(req.ck_category)
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)

        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateConsignmentRequest,
        user_id: i64,
    ) -> Result<Option<Consignment>, ApiError> {
        let mut sets = Vec::new();
        let mut idx: usize = 0;
        // 批注 2026-08-21：编辑货量（volume 数值吨）→ 更新 qk_total 标量真值。
        // 两段式独立语句（不参与下方 sets 占位符编号）——旧实现把新建标量 id 推入
        // sets 头部却在绑定尾部追加，占位符与绑定序错位（新建标量+其他字段并存必 500）
        if let Some(v) = req.volume {
            let cur_qk: Option<i64> = sqlx::query_scalar(
                r#"SELECT qk_w_qty FROM "isahl"."zc_id_deta-trade_order"
                   WHERE fk_list = $1 AND deleted_at IS NULL LIMIT 1"#,
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .flatten();
            match cur_qk {
                Some(scale_id) => {
                    // 批注 2026-08-21：重量/货量存储约束为整数——ROUND 兜底
                    sqlx::query(
                        r#"UPDATE "isahl"."zc_id_scal-weight" SET mark = ROUND($1::numeric), notice = $2 WHERE id = $3"#,
                    )
                    .bind(v)
                    .bind(format!("{}吨", v))
                    .bind(scale_id)
                    .execute(&self.pool)
                    .await?;
                }
                None => {
                    let scale_id: i64 = sqlx::query_scalar(
                        r#"INSERT INTO "isahl"."zc_id_scal-weight" (id, code, notice, mark, created_by_id)
                           VALUES (isahl.gen_next_uid(), $1, $2, ROUND($3::numeric), $4) RETURNING id"#,
                    )
                    .bind(format!("WT-{}", chrono::Utc::now().timestamp()))
                    .bind(format!("{}吨", v))
                    .bind(v)
                    .bind(user_id)
                    .fetch_one(&self.pool)
                    .await?;
                    sqlx::query(
                        r#"UPDATE "isahl"."zc_id_deta-trade_order" SET qk_w_qty = $1, updated_by_id = $2, updated_at = NOW()
                           WHERE fk_list = $3 AND deleted_at IS NULL"#,
                    )
                    .bind(scale_id)
                    .bind(user_id)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
                }
            }
        }
        // 批注 a2bd97b6：编辑运费（amount 数值）→ 更新 qk_amount 指向的 scal-amount
        // 标量真值（金额保留小数，不 ROUND）；qk_amount 为空则新建标量并回挂
        if let Some(v) = req.amount {
            let cur_qk: Option<i64> = sqlx::query_scalar(
                r#"SELECT qk_amount FROM "isahl"."zc_id_deta-trade_order"
                   WHERE fk_list = $1 AND deleted_at IS NULL LIMIT 1"#,
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .flatten();
            match cur_qk {
                Some(scale_id) => {
                    sqlx::query(
                        r#"UPDATE "isahl"."zc_id_scal-amount" SET mark = ROUND($1::numeric, 2), notice = $2 WHERE id = $3"#,
                    )
                    .bind(v)
                    .bind(format!("{}元", v))
                    .bind(scale_id)
                    .execute(&self.pool)
                    .await?;
                }
                None => {
                    let scale_id: i64 = sqlx::query_scalar(
                        r#"INSERT INTO "isahl"."zc_id_scal-amount" (id, code, notice, mark, created_by_id)
                           VALUES (isahl.gen_next_uid(), $1, $2, ROUND($3::numeric, 2), $4) RETURNING id"#,
                    )
                    .bind(format!("AMT-{}", chrono::Utc::now().timestamp()))
                    .bind(format!("{}元", v))
                    .bind(v)
                    .bind(user_id)
                    .fetch_one(&self.pool)
                    .await?;
                    sqlx::query(
                        r#"UPDATE "isahl"."zc_id_deta-trade_order" SET qk_amount = $1, updated_by_id = $2, updated_at = NOW()
                           WHERE fk_list = $3 AND deleted_at IS NULL"#,
                    )
                    .bind(scale_id)
                    .bind(user_id)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
                }
            }
        }

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
        if req.fk_subject.is_some() {
            idx += 1;
            sets.push(format!("fk_subject = ${}", idx));
        }
        if req.fk_object.is_some() {
            idx += 1;
            sets.push(format!("fk_object = ${}", idx));
        }
        if let Some(fc) = req.fk_contract {
            if fc == 0 {
                // 清除哨兵：0 -> 置 NULL（不占位、不绑定）
                sets.push("fk_contract = NULL".into());
            } else {
                idx += 1;
                sets.push(format!("fk_contract = ${}", idx));
            }
        }

        if req.qk_date.is_some() {
            idx += 1;
            sets.push(format!("qk_date = ${}", idx));
        }
        if req.sk_currency.is_some() {
            idx += 1;
            sets.push(format!("sk_currency = ${}", idx));
        }
        if req.ck_category.is_some() {
            idx += 1;
            sets.push(format!("ck_category = ${}", idx));
        }

        if sets.is_empty() {
            // 仅透传 traffic_line_id（改写创建期产品 comments）时主表无字段可更新
            self.apply_traffic_line(id, req.traffic_line_id).await?;
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_orde-land" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_subject, fk_object, fk_contract, qk_date, sk_currency, ck_category, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, Consignment>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_subject {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_object {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_contract {
            if v != 0 {
                q = q.bind(v);
            }
        }
        if let Some(ref v) = req.qk_date {
            q = q.bind(v);
        }
        if let Some(v) = req.sk_currency {
            q = q.bind(v);
        }
        if let Some(ref v) = req.ck_category {
            q = q.bind(v);
        }

        q = q.bind(user_id);
        q = q.bind(id);

        let updated = q.fetch_optional(&self.pool).await.map_err(ApiError::from)?;
        if updated.is_some() {
            self.apply_traffic_line(id, req.traffic_line_id).await?;
        }
        Ok(updated)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

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
                r#"INSERT INTO "isahl"."zc_id_stor-ctn-vehicle" (code, notice, comments, sk_unit, fk_trustee, qk_w_capacity, "ck_r-type", created_by_id, dk_scene, dk_factor, dk_function)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                                      RETURNING id, code, notice, comments, sk_unit, fk_trustee, qk_w_capacity, qk_v_capacity, "ck_r-type", created_at, updated_at, deleted_at"#,
            )
            .bind(&req.code).bind(&req.notice).bind(&req.comments)
            .bind(req.sk_unit).bind(req.fk_trustee)
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
        if req.sk_unit.is_some() {
            idx += 1;
            sets.push(format!("sk_unit = ${}", idx));
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
                   RETURNING id, code, notice, comments, sk_unit, fk_trustee, qk_w_capacity, qk_v_capacity, "ck_r-type", created_at, updated_at, deleted_at"#,
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
        if let Some(v) = req.sk_unit {
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

// ═══════════════════════════════════════════════
// NaturalPerson — "isahl"."zc_id_empl-natural"（司机/操作员主数据）
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct NaturalPersonRepository {
    generic: GenericRepository<NaturalPerson>,
    pool: PgPool,
}

impl NaturalPersonRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for NaturalPersonRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        NaturalPerson,
        CreateNaturalPersonRequest,
        UpdateNaturalPersonRequest,
        ApiError,
    > for NaturalPersonRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<NaturalPerson>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<NaturalPerson>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateNaturalPersonRequest,
        user_id: i64,
    ) -> Result<NaturalPerson, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "NaturalPerson").await?;
        sqlx::query_as::<_, NaturalPerson>(
            r#"INSERT INTO "isahl"."zc_id_empl-natural"
               (code, notice, o_number, comments, fk_user, ck_category, sk_unit, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
               RETURNING id, code, notice, o_number, comments, fk_user, ck_category, sk_unit, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code)
        .bind(&req.notice)
        .bind(&req.o_number)
        .bind(&req.comments)
        .bind(req.fk_user)
        .bind(req.ck_category)
        .bind(req.sk_unit)
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateNaturalPersonRequest,
        user_id: i64,
    ) -> Result<Option<NaturalPerson>, ApiError> {
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
        if req.o_number.is_some() {
            idx += 1;
            sets.push(format!("o_number = ${}", idx));
        }
        if req.comments.is_some() {
            idx += 1;
            sets.push(format!("comments = ${}", idx));
        }
        if req.fk_user.is_some() {
            idx += 1;
            sets.push(format!("fk_user = ${}", idx));
        }
        if req.ck_category.is_some() {
            idx += 1;
            sets.push(format!("ck_category = ${}", idx));
        }
        if req.sk_unit.is_some() {
            idx += 1;
            sets.push(format!("sk_unit = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_empl-natural" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, o_number, comments, fk_user, ck_category, sk_unit, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, NaturalPerson>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.o_number {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_user {
            q = q.bind(v);
        }
        if let Some(v) = req.ck_category {
            q = q.bind(v);
        }
        if let Some(v) = req.sk_unit {
            q = q.bind(v);
        }

        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// TransportTracking — "isahl"."zc_id_oper-transport_tracking"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct TransportTrackingRepository {
    generic: GenericRepository<TransportTracking>,
    pool: PgPool,
}

impl TransportTrackingRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for TransportTrackingRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        TransportTracking,
        CreateTransportTrackingRequest,
        UpdateTransportTrackingRequest,
        ApiError,
    > for TransportTrackingRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<TransportTracking>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<TransportTracking>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateTransportTrackingRequest,
        user_id: i64,
    ) -> Result<TransportTracking, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "TransportTracking").await?;
        // fix-fk-approve-residual-consumers：fk_approve 物理列已移除——
        // 创建后写 rr_event 桥行承载审批事件关联，RETURNING 用基表查询补派生值
        let created: TransportTracking = sqlx::query_as::<_, TransportTracking>(
            r#"INSERT INTO "isahl"."zc_id_oper-transport_tracking" (code, notice, comments, fk_operator, fk_subject, qk_arrived, qk_work_duration, "ck_cate-wh", "ck_cate-biz", created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               RETURNING id, code, notice, comments, fk_operator, fk_subject, NULL::bigint AS fk_approve, qk_arrived, qk_work_duration, "ck_cate-wh", "ck_cate-biz", created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_operator).bind(req.fk_subject)
        .bind(req.qk_arrived).bind(req.qk_work_duration)
        .bind(req.ck_cate_wh).bind(req.ck_cate_biz)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;
        if let Some(ev) = req.fk_approve {
            sqlx::query(
                r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
                   SELECT isahl.gen_next_zuid(), $1, $2, $3
                   WHERE NOT EXISTS (
                       SELECT 1 FROM isahl.zc_id_operation_rr_event rr
                       WHERE rr.ref_left = $1 AND rr.ref_right = $2 AND rr.deleted_at IS NULL
                   )"#,
            )
            .bind(created.id)
            .bind(ev)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(ApiError::from)?;
        }
        // fk_approve 为桥派生值——桥行已落库，回显请求值即可（与 get/get_refs 派生同口径）
        if req.fk_approve.is_some() {
            return Ok(TransportTracking {
                fk_approve: req.fk_approve,
                ..created
            });
        }
        Ok(created)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateTransportTrackingRequest,
        user_id: i64,
    ) -> Result<Option<TransportTracking>, ApiError> {
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

        if req.fk_operator.is_some() {
            idx += 1;
            sets.push(format!("fk_operator = ${}", idx));
        }
        if req.fk_subject.is_some() {
            idx += 1;
            sets.push(format!("fk_subject = ${}", idx));
        }
        if req.qk_arrived.is_some() {
            idx += 1;
            sets.push(format!("qk_arrived = ${}", idx));
        }
        if req.qk_work_duration.is_some() {
            idx += 1;
            sets.push(format!("qk_work_duration = ${}", idx));
        }
        if req.ck_cate_wh.is_some() {
            idx += 1;
            sets.push(format!(r#""ck_cate-wh" = ${}"#, idx));
        }
        if req.ck_cate_biz.is_some() {
            idx += 1;
            sets.push(format!(r#""ck_cate-biz" = ${}"#, idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_oper-transport_tracking" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_operator, fk_subject, NULL::bigint AS fk_approve, qk_arrived, qk_work_duration, "ck_cate-wh", "ck_cate-biz", created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, TransportTracking>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }

        if let Some(v) = req.fk_operator {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_subject {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_arrived {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_work_duration {
            q = q.bind(v);
        }
        if let Some(ref v) = req.ck_cate_wh {
            q = q.bind(v);
        }
        if let Some(ref v) = req.ck_cate_biz {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        let row = q.fetch_optional(&self.pool).await.map_err(ApiError::from)?;
        // fix-fk-approve-residual-consumers：审批事件关联改 rr_event 桥——
        // 有值时软删旧桥行后重建（update 带 fk_approve 即重绑）
        if row.is_some() {
            if let Some(ev) = req.fk_approve {
                sqlx::query(
                    r#"UPDATE isahl.zc_id_operation_rr_event SET deleted_at = NOW()
                       WHERE ref_left = $1 AND deleted_at IS NULL"#,
                )
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(ApiError::from)?;
                sqlx::query(
                    r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
                       VALUES (isahl.gen_next_zuid(), $1, $2, $3)"#,
                )
                .bind(id)
                .bind(ev)
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(ApiError::from)?;
                // 返回桥派生后的真实行（fk_approve 经 rr_event 子查询派生）
                return self.get(id).await;
            }
        }
        Ok(row)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// TradeOrder — "isahl"."zc_id_deta-trade_order"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct TradeOrderRepository {
    generic: GenericRepository<TradeOrder>,
    pool: PgPool,
}

impl TradeOrderRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for TradeOrderRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<TradeOrder, CreateTradeOrderRequest, UpdateTradeOrderRequest, ApiError>
    for TradeOrderRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<TradeOrder>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<TradeOrder>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateTradeOrderRequest,
        user_id: i64,
    ) -> Result<TradeOrder, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "TradeOrder").await?;
        sqlx::query_as::<_, TradeOrder>(
            r#"INSERT INTO "isahl"."zc_id_deta-trade_order" (code, notice, comments, fk_goods, fk_demand, fk_delivery, fk_deal, fk_biller, fk_counterparty, qk_price, qk_qty, qk_amount, sk_currency, sk_unit, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)
               RETURNING id, code, notice, comments, fk_goods, fk_demand, fk_delivery, fk_deal, fk_biller, fk_counterparty, qk_price, qk_qty, qk_amount, sk_currency, sk_unit, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_goods).bind(req.fk_demand).bind(req.fk_delivery).bind(req.fk_deal)
        .bind(req.fk_biller).bind(req.fk_counterparty)
        .bind(req.qk_price).bind(req.qk_qty).bind(req.qk_amount)
        .bind(req.sk_currency).bind(req.sk_unit)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateTradeOrderRequest,
        user_id: i64,
    ) -> Result<Option<TradeOrder>, ApiError> {
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
        if req.fk_goods.is_some() {
            idx += 1;
            sets.push(format!("fk_goods = ${}", idx));
        }
        if req.fk_demand.is_some() {
            idx += 1;
            sets.push(format!("fk_demand = ${}", idx));
        }
        if req.fk_delivery.is_some() {
            idx += 1;
            sets.push(format!("fk_delivery = ${}", idx));
        }
        if req.fk_deal.is_some() {
            idx += 1;
            sets.push(format!("fk_deal = ${}", idx));
        }
        if req.fk_biller.is_some() {
            idx += 1;
            sets.push(format!("fk_biller = ${}", idx));
        }
        if req.fk_counterparty.is_some() {
            idx += 1;
            sets.push(format!("fk_counterparty = ${}", idx));
        }
        if req.qk_price.is_some() {
            idx += 1;
            sets.push(format!("qk_price = ${}", idx));
        }
        if req.qk_qty.is_some() {
            idx += 1;
            sets.push(format!("qk_qty = ${}", idx));
        }
        if req.qk_amount.is_some() {
            idx += 1;
            sets.push(format!("qk_amount = ${}", idx));
        }
        if req.sk_currency.is_some() {
            idx += 1;
            sets.push(format!("sk_currency = ${}", idx));
        }
        if req.sk_unit.is_some() {
            idx += 1;
            sets.push(format!("sk_unit = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_deta-trade_order" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_goods, fk_demand, fk_delivery, fk_deal, fk_biller, fk_counterparty, qk_price, qk_qty, qk_amount, sk_currency, sk_unit, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, TradeOrder>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_goods {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_demand {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_delivery {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_deal {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_biller {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_counterparty {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_price {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_qty {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_amount {
            q = q.bind(v);
        }
        if let Some(v) = req.sk_currency {
            q = q.bind(v);
        }
        if let Some(v) = req.sk_unit {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// BillCheck — "isahl"."zc_id_bill-check"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct BillCheckRepository {
    generic: GenericRepository<BillCheck>,
    pool: PgPool,
}

impl BillCheckRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for BillCheckRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<BillCheck, CreateBillCheckRequest, UpdateBillCheckRequest, ApiError>
    for BillCheckRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<BillCheck>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<BillCheck>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateBillCheckRequest,
        user_id: i64,
    ) -> Result<BillCheck, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "BillCheck").await?;
        sqlx::query_as::<_, BillCheck>(
            r#"INSERT INTO "isahl"."zc_id_bill-check" (code, notice, comments, fk_settle, fk_account, qk_amount, "qk_write-off", qk_tax, sk_currency, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               RETURNING id, code, notice, comments, fk_settle, fk_account, qk_amount, "qk_write-off" AS qk_write_off, qk_tax, sk_currency, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_settle).bind(req.fk_account)
        .bind(req.qk_amount).bind(req.qk_write_off).bind(req.qk_tax)
        .bind(req.sk_currency)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateBillCheckRequest,
        user_id: i64,
    ) -> Result<Option<BillCheck>, ApiError> {
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
        if req.fk_settle.is_some() {
            idx += 1;
            sets.push(format!("fk_settle = ${}", idx));
        }
        if req.fk_account.is_some() {
            idx += 1;
            sets.push(format!("fk_account = ${}", idx));
        }
        if req.qk_amount.is_some() {
            idx += 1;
            sets.push(format!("qk_amount = ${}", idx));
        }
        if req.qk_write_off.is_some() {
            idx += 1;
            sets.push(format!("\"qk_write-off\" = ${}", idx));
        }
        if req.qk_tax.is_some() {
            idx += 1;
            sets.push(format!("qk_tax = ${}", idx));
        }
        if req.sk_currency.is_some() {
            idx += 1;
            sets.push(format!("sk_currency = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_bill-check" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_settle, fk_account, qk_amount, "qk_write-off" AS qk_write_off, qk_tax, sk_currency, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, BillCheck>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_settle {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_account {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_amount {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_write_off {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_tax {
            q = q.bind(v);
        }
        if let Some(v) = req.sk_currency {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// DetaBillCheck — "isahl"."zc_id_deta-bill-check"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct DetaBillCheckRepository {
    generic: GenericRepository<DetaBillCheck>,
    pool: PgPool,
}

impl DetaBillCheckRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for DetaBillCheckRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        DetaBillCheck,
        CreateDetaBillCheckRequest,
        UpdateDetaBillCheckRequest,
        ApiError,
    > for DetaBillCheckRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<DetaBillCheck>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<DetaBillCheck>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateDetaBillCheckRequest,
        user_id: i64,
    ) -> Result<DetaBillCheck, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "DetaBillCheck").await?;
        sqlx::query_as::<_, DetaBillCheck>(
            r#"INSERT INTO "isahl"."zc_id_deta-bill-check" (code, notice, comments, fk_list, ck_category, qk_qty, qk_price, qk_amount, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
               RETURNING id, code, notice, comments, fk_list, ck_category, qk_qty, qk_price, qk_amount, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_list)
        .bind(req.ck_category)
        .bind(req.qk_qty).bind(req.qk_price).bind(req.qk_amount)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateDetaBillCheckRequest,
        user_id: i64,
    ) -> Result<Option<DetaBillCheck>, ApiError> {
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
        if req.fk_list.is_some() {
            idx += 1;
            sets.push(format!("fk_list = ${}", idx));
        }
        if req.ck_category.is_some() {
            idx += 1;
            sets.push(format!("ck_category = ${}", idx));
        }
        if req.qk_qty.is_some() {
            idx += 1;
            sets.push(format!("qk_qty = ${}", idx));
        }
        if req.qk_price.is_some() {
            idx += 1;
            sets.push(format!("qk_price = ${}", idx));
        }
        if req.qk_amount.is_some() {
            idx += 1;
            sets.push(format!("qk_amount = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_deta-bill-check" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_list, ck_category, qk_qty, qk_price, qk_amount, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, DetaBillCheck>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_list {
            q = q.bind(v);
        }
        if let Some(ref v) = req.ck_category {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_qty {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_price {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_amount {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// Invoice — "isahl"."zc_id_invo-electric"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct InvoiceRepository {
    generic: GenericRepository<Invoice>,
    pool: PgPool,
}

impl InvoiceRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for InvoiceRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<Invoice, CreateInvoiceRequest, UpdateInvoiceRequest, ApiError>
    for InvoiceRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Invoice>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<Invoice>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(&self, req: CreateInvoiceRequest, user_id: i64) -> Result<Invoice, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "Invoice").await?;
        sqlx::query_as::<_, Invoice>(
            r#"INSERT INTO "isahl"."zc_id_invo-electric" (code, notice, comments, fk_sender, fk_recipient, qk_issue_date, qk_amount, qk_tax, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
               RETURNING id, code, notice, comments, fk_sender, fk_recipient, qk_issue_date, qk_amount, qk_tax, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_sender).bind(req.fk_recipient)
        .bind(req.qk_issue_date).bind(req.qk_amount).bind(req.qk_tax)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateInvoiceRequest,
        user_id: i64,
    ) -> Result<Option<Invoice>, ApiError> {
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
        if req.fk_sender.is_some() {
            idx += 1;
            sets.push(format!("fk_sender = ${}", idx));
        }
        if req.fk_recipient.is_some() {
            idx += 1;
            sets.push(format!("fk_recipient = ${}", idx));
        }
        if req.qk_issue_date.is_some() {
            idx += 1;
            sets.push(format!("qk_issue_date = ${}", idx));
        }
        if req.qk_amount.is_some() {
            idx += 1;
            sets.push(format!("qk_amount = ${}", idx));
        }
        if req.qk_tax.is_some() {
            idx += 1;
            sets.push(format!("qk_tax = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_invo-electric" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_sender, fk_recipient, qk_issue_date, qk_amount, qk_tax, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, Invoice>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_sender {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_recipient {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_issue_date {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_amount {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_tax {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// InvoiceDetail — "isahl"."zc_id_deta-invoice"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct InvoiceDetailRepository {
    generic: GenericRepository<InvoiceDetail>,
    pool: PgPool,
}

impl InvoiceDetailRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for InvoiceDetailRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        InvoiceDetail,
        CreateInvoiceDetailRequest,
        UpdateInvoiceDetailRequest,
        ApiError,
    > for InvoiceDetailRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<InvoiceDetail>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<InvoiceDetail>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateInvoiceDetailRequest,
        user_id: i64,
    ) -> Result<InvoiceDetail, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "InvoiceDetail").await?;
        sqlx::query_as::<_, InvoiceDetail>(
            r#"INSERT INTO "isahl"."zc_id_deta-invoice" (code, notice, comments, fk_list, fk_subject, ck_category, qk_qty, qk_price, qk_amount, qk_tax_amount, qk_tax_ratio, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
               RETURNING id, code, notice, comments, fk_list, fk_subject, ck_category, qk_qty, qk_price, qk_amount, qk_tax_amount, qk_tax_ratio, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_list).bind(req.fk_subject)
        .bind(req.ck_category)
        .bind(req.qk_qty).bind(req.qk_price).bind(req.qk_amount)
        .bind(req.qk_tax_amount).bind(req.qk_tax_ratio)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateInvoiceDetailRequest,
        user_id: i64,
    ) -> Result<Option<InvoiceDetail>, ApiError> {
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
        if req.fk_list.is_some() {
            idx += 1;
            sets.push(format!("fk_list = ${}", idx));
        }
        if req.fk_subject.is_some() {
            idx += 1;
            sets.push(format!("fk_subject = ${}", idx));
        }
        if req.ck_category.is_some() {
            idx += 1;
            sets.push(format!("ck_category = ${}", idx));
        }
        if req.qk_qty.is_some() {
            idx += 1;
            sets.push(format!("qk_qty = ${}", idx));
        }
        if req.qk_price.is_some() {
            idx += 1;
            sets.push(format!("qk_price = ${}", idx));
        }
        if req.qk_amount.is_some() {
            idx += 1;
            sets.push(format!("qk_amount = ${}", idx));
        }
        if req.qk_tax_amount.is_some() {
            idx += 1;
            sets.push(format!("qk_tax_amount = ${}", idx));
        }
        if req.qk_tax_ratio.is_some() {
            idx += 1;
            sets.push(format!("qk_tax_ratio = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_deta-invoice" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_list, fk_subject, ck_category, qk_qty, qk_price, qk_amount, qk_tax_amount, qk_tax_ratio, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, InvoiceDetail>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_list {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_subject {
            q = q.bind(v);
        }
        if let Some(ref v) = req.ck_category {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_qty {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_price {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_amount {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_tax_amount {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_tax_ratio {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// Payment — "isahl"."zc_id_oper-payment"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct PaymentRepository {
    generic: GenericRepository<Payment>,
    pool: PgPool,
}

impl PaymentRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for PaymentRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<Payment, CreatePaymentRequest, UpdatePaymentRequest, ApiError>
    for PaymentRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Payment>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<Payment>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(&self, req: CreatePaymentRequest, user_id: i64) -> Result<Payment, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "Payment").await?;
        // fix-fk-approve-residual-consumers：fk_approve 物理列已移除——
        // 创建后写 rr_event 桥行承载审批事件关联
        let created: Payment = sqlx::query_as::<_, Payment>(
            r#"INSERT INTO "isahl"."zc_id_oper-payment" (code, notice, comments, fk_operator, fk_subject, "ck_cate-wh", "ck_cate-biz", created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
               RETURNING id, code, notice, comments, fk_operator, fk_subject, NULL::bigint AS fk_approve, "ck_cate-wh", "ck_cate-biz", created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_operator).bind(req.fk_subject)
        .bind(req.ck_cate_wh).bind(req.ck_cate_biz)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;
        if let Some(ev) = req.fk_approve {
            sqlx::query(
                r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
                   SELECT isahl.gen_next_zuid(), $1, $2, $3
                   WHERE NOT EXISTS (
                       SELECT 1 FROM isahl.zc_id_operation_rr_event rr
                       WHERE rr.ref_left = $1 AND rr.ref_right = $2 AND rr.deleted_at IS NULL
                   )"#,
            )
            .bind(created.id)
            .bind(ev)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(ApiError::from)?;
            // fk_approve 为桥派生值——桥行已落库，回显请求值（与 get 派生同口径）
            return Ok(Payment {
                fk_approve: req.fk_approve,
                ..created
            });
        }
        Ok(created)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdatePaymentRequest,
        user_id: i64,
    ) -> Result<Option<Payment>, ApiError> {
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

        if req.fk_operator.is_some() {
            idx += 1;
            sets.push(format!("fk_operator = ${}", idx));
        }
        if req.fk_subject.is_some() {
            idx += 1;
            sets.push(format!("fk_subject = ${}", idx));
        }
        if req.ck_cate_wh.is_some() {
            idx += 1;
            sets.push(format!(r#""ck_cate-wh" = ${}"#, idx));
        }
        if req.ck_cate_biz.is_some() {
            idx += 1;
            sets.push(format!(r#""ck_cate-biz" = ${}"#, idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_oper-payment" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_operator, fk_subject, NULL::bigint AS fk_approve, "ck_cate-wh", "ck_cate-biz", created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, Payment>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }

        if let Some(v) = req.fk_operator {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_subject {
            q = q.bind(v);
        }
        if let Some(ref v) = req.ck_cate_wh {
            q = q.bind(v);
        }
        if let Some(ref v) = req.ck_cate_biz {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        let row = q.fetch_optional(&self.pool).await.map_err(ApiError::from)?;
        // fix-fk-approve-residual-consumers：审批事件关联改 rr_event 桥——
        // 有值时软删旧桥行后重建（update 带 fk_approve 即重绑）
        if row.is_some() {
            if let Some(ev) = req.fk_approve {
                sqlx::query(
                    r#"UPDATE isahl.zc_id_operation_rr_event SET deleted_at = NOW()
                       WHERE ref_left = $1 AND deleted_at IS NULL"#,
                )
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(ApiError::from)?;
                sqlx::query(
                    r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
                       VALUES (isahl.gen_next_zuid(), $1, $2, $3)"#,
                )
                .bind(id)
                .bind(ev)
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(ApiError::from)?;
                return self.get(id).await;
            }
        }
        Ok(row)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// SettlementBank — "isahl"."zc_id_stat-smt-bank"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct SettlementBankRepository {
    generic: GenericRepository<SettlementBank>,
    pool: PgPool,
}

impl SettlementBankRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for SettlementBankRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        SettlementBank,
        CreateSettlementBankRequest,
        UpdateSettlementBankRequest,
        ApiError,
    > for SettlementBankRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<SettlementBank>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<SettlementBank>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateSettlementBankRequest,
        user_id: i64,
    ) -> Result<SettlementBank, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "SettlementBank").await?;
        sqlx::query_as::<_, SettlementBank>(
            r#"INSERT INTO "isahl"."zc_id_stat-smt-bank" (qk_date, qk_income, qk_outgo, qk_balance, qk_total, "qk_exchange-rate", created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING id, qk_date, qk_income, qk_outgo, qk_balance, qk_total, "qk_exchange-rate", created_at, updated_at, deleted_at"#,
        )
        .bind(req.qk_date).bind(req.qk_income).bind(req.qk_outgo)
        .bind(req.qk_balance).bind(req.qk_total).bind(req.qk_exchange_rate)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateSettlementBankRequest,
        user_id: i64,
    ) -> Result<Option<SettlementBank>, ApiError> {
        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if req.qk_date.is_some() {
            idx += 1;
            sets.push(format!("qk_date = ${}", idx));
        }
        if req.qk_income.is_some() {
            idx += 1;
            sets.push(format!("qk_income = ${}", idx));
        }
        if req.qk_outgo.is_some() {
            idx += 1;
            sets.push(format!("qk_outgo = ${}", idx));
        }
        if req.qk_balance.is_some() {
            idx += 1;
            sets.push(format!("qk_balance = ${}", idx));
        }
        if req.qk_total.is_some() {
            idx += 1;
            sets.push(format!("qk_total = ${}", idx));
        }
        if req.qk_exchange_rate.is_some() {
            idx += 1;
            sets.push(format!("qk_exchange-rate = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_stat-smt-bank" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, qk_date, qk_income, qk_outgo, qk_balance, qk_total, "qk_exchange-rate", created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, SettlementBank>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.qk_date {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_income {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_outgo {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_balance {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_total {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_exchange_rate {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// SettlementCash — "isahl"."zc_id_stat-smt-cash"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct SettlementCashRepository {
    generic: GenericRepository<SettlementCash>,
    pool: PgPool,
}

impl SettlementCashRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for SettlementCashRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        SettlementCash,
        CreateSettlementCashRequest,
        UpdateSettlementCashRequest,
        ApiError,
    > for SettlementCashRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<SettlementCash>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<SettlementCash>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateSettlementCashRequest,
        user_id: i64,
    ) -> Result<SettlementCash, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "SettlementCash").await?;
        sqlx::query_as::<_, SettlementCash>(
            r#"INSERT INTO "isahl"."zc_id_stat-smt-cash" (qk_date, qk_income, qk_outgo, qk_amount, qk_total, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING id, qk_date, qk_income, qk_outgo, qk_amount, qk_total, created_at, updated_at, deleted_at"#,
        )
        .bind(req.qk_date).bind(req.qk_income).bind(req.qk_outgo)
        .bind(req.qk_amount).bind(req.qk_total)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateSettlementCashRequest,
        user_id: i64,
    ) -> Result<Option<SettlementCash>, ApiError> {
        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if req.qk_date.is_some() {
            idx += 1;
            sets.push(format!("qk_date = ${}", idx));
        }
        if req.qk_income.is_some() {
            idx += 1;
            sets.push(format!("qk_income = ${}", idx));
        }
        if req.qk_outgo.is_some() {
            idx += 1;
            sets.push(format!("qk_outgo = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_stat-smt-cash" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, qk_date, qk_income, qk_outgo, qk_amount, qk_total, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, SettlementCash>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.qk_date {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_income {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_outgo {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_amount {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_total {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// SettlementChannel — "isahl"."zc_id_stat-smt-channel"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct SettlementChannelRepository {
    generic: GenericRepository<SettlementChannel>,
    pool: PgPool,
}

impl SettlementChannelRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for SettlementChannelRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        SettlementChannel,
        CreateSettlementChannelRequest,
        UpdateSettlementChannelRequest,
        ApiError,
    > for SettlementChannelRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<SettlementChannel>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<SettlementChannel>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateSettlementChannelRequest,
        user_id: i64,
    ) -> Result<SettlementChannel, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "SettlementChannel").await?;
        sqlx::query_as::<_, SettlementChannel>(
            r#"INSERT INTO "isahl"."zc_id_stat-smt-channel" (qk_date, qk_income, qk_outgo, qk_amount, qk_total, "qk_exchange-rate", created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING id, qk_date, qk_income, qk_outgo, qk_amount, qk_total, "qk_exchange-rate", created_at, updated_at, deleted_at"#,
        )
        .bind(req.qk_date).bind(req.qk_income).bind(req.qk_outgo)
        .bind(req.qk_amount).bind(req.qk_total).bind(req.qk_exchange_rate)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateSettlementChannelRequest,
        user_id: i64,
    ) -> Result<Option<SettlementChannel>, ApiError> {
        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if req.qk_date.is_some() {
            idx += 1;
            sets.push(format!("qk_date = ${}", idx));
        }
        if req.qk_income.is_some() {
            idx += 1;
            sets.push(format!("qk_income = ${}", idx));
        }
        if req.qk_outgo.is_some() {
            idx += 1;
            sets.push(format!("qk_outgo = ${}", idx));
        }

        if req.qk_exchange_rate.is_some() {
            idx += 1;
            sets.push(format!("qk_exchange-rate = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_stat-smt-channel" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, qk_date, qk_income, qk_outgo, qk_amount, qk_total, "qk_exchange-rate", created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, SettlementChannel>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.qk_date {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_income {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_outgo {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_amount {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_total {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_exchange_rate {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// InventorySales Repository — zc_id_production_rr_storage（库存统计关系）
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct InventorySalesRepository {
    generic: GenericRepository<InventorySales>,
    pool: PgPool,
}

impl InventorySalesRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 校验 ref_left：必须是 zc_id_production 的一个有效成员
    async fn validate_ref_left(&self, fk: Option<i64>) -> Result<(), ApiError> {
        if let Some(prod_id) = fk {
            let exists: (bool,) = sqlx::query_as(
                r#"SELECT EXISTS(SELECT 1 FROM isahl.zc_id_production WHERE id = $1 AND deleted_at IS NULL)"#
            )
            .bind(prod_id)
            .fetch_one(&self.pool)
            .await
            .map_err(ApiError::from)?;

            if !exists.0 {
                return Err(ApiError::Validation {
                    field: "ref_left".into(),
                    message: format!(
                        "无效的 production 引用: ref_left={} 不存在或已被删除",
                        prod_id
                    ),
                });
            }
        }
        Ok(())
    }
}

impl From<PgPool> for InventorySalesRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        InventorySales,
        CreateInventorySalesRequest,
        UpdateInventorySalesRequest,
        ApiError,
    > for InventorySalesRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<InventorySales>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<InventorySales>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateInventorySalesRequest,
        user_id: i64,
    ) -> Result<InventorySales, ApiError> {
        // 校验 ref_left 必须指向 zc_id_production 的一个有效成员
        self.validate_ref_left(req.ref_left).await?;

        sqlx::query_as::<_, InventorySales>(
            r#"INSERT INTO "isahl"."zc_id_production_rr_storage" (code, notice, comments, ref_left, ref_right, qk_p_capacity, qk_qty, sk_unit, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING id, code, notice, comments, ref_left, ref_right, qk_p_capacity, qk_qty, sk_unit, created_at, updated_at, deleted_at"#,
        )
        .bind(req.code)
        .bind(req.notice)
        .bind(req.comments)
        .bind(req.ref_left)
        .bind(req.ref_right)
        .bind(req.qk_p_capacity)
        .bind(req.qk_qty)
        .bind(req.sk_unit)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateInventorySalesRequest,
        user_id: i64,
    ) -> Result<Option<InventorySales>, ApiError> {
        // 校验 ref_left 必须指向 zc_id_production 的一个有效成员
        self.validate_ref_left(req.ref_left).await?;

        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if let Some(ref _v) = req.code {
            idx += 1;
            sets.push(format!("code = ${}", idx));
        }
        if let Some(ref _v) = req.notice {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
        if let Some(ref _v) = req.comments {
            idx += 1;
            sets.push(format!("comments = ${}", idx));
        }
        if let Some(_v) = &req.ref_left {
            idx += 1;
            sets.push(format!("ref_left = ${}", idx));
        }
        if let Some(_v) = &req.ref_right {
            idx += 1;
            sets.push(format!("ref_right = ${}", idx));
        }
        if let Some(_v) = &req.qk_p_capacity {
            idx += 1;
            sets.push(format!("qk_p_capacity = ${}", idx));
        }
        if let Some(_v) = &req.qk_qty {
            idx += 1;
            sets.push(format!("qk_qty = ${}", idx));
        }
        if let Some(_v) = &req.sk_unit {
            idx += 1;
            sets.push(format!("sk_unit = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;
        let sql = format!(
            r#"UPDATE "isahl"."zc_id_production_rr_storage" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, ref_left, ref_right, qk_p_capacity, qk_qty, sk_unit, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );
        let mut q = sqlx::query_as::<_, InventorySales>(AssertSqlSafe(sql.as_str()));
        if let Some(v) = &req.code {
            q = q.bind(v);
        }
        if let Some(v) = &req.notice {
            q = q.bind(v);
        }
        if let Some(v) = &req.comments {
            q = q.bind(v);
        }
        if let Some(v) = &req.ref_left {
            q = q.bind(v);
        }
        if let Some(v) = &req.ref_right {
            q = q.bind(v);
        }
        if let Some(v) = &req.qk_p_capacity {
            q = q.bind(v);
        }
        if let Some(v) = &req.qk_qty {
            q = q.bind(v);
        }
        if let Some(v) = &req.sk_unit {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// Fence Repository
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct SealRepository {
    generic: GenericRepository<Seal>,
    pool: PgPool,
}

impl SealRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 按 id 取完整实体（含引用解析 _refs）
    pub async fn get_refs(&self, id: i64) -> Result<Option<Seal>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    /// 批量创建铅封（add-wz-seal-batch-creation；refactor-dispatch-seal-code-generation 重构）
    ///
    /// - `sealType` 类型 code（固定常量列表；即编号前缀，`ck_category` 置 NULL——字典空，类型由 code 承载）
    /// - `count` 1..=100，缺省 1（1=单号、N=连号）
    /// - `startCode` 缺省 → code 前缀自动续号：取该前缀现有最大尾部序号 +1 等宽 4 位起
    /// - `startCode` 显式 → 起始号等宽递增（铅封管理页手输场景保留）
    /// - 事务内逐号查重，任一冲突整体回滚（400 + 冲突号清单）
    pub async fn batch_create(
        &self,
        req: CreateSealBatchRequest,
        user_id: i64,
    ) -> Result<Vec<Seal>, ApiError> {
        let seal_type = req
            .seal_type
            .as_deref()
            .ok_or_else(|| ApiError::BadRequest("铅封类型必填".to_string()))?;
        // 前缀仅字母数字（防 LIKE 通配注入；固定常量或手输均受此约束）
        if seal_type.is_empty() || !seal_type.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(ApiError::BadRequest(format!("无效的铅封类型: {seal_type}")));
        }
        let count = req.count.unwrap_or(1);
        if !(1..=100).contains(&count) {
            return Err(ApiError::BadRequest("批量数量须在 1-100 之间".to_string()));
        }

        // 编号在事务内计算（自动续号与查重同事务，防并发同前缀重号）
        let mut tx = self.pool.begin().await.map_err(ApiError::from)?;
        let codes = match req.start_code.as_deref() {
            Some(start_code) => Self::codes_from_start(start_code, count)?,
            None => {
                self.next_codes_for_prefix(seal_type, count, &mut *tx)
                    .await?
            }
        };

        // 逐号查重 → 冲突整体回滚；无冲突批量插入
        let mut conflicts: Vec<String> = Vec::new();
        for code in &codes {
            let exists: Option<i64> = sqlx::query_scalar(
                r#"SELECT id FROM "isahl"."zc_id_devi-seal"
                       WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
            )
            .bind(code)
            .fetch_optional(&mut *tx)
            .await
            .map_err(ApiError::from)?;
            if exists.is_some() {
                conflicts.push(code.clone());
            }
        }
        if !conflicts.is_empty() {
            return Err(ApiError::BadRequest(format!(
                "铅封号已被使用: {}",
                conflicts.join(", ")
            )));
        }
        let mut items = Vec::with_capacity(codes.len());
        for code in &codes {
            // 与单条创建一致：waybill_id 存 comments JSON（Seal 无运单列）
            let comments = seal_comments_with_waybill(req.comments.clone(), req.waybill_id);
            let item = sqlx::query_as::<_, Seal>(
                    r#"INSERT INTO "isahl"."zc_id_devi-seal" (notice, code, comments, ck_category, created_by_id)
                       VALUES ($1, $2, $3, $4, $5)
                       RETURNING id, notice, code, comments, ck_category, created_at, updated_at, deleted_at"#,
                )
                .bind(&req.notice)
                .bind(code)
                .bind(comments)
                .bind(Option::<i64>::None)
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::from)?;
            items.push(item);
        }
        tx.commit().await.map_err(ApiError::from)?;
        Ok(items)
    }

    /// 显式起始号 → 等宽递增 code 序列（尾部数字段扫描，非正则——NO_REGEX_FOR_PARSING 合规）
    fn codes_from_start(start_code: &str, count: i64) -> Result<Vec<String>, ApiError> {
        let digit_start = start_code
            .rfind(|c: char| !c.is_ascii_digit())
            .map(|i| i + 1)
            .unwrap_or(0);
        let num_str = &start_code[digit_start..];
        if num_str.is_empty() {
            return Err(ApiError::BadRequest(
                "起始铅封号须以数字结尾（如 SEAL-0001）".to_string(),
            ));
        }
        let start_num: i64 = num_str
            .parse()
            .map_err(|_| ApiError::BadRequest("起始铅封号数字段无效".to_string()))?;
        let width = num_str.len();
        let prefix = &start_code[..digit_start];
        let mut codes = Vec::with_capacity(count as usize);
        for i in 0..count {
            let next = format!("{:0width$}", start_num + i);
            if next.len() > width {
                return Err(ApiError::BadRequest(format!(
                    "起始号 {start_code} 连号超出等宽数字段（第 {} 个）",
                    i + 1
                )));
            }
            codes.push(format!("{prefix}{next}"));
        }
        Ok(codes)
    }

    /// code 前缀自动续号：取 `<prefix>-` 现有最大尾部序号 +1 起 count 个
    /// （等宽 4 位起、溢出扩宽；尾部数字段扫描，非正则——NO_REGEX_FOR_PARSING 合规）
    async fn next_codes_for_prefix<'e, E>(
        &self,
        seal_type: &str,
        count: i64,
        executor: E,
    ) -> Result<Vec<String>, ApiError>
    where
        E: sqlx::PgExecutor<'e>,
    {
        let like = format!("{seal_type}-%");
        let rows: Vec<(String,)> = sqlx::query_as(
            r#"SELECT code FROM "isahl"."zc_id_devi-seal"
                   WHERE code LIKE $1 AND deleted_at IS NULL"#,
        )
        .bind(like)
        .fetch_all(executor)
        .await
        .map_err(ApiError::from)?;
        let mut max_num: i64 = 0;
        let mut width = 4usize;
        for (code,) in &rows {
            let tail = &code[seal_type.len() + 1..];
            let digit_start = tail
                .rfind(|c: char| !c.is_ascii_digit())
                .map(|i| i + 1)
                .unwrap_or(0);
            let num_str = &tail[digit_start..];
            if let Ok(n) = num_str.parse::<i64>() {
                if n > max_num {
                    max_num = n;
                    width = width.max(num_str.len());
                }
            }
        }
        let mut codes = Vec::with_capacity(count as usize);
        for i in 0..count {
            let n = max_num + 1 + i;
            let pad = width.max(n.to_string().len());
            codes.push(format!("{seal_type}-{:0pad$}", n));
        }
        Ok(codes)
    }
}

impl From<PgPool> for SealRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl AliothRepository<Seal, CreateSealRequest, UpdateSealRequest, ApiError> for SealRepository {
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Seal>, ApiError> {
        self.generic.list_refs(query).await
    }
    async fn get(&self, id: i64) -> Result<Option<Seal>, ApiError> {
        self.generic.get_refs(id, None).await
    }
    async fn create(&self, req: CreateSealRequest, user_id: i64) -> Result<Seal, ApiError> {
        // 批注轮 69：关联运单（waybill_id）存 comments JSON——Seal 无运单列（不改 DDL）
        let comments = seal_comments_with_waybill(req.comments.clone(), req.waybill_id);
        sqlx::query_as::<_, Seal>(
            r#"INSERT INTO "isahl"."zc_id_devi-seal" (notice, code, comments, ck_category, created_by_id)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id, notice, code, comments, ck_category, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(comments)
        .bind(Option::<i64>::None)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }
    async fn update(
        &self,
        id: i64,
        req: UpdateSealRequest,
        user_id: i64,
    ) -> Result<Option<Seal>, ApiError> {
        let current = self.get(id).await?;
        let Some(mut entity) = current else {
            return Ok(None);
        };
        if let Some(v) = req.notice {
            entity.notice = Some(v);
        }
        if let Some(v) = req.code {
            entity.code = Some(v);
        }
        if let Some(v) = req.comments {
            entity.comments = Some(v);
        }
        // 批注轮 69：waybill_id 更新——comments JSON merge（保留原 comments）
        if req.waybill_id.is_some() {
            entity.comments = seal_comments_with_waybill(entity.comments.clone(), req.waybill_id);
        }
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_devi-seal" SET notice = $1, code = $2, comments = $3,
               ck_category = $4, updated_at = NOW(), updated_by_id = $5 WHERE id = $6"#,
        )
        .bind(&entity.notice)
        .bind(&entity.code)
        .bind(&entity.comments)
        .bind(entity.ck_category)
        .bind(user_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(ApiError::from)?;
        Ok(Some(entity))
    }
    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

/// 围栏几何类型（add-fence-geometry-types）：circle=方圆、area=区域、polygon=自定义多边形。
/// 类型决定物理叶表（zc_id_geog-circle / zc_id_geog-area / zc_id_geog-polygon），
/// 创建后不可变更——换类型必须删除重建，禁止隐式迁移或回退兜底。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FenceKind {
    Circle,
    Area,
    Polygon,
}

impl FenceKind {
    /// 请求侧 `fence_type` 解析：None → circle（缺省）；非法值 → 400。
    fn from_request(v: Option<&str>) -> Result<Self, ApiError> {
        match v {
            None | Some("circle") => Ok(Self::Circle),
            Some("area") => Ok(Self::Area),
            Some("polygon") => Ok(Self::Polygon),
            Some(other) => Err(ApiError::Validation {
                field: "fence_type".into(),
                message: format!("非法围栏类型 `{other}`（允许 circle/area/polygon）"),
            }),
        }
    }
}

/// 三叶表跨类型 SELECT（UNION ALL，分支字面量回标 fence_type）。
/// 经继承根读子表专有列不可行（父表 SELECT 无子列），故逐叶表 SELECT 后 UNION。
const FENCE_UNION_SELECT: &str = r#"SELECT 'circle' AS fence_type, id, notice, code, comments,
              sk_unit, t_color_, created_at, updated_at, deleted_at,
              ST_AsGeoJSON(circle)::jsonb AS circle, ST_AsGeoJSON(circle)::jsonb AS geometry,
              (SELECT sd.mark::bigint FROM "isahl"."zc_id_scal-distance" sd
               WHERE sd.id = c.qk_radius AND sd.deleted_at IS NULL) AS qk_radius
            FROM "isahl"."zc_id_geog-circle" c WHERE c.deleted_at IS NULL
            UNION ALL
            SELECT 'area', id, notice, code, comments, sk_unit, t_color_,
                   created_at, updated_at, deleted_at,
                   NULL::jsonb, ST_AsGeoJSON(box)::jsonb, NULL::bigint
            FROM "isahl"."zc_id_geog-area" WHERE deleted_at IS NULL
            UNION ALL
            SELECT 'polygon', id, notice, code, comments, sk_unit, t_color_,
                   created_at, updated_at, deleted_at,
                   NULL::jsonb, ST_AsGeoJSON(polygon)::jsonb, NULL::bigint
            FROM "isahl"."zc_id_geog-polygon" WHERE deleted_at IS NULL"#;

/// bounds JSON → 对角两点（sw_lng, sw_lat, ne_lng, ne_lat）；area 类型专用，缺失/非法 → 400。
fn normalize_area_bounds(v: &Value) -> Result<(f64, f64, f64, f64), ApiError> {
    let obj = v.as_object().ok_or_else(|| ApiError::Validation {
        field: "bounds".into(),
        message: "bounds 必须为对象 {southwest:{lng,lat}, northeast:{lng,lat}}".into(),
    })?;
    let point = |key: &str| -> Result<(f64, f64), ApiError> {
        let p = obj.get(key).ok_or_else(|| ApiError::Validation {
            field: "bounds".into(),
            message: format!("bounds 缺少 {key}"),
        })?;
        let num = |name: &str| -> Result<f64, ApiError> {
            p.get(name)
                .and_then(Value::as_f64)
                .ok_or_else(|| ApiError::Validation {
                    field: "bounds".into(),
                    message: format!("bounds.{key} 缺少 {name}"),
                })
        };
        Ok((num("lng")?, num("lat")?))
    };
    let (sw_lng, sw_lat) = point("southwest")?;
    let (ne_lng, ne_lat) = point("northeast")?;
    if !(sw_lng < ne_lng && sw_lat < ne_lat) {
        return Err(ApiError::Validation {
            field: "bounds".into(),
            message: "southwest 必须严格小于 northeast（对角两点）".into(),
        });
    }
    Ok((sw_lng, sw_lat, ne_lng, ne_lat))
}

/// points JSON → 顶点数组（≥3 点）；polygon 类型专用，缺失/不足 → 400。
fn normalize_polygon_points(v: &Value) -> Result<Vec<(f64, f64)>, ApiError> {
    let arr = v.as_array().ok_or_else(|| ApiError::Validation {
        field: "points".into(),
        message: "points 必须为数组 [{lng,lat},...]".into(),
    })?;
    let mut pts = Vec::with_capacity(arr.len());
    for p in arr {
        let lng = p
            .get("lng")
            .and_then(Value::as_f64)
            .ok_or_else(|| ApiError::Validation {
                field: "points".into(),
                message: "顶点缺少 lng".into(),
            })?;
        let lat = p
            .get("lat")
            .and_then(Value::as_f64)
            .ok_or_else(|| ApiError::Validation {
                field: "points".into(),
                message: "顶点缺少 lat".into(),
            })?;
        pts.push((lng, lat));
    }
    if pts.len() < 3 {
        return Err(ApiError::Validation {
            field: "points".into(),
            message: format!("自定义多边形至少需要 3 个顶点（当前 {} 个）", pts.len()),
        });
    }
    Ok(pts)
}

/// 顶点 → PostGIS POLYGON WKT（环显式闭合；已闭合输入不重复追加）。
fn polygon_wkt(pts: &[(f64, f64)]) -> String {
    let mut ring: Vec<String> = pts
        .iter()
        .map(|(lng, lat)| format!("{lng} {lat}"))
        .collect();
    if !(pts.len() > 1 && pts[0] == pts[pts.len() - 1]) {
        if let Some((lng, lat)) = pts.first() {
            ring.push(format!("{lng} {lat}"));
        }
    }
    format!("POLYGON(({}))", ring.join(","))
}

#[derive(Clone)]
pub struct FenceRepository {
    pool: PgPool,
}

impl FenceRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// radius → zc_id_scal-distance 标量行 find-or-create（mark=半径值，notice=语义标签）。
    /// 同 mark 未删行复用（幂等），否则新建标量行，返回行 id 供 qk_radius 引用。
    async fn upsert_distance_scalar(&self, radius: f64, user_id: i64) -> Result<i64, ApiError> {
        if let Some(id) = sqlx::query_scalar::<_, i64>(
            r#"SELECT id FROM "isahl"."zc_id_scal-distance" WHERE mark = $1 AND deleted_at IS NULL LIMIT 1"#,
        )
        .bind(radius)
        .fetch_optional(&self.pool)
        .await
        .map_err(ApiError::from)?
        {
            return Ok(id);
        }
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_scal-distance" (id, mark, notice, created_by_id)
               VALUES (isahl.gen_next_uid(), $1, $2, $3) RETURNING id"#,
        )
        .bind(radius)
        .bind(format!("围栏半径 {radius}"))
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;
        Ok(id)
    }

    /// tableoid 探测围栏行所在物理叶表（继承根 SELECT 跨全部叶表后代）。
    /// 未命中（不存在/已删）→ NotFound；禁止「逐表试写」式回退。
    async fn probe_fence_kind(&self, id: i64) -> Result<FenceKind, ApiError> {
        let row = sqlx::query_as::<_, (bool, bool)>(
            r#"SELECT tableoid = 'isahl."zc_id_geog-circle"'::regclass,
                      tableoid = 'isahl."zc_id_geog-area"'::regclass
               FROM "isahl"."zc_id_geometry" WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(ApiError::from)?;
        match row {
            Some((true, _)) => Ok(FenceKind::Circle),
            Some((false, true)) => Ok(FenceKind::Area),
            Some((false, false)) => Ok(FenceKind::Polygon),
            None => Err(ApiError::NotFound(format!("fence {id} not found"))),
        }
    }
    /// 方圆围栏 → zc_id_geog-circle（圆心 Point 几何 + qk_radius 半径标量引用）。
    async fn create_circle(
        &self,
        req: CreateFenceRequest,
        user_id: i64,
    ) -> Result<Fence, ApiError> {
        // sk_unit 语义 = 图商坐标系（zc_id_unit-geo）；radius → zc_id_scal-distance 标量行（qk_radius 引用）
        let coord_sys_id =
            resolve_coord_sys_id(&self.pool, req.coord_sys.as_deref(), req.sk_unit).await?;
        let comments = req.comments.clone();
        // 圆心：PostGIS geometry(Point,4326) 直存 circle 列（不建 coordinate 引用行）
        let (centre_lng, centre_lat) = match &req.circle {
            Some(v) => normalize_circle_point(v)?,
            None => {
                return Err(ApiError::Validation {
                    field: "circle".into(),
                    message: "方圆围栏必须提供圆心 circle".into(),
                });
            }
        };
        // 半径：find-or-create zc_id_scal-distance 标量行（mark=半径值），qk_radius 引用
        let qk_radius = match req.radius {
            Some(r) => Some(self.upsert_distance_scalar(r, user_id).await?),
            None => None,
        };
        sqlx::query_as::<_, Fence>(
            r#"INSERT INTO "isahl"."zc_id_geog-circle" (notice, code, comments, circle, sk_unit, qk_radius, t_color_, created_by_id)
               VALUES ($1, $2, $3, ST_SetSRID(ST_MakePoint($4, $5), 4326), $6, $7, $8, $9)
               RETURNING id, notice, code, comments,
                         to_jsonb(circle) as circle, 'circle' AS fence_type,
                         ST_AsGeoJSON(circle)::jsonb AS geometry, sk_unit,
                         (SELECT sd.mark::bigint FROM "isahl"."zc_id_scal-distance" sd
                          WHERE sd.id = qk_radius AND sd.deleted_at IS NULL) AS qk_radius,
                         t_color_, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.notice).bind(&req.code).bind(comments.as_deref())
        .bind(centre_lng).bind(centre_lat)
        .bind(coord_sys_id).bind(qk_radius).bind(&req.t_color_)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }
    /// 区域围栏（对角两点矩形）→ zc_id_geog-area（box 列存闭合矩形环）。
    async fn create_area(&self, req: CreateFenceRequest, user_id: i64) -> Result<Fence, ApiError> {
        let coord_sys_id =
            resolve_coord_sys_id(&self.pool, req.coord_sys.as_deref(), req.sk_unit).await?;
        let (sw_lng, sw_lat, ne_lng, ne_lat) = match &req.bounds {
            Some(v) => normalize_area_bounds(v)?,
            None => {
                return Err(ApiError::Validation {
                    field: "bounds".into(),
                    message: "区域围栏必须提供对角两点 bounds".into(),
                });
            }
        };
        let wkt = format!(
            "POLYGON(({sw_lng} {sw_lat},{ne_lng} {sw_lat},{ne_lng} {ne_lat},{sw_lng} {ne_lat},{sw_lng} {sw_lat}))"
        );
        sqlx::query_as::<_, Fence>(
            r#"INSERT INTO "isahl"."zc_id_geog-area" (notice, code, comments, box, sk_unit, t_color_, created_by_id)
               VALUES ($1, $2, $3, ST_SetSRID(ST_GeomFromText($4), 4326), $5, $6, $7)
               RETURNING id, notice, code, comments, NULL::jsonb AS circle, 'area' AS fence_type,
                         ST_AsGeoJSON(box)::jsonb AS geometry, NULL::bigint AS qk_radius,
                         sk_unit, t_color_, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.notice).bind(&req.code).bind(req.comments.as_deref())
        .bind(wkt)
        .bind(coord_sys_id).bind(&req.t_color_)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    /// 自定义多边形围栏（≥3 点）→ zc_id_geog-polygon（polygon 列存闭合环）。
    async fn create_polygon(
        &self,
        req: CreateFenceRequest,
        user_id: i64,
    ) -> Result<Fence, ApiError> {
        let coord_sys_id =
            resolve_coord_sys_id(&self.pool, req.coord_sys.as_deref(), req.sk_unit).await?;
        let wkt = match &req.points {
            Some(v) => polygon_wkt(&normalize_polygon_points(v)?),
            None => {
                return Err(ApiError::Validation {
                    field: "points".into(),
                    message: "自定义多边形围栏必须提供顶点 points（≥3 点）".into(),
                });
            }
        };
        sqlx::query_as::<_, Fence>(
            r#"INSERT INTO "isahl"."zc_id_geog-polygon" (notice, code, comments, polygon, sk_unit, t_color_, created_by_id)
               VALUES ($1, $2, $3, ST_SetSRID(ST_GeomFromText($4), 4326), $5, $6, $7)
               RETURNING id, notice, code, comments, NULL::jsonb AS circle, 'polygon' AS fence_type,
                         ST_AsGeoJSON(polygon)::jsonb AS geometry, NULL::bigint AS qk_radius,
                         sk_unit, t_color_, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.notice).bind(&req.code).bind(req.comments.as_deref())
        .bind(wkt)
        .bind(coord_sys_id).bind(&req.t_color_)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }
    /// 方圆围栏更新 → zc_id_geog-circle（动态 SET，仅写请求携带的列）。
    async fn update_circle(
        &self,
        id: i64,
        req: UpdateFenceRequest,
        user_id: i64,
    ) -> Result<Option<Fence>, ApiError> {
        // sk_unit 语义 = 图商坐标系（zc_id_unit-geo）
        let coord_sys_id =
            resolve_coord_sys_id(&self.pool, req.coord_sys.as_deref(), req.sk_unit).await?;
        // 圆心：直写 circle 几何列（PostGIS geometry(Point,4326)）
        let centre_lnglat: Option<(f64, f64)> = match &req.circle {
            Some(v) => Some(normalize_circle_point(v)?),
            None => None,
        };
        // 半径更新：Some(r) → find-or-create scal-distance 行并更新 qk_radius 引用；None → 不动
        let radius_to_set: Option<i64> = match req.radius {
            Some(r) => Some(self.upsert_distance_scalar(r, user_id).await?),
            None => None,
        };
        let radius_is_some = radius_to_set.is_some();

        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if req.notice.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
        if req.code.is_some() {
            idx += 1;
            sets.push(format!("code = ${}", idx));
        }
        let comments_to_set: Option<&str> = req.comments.as_deref();
        if comments_to_set.is_some() {
            idx += 1;
            sets.push(format!("comments = ${}", idx));
        }
        if centre_lnglat.is_some() {
            idx += 1;
            sets.push(format!(
                "circle = ST_SetSRID(ST_MakePoint(${}, ${}), 4326)",
                idx,
                idx + 1
            ));
            idx += 1;
        }
        if req.sk_unit.is_some() || req.coord_sys.is_some() {
            idx += 1;
            sets.push(format!("sk_unit = ${}", idx));
        }
        if radius_is_some {
            idx += 1;
            sets.push(format!("qk_radius = ${}", idx));
        }
        if req.t_color_.is_some() {
            idx += 1;
            sets.push(format!("t_color_ = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        // qk_radius 输出与 create/list/get 对齐：mark 直出（km），非行 id——读回断裂修复契约统一
        let sql = format!(
            r#"UPDATE "isahl"."zc_id_geog-circle" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, notice, code, comments, NULL::jsonb AS circle, 'circle' AS fence_type,
                         ST_AsGeoJSON(circle)::jsonb AS geometry, sk_unit,
                         (SELECT sd.mark::bigint FROM "isahl"."zc_id_scal-distance" sd
                          WHERE sd.id = "isahl"."zc_id_geog-circle".qk_radius AND sd.deleted_at IS NULL) AS qk_radius,
                         t_color_, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, Fence>(AssertSqlSafe(sql.as_str()));
        if let Some(v) = &req.notice {
            q = q.bind(v);
        }
        if let Some(v) = &req.code {
            q = q.bind(v);
        }
        if let Some(v) = comments_to_set {
            q = q.bind(v);
        }
        if let Some((lng, lat)) = centre_lnglat {
            q = q.bind(lng);
            q = q.bind(lat);
        }
        if req.sk_unit.is_some() || req.coord_sys.is_some() {
            q = q.bind(coord_sys_id);
        }
        if radius_is_some {
            q = q.bind(radius_to_set.expect("radius_is_some 保证 Some"));
        }
        if let Some(v) = &req.t_color_ {
            q = q.bind(v);
        }

        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    /// 区域围栏更新 → zc_id_geog-area（未携带字段回填既有值，静态 SQL 全列覆盖）。
    async fn update_area(
        &self,
        id: i64,
        req: UpdateFenceRequest,
        user_id: i64,
    ) -> Result<Option<Fence>, ApiError> {
        let existing = self
            .get(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("fence {id} not found")))?;
        let wkt: Option<String> = match &req.bounds {
            Some(v) => {
                let (sw_lng, sw_lat, ne_lng, ne_lat) = normalize_area_bounds(v)?;
                Some(format!(
                    "POLYGON(({sw_lng} {sw_lat},{ne_lng} {sw_lat},{ne_lng} {ne_lat},{sw_lng} {ne_lat},{sw_lng} {sw_lat}))"
                ))
            }
            None => None,
        };
        if wkt.is_none()
            && req.notice.is_none()
            && req.code.is_none()
            && req.comments.is_none()
            && req.t_color_.is_none()
            && req.sk_unit.is_none()
            && req.coord_sys.is_none()
        {
            return Ok(Some(existing));
        }
        let notice = req.notice.or(existing.notice);
        let code = req.code.or(existing.code);
        let comments = req.comments.or(existing.comments);
        let t_color_ = req.t_color_.or(existing.t_color_);
        let sk_unit: Option<i64> = if req.sk_unit.is_some() || req.coord_sys.is_some() {
            Some(resolve_coord_sys_id(&self.pool, req.coord_sys.as_deref(), req.sk_unit).await?)
        } else {
            existing.sk_unit
        };
        match wkt {
            Some(wkt) => sqlx::query_as::<_, Fence>(
                r#"UPDATE "isahl"."zc_id_geog-area"
                       SET notice=$1, code=$2, comments=$3, sk_unit=$4, t_color_=$5,
                           box=ST_SetSRID(ST_GeomFromText($6), 4326),
                           updated_at=NOW(), updated_by_id=$7
                       WHERE id=$8 AND deleted_at IS NULL
                       RETURNING id, notice, code, comments, NULL::jsonb AS circle,
                                 'area' AS fence_type, ST_AsGeoJSON(box)::jsonb AS geometry,
                                 NULL::bigint AS qk_radius, sk_unit, t_color_,
                                 created_at, updated_at, deleted_at"#,
            )
            .bind(notice)
            .bind(code)
            .bind(comments)
            .bind(sk_unit)
            .bind(t_color_)
            .bind(wkt)
            .bind(user_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(ApiError::from),
            None => sqlx::query_as::<_, Fence>(
                r#"UPDATE "isahl"."zc_id_geog-area"
                       SET notice=$1, code=$2, comments=$3, sk_unit=$4, t_color_=$5,
                           updated_at=NOW(), updated_by_id=$6
                       WHERE id=$7 AND deleted_at IS NULL
                       RETURNING id, notice, code, comments, NULL::jsonb AS circle,
                                 'area' AS fence_type, ST_AsGeoJSON(box)::jsonb AS geometry,
                                 NULL::bigint AS qk_radius, sk_unit, t_color_,
                                 created_at, updated_at, deleted_at"#,
            )
            .bind(notice)
            .bind(code)
            .bind(comments)
            .bind(sk_unit)
            .bind(t_color_)
            .bind(user_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(ApiError::from),
        }
    }

    /// 自定义多边形围栏更新 → zc_id_geog-polygon（未携带字段回填既有值，静态 SQL 全列覆盖）。
    async fn update_polygon(
        &self,
        id: i64,
        req: UpdateFenceRequest,
        user_id: i64,
    ) -> Result<Option<Fence>, ApiError> {
        let existing = self
            .get(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("fence {id} not found")))?;
        let wkt: Option<String> = match &req.points {
            Some(v) => Some(polygon_wkt(&normalize_polygon_points(v)?)),
            None => None,
        };
        if wkt.is_none()
            && req.notice.is_none()
            && req.code.is_none()
            && req.comments.is_none()
            && req.t_color_.is_none()
            && req.sk_unit.is_none()
            && req.coord_sys.is_none()
        {
            return Ok(Some(existing));
        }
        let notice = req.notice.or(existing.notice);
        let code = req.code.or(existing.code);
        let comments = req.comments.or(existing.comments);
        let t_color_ = req.t_color_.or(existing.t_color_);
        let sk_unit: Option<i64> = if req.sk_unit.is_some() || req.coord_sys.is_some() {
            Some(resolve_coord_sys_id(&self.pool, req.coord_sys.as_deref(), req.sk_unit).await?)
        } else {
            existing.sk_unit
        };
        match wkt {
            Some(wkt) => sqlx::query_as::<_, Fence>(
                r#"UPDATE "isahl"."zc_id_geog-polygon"
                       SET notice=$1, code=$2, comments=$3, sk_unit=$4, t_color_=$5,
                           polygon=ST_SetSRID(ST_GeomFromText($6), 4326),
                           updated_at=NOW(), updated_by_id=$7
                       WHERE id=$8 AND deleted_at IS NULL
                       RETURNING id, notice, code, comments, NULL::jsonb AS circle,
                                 'polygon' AS fence_type, ST_AsGeoJSON(polygon)::jsonb AS geometry,
                                 NULL::bigint AS qk_radius, sk_unit, t_color_,
                                 created_at, updated_at, deleted_at"#,
            )
            .bind(notice)
            .bind(code)
            .bind(comments)
            .bind(sk_unit)
            .bind(t_color_)
            .bind(wkt)
            .bind(user_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(ApiError::from),
            None => sqlx::query_as::<_, Fence>(
                r#"UPDATE "isahl"."zc_id_geog-polygon"
                       SET notice=$1, code=$2, comments=$3, sk_unit=$4, t_color_=$5,
                           updated_at=NOW(), updated_by_id=$6
                       WHERE id=$7 AND deleted_at IS NULL
                       RETURNING id, notice, code, comments, NULL::jsonb AS circle,
                                 'polygon' AS fence_type, ST_AsGeoJSON(polygon)::jsonb AS geometry,
                                 NULL::bigint AS qk_radius, sk_unit, t_color_,
                                 created_at, updated_at, deleted_at"#,
            )
            .bind(notice)
            .bind(code)
            .bind(comments)
            .bind(sk_unit)
            .bind(t_color_)
            .bind(user_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(ApiError::from),
        }
    }
}

impl From<PgPool> for FenceRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<Fence, CreateFenceRequest, UpdateFenceRequest, ApiError> for FenceRepository {
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Fence>, ApiError> {
        // add-fence-geometry-types：三叶表 UNION 跨类型列表（分支字面量回标 fence_type，
        // radius 经 qk_radius → zc_id_scal-distance 子查询直出 mark）。
        // 现调用方仅用分页参数；排序固定 created_at DESC（新建在前）。
        let total: i64 = sqlx::query_scalar(
            r#"SELECT (SELECT count(*) FROM "isahl"."zc_id_geog-circle" WHERE deleted_at IS NULL)
                    + (SELECT count(*) FROM "isahl"."zc_id_geog-area" WHERE deleted_at IS NULL)
                    + (SELECT count(*) FROM "isahl"."zc_id_geog-polygon" WHERE deleted_at IS NULL)"#,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;
        let sql = format!(
            "SELECT * FROM ({FENCE_UNION_SELECT}) u \
             ORDER BY created_at DESC, id DESC LIMIT $1 OFFSET $2"
        );
        let items: Vec<Fence> = sqlx::query_as::<_, Fence>(AssertSqlSafe(sql.as_str()))
            .bind(query.page_size)
            .bind(query.offset())
            .fetch_all(&self.pool)
            .await
            .map_err(ApiError::from)?;
        Ok(PaginatedResponse::new(
            items,
            total,
            query.page,
            query.page_size,
        ))
    }
    async fn get(&self, id: i64) -> Result<Option<Fence>, ApiError> {
        // 跨叶表按 id 取行（类型由 UNION 分支字面量回标）
        let sql = format!("SELECT * FROM ({FENCE_UNION_SELECT}) u WHERE u.id = $1");
        sqlx::query_as::<_, Fence>(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(ApiError::from)
    }
    async fn create(&self, req: CreateFenceRequest, user_id: i64) -> Result<Fence, ApiError> {
        // add-fence-geometry-types：类型分派叶表（缺省 circle；非法值 400；禁止回退）
        match FenceKind::from_request(req.fence_type.as_deref())? {
            FenceKind::Circle => self.create_circle(req, user_id).await,
            FenceKind::Area => self.create_area(req, user_id).await,
            FenceKind::Polygon => self.create_polygon(req, user_id).await,
        }
    }
    async fn update(
        &self,
        id: i64,
        req: UpdateFenceRequest,
        user_id: i64,
    ) -> Result<Option<Fence>, ApiError> {
        // add-fence-geometry-types：tableoid 探测物理叶表后分支执行，禁止跨表误写与回退兜底
        let kind = self.probe_fence_kind(id).await?;
        if let Some(t) = req.fence_type.as_deref() {
            let wanted = FenceKind::from_request(Some(t))?;
            if wanted != kind {
                return Err(ApiError::Validation {
                    field: "fence_type".into(),
                    message: "围栏类型创建后不可变更（请删除后重建）".into(),
                });
            }
        }
        match kind {
            FenceKind::Circle => self.update_circle(id, req, user_id).await,
            FenceKind::Area => self.update_area(id, req, user_id).await,
            FenceKind::Polygon => self.update_polygon(id, req, user_id).await,
        }
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        // add-fence-geometry-types：探测物理叶表后在该叶表软删（通用路径绑定单表会脱靶）
        let kind = self.probe_fence_kind(id).await?;
        let table = match kind {
            FenceKind::Circle => r#""isahl"."zc_id_geog-circle""#,
            FenceKind::Area => r#""isahl"."zc_id_geog-area""#,
            FenceKind::Polygon => r#""isahl"."zc_id_geog-polygon""#,
        };
        let sql = format!(
            "UPDATE {table} SET deleted_at = NOW(), deleted_by_id = $1 WHERE id = $2 AND deleted_at IS NULL"
        );
        let rows = sqlx::query(AssertSqlSafe(sql.as_str()))
            .bind(user_id)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(ApiError::from)?
            .rows_affected();
        if rows == 0 {
            return Err(ApiError::NotFound(format!("fence {id} not found")));
        }
        Ok(())
    }
}

// ═══════════════════════════════════════════════
// FreightProduct Repository — zc_id_prod-freight_road-sales
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct FreightProductRepository {
    generic: GenericRepository<FreightProduct>,
    pool: PgPool,
}

impl FreightProductRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for FreightProductRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        FreightProduct,
        CreateFreightProductRequest,
        UpdateFreightProductRequest,
        ApiError,
    > for FreightProductRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<FreightProduct>, ApiError> {
        self.generic.list_refs(query).await
    }
    async fn get(&self, id: i64) -> Result<Option<FreightProduct>, ApiError> {
        self.generic.get_refs(id, None).await
    }
    async fn create(
        &self,
        req: CreateFreightProductRequest,
        user_id: i64,
    ) -> Result<FreightProduct, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "FreightProduct").await?;

        sqlx::query_as::<_, FreightProduct>(
            r#"INSERT INTO "isahl"."zc_id_prod-freight_road-sales"
               (code, notice, comments, fk_previous, "ck_vehicle-form", qk_price, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING id, code, notice, comments, fk_previous, "ck_vehicle-form", qk_price, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_previous)
        .bind(req.ck_vehicle_form).bind(req.qk_price)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateFreightProductRequest,
        user_id: i64,
    ) -> Result<Option<FreightProduct>, ApiError> {
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
        if req.fk_previous.is_some() {
            idx += 1;
            sets.push(format!("fk_previous = ${}", idx));
        }
        if req.ck_vehicle_form.is_some() {
            idx += 1;
            sets.push(format!("\"ck_vehicle-form\" = ${}", idx));
        }
        if req.qk_price.is_some() {
            idx += 1;
            sets.push(format!("qk_price = ${}", idx));
        }
        if sets.is_empty() {
            return self.get(id).await;
        }
        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_prod-freight_road-sales" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_previous, "ck_vehicle-form", qk_price, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, FreightProduct>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_previous {
            q = q.bind(v);
        }
        if let Some(v) = req.ck_vehicle_form {
            q = q.bind(v);
        }
        if let Some(v) = req.qk_price {
            q = q.bind(v);
        }

        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// TransitRoute Repository
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct TransitRouteRepository {
    generic: GenericRepository<TransitRoute>,
    pool: PgPool,
}

impl TransitRouteRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for TransitRouteRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<TransitRoute, CreateTransitRouteRequest, UpdateTransitRouteRequest, ApiError>
    for TransitRouteRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<TransitRoute>, ApiError> {
        self.generic.list_refs(query).await
    }
    async fn get(&self, id: i64) -> Result<Option<TransitRoute>, ApiError> {
        self.generic.get_refs(id, None).await
    }
    async fn create(
        &self,
        req: CreateTransitRouteRequest,
        user_id: i64,
    ) -> Result<TransitRoute, ApiError> {
        // 方案 B：sk_unit 语义 = 图商坐标系（zc_id_unit-geo）
        let coord_sys_id =
            resolve_coord_sys_id(&self.pool, req.coord_sys.as_deref(), req.sk_unit).await?;
        // Create coordinate records for each waypoint in ak_nodes
        let node_ids: Vec<i64> = if let Some(ref nodes) = req.ak_nodes {
            if let Some(arr) = nodes.as_array() {
                let mut ids = Vec::with_capacity(arr.len());
                for node in arr {
                    if let Some(n) = node.as_i64() {
                        ids.push(n);
                    } else if let (Some(lat), Some(lng)) = (
                        node.get("lat").and_then(|v| v.as_f64()),
                        node.get("lng").and_then(|v| v.as_f64()),
                    ) {
                        let name = node
                            .get("notice")
                            .and_then(|v| v.as_str())
                            .unwrap_or("waypoint");
                        let cid: i64 = sqlx::query_scalar(
                            r#"INSERT INTO "isahl"."zc_id_geom-coordinate" (notice, point, sk_unit, created_by_id)
                               VALUES ($1, ST_SetSRID(ST_MakePoint($2, $3), 4326), $4, $5) RETURNING id"#,
                        )
                        .bind(name).bind(lng).bind(lat).bind(coord_sys_id).bind(user_id)
                        .fetch_one(&self.pool)
                        .await
                        .map_err(ApiError::from)?;
                        ids.push(cid);
                    }
                }
                ids
            } else {
                return Err(ApiError::Validation {
                    field: "ak_nodes".into(),
                    message: "must be an array of {lat,lng} objects or coordinate IDs".into(),
                });
            }
        } else {
            Vec::new()
        };
        let created = sqlx::query_as::<_, TransitRoute>(
            r#"INSERT INTO "isahl"."zc_id_geom-path" (notice, code, comments, ak_nodes, sk_unit, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id, notice, code, comments, ak_nodes, sk_unit, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&node_ids)
        .bind(coord_sys_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;
        if let Err(e) = sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_stor-traffic_line" (id, notice, code, comments, qk_path, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1,
                       'TL-' || COALESCE(NULLIF($2, ''), 'R' || (($3) % 1000000)),
                       $4, $3, $5)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(&created.notice)
        .bind(&created.code)
        .bind(created.id)
        .bind(&created.comments)
        .bind(user_id)
        .execute(&self.pool)
        .await
        {
            eprintln!("transit-route → traffic_line 同步失败: {}", e);
        }
        Ok(created)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateTransitRouteRequest,
        user_id: i64,
    ) -> Result<Option<TransitRoute>, ApiError> {
        // 方案 B：sk_unit 语义 = 图商坐标系（zc_id_unit-geo）
        let coord_sys_id =
            resolve_coord_sys_id(&self.pool, req.coord_sys.as_deref(), req.sk_unit).await?;
        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if req.notice.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
        if req.code.is_some() {
            idx += 1;
            sets.push(format!("code = ${}", idx));
        }
        if req.comments.is_some() {
            idx += 1;
            sets.push(format!("comments = ${}", idx));
        }
        if req.ak_nodes.is_some() {
            idx += 1;
            sets.push(format!("ak_nodes = ${}", idx));
        }
        if req.sk_unit.is_some() || req.coord_sys.is_some() {
            idx += 1;
            sets.push(format!("sk_unit = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_geom-path" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, notice, code, comments, ak_nodes, sk_unit, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, TransitRoute>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(ref v) = req.ak_nodes {
            q = q.bind(v);
        }
        if req.sk_unit.is_some() || req.coord_sys.is_some() {
            q = q.bind(coord_sys_id);
        }

        q = q.bind(user_id);
        q = q.bind(id);

        let updated = q.fetch_optional(&self.pool).await.map_err(ApiError::from)?;

        // 同步约束（批注：路线改名委托页不同步）：
        // 名称变更时同步委托页线路（zc_id_stor-traffic_line，qk_path 匹配本路线）
        if let Some(route) = &updated {
            if let Some(new_notice) = &route.notice {
                let _ = sqlx::query(
                    r#"UPDATE "isahl"."zc_id_stor-traffic_line"
                       SET notice = $1, updated_at = NOW()
                       WHERE qk_path = $2 AND deleted_at IS NULL"#,
                )
                .bind(new_notice)
                .bind(id)
                .execute(&self.pool)
                .await;
            }
        }
        Ok(updated)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// TrafficLine Repository — "isahl"."zc_id_stor-traffic_line"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct TrafficLineRepository {
    generic: GenericRepository<TrafficLine>,
    pool: PgPool,
}

impl TrafficLineRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

/// 图商坐标系 → zc_id_unit-geo id（方案 B：sk_unit 语义 = 坐标系）。
///
/// 优先级：`coord_sys` code（WGS84/GCJ-02/BD-09）→ `sk_unit`（直传 id，校验存在）
/// → 缺省 WGS84。非法 code / 悬空 sk_unit → Validation 错误（坐标输入必须明确坐标系）。
async fn resolve_coord_sys_id(
    pool: &PgPool,
    coord_sys: Option<&str>,
    sk_unit: Option<i64>,
) -> Result<i64, ApiError> {
    if let Some(code) = coord_sys {
        let code = code.trim();
        if code.is_empty() {
            return Err(ApiError::Validation {
                field: "coord_sys".into(),
                message: "坐标系 code 不能为空（合法：WGS84/GCJ-02/BD-09）".into(),
            });
        }
        let id = sqlx::query_scalar::<_, i64>(
            r#"SELECT id FROM isahl."zc_id_unit-geo" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
        )
        .bind(code)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::from)?;
        return id.ok_or_else(|| ApiError::Validation {
            field: "coord_sys".into(),
            message: format!("未知坐标系 '{}'（合法：WGS84/GCJ-02/BD-09）", code),
        });
    }
    if let Some(sid) = sk_unit {
        let exists = sqlx::query_scalar::<_, bool>(
            r#"SELECT EXISTS(SELECT 1 FROM isahl."zc_id_unit-geo" WHERE id = $1 AND deleted_at IS NULL)"#,
        )
        .bind(sid)
        .fetch_one(pool)
        .await
        .map_err(ApiError::from)?;
        if !exists {
            return Err(ApiError::Validation {
                field: "sk_unit".into(),
                message: format!("sk_unit {} 不在坐标系字典（zc_id_unit-geo）中", sid),
            });
        }
        return Ok(sid);
    }
    // 缺省：WGS84（GPS 原始坐标）
    let wgs84 = sqlx::query_scalar::<_, i64>(
        r#"SELECT id FROM isahl."zc_id_unit-geo" WHERE code = 'WGS84' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from)?;
    wgs84.ok_or_else(|| ApiError::Validation {
        field: "coord_sys".into(),
        message: "坐标系字典缺失 WGS84（zc_id_unit-geo 未种子）".into(),
    })
}

/// 圆心输入规范化 → (lng, lat)。
/// 接受：GeoJSON Point object（{type:'Point',coordinates:[lng,lat]}）、{lng,lat}、{lat,lng}。
fn normalize_circle_point(val: &serde_json::Value) -> Result<(f64, f64), ApiError> {
    if let Some(coords) = val.get("coordinates").and_then(|c| c.as_array()) {
        if val.get("type").and_then(|t| t.as_str()) == Some("Point") && coords.len() >= 2 {
            let lng = coords[0].as_f64().ok_or_else(|| ApiError::Validation {
                field: "circle".into(),
                message: "circle.coordinates[0]（经度）必须是数字".into(),
            })?;
            let lat = coords[1].as_f64().ok_or_else(|| ApiError::Validation {
                field: "circle".into(),
                message: "circle.coordinates[1]（纬度）必须是数字".into(),
            })?;
            return Ok((lng, lat));
        }
    }
    if let Some(lng) = val.get("lng").and_then(|v| v.as_f64()) {
        let lat = val
            .get("lat")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ApiError::Validation {
                field: "circle".into(),
                message: "circle 缺少 lat（接受 {lng,lat}/{lat,lng}）".into(),
            })?;
        return Ok((lng, lat));
    }
    if let Some(lat) = val.get("lat").and_then(|v| v.as_f64()) {
        let lng = val
            .get("lng")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ApiError::Validation {
                field: "circle".into(),
                message: "circle 缺少 lng（接受 {lng,lat}/{lat,lng}）".into(),
            })?;
        return Ok((lng, lat));
    }
    Err(ApiError::Validation {
        field: "circle".into(),
        message: "circle 必须是 GeoJSON Point object 或 {lng,lat}/{lat,lng} object".into(),
    })
}

async fn enrich_qk_path_ak_nodes(pool: &PgPool, items: &mut [TrafficLine]) -> Result<(), ApiError> {
    let path_ids: Vec<i64> = items
        .iter()
        .filter_map(|t| t.qk_path)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    if path_ids.is_empty() {
        return Ok(());
    }
    let rows: Vec<(i64, serde_json::Value)> = sqlx::query_as(
        // zc_id_geom-coordinate 经纬度在 point 列（EWKB hex，schema 迁移替代 mark_axis0/1）——
        // 批注：traffic-lines 500 根因（c.mark_axis0 不存在）——postgis 解码
        r#"SELECT gp.id,
               COALESCE(
                 (SELECT jsonb_agg(
                    jsonb_build_object('id', c.id, 'name', c.notice,
                                       'lng', ST_X(ST_GeomFromEWKB(decode(c.point, 'hex'))),
                                       'lat', ST_Y(ST_GeomFromEWKB(decode(c.point, 'hex'))))
                    ORDER BY n.ord)
                  FROM unnest(gp.ak_nodes) WITH ORDINALITY AS n(id, ord)
                  LEFT JOIN "isahl"."zc_id_geom-coordinate" c
                    ON c.id = n.id AND c.deleted_at IS NULL),
                 '[]'::jsonb)
           FROM "isahl"."zc_id_geom-path" gp
           WHERE gp.id = ANY($1) AND gp.deleted_at IS NULL"#,
    )
    .bind(&path_ids)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    let mut by_path: std::collections::HashMap<i64, serde_json::Value> = rows.into_iter().collect();
    for item in items.iter_mut() {
        let Some(pid) = item.qk_path else { continue };
        let Some(coords) = by_path.remove(&pid) else {
            continue;
        };
        if let Some(refs) = item._refs.as_mut() {
            if let Some(qk) = refs.get_mut("qk_path") {
                qk["ak_nodes"] = coords;
            }
        }
    }
    Ok(())
}

impl From<PgPool> for TrafficLineRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<TrafficLine, CreateTrafficLineRequest, UpdateTrafficLineRequest, ApiError>
    for TrafficLineRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<TrafficLine>, ApiError> {
        let mut page = self.generic.list_refs(query).await?;
        // 批注（用户要求真实途经点）：qk_path.ak_nodes 从 id 数组替换为坐标对象数组
        // （{id,name,lng,lat}）——前端路线管理直接解析真实途经点名称
        enrich_qk_path_ak_nodes(&self.pool, &mut page.items).await?;
        Ok(page)
    }

    async fn get(&self, id: i64) -> Result<Option<TrafficLine>, ApiError> {
        let mut item = self.generic.get_refs(id, None).await?;
        if let Some(ref mut it) = item {
            enrich_qk_path_ak_nodes(&self.pool, std::slice::from_mut(it)).await?;
        }
        Ok(item)
    }

    async fn create(
        &self,
        req: CreateTrafficLineRequest,
        user_id: i64,
    ) -> Result<TrafficLine, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "TrafficLine").await?;
        sqlx::query_as::<_, TrafficLine>(
            r#"INSERT INTO "isahl"."zc_id_stor-traffic_line" (code, notice, comments, fk_trustee, qk_path, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING id, code, notice, comments, fk_trustee, qk_path, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code)
        .bind(&req.notice)
        .bind(&req.comments)
        .bind(req.fk_trustee)
        .bind(req.qk_path)
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateTrafficLineRequest,
        _user_id: i64,
    ) -> Result<Option<TrafficLine>, ApiError> {
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
        if req.qk_path.is_some() {
            idx += 1;
            sets.push(format!("qk_path = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_stor-traffic_line" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_trustee, qk_path, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, TrafficLine>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(ref v) = req.fk_trustee {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_path {
            q = q.bind(v);
        }
        q = q.bind(_user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// PricingAgreement Repository — "isahl"."zc_id_agre-pricing"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct PricingAgreementRepository {
    generic: GenericRepository<PricingAgreement>,
    pool: PgPool,
}

impl PricingAgreementRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for PricingAgreementRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        PricingAgreement,
        CreatePricingAgreementRequest,
        UpdatePricingAgreementRequest,
        ApiError,
    > for PricingAgreementRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<PricingAgreement>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<PricingAgreement>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreatePricingAgreementRequest,
        user_id: i64,
    ) -> Result<PricingAgreement, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "PricingAgreement").await?;
        sqlx::query_as::<_, PricingAgreement>(
            r#"INSERT INTO "isahl"."zc_id_agre-pricing" (code, notice, comments, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               RETURNING id, code, notice, comments, t_color_, tpl_id, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code)
        .bind(&req.notice)
        .bind(&req.comments)
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdatePricingAgreementRequest,
        _user_id: i64,
    ) -> Result<Option<PricingAgreement>, ApiError> {
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
        if sets.is_empty() {
            return self.get(id).await;
        }
        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;
        let sql = format!(
            r#"UPDATE "isahl"."zc_id_agre-pricing" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, t_color_, tpl_id, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );
        let mut q = sqlx::query_as::<_, PricingAgreement>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        q = q.bind(_user_id);
        q = q.bind(id);
        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

// ═══════════════════════════════════════════════
// Contract Repository — "isahl"."zc_id_contract"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct ContractRepository {
    generic: GenericRepository<Contract>,
    pool: PgPool,
}

impl ContractRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for ContractRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<Contract, CreateContractRequest, UpdateContractRequest, ApiError>
    for ContractRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Contract>, ApiError> {
        self.generic.list_refs(query).await
    }
    async fn get(&self, id: i64) -> Result<Option<Contract>, ApiError> {
        self.generic.get_refs(id, None).await
    }
    async fn create(&self, req: CreateContractRequest, user_id: i64) -> Result<Contract, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "Contract").await?;
        sqlx::query_as::<_, Contract>(
            r#"INSERT INTO "isahl"."zc_id_contract" (code, notice, comments, qk_date, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               RETURNING id, notice, code, o_number, comments, projection, t_color_, qk_date, "qk_valid-segm", tpl_id, dk_scene, dk_factor, dk_function, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code)
        .bind(&req.notice)
        .bind(&req.comments)
        .bind(req.sign_date)
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }
    async fn update(
        &self,
        id: i64,
        req: UpdateContractRequest,
        _user_id: i64,
    ) -> Result<Option<Contract>, ApiError> {
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
        if req.sign_date.is_some() {
            idx += 1;
            sets.push(format!("qk_date = ${}", idx));
        }
        if sets.is_empty() {
            return self.get(id).await;
        }
        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;
        let sql = format!(
            r#"UPDATE "isahl"."zc_id_contract" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, notice, code, o_number, comments, projection, t_color_, qk_date, "qk_valid-segm", tpl_id, dk_scene, dk_factor, dk_function, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );
        let mut q = sqlx::query_as::<_, Contract>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.sign_date {
            q = q.bind(v);
        }
        q = q.bind(_user_id);
        q = q.bind(id);
        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }
    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

#[cfg(test)]
mod drift_guard {
    use super::ontology_binding;

    /// Entity names that call ontology_binding::resolve() in their create method.
    /// Adding a new create? Append its entity name here.
    const RUNTIME_ENTITIES: &[&str] = &[
        "Identity",
        "Environment",
        "License",
        "Consignment",
        "Vehicle",
        "TransportTracking",
        "TradeOrder",
        "BillCheck",
        "DetaBillCheck",
        "Invoice",
        "InvoiceDetail",
        "Payment",
        "SettlementBank",
        "SettlementCash",
        "SettlementChannel",
        "TrafficLine",
        "PricingAgreement",
        "Contract",
        "SubjectGroup",
        "SubjectEmployee",
        "EmploymentAgent",
        "SubjectCountry",
        "SubjectBank",
        "SubjectMinistry",
        "SubjectSovereign",
        "SubjectSupranational",
    ];

    #[test]
    fn coords_for_entity_covers_all_runtime_call_sites() {
        let mut missing: Vec<&str> = Vec::new();
        for name in RUNTIME_ENTITIES {
            if ontology_binding::coords_for_entity(name).is_err() {
                missing.push(name);
            }
        }
        assert!(
            missing.is_empty(),
            "coords_for_entity missing arms for: {missing:?}"
        );
    }
}

use crate::models::{CreateNaturalPersonRequest, NaturalPerson, UpdateNaturalPersonRequest};

// ═══════════════════════════════════════════════════════════════════════════════

mod ontology_binding {
    use common::AliothError as ApiError;
    use sqlx::PgPool;

    /// (scene_code, factor_code, function_code)
    pub type Coords = (&'static str, &'static str, &'static str);

    pub fn coords_for_entity(entity: &str) -> Result<Coords, ApiError> {
        match entity {
            "Identity" => Ok(("JE", "FJA", "↑_DA")),
            "Environment" => Ok(("JC", "GEC", "↑_DA")),
            "License" => Ok(("JC", "GID", "↑_DA")),
            "Consignment" => Ok(("GC", "FJA", "↓_GG")),
            "Vehicle" => Ok(("GC", "FJA", "↓_GG")),
            "NaturalPerson" => Ok(("GC", "FJA", "↓_GG")),
            "TransportTracking" => Ok(("GC", "FJA", "↓_GG")),
            "TradeOrder" => Ok(("GC", "FJA", "↓_GG")),
            "BillCheck" => Ok(("GC", "FJA", "↓_GD")),
            "DetaBillCheck" => Ok(("GC", "FJA", "↓_GD")),
            "Invoice" => Ok(("GC", "FJA", "↓_GD")),
            "InvoiceDetail" => Ok(("GC", "FJA", "↓_GD")),
            "Payment" => Ok(("TX", "FJA", "↓_EV")),
            "SettlementBank" => Ok(("TX", "FJA", "↓_EV")),
            "SettlementCash" => Ok(("TX", "FJA", "↓_EV")),
            "SettlementChannel" => Ok(("TX", "FJA", "↓_EV")),
            "TrafficLine" => Ok(("GC", "FJA", "↑_GG")),
            "FreightProduct" => Ok(("GC", "FJA", "↓_GG")),
            "PricingAgreement" => Ok(("GC", "FJA", "↓_GG")),
            "Contract" => Ok(("GC", "FJA", "↓_GG")),
            // strengthen-identity-org 主体域叶表（scene/factor/function 均经 DB 维度表核实）：
            // ZB=组织架构 ZJ=行政人事 ZH=主体管理 UB=主权管理；LNC=权责主体 LNK=经营主体；
            // ↓_DA=权责复检 ↓_EH=人事劳动
            "SubjectGroup" => Ok(("ZB", "LNC", "↓_DA")),
            "SubjectEmployee" => Ok(("ZJ", "LNC", "↓_EH")),
            "EmploymentAgent" => Ok(("ZJ", "LNC", "↓_EH")),
            "SubjectCountry" => Ok(("UB", "LNC", "↓_DA")),
            "SubjectBank" => Ok(("ZH", "LNK", "↓_DA")),
            "SubjectMinistry" => Ok(("UB", "LNC", "↓_DA")),
            "SubjectSovereign" => Ok(("UB", "LNC", "↓_DA")),
            "SubjectSupranational" => Ok(("UB", "LNC", "↓_DA")),
            _ => Err(ApiError::Internal(format!(
                "no ontology coordinates for entity: {entity}"
            ))),
        }
    }

    pub async fn resolve(pool: &PgPool, entity: &str) -> Result<(i64, i64, Option<i64>), ApiError> {
        let coords = coords_for_entity(entity)?;
        let (scene_id, factor_id, function_id) = ontology_binding::resolve(pool, coords)
            .await
            .map_err(|e| ApiError::Internal(format!("resolve {:?}: {e}", coords)))?;
        let scene_id =
            scene_id.ok_or_else(|| ApiError::Internal(format!("resolve scene {}", coords.0)))?;
        let factor_id =
            factor_id.ok_or_else(|| ApiError::Internal(format!("resolve factor {}", coords.1)))?;
        Ok((scene_id, factor_id, function_id))
    }
}

#[derive(Clone)]
pub struct IdentityRepository {
    pool: PgPool,
}

impl IdentityRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl From<PgPool> for IdentityRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

/// MDM 主数据编码 upsert/清除（wz_fssc.subject_mdm 侧表；空串=删除记录）。
/// isahl schema 冻结 + comments 禁嵌 JSON，主体级 MDM 编码落 WZ 扩展表
/// （change: add-subject-mdm-code；先例：subject_bank_card / subject_invoice_info）。
async fn sync_subject_mdm(
    pool: &sqlx::PgPool,
    subject_id: i64,
    mdm_code: &str,
    user_id: i64,
) -> Result<(), ApiError> {
    crate::handlers::subjects::ensure_subject_mdm(pool).await?;
    let trimmed = mdm_code.trim();
    if trimmed.is_empty() {
        sqlx::query("DELETE FROM wz_fssc.subject_mdm WHERE subject_id = $1")
            .bind(subject_id)
            .execute(pool)
            .await
            .map_err(ApiError::from)?;
    } else {
        sqlx::query(
            r#"INSERT INTO wz_fssc.subject_mdm (subject_id, mdm_code, created_by_id, updated_by_id)
               VALUES ($1, $2, $3, $3)
               ON CONFLICT (subject_id)
               DO UPDATE SET mdm_code = EXCLUDED.mdm_code,
                             updated_by_id = EXCLUDED.updated_by_id,
                             updated_at = now()"#,
        )
        .bind(subject_id)
        .bind(trimmed)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(ApiError::from)?;
    }
    Ok(())
}

#[async_trait]
impl AliothRepository<Identity, CreateIdentityRequest, UpdateIdentityRequest, ApiError>
    for IdentityRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Identity>, ApiError> {
        QueryBuilder::<Identity>::from_list_query(&self.pool, query)
            .fetch_refs(query.page, query.page_size)
            .await
    }
    async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<PaginatedResponse<Identity>, ApiError> {
        let mut qb = QueryBuilder::<Identity>::from_list_query(&self.pool, query);
        if let Some(ids) = visible_ids {
            qb = qb.with_visible_ids(ids.to_vec());
        }
        if let Some(cols) = authorized_columns {
            qb = qb.with_authorized_columns(cols.to_vec());
        }
        qb.fetch_refs(query.page, query.page_size).await
    }

    async fn get(&self, id: i64) -> Result<Option<Identity>, ApiError> {
        QueryBuilder::<Identity>::get_refs(&self.pool, id, None).await
    }

    async fn create(&self, req: CreateIdentityRequest, user_id: i64) -> Result<Identity, ApiError> {
        let table = req.resolve_subtable(Some(&req.subject_type))?;
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "Identity").await?;
        let sql = format!(
            r#"INSERT INTO {} (notice, code, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id, notice AS name, code, notice, created_at, updated_at, deleted_at"#,
            table
        );
        sqlx::query_as::<_, Identity>(AssertSqlSafe(sql.as_str()))
            .bind(&req.name)
            .bind(&req.code)
            .bind(user_id)
            .bind(dk_scene)
            .bind(dk_factor)
            .bind(dk_function)
            .fetch_one(&self.pool)
            .await
            .map_err(ApiError::from)
    }
    async fn update(
        &self,
        id: i64,
        req: UpdateIdentityRequest,
        user_id: i64,
    ) -> Result<Option<Identity>, ApiError> {
        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if req.name.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
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

        if sets.is_empty() {
            // 仅 MDM 编码更新的场景（无主体列变更）：确认主体存在后写侧表
            if let Some(ref mdm_code) = req.mdm_code {
                if self.get(id).await?.is_none() {
                    return Ok(None);
                }
                sync_subject_mdm(&self.pool, id, mdm_code, user_id).await?;
            }
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE isahl.zc_id_subjects SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, notice AS name, code, notice, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, Identity>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.name {
            q = q.bind(v);
        }
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }

        q = q.bind(user_id);
        q = q.bind(id);

        let updated = q.fetch_optional(&self.pool).await.map_err(ApiError::from)?;
        // MDM 主数据编码（wz_fssc.subject_mdm 侧表）：主体更新成功后写入
        if updated.is_some() {
            if let Some(ref mdm_code) = req.mdm_code {
                sync_subject_mdm(&self.pool, id, mdm_code, user_id).await?;
            }
        }
        Ok(updated)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        let rows = sqlx::query(
        "UPDATE isahl.zc_id_subjects SET deleted_at = NOW(), updated_by_id = $2 WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(ApiError::from)?;

        if rows.rows_affected() == 0 {
            return Err(ApiError::NotFound(format!("Identity {} not found", id)));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_circle_point;
    use serde_json::json;

    /// 圆心输入规范化：GeoJSON Point / {lng,lat} / {lat,lng}
    #[test]
    fn test_normalize_circle_point() {
        // GeoJSON Point
        let g = json!({"type":"Point","coordinates":[113.752,23.021]});
        assert_eq!(normalize_circle_point(&g).unwrap(), (113.752, 23.021));
        // {lng,lat}
        assert_eq!(
            normalize_circle_point(&json!({"lng": 114.057, "lat": 22.543})).unwrap(),
            (114.057, 22.543)
        );
        // {lat,lng}（兼容旧形态）
        assert_eq!(
            normalize_circle_point(&json!({"lat": 22.543, "lng": 114.057})).unwrap(),
            (114.057, 22.543)
        );
        // 非法：缺坐标
        assert!(normalize_circle_point(&json!({"type": "Point"})).is_err());
        // 非法：非 Point 类型
        assert!(
            normalize_circle_point(&json!({"type": "Polygon", "coordinates": [[0,0]]})).is_err()
        );
        // 非法：缺 lng
        assert!(normalize_circle_point(&json!({"lat": 1.0})).is_err());
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 主体域叶表 Repository（strengthen-identity-org）——同构 CRUD + dk 坐标注入，
use crate::models::{
    CreateEmploymentAgentRequest, CreateSubjectBankRequest, CreateSubjectCountryRequest,
    CreateSubjectEmployeeRequest, CreateSubjectGroupRequest, CreateSubjectMinistryRequest,
    CreateSubjectSovereignRequest, CreateSubjectSupranationalRequest, EmploymentAgent, SubjectBank,
    SubjectCountry, SubjectEmployee, SubjectGroup, SubjectMinistry, SubjectSovereign,
    SubjectSupranational, UpdateEmploymentAgentRequest, UpdateSubjectBankRequest,
    UpdateSubjectCountryRequest, UpdateSubjectEmployeeRequest, UpdateSubjectGroupRequest,
    UpdateSubjectMinistryRequest, UpdateSubjectSovereignRequest, UpdateSubjectSupranationalRequest,
};
// 经宏生成。坐标见 coords_for_entity 对应臂（scene/factor/function 均经 DB 维度表核实）。
// ═══════════════════════════════════════════════════════════════════════════════

/// 生成主体域叶表 Repository：list/get 走 GenericRepository（refs 解析），
/// create 注入 dk 三元组，update 动态 SET，delete 软删委托 GenericRepository。
macro_rules! subject_leaf_repository {
    ($repo:ident, $entity:ident, $create:ident, $update:ident, $table:literal, $coord:literal) => {
        #[derive(Clone)]
        pub struct $repo {
            generic: GenericRepository<$entity>,
            pool: PgPool,
        }

        impl $repo {
            pub fn new(pool: PgPool) -> Self {
                Self {
                    generic: GenericRepository::new(pool.clone()),
                    pool,
                }
            }
        }

        impl From<PgPool> for $repo {
            fn from(pool: PgPool) -> Self {
                Self::new(pool)
            }
        }

        #[async_trait]
        impl AliothRepository<$entity, $create, $update, ApiError> for $repo {
            async fn list(
                &self,
                query: &ListQuery,
            ) -> Result<PaginatedResponse<$entity>, ApiError> {
                self.generic.list_refs(query).await
            }

            async fn get(&self, id: i64) -> Result<Option<$entity>, ApiError> {
                self.generic.get_refs(id, None).await
            }

            async fn create(&self, req: $create, user_id: i64) -> Result<$entity, ApiError> {
                let (dk_scene, dk_factor, dk_function) =
                    ontology_binding::resolve(&self.pool, $coord).await?;
                sqlx::query_as::<_, $entity>(
                    concat!(
                        "INSERT INTO ", $table,
                        " (code, notice, o_number, comments, created_by_id, dk_scene, dk_factor, dk_function)",
                        " VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                        " RETURNING id, code, notice, o_number, comments, created_at, updated_at, deleted_at"
                    ),
                )
                .bind(&req.code)
                .bind(&req.notice)
                .bind(&req.o_number)
                .bind(&req.comments)
                .bind(user_id)
                .bind(dk_scene)
                .bind(dk_factor)
                .bind(dk_function)
                .fetch_one(&self.pool)
                .await
                .map_err(ApiError::from)
            }

            async fn update(
                &self,
                id: i64,
                req: $update,
                user_id: i64,
            ) -> Result<Option<$entity>, ApiError> {
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
                if req.o_number.is_some() {
                    idx += 1;
                    sets.push(format!("o_number = ${}", idx));
                }
                if req.comments.is_some() {
                    idx += 1;
                    sets.push(format!("comments = ${}", idx));
                }
                if sets.is_empty() {
                    return self.get(id).await;
                }
                sets.push("updated_at = NOW()".into());
                idx += 1;
                sets.push(format!("updated_by_id = ${}", idx));
                let id_param = idx + 1;
                let sql = format!(
                    concat!(
                        "UPDATE ", $table, " SET {} WHERE id = ${} AND deleted_at IS NULL",
                        " RETURNING id, code, notice, o_number, comments, created_at, updated_at, deleted_at"
                    ),
                    sets.join(", "),
                    id_param
                );
                let mut q = sqlx::query_as::<_, $entity>(AssertSqlSafe(sql.as_str()));
                if let Some(ref v) = req.code {
                    q = q.bind(v);
                }
                if let Some(ref v) = req.notice {
                    q = q.bind(v);
                }
                if let Some(ref v) = req.o_number {
                    q = q.bind(v);
                }
                if let Some(ref v) = req.comments {
                    q = q.bind(v);
                }
                q = q.bind(user_id);
                q = q.bind(id);
                q.fetch_optional(&self.pool).await.map_err(ApiError::from)
            }

            async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
                self.generic.delete(id, user_id).await
            }
        }
    };
}

subject_leaf_repository!(
    SubjectGroupRepository,
    SubjectGroup,
    CreateSubjectGroupRequest,
    UpdateSubjectGroupRequest,
    "\"isahl\".\"zc_id_subj-group\"",
    "SubjectGroup"
);
subject_leaf_repository!(
    SubjectEmployeeRepository,
    SubjectEmployee,
    CreateSubjectEmployeeRequest,
    UpdateSubjectEmployeeRequest,
    "\"isahl\".\"zc_id_subj-employee\"",
    "SubjectEmployee"
);
subject_leaf_repository!(
    EmploymentAgentRepository,
    EmploymentAgent,
    CreateEmploymentAgentRequest,
    UpdateEmploymentAgentRequest,
    "\"isahl\".\"zc_id_empl-agent\"",
    "EmploymentAgent"
);
subject_leaf_repository!(
    SubjectCountryRepository,
    SubjectCountry,
    CreateSubjectCountryRequest,
    UpdateSubjectCountryRequest,
    "\"isahl\".\"zc_id_subj-country\"",
    "SubjectCountry"
);
subject_leaf_repository!(
    SubjectBankRepository,
    SubjectBank,
    CreateSubjectBankRequest,
    UpdateSubjectBankRequest,
    "\"isahl\".\"zc_id_subj-bank\"",
    "SubjectBank"
);
subject_leaf_repository!(
    SubjectMinistryRepository,
    SubjectMinistry,
    CreateSubjectMinistryRequest,
    UpdateSubjectMinistryRequest,
    "\"isahl\".\"zc_id_subj-ministry\"",
    "SubjectMinistry"
);
subject_leaf_repository!(
    SubjectSovereignRepository,
    SubjectSovereign,
    CreateSubjectSovereignRequest,
    UpdateSubjectSovereignRequest,
    "\"isahl\".\"zc_id_subj-sovereign\"",
    "SubjectSovereign"
);
subject_leaf_repository!(
    SubjectSupranationalRepository,
    SubjectSupranational,
    CreateSubjectSupranationalRequest,
    UpdateSubjectSupranationalRequest,
    "\"isahl\".\"zc_id_subj-supranational\"",
    "SubjectSupranational"
);

/// 封签 comments 合并关联运单（JSON 承载 waybill_id；保留原文本——追加 JSON 对象，批注轮 69）
fn seal_comments_with_waybill(comments: Option<String>, waybill_id: Option<i64>) -> Option<String> {
    match waybill_id {
        Some(wid) => {
            let base = comments.unwrap_or_default();
            if base.trim().is_empty() {
                Some(format!(r#"{{"waybill_id": {wid}}}"#))
            } else if base.trim_start().starts_with('{') {
                // 已是 JSON——merge waybill_id
                match serde_json::from_str::<serde_json::Value>(&base) {
                    Ok(mut v) => {
                        v["waybill_id"] = serde_json::json!(wid);
                        Some(v.to_string())
                    }
                    Err(_) => Some(format!(
                        r#"{{"waybill_id": {wid}, "note": {}}}"#,
                        serde_json::to_string(&base).unwrap_or_default()
                    )),
                }
            } else {
                // 纯文本——包裹 JSON（note 保留原文）
                Some(format!(
                    r#"{{"waybill_id": {wid}, "note": {}}}"#,
                    serde_json::to_string(&base).unwrap_or_default()
                ))
            }
        }
        None => comments,
    }
}
