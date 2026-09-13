// TrafficLineRepository（split 自 repository.rs 单体，④ 候选）
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

use crate::models::{CreateTrafficLineRequest, TrafficLine, UpdateTrafficLineRequest};

use super::ontology_binding;

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
pub(crate) async fn resolve_coord_sys_id(
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
pub(crate) fn normalize_circle_point(val: &serde_json::Value) -> Result<(f64, f64), ApiError> {
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
            r#"INSERT INTO "isahl"."zc_id_stor-traffic_line" (code, notice, comments, "_f_", "_t_", fk_trustee, qk_path, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
               RETURNING id, code, notice, comments, "_f_", "_t_", fk_trustee, qk_path, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code)
        .bind(&req.notice)
        .bind(&req.comments)
        .bind(&req._f_)
        .bind(&req._t_)
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
        if req._f_.is_some() {
            idx += 1;
            sets.push(format!("\"_f_\" = ${}", idx));
        }
        if req._t_.is_some() {
            idx += 1;
            sets.push(format!("\"_t_\" = ${}", idx));
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
               RETURNING id, code, notice, comments, "_f_", "_t_", fk_trustee, qk_path, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, TrafficLine>(AssertSqlSafe(sql.as_str()));
        if let Some(v) = &req.code {
            q = q.bind(v);
        }
        if let Some(v) = &req.notice {
            q = q.bind(v);
        }
        if let Some(v) = &req.comments {
            q = q.bind(v);
        }
        if let Some(v) = &req._f_ {
            q = q.bind(v);
        }
        if let Some(v) = &req._t_ {
            q = q.bind(v);
        }
        if let Some(v) = &req.fk_trustee {
            q = q.bind(v);
        }
        if let Some(v) = &req.qk_path {
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
