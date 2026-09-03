//! 审批时间线 Handler — 延迟加载审批实例的完整节点时间线
//! GET /api/service/approval/approval-instances/{id}/timeline

use actix_web::{web, HttpRequest, HttpResponse};
use common::error::AliothError;
use common::permissions::require_resource_access;
use serde::Serialize;
use sqlx::PgPool;

#[derive(Debug, Serialize)]
pub struct TimelineNode {
    pub node_name: Option<String>,
    pub approver: Option<String>,
    pub time: Option<String>,
    pub status: Option<String>,
    pub opinion: Option<String>,
}

/// 时间线查询行（带 LEFT JOIN 字段）
#[derive(Debug, sqlx::FromRow)]
struct TimelineRow {
    pub notice: Option<String>,
    pub opinion: Option<String>,
    pub time: Option<chrono::DateTime<chrono::Utc>>,
    pub subject_name: Option<String>,
}

/// 将 notice 文本映射为状态标签
fn status_from_notice(notice: &str) -> &str {
    match notice {
        "审批通过" => "approved",
        "审批驳回" => "rejected",
        "审批转交" => "transferred",
        "审批加签" => "cc",
        "弃权" => "abstained",
        _ => "pending",
    }
}

/// GET /api/service/approval/approval-instances/{id}/timeline
pub async fn get_timeline(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    let user_id = common::context::require_auth(&req)?;
    let instance_id = path.into_inner();
    require_resource_access(
        pool.get_ref(),
        user_id,
        "approval-instances",
        instance_id,
        "read",
    )
    .await?;

    // 1. 校验审批实例存在（时间线按实例聚合意见，fk_list = 实例 id——fk_index 契约）
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM isahl.\"zc_id_oper-approve\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(instance_id)
    .fetch_optional(&**pool)
    .await?
    .ok_or_else(|| AliothError::NotFound("ApprovalInstance".into()))?;

    // 2. 查询时间线: zc_id_deta-opinion + LEFT JOIN 审批人
    let rows = sqlx::query_as::<_, TimelineRow>(
        "SELECT a.notice, \
         a.opinion AS opinion, \
         COALESCE(sd.date, a.created_at) AS time, \
         s.notice AS subject_name \
         FROM isahl.\"zc_id_deta-opinion\" a \
         LEFT JOIN isahl.\"zc_id_scal-date\" sd ON sd.id = a.qk_date \
         LEFT JOIN isahl.\"zc_id_subj-employee\" s ON s.id = a.fk_biller \
         WHERE a.fk_list = $1 AND a.deleted_at IS NULL \
         ORDER BY a.created_at ASC, a.id ASC",
    )
    .bind(instance_id)
    .fetch_all(&**pool)
    .await?;

    // 3. 转换为 TimelineNode
    let nodes: Vec<TimelineNode> = rows
        .into_iter()
        .map(|r| TimelineNode {
            node_name: r.notice.clone(),
            approver: r.subject_name,
            time: r.time.map(|dt| dt.to_rfc3339()),
            status: r
                .notice
                .as_deref()
                .map(status_from_notice)
                .map(|s| s.to_string()),
            opinion: r.opinion,
        })
        .collect();

    Ok(HttpResponse::Ok().json(nodes))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/approval-instances/{id}/timeline",
        web::get().to(get_timeline),
    );
}
