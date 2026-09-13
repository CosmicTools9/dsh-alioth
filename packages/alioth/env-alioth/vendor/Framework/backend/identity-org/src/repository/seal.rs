// SealRepository（split 自 repository.rs 单体，④ 候选）
//! 身份实体 Repository — 标准 CRUD 实现
//!
//! Identity 使用自定义 Repository，其余实体组合 GenericRepository，
//! 仅自定义 create/update 的 INSERT/UPDATE SQL。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::repository::AliothRepository;
use crud::GenericRepository;
use serde_json::Value;
use sqlx::PgPool;

use crate::models::{CreateSealBatchRequest, CreateSealRequest, Seal, UpdateSealRequest};

use super::ontology_binding;
use super::resolve_seal_waybill_code;

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

        // 关联运单编号落 projection（Seal 无运单列；comments 保持自由文本）——
        // 先于事务解析：非法运单 id 立即 400，不留半成品
        let waybill_code = match req.waybill_id {
            Some(wid) => Some(resolve_seal_waybill_code(&self.pool, wid).await?),
            None => None,
        };

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
        // 叶表坐标（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "Seal").await?;
        let mut items = Vec::with_capacity(codes.len());
        for code in &codes {
            let item = sqlx::query_as::<_, Seal>(
                    r#"INSERT INTO "isahl"."zc_id_devi-seal" (notice, code, comments, projection, ck_category, created_by_id, dk_scene, dk_factor, dk_function)
                       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                       RETURNING id, notice, code, comments, ck_category, created_at, updated_at, deleted_at"#,
                )
                .bind(&req.notice)
                .bind(code)
                .bind(&req.comments)
                .bind(&waybill_code)
                .bind(Option::<i64>::None)
                .bind(user_id)
                .bind(dk_scene)
                .bind(dk_factor)
                .bind(dk_function)
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
        // 关联运单编号落 projection（Seal 无运单列；comments 保持自由文本，不再承载 JSON）
        let waybill_code = match req.waybill_id {
            Some(wid) => Some(resolve_seal_waybill_code(&self.pool, wid).await?),
            None => None,
        };
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "Seal").await?;
        sqlx::query_as::<_, Seal>(
            r#"INSERT INTO "isahl"."zc_id_devi-seal" (notice, code, comments, projection, ck_category, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING id, notice, code, comments, ck_category, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&waybill_code)
        .bind(Option::<i64>::None)
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
        // 关联运单变更：运单 id → 编号落 projection；None = 不改动该列（comments 不承载 JSON）
        let waybill_code = match req.waybill_id {
            Some(wid) => Some(resolve_seal_waybill_code(&self.pool, wid).await?),
            None => None,
        };
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_devi-seal" SET notice = $1, code = $2, comments = $3,
               ck_category = $4, updated_at = NOW(), updated_by_id = $5,
               projection = COALESCE($6, projection) WHERE id = $7"#,
        )
        .bind(&entity.notice)
        .bind(&entity.code)
        .bind(&entity.comments)
        .bind(entity.ck_category)
        .bind(user_id)
        .bind(&waybill_code)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(ApiError::from)?;
        // 回读：更新后的 waybill_no（projection → 编号）随行返回，避免陈旧关联
        self.get(id).await
    }
    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

/// 围栏几何类型（add-fence-geometry-types）：circle=方圆、area=区域、polygon=自定义多边形。
/// 类型决定物理叶表（zc_id_geog-circle / zc_id_geog-area / zc_id_geog-polygon），
/// 创建后不可变更——换类型必须删除重建，禁止隐式迁移或回退兜底。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FenceKind {
    Circle,
    Area,
    Polygon,
}

impl FenceKind {
    /// 请求侧 `fence_type` 解析：None → circle（缺省）；非法值 → 400。
    pub(crate) fn from_request(v: Option<&str>) -> Result<Self, ApiError> {
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
pub(crate) const FENCE_UNION_SELECT: &str = r#"SELECT 'circle' AS fence_type, id, notice, code, comments,
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
pub(crate) fn normalize_area_bounds(v: &Value) -> Result<(f64, f64, f64, f64), ApiError> {
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
pub(crate) fn normalize_polygon_points(v: &Value) -> Result<Vec<(f64, f64)>, ApiError> {
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
pub(crate) fn polygon_wkt(pts: &[(f64, f64)]) -> String {
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
