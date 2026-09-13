// FenceRepository（split 自 repository.rs 单体，④ 候选）
//! 身份实体 Repository — 标准 CRUD 实现
//!
//! Identity 使用自定义 Repository，其余实体组合 GenericRepository，
//! 仅自定义 create/update 的 INSERT/UPDATE SQL。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::repository::AliothRepository;
use sqlx::{AssertSqlSafe, PgPool};

use crate::models::{CreateFenceRequest, Fence, UpdateFenceRequest};

use super::FenceKind;
use super::{
    normalize_area_bounds, normalize_circle_point, normalize_polygon_points, polygon_wkt,
    resolve_coord_sys_id, FENCE_UNION_SELECT,
};

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
