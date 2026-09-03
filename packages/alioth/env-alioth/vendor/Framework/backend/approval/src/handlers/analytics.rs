//! 审批效能分析 Handler
//!
//! 3 个聚合端点：node-durations / approver-workloads / bottlenecks
//! 数据来源：isahl.zc_id_oper-approve + isahl.zc_id_deta-opinion

use actix_web::{web, HttpResponse};
use common::error::AliothError;
use common::ApiResponse;
use serde::Serialize;
use sqlx::PgPool;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExemplarEfficiency {
    #[serde(with = "common::serde_zuid")]
    pub exemplar_id: i64,
    pub exemplar_name: String,
    #[serde(with = "common::serde_zuid")]
    pub execution_count: i64,
    #[serde(with = "common::serde_zuid")]
    pub completed_count: i64,
    #[serde(with = "common::serde_zuid")]
    pub in_flight_count: i64,
    pub avg_duration_ms: f64,
    pub p50_duration_ms: f64,
    pub p95_duration_ms: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeDuration {
    pub node_name: String,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    #[serde(with = "common::serde_zuid")]
    pub sample_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproverWorkload {
    pub approver_name: String,
    #[serde(with = "common::serde_zuid")]
    pub pending_count: i64,
    pub daily_avg: f64,
    pub avg_duration_ms: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bottleneck {
    #[serde(with = "common::serde_zuid")]
    pub instance_id: i64,
    pub flow_name: String,
    pub current_node: String,
    #[serde(with = "common::serde_zuid")]
    pub sla_hours: i64,
    pub elapsed_hours: f64,
    pub exceeded_hours: f64,
}

/// GET /analytics/node-durations?from=&to=
async fn node_durations(pool: web::Data<PgPool>) -> Result<HttpResponse, AliothError> {
    let rows = sqlx::query_as::<_, (String, f64, f64, f64, f64, i64)>(
        r#"
        SELECT
            COALESCE(an.notice, a.notice, 'unknown') AS node_name,
            PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY dur::float8) AS p50,
            PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY dur::float8) AS p95,
            PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY dur::float8) AS p99,
            MAX(dur::float8) AS max_dur,
            COUNT(*)::bigint AS sample_count
        FROM (
            SELECT a.*,
                EXTRACT(EPOCH FROM (
                    (SELECT o.created_at
                     FROM isahl."zc_id_deta-opinion" o
                     WHERE o.fk_list = a.id AND o.deleted_at IS NULL
                       AND o.notice IN ('审批通过', '审批驳回')
                     ORDER BY o.created_at DESC LIMIT 1)
                    - a.created_at
                )) AS dur
            FROM isahl."zc_id_oper-approve" a
            WHERE a.deleted_at IS NULL
        ) a
        LEFT JOIN isahl.zc_id_operation_rr_event oe2 ON oe2.ref_left = a.id AND oe2.deleted_at IS NULL
        LEFT JOIN isahl."zc_id_even-approve" an ON an.id = oe2.ref_right
            AND EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_event oe_n
                        JOIN isahl.zc_id_process_rr_operation rro_n
                          ON rro_n.ref_right = oe_n.ref_left AND rro_n.deleted_at IS NULL
                        WHERE oe_n.ref_right = an.id AND oe_n.deleted_at IS NULL)
        WHERE a.dur IS NOT NULL
        GROUP BY COALESCE(an.notice, a.notice, 'unknown')
        ORDER BY sample_count DESC
        "#,
    )
    .fetch_all(&**pool)
    .await
    .map_err(AliothError::from)?;

    let result: Vec<NodeDuration> = rows
        .into_iter()
        .map(|r| NodeDuration {
            node_name: r.0,
            p50_ms: r.1 * 1000.0,
            p95_ms: r.2 * 1000.0,
            p99_ms: r.3 * 1000.0,
            max_ms: r.4 * 1000.0,
            sample_count: r.5,
        })
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(result)))
}

/// GET /analytics/approver-workloads
async fn approver_workloads(pool: web::Data<PgPool>) -> Result<HttpResponse, AliothError> {
    let rows = sqlx::query_as::<_, (String, i64, f64, f64)>(
        r#"
        SELECT
            COALESCE(u.username, 'unknown') AS approver_name,
            COUNT(*) FILTER (WHERE a.deleted_at IS NULL
              AND NOT EXISTS (
                  SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                  JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
                  WHERE ls.ref_left = a.id AND ls.deleted_at IS NULL
                    AND s.code IN ('approved', 'rejected', 'withdrawn', 'cancelled', 'abstained')
              ))::bigint AS pending_count,
            ROUND(COUNT(*)::numeric / GREATEST(EXTRACT(DAY FROM (NOW() - MIN(a.created_at)))::numeric, 1), 1)::float8 AS daily_avg,
            COALESCE(AVG(EXTRACT(EPOCH FROM (
                (SELECT o.created_at
                 FROM isahl."zc_id_deta-opinion" o
                 WHERE o.fk_list = a.id AND o.deleted_at IS NULL
                 ORDER BY o.created_at DESC LIMIT 1)
                 - a.created_at
                 ))::float8) * 1000, 0) AS avg_duration_ms
        FROM isahl."zc_id_oper-approve" a
        LEFT JOIN isahl_auth.auth_users u ON u.id = COALESCE(a.fk_operator, a.fk_subject)
        WHERE a.deleted_at IS NULL
        GROUP BY u.username
        ORDER BY pending_count DESC
        "#
    )
    .fetch_all(&**pool)
    .await
    .map_err(AliothError::from)?;

    let result: Vec<ApproverWorkload> = rows
        .into_iter()
        .map(|r| ApproverWorkload {
            approver_name: r.0,
            pending_count: r.1,
            daily_avg: r.2,
            avg_duration_ms: r.3,
        })
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(result)))
}

/// GET /analytics/bottlenecks
async fn bottlenecks(pool: web::Data<PgPool>) -> Result<HttpResponse, AliothError> {
    let rows = sqlx::query_as::<_, (i64, String, String, i64, f64, f64)>(
        r#"
        SELECT
            a.id AS instance_id,
            COALESCE(a.notice, 'unknown') AS flow_name,
            COALESCE(an.notice, a.notice, 'unknown') AS current_node,
            sd.mark::bigint AS sla_hours,
            EXTRACT(EPOCH FROM (NOW() - a.created_at))::float8 / 3600.0 AS elapsed_hours,
            GREATEST(EXTRACT(EPOCH FROM (NOW() - a.created_at))::float8 / 3600.0 - sd.mark::float8, 0) AS exceeded_hours
        FROM isahl."zc_id_oper-approve" a
        LEFT JOIN isahl.zc_id_operation_rr_event oe3 ON oe3.ref_left = a.id AND oe3.deleted_at IS NULL
        LEFT JOIN isahl."zc_id_even-approve" an ON an.id = oe3.ref_right
            AND EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_event oe_n
                        JOIN isahl.zc_id_process_rr_operation rro_n
                          ON rro_n.ref_right = oe_n.ref_left AND rro_n.deleted_at IS NULL
                        WHERE oe_n.ref_right = an.id AND oe_n.deleted_at IS NULL)
        LEFT JOIN isahl."zc_id_scal-duration" sd ON sd.id = an.qk_sla AND sd.deleted_at IS NULL
        WHERE a.deleted_at IS NULL
          AND sd.id IS NOT NULL
          AND NOT EXISTS (
              SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
              JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
              WHERE ls.ref_left = a.id AND ls.deleted_at IS NULL
                AND s.code IN ('approved', 'rejected', 'withdrawn', 'cancelled', 'abstained')
          )
          AND EXTRACT(EPOCH FROM (NOW() - a.created_at)) / 3600.0 > sd.mark::float8
        ORDER BY exceeded_hours DESC
        LIMIT 50
        "#
    )
    .fetch_all(&**pool)
    .await
    .map_err(AliothError::from)?;

    let result: Vec<Bottleneck> = rows
        .into_iter()
        .map(|r| Bottleneck {
            instance_id: r.0,
            flow_name: r.1,
            current_node: r.2,
            sla_hours: r.3,
            elapsed_hours: r.4,
            exceeded_hours: r.5,
        })
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(result)))
}

/// GET /analytics/exemplar-efficiency — 范例视角执行效能（flow-lifecycle-split）。
///
/// 执行 = proc 族「实现·实例」行（tpl_id → 范例行）。成员 = oper 实例链：
/// 链根 fk_previous 跨表锚定在执行行上，下游成员沿 oper 表内 fk_previous 递归
/// 展开。终态成员 = 生命周期桥终态存在（D7，替换原「最新审批意见非空」）；
/// 执行时长 = 执行行 created_at → 链内
/// 最后一条意见时间（仅完结执行计入分位/均值）。
async fn exemplar_efficiency(pool: web::Data<PgPool>) -> Result<HttpResponse, AliothError> {
    let rows = sqlx::query_as::<_, (i64, String, i64, i64, i64, f64, f64, f64)>(
        r#"
        WITH RECURSIVE exec AS (
            SELECT p.id AS exec_id, p.tpl_id AS exemplar_id, p.notice AS exemplar_name,
                   p.created_at AS started_at
            FROM isahl.zc_id_process p
            WHERE p.deleted_at IS NULL AND p._f_ = '实现' AND p._t_ = '实例'
              AND p.tpl_id IS NOT NULL
        ),
        member AS (
            SELECT o.id, o.fk_previous AS exec_ref, o.created_at
            FROM isahl."zc_id_oper-approve" o
            WHERE o.deleted_at IS NULL
              AND o.fk_previous IN (SELECT exec_id FROM exec)
            UNION ALL
            SELECT o.id, m.exec_ref, o.created_at
            FROM isahl."zc_id_oper-approve" o
            JOIN member m ON o.fk_previous = m.id
            WHERE o.deleted_at IS NULL
        ),
        exec_stat AS (
            SELECT m.exec_ref,
                   COUNT(*) FILTER (WHERE NOT EXISTS (
                       SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                       JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
                       WHERE ls.ref_left = m.id AND ls.deleted_at IS NULL
                         AND s.code IN ('approved', 'rejected', 'withdrawn', 'cancelled', 'abstained')
                   )) AS pending_count,
                   MAX(COALESCE(lo.last_at, m.created_at)) AS last_at
            FROM member m
            LEFT JOIN LATERAL (
                SELECT a.notice, a.created_at AS last_at
                FROM isahl."zc_id_deta-opinion" a
                WHERE a.fk_list = m.id AND a.deleted_at IS NULL
                ORDER BY a.created_at DESC LIMIT 1
            ) lo ON true
            GROUP BY m.exec_ref
        )
        SELECT x.exemplar_id, x.exemplar_name,
               COUNT(*)::bigint AS execution_count,
               COUNT(*) FILTER (WHERE s.pending_count = 0)::bigint AS completed_count,
               COUNT(*) FILTER (WHERE s.pending_count > 0)::bigint AS in_flight_count,
               COALESCE(AVG(EXTRACT(EPOCH FROM (s.last_at - x.started_at)))
                   FILTER (WHERE s.pending_count = 0), 0)::float8,
               COALESCE(PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY
                   EXTRACT(EPOCH FROM (s.last_at - x.started_at)))
                   FILTER (WHERE s.pending_count = 0), 0)::float8,
               COALESCE(PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY
                   EXTRACT(EPOCH FROM (s.last_at - x.started_at)))
                   FILTER (WHERE s.pending_count = 0), 0)::float8
        FROM exec x
        LEFT JOIN exec_stat s ON s.exec_ref = x.exec_id
        GROUP BY x.exemplar_id, x.exemplar_name
        ORDER BY execution_count DESC
        LIMIT 100
        "#,
    )
    .fetch_all(pool.get_ref())
    .await
    .map_err(AliothError::from)?;

    let result: Vec<ExemplarEfficiency> = rows
        .into_iter()
        .map(|r| ExemplarEfficiency {
            exemplar_id: r.0,
            exemplar_name: r.1,
            execution_count: r.2,
            completed_count: r.3,
            in_flight_count: r.4,
            avg_duration_ms: r.5 * 1000.0,
            p50_duration_ms: r.6 * 1000.0,
            p95_duration_ms: r.7 * 1000.0,
        })
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(result)))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route("/analytics/node-durations", web::get().to(node_durations));
    cfg.route(
        "/analytics/approver-workloads",
        web::get().to(approver_workloads),
    );
    cfg.route("/analytics/bottlenecks", web::get().to(bottlenecks));
    cfg.route(
        "/analytics/exemplar-efficiency",
        web::get().to(exemplar_efficiency),
    );
}
