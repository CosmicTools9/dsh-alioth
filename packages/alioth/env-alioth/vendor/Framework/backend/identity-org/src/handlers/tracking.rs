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
           FROM isahl."zc_id_even-tracking"
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

/// 车型目录查询（含 vehicle_count 与 `r-form`）；注册为 test seam：
/// 计数回归测试（`tests/vehicle_type_count_test.rs`）与本 handler 共用同一 SQL 文本。
///
/// 计数口径 = 该行**及其子孙型号**的实车数（母类聚合；型号只算自身与更深子孙）——
/// MUST NOT 把「挂在母类上的车」下沉计入兄弟型号
///（openspec/specs/vehicle-form-dict :: vehicle-form-count-scope-excludes-ancestors）。
pub const VEHICLE_TYPE_LIST_SQL: &str = r#"SELECT c.id, c.code, c.notice,
                  (SELECT COUNT(*) FROM isahl."zc_id_stor-ctn-vehicle" v
                   JOIN isahl."zc_id_cons-r-type-cate" vc ON vc.id = v."ck_r-type"
                   WHERE v.deleted_at IS NULL
                     AND (vc.code = c.code
                          OR vc.code LIKE c.code || '-%')),
                  c."r-form"
           FROM isahl."zc_id_cons-r-type-cate" c
           WHERE c.deleted_at IS NULL AND c.notice IS NOT NULL
           ORDER BY c.id LIMIT 100"#;

/// 车型目录查询行型（id / code / notice / vehicle_count / r-form）
pub type VehicleTypeRow = (
    i64,
    Option<String>,
    Option<String>,
    i64,
    Option<serde_json::Value>,
);

/// GET /api/service/isahl-db/vehicle-types — 车辆类型（新建委托选车型）
pub async fn list_vehicle_types(
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    let user_id = common::context::require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "list").await?;
    // 归类判据 = 本表列 "r-form"（is_group_from_form）：有有效规格 = 具体型号（下拉可选）；
    // 空 = 母类/分组节点（下拉不显示，code 前缀供分组与计数聚合）。
    // 计数口径见 VEHICLE_TYPE_LIST_SQL 文档：型号行不含祖先（母类）绑定，避免计数膨胀
    let rows = sqlx::query_as::<_, VehicleTypeRow>(VEHICLE_TYPE_LIST_SQL)
        .fetch_all(pool.get_ref())
        .await?;
    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, code, name, count, r_form)| {
            serde_json::json!({
                // ZUID 安全（serde_zuid 同款约定）：id 为 i64 大整数，26/32 超 JS 2^53——
                // number 直出经 JSON.parse 精度截断 → 前端选中值与真实 id 失配
                //（车型选中丢失/提交错 ck_r-type）。id 字符串化，vehicle_count 量级小保留 number。
                "id": id.to_string(), "code": code, "name": name, "vehicle_count": count,
                "is_group": is_group_from_form(r_form.as_ref()),
            })
        })
        .collect();
    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

/// 车型字典归类判据（`isahl."zc_id_cons-r-type-cate"."r-form"`）：
/// 具备有效规格（`length` 或 `width` 为非空字符串）= 具体型号；否则 = 母类/分组节点。
/// 判据 MUST 取自模型列，MUST NOT 依赖显示名（是否含数字）或硬编码 code 清单。
fn is_group_from_form(form: Option<&serde_json::Value>) -> bool {
    let Some(obj) = form.and_then(|v| v.as_object()) else {
        return true;
    };
    let non_empty = |key: &str| {
        obj.get(key)
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    };
    !(non_empty("length") || non_empty("width"))
}

#[cfg(test)]
mod vehicle_form_group_tests {
    use super::is_group_from_form;
    use serde_json::json;

    #[test]
    fn size_bearing_form_is_model() {
        assert!(!is_group_from_form(Some(&json!({
            "width": "2.4m", "height": "2.6m", "length": "13m"
        }))));
        assert!(!is_group_from_form(Some(&json!({"length": "13m"}))));
        assert!(!is_group_from_form(Some(&json!({"width": "2.4m"}))));
    }

    #[test]
    fn form_without_usable_size_is_group() {
        assert!(is_group_from_form(None));
        assert!(is_group_from_form(Some(&json!({}))));
        assert!(is_group_from_form(Some(&json!({
            "length": "", "width": "  ", "height": "2m"
        }))));
    }

    /// 回归：具体型号的显示名不含 ASCII 数字（如「面包车(依维柯)」形态）时，
    /// 旧判据（名称启发式）会把它归母类而从下拉消失；判据切到 r-form 后，
    /// 只要行带规格就必须是型号——本函数不接收名称，名称不参与判定。
    #[test]
    fn model_classification_never_depends_on_name() {
        let spec = json!({"width": "1.8m", "height": "1.9m", "length": "4.2m"});
        assert!(!is_group_from_form(Some(&spec)));
    }
}
