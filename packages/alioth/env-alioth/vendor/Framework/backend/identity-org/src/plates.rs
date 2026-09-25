//! 车辆号牌读写件 —— 「实体↔身份」桥的唯一实现（change `align-vehicle-plate-identity`）。
//!
//! 模型口径（用户 2026-09-23）：
//! - 车辆 `notice` = 描述信息、`code` = 序列号（车辆出厂车架号/VIN）——**均不承载号牌**；
//! - 号牌号 = `zc_id_identity.identity`，分类 = `zc_id_cate-identity.code = 'plate'`；
//! - 车辆↔号牌 = `zc_id_entity_rr_identity`（`ref_left` = 车辆、`ref_right` = 身份行、
//!   `qk_period` → `zc_id_segm-date` 有效期段）；
//! - 一车可多牌（多条活动桥行）；换牌 = 新增桥行 + 软删旧桥行（软删行保留 = 历史可查）；
//! - 外部核发标识 = 同值同分类一行（change `align-storage-issuer-and-holding` §D3）：写径按
//!   `upper(identity)` + 分类 **find-or-create**，命中即复用（MUST NOT 每次登记插新行）；
//! - 同一号牌任一时刻至多绑一个实体（§D4）：判重域 = **身份行全局**（跨车辆、跨组织），
//!   判据 = 生效期**不重叠**（`qk_period` 为空 = 覆盖全部时间）；换牌/历史段不重叠即允许。
//!
//! 模型事实：车辆 ⊂ `zc_id_carrier` ⊂ `zc_id_entity`（`pg_inherits` 闭包），故
//! `isahl_meta.meta_fields` 在 `zc_id_stor-ctn-vehicle.identity`（m2n，junction-only）声明的
//! 桥正是本件；用户 2026-09-21 裁决确认该桥用法合规（`Pre-Proc/WZ/model-center-requests.md §R12`）。
//!
//! 调用方（平台号牌端点 / 车辆写径 / 门户登记）MUST 走本件，禁各自拼 SQL。

use chrono::{DateTime, Utc};
use sqlx::{Acquire, PgConnection, PgPool};

use common::AliothError as ApiError;

use crate::models::VehiclePlateInput;

/// 号牌输入预处理：入口归一（去空白/圆点 + 大写）+ GA 36-2018 形态校验 + 有效期次序。
///
/// 供车辆创建/更新随车落牌使用——**先于车辆行落地**校验，避免车辆建成而号牌非法留下半成品。
pub fn prepare_plates(inputs: &[VehiclePlateInput]) -> Result<Vec<PlateInput>, ApiError> {
    let mut out = Vec::with_capacity(inputs.len());
    for input in inputs {
        let plate = common::plate::normalize_plate(&input.plate);
        if plate.is_empty() {
            return Err(ApiError::BadRequest("号牌号不能为空".into()));
        }
        if !common::plate::plate_format_ok(&plate) {
            return Err(ApiError::BadRequest(format!(
                "号牌不符合中国民用号牌规则（如 京A12345、蒙BD22345）：{plate}"
            )));
        }
        if let (Some(from), Some(to)) = (input.valid_from, input.valid_to) {
            if from > to {
                return Err(ApiError::BadRequest("有效期起始不得晚于终止".into()));
            }
        }
        out.push(PlateInput {
            plate,
            description: input.description.clone(),
            valid_from: input.valid_from,
            valid_to: input.valid_to,
        });
    }
    Ok(out)
}

/// 号牌分类字典 code（模型种子 `Meta/backend/src/data_content/bundles/identity-cate.json`）。
pub const PLATE_CATEGORY_CODE: &str = "plate";

/// 号牌输入（新增/换牌）。
#[derive(Debug, Clone)]
pub struct PlateInput {
    /// 号牌号（调用方 MUST 已用 `common::plate::normalize_plate` 归一）
    pub plate: String,
    /// 号牌描述（缺省写 `车牌-{号牌}`，对齐既有身份行 `营业执照-{主体}` 命名）
    pub description: Option<String>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
}

/// 号牌行（列表输出；含历史行，`active` 标记活动态）。
#[derive(Debug, Clone)]
pub struct PlateRow {
    /// 桥行 id（解除/换牌入参）
    pub rel_id: i64,
    /// 身份行 id
    pub identity_id: i64,
    pub plate: String,
    pub description: Option<String>,
    /// 生效期起（`qk_period` → `zc_id_segm-date.date_st`）；`None` = 无下界（`-∞`）
    pub valid_from: Option<DateTime<Utc>>,
    /// 生效期止（同上 `date_ed`）；`None` = 无上界（`+∞`）
    pub valid_to: Option<DateTime<Utc>>,
    /// 活动 = 桥行与身份行皆未软删，且未过有效期
    /// （`valid_to` 为空 = 覆盖全部时间 `[-∞,+∞]`，故未过期为真）
    pub active: bool,
}

/// 号牌分类字典行 id。
///
/// **fail-visible**：分类由模型种子供给（用户裁决「分类是模型级种子数据」），本件
/// **MUST NOT** 自动补行（避免绕过模型种子产生非模型 id）；缺行 → 400 明确错误码。
pub async fn plate_category_id(pool: &PgPool) -> Result<i64, ApiError> {
    let id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM \"isahl\".\"zc_id_cate-identity\" \
         WHERE code = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1",
    )
    .bind(PLATE_CATEGORY_CODE)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    id.ok_or_else(|| ApiError::BadRequest(missing_category_message()))
}

fn missing_category_message() -> String {
    format!(
        "号牌分类缺失：字典 zc_id_cate-identity.code='{PLATE_CATEGORY_CODE}' 无活行（请发布模型种子 identity-cate）"
    )
}

/// 车辆存在性（软删行视为不存在）。
pub async fn ensure_vehicle_exists(
    conn: &mut PgConnection,
    vehicle_id: i64,
) -> Result<(), ApiError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM \"isahl\".\"zc_id_stor-ctn-vehicle\" \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(vehicle_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !exists {
        return Err(ApiError::NotFound(format!("车辆不存在: {vehicle_id}")));
    }
    Ok(())
}

/// 车辆号牌列表（活动在前、桥行 id 倒序；含历史行 → `active=false`）。
pub async fn list_vehicle_plates(
    conn: &mut PgConnection,
    vehicle_id: i64,
) -> Result<Vec<PlateRow>, ApiError> {
    #[allow(clippy::type_complexity)] // sqlx 行类型
    let rows: Vec<(
        i64,
        i64,
        String,
        Option<String>,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        bool,
    )> = sqlx::query_as(
        "SELECT r.id, i.id, i.identity, i.notice, d.date_st, d.date_ed, \
                (r.deleted_at IS NOT NULL OR i.deleted_at IS NOT NULL) AS removed \
         FROM \"isahl\".\"zc_id_entity_rr_identity\" r \
         JOIN \"isahl\".\"zc_id_identity\" i ON i.id = r.ref_right \
         JOIN \"isahl\".\"zc_id_cate-identity\" c ON c.id = i.ck_category AND c.code = $2 \
         LEFT JOIN \"isahl\".\"zc_id_segm-date\" d ON d.id = r.qk_period AND d.deleted_at IS NULL \
         WHERE r.ref_left = $1 \
         ORDER BY r.id DESC",
    )
    .bind(vehicle_id)
    .bind(PLATE_CATEGORY_CODE)
    .fetch_all(&mut *conn)
    .await
    .map_err(ApiError::from_sqlx)?;

    let now = Utc::now();
    Ok(rows
        .into_iter()
        .map(
            |(rel_id, identity_id, plate, description, st, ed, removed)| PlateRow {
                rel_id,
                identity_id,
                plate,
                description,
                valid_from: st,
                valid_to: ed,
                active: !removed && ed.is_none_or(|e| e >= now),
            },
        )
        .collect())
}

/// 新增号牌（身份行 find-or-create + 桥行 + 可选有效期段），返回 `(rel_id, identity_id)`。
///
/// 判重（change `align-storage-issuer-and-holding` §D3/§D4）：
/// - 域 = **同身份行全局**（跨车辆、跨组织），不再限于本车辆；
/// - 判据 = 生效期**不重叠**——新段与该身份行既有活动绑定的 `qk_period` 段逐一比对，
///   重叠（含任一侧为空 = `[-∞,+∞]`）→ 409（既有 `Conflict` 语义）；
/// - 不重叠（换牌 / 历史段）→ 允许，并**复用**既有身份行（MUST NOT 新建同值身份行）。
pub async fn create_vehicle_plate(
    conn: &mut PgConnection,
    vehicle_id: i64,
    user_id: i64,
    input: &PlateInput,
) -> Result<(i64, i64), ApiError> {
    let plate = input.plate.trim();
    if plate.is_empty() {
        return Err(ApiError::BadRequest("号牌号不能为空".into()));
    }

    let mut tx = conn.begin().await.map_err(ApiError::from_sqlx)?;

    // 分类字典（fail-visible）——同时是身份行 find-or-create 的分类键
    let category_id: i64 = sqlx::query_scalar(
        "SELECT id FROM \"isahl\".\"zc_id_cate-identity\" \
         WHERE code = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1",
    )
    .bind(PLATE_CATEGORY_CODE)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?
    .ok_or_else(|| ApiError::BadRequest(missing_category_message()))?;

    // 身份行 find-or-create（§D3）：同值（`upper()` 兜历史小写行）+ 同分类的活动身份行，
    // 命中即复用；软删行不参与（身份失效后可重新登记）。
    let existing_identity: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM \"isahl\".\"zc_id_identity\" \
         WHERE upper(identity) = upper($1) AND ck_category = $2 AND deleted_at IS NULL \
         ORDER BY id LIMIT 1",
    )
    .bind(plate)
    .bind(category_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    // 期间重叠判定（§D4，应用层单事务内完成——无 DDL 约束）：
    // - 期间 = 既有活动桥行 `qk_period` → `zc_id_segm-date`；无段 / `date_st`/`date_ed` 为空
    //   = `[-∞,+∞]`（无段默认全覆盖）；
    // - 绑定实体为**已软删车辆**时不计（模块契约：车辆软删的残留桥行不遮蔽他车）；
    // - 重叠 = `(new_st IS NULL OR old_ed IS NULL OR new_st <= old_ed) AND
    //            (new_ed IS NULL OR old_st IS NULL OR old_st <= new_ed)`。
    if let Some(identity_id) = existing_identity {
        let overlapped: bool = sqlx::query_scalar(
            "SELECT COUNT(*) > 0 FROM \"isahl\".\"zc_id_entity_rr_identity\" r \
             LEFT JOIN \"isahl\".\"zc_id_stor-ctn-vehicle\" v ON v.id = r.ref_left \
             LEFT JOIN \"isahl\".\"zc_id_segm-date\" d ON d.id = r.qk_period AND d.deleted_at IS NULL \
             WHERE r.ref_right = $1 AND r.deleted_at IS NULL \
               AND (v.id IS NULL OR v.deleted_at IS NULL) \
               AND ($2::timestamptz IS NULL OR d.date_ed IS NULL OR d.date_ed >= $2::timestamptz) \
               AND ($3::timestamptz IS NULL OR d.date_st IS NULL OR d.date_st <= $3::timestamptz)",
        )
        .bind(identity_id)
        .bind(input.valid_from)
        .bind(input.valid_to)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        if overlapped {
            return Err(ApiError::Conflict(format!(
                "号牌已被占用（同号牌既有绑定的生效期与本段重叠）: {plate}"
            )));
        }
    }

    // 坐标三元组：身份族 = JE / FJA / ↑_DA（与 identity-org 既有身份写径同源）
    let identity_id: i64 = match existing_identity {
        // 复用既有活动身份行（同值同分类一行；本段与既有绑定期间不重叠）
        Some(id) => id,
        None => {
            let (dk_scene, dk_factor, dk_function) =
                ontology_binding::resolve_conn(&mut tx, ("JE", "FJA", "↑_DA"))
                    .await
                    .map_err(ApiError::from_sqlx)?;
            let description = input
                .description
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("车牌-{plate}"));
            sqlx::query_scalar(
                "INSERT INTO \"isahl\".\"zc_id_identity\" \
                 (notice, identity, ck_category, created_by_id, updated_by_id, dk_scene, dk_factor, dk_function) \
                 VALUES ($1, $2, $3, $4, $4, $5, $6, $7) RETURNING id",
            )
            .bind(&description)
            .bind(plate)
            .bind(category_id)
            .bind(user_id)
            .bind(dk_scene)
            .bind(dk_factor)
            .bind(dk_function)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::from_sqlx)?
        }
    };

    // 2. 有效期段（缺省不建段）
    let period_id: Option<i64> = if input.valid_from.is_some() || input.valid_to.is_some() {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO \"isahl\".\"zc_id_segm-date\" \
             (notice, date_st, date_ed, created_by_id, updated_by_id) \
             VALUES ($1, $2, $3, $4, $4) RETURNING id",
        )
        .bind(format!("vehicle-{vehicle_id}-plate validity"))
        .bind(input.valid_from)
        .bind(input.valid_to)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        Some(id)
    } else {
        None
    };

    // 3. 车辆↔号牌桥行
    let rel_id: i64 = sqlx::query_scalar(
        "INSERT INTO \"isahl\".\"zc_id_entity_rr_identity\" \
         (notice, ref_left, ref_right, qk_period, created_by_id, updated_by_id) \
         VALUES ($1, $2, $3, $4, $5, $5) RETURNING id",
    )
    .bind(format!("vehicle-{vehicle_id} plate"))
    .bind(vehicle_id)
    .bind(identity_id)
    .bind(period_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    tx.commit().await.map_err(ApiError::from_sqlx)?;
    Ok((rel_id, identity_id))
}

/// 解除/换牌：软删桥行（保留历史）并在无其他活动桥行引用时软删身份行。
pub async fn retire_vehicle_plate(
    conn: &mut PgConnection,
    vehicle_id: i64,
    rel_id: i64,
    user_id: i64,
) -> Result<(), ApiError> {
    let identity_id: Option<i64> = sqlx::query_scalar(
        "SELECT r.ref_right FROM \"isahl\".\"zc_id_entity_rr_identity\" r \
         JOIN \"isahl\".\"zc_id_identity\" i ON i.id = r.ref_right \
         JOIN \"isahl\".\"zc_id_cate-identity\" c ON c.id = i.ck_category AND c.code = $3 \
         WHERE r.id = $1 AND r.ref_left = $2 AND r.deleted_at IS NULL",
    )
    .bind(rel_id)
    .bind(vehicle_id)
    .bind(PLATE_CATEGORY_CODE)
    .fetch_optional(&mut *conn)
    .await
    .map_err(ApiError::from_sqlx)?;
    let identity_id =
        identity_id.ok_or_else(|| ApiError::NotFound(format!("号牌关联不存在: {rel_id}")))?;

    let mut tx = conn.begin().await.map_err(ApiError::from_sqlx)?;
    sqlx::query(
        "UPDATE \"isahl\".\"zc_id_entity_rr_identity\" SET deleted_at = now(), deleted_by_id = $1 \
         WHERE id = $2 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(rel_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    // 同一身份行可被多车按**不重叠期间**先后共用（§D4；同值同分类仅一行）——仍有活动桥行
    // 则保留身份行（解除 = 软删桥行；身份本体归最后一处绑定退役）
    let still_referenced: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM \"isahl\".\"zc_id_entity_rr_identity\" \
         WHERE ref_right = $1 AND deleted_at IS NULL",
    )
    .bind(identity_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !still_referenced {
        sqlx::query(
            "UPDATE \"isahl\".\"zc_id_identity\" SET deleted_at = now(), deleted_by_id = $1 \
             WHERE id = $2 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(identity_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
    }
    tx.commit().await.map_err(ApiError::from_sqlx)?;
    Ok(())
}

// 契约（调用方 MUST 遵守）：跨车辆的**反查/列表** MUST 以**车辆存活**为谓词
// （`zc_id_stor-ctn-vehicle.deleted_at IS NULL`），故车辆软删后残留的号牌桥行不会遮蔽他车、
// 也不会误命中已删车辆——号牌桥行因此**不随车辆删除而退役**（历史事实保留；仅「解除号牌 /
// 换牌」显式软删桥行）。**判重**（[`create_vehicle_plate`]）同口径：已软删车辆的残留绑定
// 不计入期间占用，号牌随即恢复可绑。
