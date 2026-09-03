//! 运输追踪子资源 Handler — 提供 `/{id}/points` 轨迹点数据
//!
//! 从 `zc_id_even-tracking` 中查询与 transport-tracking 记录关联的轨迹事件。

use actix_web::{web, HttpRequest, HttpResponse};
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::Serialize;
use sqlx::PgPool;

#[derive(Debug, Serialize)]
pub struct TrackingPoint {
    pub loc: String,
    pub time: String,
    pub status: String,
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/transport-tracking/{id}/points",
        web::get().to(get_tracking_points),
    )
    .route("/vehicle-types", web::get().to(list_vehicle_types));
}

/// GET /api/service/identity/transport-tracking/{id}/points
async fn get_tracking_points(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = common::context::require_auth(&req)?;
    let tracking_id = path.into_inner();
    require_resource_access(
        pool.get_ref(),
        user_id,
        "transport-tracking",
        tracking_id,
        "read",
    )
    .await?;

    // 从 zc_id_even-tracking 查关联此追踪记录的轨迹点
    // 通过 fk_subject 关联（tracking 记录 ID 作为 subject）
    let rows = sqlx::query_as::<_, (String, Option<i64>, String)>(
        r#"SELECT COALESCE(notice, '') AS loc,
                  qk_date AS time,
                  COALESCE(code, 'enroute') AS status
           FROM "isahl.zc_id_even-tracking"
           WHERE fk_subject = $1 AND deleted_at IS NULL
           ORDER BY COALESCE(qk_date, 0), id
           LIMIT 50"#,
    )
    .bind(tracking_id)
    .fetch_all(pool.get_ref())
    .await?;

    let points: Vec<TrackingPoint> = rows
        .into_iter()
        .map(|(loc, time, status)| TrackingPoint {
            loc,
            time: time.map(|t| t.to_string()).unwrap_or_default(),
            status,
        })
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(points)))
}

/// GET /api/service/isahl-db/vehicle-types — 车辆类型（新建委托选车型）
pub async fn list_vehicle_types(
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    let user_id = common::context::require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "list").await?;
    // 批注（用户）：母类保留字典（分组用）但下拉不显示——is_group 标记（无规格后缀的 6 个母类）
    // 批注 2026-08-20：vehicle_count 按 code 前缀继承——车辆挂母类（VT-HIGH）时，
    // 具体型号（VT-HIGH-13）也能统计到（vc.code = c.code 或互为前缀），
    // 修复「有该类型车辆却显示无该类型车辆」
    let rows = sqlx::query_as::<_, (i64, Option<String>, Option<String>, i64)>(
        r#"SELECT c.id, c.code, c.notice,
                  (SELECT COUNT(*) FROM isahl."zc_id_stor-ctn-vehicle" v
                   JOIN isahl."zc_id_cons-r-type-cate" vc ON vc.id = v."ck_r-type"
                   WHERE v.deleted_at IS NULL
                     AND (vc.code = c.code
                          OR vc.code LIKE c.code || '-%'
                          OR c.code LIKE vc.code || '-%'))
           FROM isahl."zc_id_cons-r-type-cate" c
           WHERE c.deleted_at IS NULL AND c.notice IS NOT NULL
           ORDER BY c.id LIMIT 100"#,
    )
    .fetch_all(pool.get_ref())
    .await?;
    // 批注（用户）：无数字规格的类型（飞翼车/自卸车/集装箱/危险品/面包车等）也是大类——不显示
    // 判定：名称含数字规格（9.6m/13m/17.5m/40吨/4.2m）→ 具体型号显示；否则隐藏
    let groups: std::collections::HashSet<&str> = [
        "VT-BOX", "VT-FLAT", "VT-HIGH", "VT-COLD", "VT-TANK", "VT-OPEN",
    ]
    .into_iter()
    .collect();
    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, code, name, count)| {
            let has_spec = name
                .as_deref()
                .map(|n| n.chars().any(|c| c.is_ascii_digit()))
                .unwrap_or(false);
            let is_group =
                code.as_deref().map(|c| groups.contains(c)).unwrap_or(false) || !has_spec;
            serde_json::json!({
                // ZUID 安全（serde_zuid 同款约定）：id 为 i64 大整数，26/32 超 JS 2^53——
                // number 直出经 JSON.parse 精度截断 → 前端选中值与真实 id 失配
                //（车型选中丢失/提交错 ck_r-type）。id 字符串化，vehicle_count 量级小保留 number。
                "id": id.to_string(), "code": code, "name": name, "vehicle_count": count,
                "is_group": is_group,
            })
        })
        .collect();
    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}
