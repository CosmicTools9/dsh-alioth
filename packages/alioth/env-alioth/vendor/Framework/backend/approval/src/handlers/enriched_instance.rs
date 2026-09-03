//! Enriched ApprovalInstance Handler
//! GET /service/approval/approval-instances/enriched
//! Registered BEFORE CRUD /{id} to avoid catch-all conflict.

use actix_web::{web, HttpRequest, HttpResponse};
use common::error::AliothError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

#[derive(Debug, Deserialize, Default)]
pub struct EnrichedListQuery {
    #[serde(default, with = "common::serde_zuid::opt")]
    pub page: Option<i64>,
    #[serde(alias = "pageSize", default, with = "common::serde_zuid::opt")]
    pub page_size: Option<i64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    /// 模糊搜索：节点名（实例 notice）/ 申请人姓名 ILIKE
    #[serde(default)]
    pub q: Option<String>,
}

#[derive(Debug, FromRow)]
#[allow(dead_code)]
struct EnrichedRow {
    pub id: i64,
    // 列可空（实例 notice 允许 NULL）：Option 解码防御，避免共享库历史 NULL 行
    // 触发整表 500（2026-08-26 实证：employee_onboarding 遗留 NULL notice 行）
    pub node_name: Option<String>,
    pub code: Option<String>,
    pub fk_approve: Option<i64>,
    pub fk_subject: Option<i64>,
    pub comments: Option<String>,
    #[allow(dead_code)]
    pub lk_urgent: Option<i64>,
    pub priority_label: Option<String>,
    pub sla_hours: Option<i64>,
    pub timeline: Option<serde_json::Value>,
    pub applicant_name: Option<String>,
    pub derived_status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub total_count: i64,
}

#[derive(Debug, Serialize)]
pub struct EnrichedItem {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    // 与 EnrichedRow 对齐：实例 notice 可空，响应返回 null 而非 500
    pub node_name: Option<String>,
    pub code: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_approve: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_subject: Option<i64>,
    pub comments: Option<String>,
    pub applicant: Option<String>,
    pub status: String,
    pub result: String,
    pub priority: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sla: Option<i64>,
    pub timeline: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize)]
struct EnrichedListResponse {
    pub items: Vec<EnrichedItem>,
    #[serde(with = "common::serde_zuid")]
    pub total: i64,
    #[serde(with = "common::serde_zuid")]
    pub page: i64,
    #[serde(with = "common::serde_zuid")]
    pub page_size: i64,
}

pub async fn list_enriched(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    query: web::Query<EnrichedListQuery>,
) -> Result<HttpResponse, AliothError> {
    // Validate params——scope 三态：todo（默认，我的待办：处理人=我或岗位命中我，
    // 持 admin UA 的用户按管理视角全量可见）/ my-request（我发起的）/ all（全量，管理视角）
    let scope_key = query.scope.as_deref().unwrap_or("todo").to_string();
    match scope_key.as_str() {
        "todo" | "my-request" | "all" => {}
        other => {
            return Err(AliothError::Validation {
                field: "scope".into(),
                message: format!("invalid scope: {other}"),
            })
        }
    }
    let user_id = common::context::require_auth(&req)?;
    if let Some(s) = &query.status {
        if ![
            "pending",
            "approved",
            "rejected",
            "withdrawn",
            "cancelled",
            "abstained",
        ]
        .contains(&s.as_str())
        {
            return Err(AliothError::Validation {
                field: "status".into(),
                message: format!("invalid status: {s}"),
            });
        }
    }

    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1) * page_size;
    let q_pattern = query
        .q
        .as_deref()
        .map(|q| format!("%{}%", q.trim().replace('%', "")))
        .filter(|p| p != "%%");

    // Single CTE: derive status in SQL, apply ALL filters, paginate, count via window function
    let rows: Vec<EnrichedRow> = sqlx::query_as(
        r#"
        WITH base AS (
            SELECT i.id, i.notice AS node_name, i.code,
                   (SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
                    WHERE oe.ref_left = i.id AND oe.deleted_at IS NULL
                      AND EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_event oe2
                                  JOIN isahl.zc_id_process_rr_operation rro2
                                    ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL
                                  WHERE oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL)
                    ORDER BY oe.created_at LIMIT 1) AS fk_approve,
                   i.fk_subject, i.comments,
                   ev.lk_urgent, ev.timeline, e.notice AS applicant_name,
                   lu.notice AS priority_label,
                   sd.mark::bigint AS sla_hours,
                   COALESCE(st.code, 'pending') AS derived_status,
                   i.created_at, i.updated_at
            FROM isahl."zc_id_oper-approve" i
            LEFT JOIN isahl."zc_id_subj-employee" e ON e.id = i.fk_subject
            LEFT JOIN isahl."zc_id_even-approve" ev ON ev.id = (SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
                    WHERE oe.ref_left = i.id AND oe.deleted_at IS NULL
                      AND EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_event oe2
                                  JOIN isahl.zc_id_process_rr_operation rro2
                                    ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL
                                  WHERE oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL)
                    ORDER BY oe.created_at LIMIT 1) AND ev.deleted_at IS NULL
            LEFT JOIN isahl."zc_id_leve-urgent" lu ON lu.id = ev.lk_urgent
            LEFT JOIN isahl."zc_id_scal-duration" sd ON sd.id = ev.qk_sla
            LEFT JOIN LATERAL (
                SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" ls
                JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
                WHERE ls.ref_left = i.id AND ls.deleted_at IS NULL
                ORDER BY ls.id DESC LIMIT 1
            ) st ON true
            WHERE i.deleted_at IS NULL
              AND (
                $2::text = 'all'
                OR ($2::text = 'my-request' AND i.fk_subject = $3)
                OR ($2::text = 'todo' AND (
                  i.fk_operator = $3
                  OR EXISTS (
                    SELECT 1
                    FROM isahl.zc_id_operation_rr_event ie2
                    JOIN isahl."zc_id_even-approve" ea2
                      ON ea2.id = ie2.ref_right AND ea2.deleted_at IS NULL
                    JOIN isahl.zc_id_operation_rr_event oe2
                      ON oe2.ref_right = ea2.id AND oe2.deleted_at IS NULL
                      AND EXISTS (SELECT 1 FROM isahl.zc_id_process_rr_operation rro2
                                  WHERE rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL)
                    JOIN (
                      SELECT ref_left, ref_right FROM isahl."zc_id_operation_rr_approve" WHERE deleted_at IS NULL
                      UNION ALL
                      SELECT ref_left, ref_right FROM isahl."zc_id_operation_rr_review" WHERE deleted_at IS NULL
                      UNION ALL
                      SELECT ref_left, ref_right FROM isahl."zc_id_operation_rr_post" WHERE deleted_at IS NULL
                    ) br2 ON br2.ref_left = oe2.ref_left
                    JOIN isahl."zc_id_subj-position" pos2
                      ON pos2.id = br2.ref_right AND pos2.deleted_at IS NULL
                     AND pos2.fk_user = $3
                    WHERE ie2.ref_left = i.id AND ie2.deleted_at IS NULL
                  )
                  OR EXISTS (
                    SELECT 1 FROM isahl_auth.ngac_user_rr_attribute ur
                    JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute
                    WHERE ur.fk_user = $3 AND ua.o_name = 'admin'
                      AND ur.deleted_at IS NULL AND ua.deleted_at IS NULL
                  )
                ))
              )
        )
        SELECT *, COUNT(*) OVER() AS total_count
        FROM base
        WHERE ($1::text IS NULL OR derived_status = $1)
          AND ($4::text IS NULL OR node_name ILIKE $4 OR applicant_name ILIKE $4)
        ORDER BY created_at DESC
        LIMIT $5 OFFSET $6
        "#,
    )
    .bind(query.status.as_deref())  // $1: status filter (None = no filter)
    .bind(&scope_key)               // $2: scope key (todo/my-request/all)
    .bind(user_id)                  // $3: user_id for scope filter
    .bind(q_pattern.as_deref())     // $4: q ILIKE pattern (None = no filter)
    .bind(page_size)                // $5: limit
    .bind(offset)                   // $6: offset
    .fetch_all(pool.get_ref())
    .await
    .map_err(|e| AliothError::Database(e.to_string()))?;

    let total = rows.first().map(|r| r.total_count).unwrap_or(0);

    let items = rows
        .into_iter()
        .map(|r| {
            let result = match r.derived_status.as_str() {
                "approved" => "approved",
                "rejected" => "rejected",
                _ => "pending",
            };
            EnrichedItem {
                id: r.id,
                node_name: r.node_name,
                code: r.code,
                fk_approve: r.fk_approve,
                fk_subject: r.fk_subject,
                comments: r.comments,
                applicant: r.applicant_name,
                status: r.derived_status,
                result: result.to_string(),
                sla: r.sla_hours,
                timeline: r.timeline,
                priority: r.priority_label,
                created_at: r.created_at,
                updated_at: r.updated_at,
            }
        })
        .collect();

    Ok(HttpResponse::Ok().json(EnrichedListResponse {
        items,
        total,
        page,
        page_size,
    }))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route("/approval-instances/enriched", web::get().to(list_enriched));
}
