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
use sqlx::PgPool;

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

/// 几何列形态前置断言：运行时支持形态 = PostGIS `geometry(…,4326)`。
///
/// 用户裁决 2026-09-21（ADR `D-028` §7）：运行时**仅**支持 PostGIS 形态库；发布产物的
/// PG 原生几何形态（`point`/`path`/`polygon`）仅用于开源模型分发。非支持形态下 MUST
/// fail-loud 点名报错（实际列类型 + 整改指引），MUST NOT 透出底层方言错误
/// （原生形态下富化 SQL 会在计划期报 `函数 decode(point, unknown) 不存在`）。
///
/// 探测走 `information_schema.columns`（无需行数据，空表亦可判定）；判据 = `udt_schema`
/// 为 `postgis`（实测运行时形态 `postgis.geometry` / 发布形态 `pg_catalog.point`）。
async fn require_postgis_geometry_form(pool: &PgPool) -> Result<(), ApiError> {
    let udt: Option<String> = sqlx::query_scalar(
        r#"SELECT udt_schema || '.' || udt_name
           FROM information_schema.columns
           WHERE table_schema = 'isahl'
             AND table_name = 'zc_id_geom-coordinate'
             AND column_name = 'point'"#,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    match udt.as_deref() {
        Some(t) if t.starts_with("postgis.") => Ok(()),
        other => Err(ApiError::Internal(format!(
            "几何列形态不受支持：isahl.\"zc_id_geom-coordinate\".point 期望 postgis.geometry(Point,4326)，实际 {}。\
             所连库是「零 PostGIS」发布形态（仅用于开源模型分发，见 ADR D-028 §7）；\
             运行时几何读径需要 PostGIS 形态库。",
            other.unwrap_or("（列不存在）")
        ))),
    }
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
    // 形态断言只在真正要走几何读径时执行（`qk_path` 全空的常规列表路径零额外查询）
    require_postgis_geometry_form(pool).await?;
    let rows: Vec<(i64, serde_json::Value)> = sqlx::query_as(
        // zc_id_geom-coordinate 经纬度在 point 列（EWKB hex，schema 迁移替代 mark_axis0/1）——
        // 批注：traffic-lines 500 根因（c.mark_axis0 不存在）——postgis 解码
        r#"SELECT gp.id,
               COALESCE(
                 -- ID_JSON_PRECISION §规约：`id` 为 id 语义键，MUST 字符串化输出——
                 -- 坐标行 id 量级 10^17 > 2^53，jsonb number 直出经前端 JSON.parse 静默截断
                 -- （实测：同路径两条节点解析成同一值）
                 (SELECT jsonb_agg(
                    jsonb_build_object('id', c.id::text, 'name', c.notice,
                                       'lng', postgis.ST_X(postgis.ST_GeomFromEWKB(decode(c.point, 'hex'))),
                                       'lat', postgis.ST_Y(postgis.ST_GeomFromEWKB(decode(c.point, 'hex'))))
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

/// 线路目录读径谓词 = 运营态（功能阶段「实现」）。
///
/// 「线路有重复的」根因：本表同时存在 实现·实例（运营线路，种子 `TL-ROUTE-*`）与
/// 设计·实例（线路方案/运力池归属锚点）两类行，二者在列表里同名并列 ⇒ 同一线路出现两次。
/// 运营目录只列运营态行；设计态行**不删不丢**（`get`/`update` 仍按 id 可达，
/// 并被运力池产品 `fk_line` 引用），只是不进运营线路列表。
///
/// 谓词取 `_f_`（`dk_function.code` 前缀派生列，ALIOTH_ONTOLOGY_SPEC §4.3.1）——
/// 类谓词消费者是派生列的合法用法；`_f_`/`_t_` 仍禁止出现在 DTO 中（§4.3）。
/// 静态字面量、无绑定占位符（`crud::entity::ROW_FILTER` 的拼接纪律）。
const OPERATIONAL_LINE_FILTER: &str = r#""_f_" = '实现'"#;

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
        let pg = &self.pool;
        let mut page = crud::query_builder::QueryBuilder::<TrafficLine>::from_list_query(pg, query)
            .raw_filter(OPERATIONAL_LINE_FILTER.to_string())
            .fetch_refs(query.page, query.page_size)
            .await?;
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
        let coords = ontology_binding::coords_for_entity("TrafficLine")?;
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "TrafficLine").await?;
        // `_f_`/`_t_` 单一派生源：`dk_function.code` 前缀（ALIOTH_ONTOLOGY_SPEC §4.3.3 形态 1）。
        // 本仓储是裸 SQL 写路径（不经 GenericRepository + LifecycleBizTemplate），故与
        // consignment-writer / contract-writer / waybill-writer 同款——取值后参数绑定；
        // **禁字面量对**、**禁客户端传入**（DTO 已移除该两列）。
        let (form, tier) =
            trigger_registry::lifecycle::derive_form_type(coords.2).ok_or_else(|| {
                ApiError::Internal(format!(
                    "TrafficLine 职能码 {} 无法派生 _f_/_t_（须为 !./!_/↑./↑_/↓./↓_ 六前缀之一）",
                    coords.2
                ))
            })?;
        sqlx::query_as::<_, TrafficLine>(
            r#"INSERT INTO "isahl"."zc_id_stor-traffic_line" (code, notice, comments, "_f_", "_t_", fk_trustee, qk_path, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
               RETURNING id, code, notice, comments, fk_trustee, qk_path, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code)
        .bind(&req.notice)
        .bind(&req.comments)
        .bind(form)
        .bind(tier)
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
        // 全静态 SQL：表名/列名编译期固定（表位+列位零 `format!` 插值）。
        // 缺省字段沿用既有值（`None` = 不改动该列，与旧动态 `SET` 逐条等价）——由 SQL 侧
        // `COALESCE($n, col)` 承担，故**单条语句**即可，无需读—改—写，且返回形态与旧实现
        // 一致（`RETURNING` 原始行列，不经 `get` 的 `_refs`/坐标节点回读）。
        // `_f_`/`_t_` 不接受写入（§4.3/§4.3.3）：生命周期轴随 dk_function 派生，改形态须走类转换原语。
        if req.code.is_none()
            && req.notice.is_none()
            && req.comments.is_none()
            && req.fk_trustee.is_none()
            && req.qk_path.is_none()
        {
            // 无字段改动：保持旧行为（不写库、不推进 updated_at/updated_by_id）
            return self.get(id).await;
        }

        sqlx::query_as::<_, TrafficLine>(
            r#"UPDATE "isahl"."zc_id_stor-traffic_line"
               SET code = COALESCE($1, code), notice = COALESCE($2, notice),
                   comments = COALESCE($3, comments), fk_trustee = COALESCE($4, fk_trustee),
                   qk_path = COALESCE($5, qk_path),
                   updated_at = NOW(), updated_by_id = $6
               WHERE id = $7 AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_trustee, qk_path, created_at, updated_at, deleted_at"#,
        )
        .bind(req.code)
        .bind(req.notice)
        .bind(req.comments)
        .bind(req.fk_trustee)
        .bind(req.qk_path)
        .bind(_user_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
