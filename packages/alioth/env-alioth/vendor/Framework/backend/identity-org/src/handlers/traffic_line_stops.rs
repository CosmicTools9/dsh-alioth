//! 线路↔场所桥 Handler（add-route-stop-bridge）
//!
//! 覆盖 `zc_id_stor-traffic_line_rr_stop`（ref_left=线路 id, ref_right=place id）：
//! - `GET /traffic-lines/{id}/stops` — 按 `over-seq` 升序返回 stops
//!   （place 名称/编码/坐标（qk_fence→geom-circle 圆心）+ ck_category code/notice）
//! - `PUT /traffic-lines/{id}/stops` — 单事务全量替换：`over-seq` = 数组下标
//!   （0=起点，count-1=终点）；place/category 存在性校验；旧行软删后按序插入
//!
//! 模式复制 subject_bridges.rs（同 lifecycle_rr_non_self 桥族）。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::Deserialize;
use sqlx::PgPool;

#[derive(Debug, Deserialize)]
pub struct StopInput {
    /// 场所 id（ref_right → zc_id_place）
    #[serde(with = "common::serde_zuid")]
    pub place_id: i64,
    /// 节点类型（ck_category → zc_id_cate-traffic，可空）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_category: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ReplaceStopsRequest {
    pub stops: Vec<StopInput>,
}

/// stops 列表行：桥 id / place id / 名称 / 编码 / 坐标 / over-seq / ck_category / 类目 code+notice
#[allow(clippy::type_complexity)]
type StopRow = (
    i64,
    i64,
    Option<String>,
    Option<String>,
    Option<f64>,
    Option<f64>,
    Option<i32>,
    Option<i64>,
    Option<String>,
    Option<String>,
);

async fn ensure_line_exists(pool: &PgPool, line_id: i64) -> Result<(), ApiError> {
    let exists: bool = sqlx::query_scalar(
        r#"SELECT COUNT(*) > 0 FROM "isahl"."zc_id_stor-traffic_line" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(line_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !exists {
        return Err(ApiError::NotFound(format!("线路不存在: {line_id}")));
    }
    Ok(())
}

async fn list_stops(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let line_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "traffic_line", line_id, "read").await?;
    let rows: Vec<StopRow> = sqlx::query_as(
        r#"
          SELECT b.id, b.ref_right, p.notice, p.code,
                  f.lng, f.lat,
                  b."over-seq", b.ck_category, c.code AS category_code, c.notice AS category_notice
           FROM "isahl"."zc_id_stor-traffic_line_rr_stop" b
           LEFT JOIN "isahl"."zc_id_place" p ON p.id = b.ref_right AND p.deleted_at IS NULL
           -- 坐标回传（fix-wz-fence-data-chain）：circle 用圆心；area/polygon 用质心兜底；
           -- LATERAL 0-or-1 行，未绑/软删/几何缺失回 NULL——禁止经非叶父表 zc_id_geom-circle 读围栏
           LEFT JOIN LATERAL (
               SELECT lng, lat FROM (
                   SELECT ST_X(g.circle::geometry) AS lng, ST_Y(g.circle::geometry) AS lat
                   FROM "isahl"."zc_id_geog-circle" g
                   WHERE g.id = p.qk_fence AND g.deleted_at IS NULL
                   UNION ALL
                   SELECT ST_X(ST_Centroid(a.box::geometry)), ST_Y(ST_Centroid(a.box::geometry))
                   FROM "isahl"."zc_id_geog-area" a
                   WHERE a.id = p.qk_fence AND a.deleted_at IS NULL
                   UNION ALL
                   SELECT ST_X(ST_Centroid(pg.polygon::geometry)), ST_Y(ST_Centroid(pg.polygon::geometry))
                   FROM "isahl"."zc_id_geog-polygon" pg
                   WHERE pg.id = p.qk_fence AND pg.deleted_at IS NULL
               ) coords LIMIT 1
           ) f ON true
           LEFT JOIN "isahl"."zc_id_cate-traffic" c ON c.id = b.ck_category AND c.deleted_at IS NULL
           WHERE b.ref_left = $1 AND b.deleted_at IS NULL
           ORDER BY b."over-seq" ASC NULLS LAST, b.id"#,
    )
    .bind(line_id)
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(
            |(id, place_id, pname, pcode, lng, lat, seq, cat, cat_code, cat_notice)| {
                serde_json::json!({
                    "id": id.to_string(),
                    "place_id": place_id.to_string(),
                    "place_name": pname,
                    "place_code": pcode,
                    "lng": lng,
                    "lat": lat,
                    "seq": seq,
                    "ck_category": cat.map(|v| v.to_string()),
                    "category_code": cat_code,
                    "category_notice": cat_notice,
                })
            },
        )
        .collect();
    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

/// 全量替换（单事务）：旧行全部软删 → 按数组顺序插入（over-seq = 下标）。
/// 决策记录：不做逐行 diff——桥行 id 无外部引用，替换语义简单可预测；
/// 事务保证「校验失败零变更」。
async fn replace_stops(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<ReplaceStopsRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let line_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "traffic_line", line_id, "update").await?;
    ensure_line_exists(pool.get_ref(), line_id).await?;
    let stops = &body.stops;

    // 校验：place 全部存在于 zc_id_place（叶表直查——前端传入的是 zc_id_place 行 id）
    if !stops.is_empty() {
        let place_ids: Vec<i64> = stops.iter().map(|s| s.place_id).collect();
        let found: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*) FROM "isahl"."zc_id_place" WHERE id = ANY($1) AND deleted_at IS NULL"#,
        )
        .bind(&place_ids)
        .fetch_one(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?;
        if found != place_ids.len() as i64 {
            return Err(ApiError::Validation {
                field: "stops.place_id".into(),
                message: "存在悬空 place id（不在 zc_id_place 或已删除）".into(),
            });
        }
        let cat_ids: Vec<i64> = stops.iter().filter_map(|s| s.ck_category).collect();
        if !cat_ids.is_empty() {
            let found: i64 = sqlx::query_scalar(
                r#"SELECT COUNT(*) FROM "isahl"."zc_id_cate-traffic" WHERE id = ANY($1) AND deleted_at IS NULL"#,
            )
            .bind(&cat_ids)
            .fetch_one(pool.get_ref())
            .await
            .map_err(ApiError::from_sqlx)?;
            if found != cat_ids.len() as i64 {
                return Err(ApiError::Validation {
                    field: "stops.ck_category".into(),
                    message: "存在悬空类目 id（不在 zc_id_cate-traffic 或已删除）".into(),
                });
            }
        }
    }

    let mut tx = pool.get_ref().begin().await.map_err(ApiError::from_sqlx)?;
    sqlx::query(
        r#"UPDATE "isahl"."zc_id_stor-traffic_line_rr_stop"
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(line_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    for (idx, stop) in stops.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_stor-traffic_line_rr_stop"
               (notice, ref_left, ref_right, ck_category, "over-seq", created_by_id, updated_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $6)"#,
        )
        .bind(format!("line-{line_id} stop-{idx}"))
        .bind(line_id)
        .bind(stop.place_id)
        .bind(stop.ck_category)
        .bind(idx as i32)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
    }
    tx.commit().await.map_err(ApiError::from_sqlx)?;
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "line_id": line_id.to_string(),
            "count": stops.len(),
        }))),
    )
}

/// 注册线路 stops 桥路由（add-route-stop-bridge）
///
/// 与 crud_routes("/traffic-lines") 不同路径深度，无 actix 匹配冲突
/// （NGAC_SPEC §7.2 路由注册顺序铁律不涉及——非同径）。
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/traffic-lines/{id}/stops")
            .route(web::get().to(list_stops))
            .route(web::put().to(replace_stops)),
    );
}
