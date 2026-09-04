//! Gateway 全局工作区概览 API
//!
//! 聚合跨模块数据，供 Gateway 右侧 WorkspaceDock 使用。
//! 路由前缀: /api/global/overview
//!
//! 当前覆盖：
//! - approval：从 zc_id_oper-approve（审批实例）查询待我审批/我发起的审批项
//! - message：从 zc_id_message 查询最新站内信
//!
//! TODO：后续可扩展为统一聚合接口，接入更多全局数据源。
//!
//! 聚合查询边界（P1-5 预研结论）：本端点属跨表聚合（approval JOIN opinion
//! LATERAL、message JOIN contact-info、tableoid 派生列、用户 OR 过滤），
//! _refs 机制（HasReferenceJoins，单实体单跳 FK→目标表 display 字段）不覆盖；
//! convention_checker 确立「CRUD 归 list_refs/get_refs、聚合查询归 service 直写 SQL」
//! 分界。维持现状，勿重构为 _refs。

use actix_web::{web, HttpMessage, HttpResponse, Result};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// ============================================
// Response Types
// ============================================

#[derive(Debug, Serialize)]
pub struct GlobalOverviewResponse {
    pub success: bool,
    pub data: GlobalOverviewData,
}

#[derive(Debug, Serialize)]
pub struct GlobalOverviewData {
    pub approvals: Vec<ApprovalItem>,
    pub messages: Vec<MessageItem>,
    /// 精确计数（不受 LIMIT 20 截断）
    pub counts: Counts,
}

#[derive(Debug, Serialize)]
pub struct Counts {
    /// 待我审批 + 我发起的总数
    #[serde(with = "common::serde_zuid")]
    pub pending_total: i64,
    /// 未读消息数
    #[serde(with = "common::serde_zuid")]
    pub unread_total: i64,
}

/// 审批项（贴合前端 ApprovalItem 契约）
/// 新增 mine（是否为「我发起的」）与 operator_id（当前处理人）
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ApprovalItem {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub title: String,
    pub applicant: String,
    pub dept: String,
    pub code: String,
    pub status: String,
    pub time: String,
    /// 我发起的审批（created_by_id = 当前用户）
    pub mine: bool,
    /// 当前处理人（fk_operator）
    #[serde(with = "common::serde_zuid::opt")]
    pub operator_id: Option<i64>,
}

/// 消息项（贴合前端 InboxMessage 契约）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MessageItem {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[sqlx(rename = "from_user")]
    pub from_user: String,
    pub title: String,
    pub content: String,
    pub time: String,
    pub unread: bool,
    #[sqlx(rename = "msg_type")]
    pub msg_type: String,
}

// ============================================
// Handlers
// ============================================

pub async fn get_global_overview(
    req: actix_web::HttpRequest,
    pool: web::Data<sqlx::PgPool>,
) -> Result<HttpResponse> {
    let user_id = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id);
    // RLS（P1-4 完成态）：PEP 注入 global 资源可见 ID 集（registry 已注册），
    // Some 且非空 → SQL 叠加 id = ANY(visible_ids)；None → 维持 owner 过滤。
    let visible_ids = req
        .extensions()
        .get::<common::context::RequestContext>()
        .and_then(|ctx| ctx.get_visible_resource_ids("global").cloned())
        .filter(|ids| !ids.is_empty());

    let (approvals, messages, counts) = tokio::join!(
        fetch_pending_approvals(pool.get_ref(), user_id, visible_ids.as_deref()),
        fetch_recent_messages(pool.get_ref(), user_id),
        fetch_counts(pool.get_ref(), user_id),
    );

    Ok(HttpResponse::Ok().json(GlobalOverviewResponse {
        success: true,
        data: GlobalOverviewData {
            approvals,
            messages,
            counts,
        },
    }))
}

// ── T1.1: fetch_pending_approvals ─────────────────────────────────────────────
//
// WHERE 条件：fk_operator = $1 OR created_by_id = $1
//   - fk_operator = $1  → 待我审批
//   - created_by_id = $1 → 我发起的
// Response 扩展：mine（created_by_id = $1）、operator_id（fk_operator）
// RLS：visible_ids 非空时叠加 AND i.id = ANY($2)

async fn fetch_pending_approvals(
    pool: &sqlx::PgPool,
    user_id: Option<i64>,
    visible_ids: Option<&[i64]>,
) -> Vec<ApprovalItem> {
    let rls_clause = if visible_ids.is_some() {
        "AND i.id = ANY($2)"
    } else {
        ""
    };
    let sql = format!(
        r#"
        WITH enriched AS (
            SELECT
                i.id,
                COALESCE(i.notice, '未命名审批') AS title,
                COALESCE(e.notice, i.notice, '未知用户') AS applicant,
                COALESCE(i.code, '') AS code,
                CASE
                    WHEN act.notice IS NULL THEN 'pending'
                    WHEN act.notice = '审批通过' THEN 'approved'
                    WHEN act.notice = '审批驳回' THEN 'rejected'
                    ELSE 'active'
                END AS status,
                CASE
                    WHEN i.tableoid::regclass::text LIKE '%zc_id_appr%' THEN split_part(i.tableoid::regclass::text, '_', 3)
                    ELSE ''
                END AS dept,
                TO_CHAR(i.created_at, 'MM-DD HH24:MI') AS time,
                i.fk_operator,
                COALESCE(i.created_by_id = $1, false) AS mine
            FROM isahl."zc_id_oper-approve" i
            LEFT JOIN LATERAL (
                SELECT o.notice FROM isahl."zc_id_deta-opinion" o
                WHERE o.fk_list = i.id AND o.deleted_at IS NULL
                ORDER BY o.created_at DESC LIMIT 1
            ) act ON true
            LEFT JOIN isahl."zc_id_subj-employee" e ON e.id = i.fk_subject
            WHERE i.deleted_at IS NULL
              AND (i.fk_operator = $1 OR i.created_by_id = $1
                   OR EXISTS (
                       SELECT 1 FROM isahl_auth.ngac_user_rr_attribute ur
                       JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute
                       WHERE ur.fk_user = $1 AND ua.o_name = 'admin'
                         AND ur.deleted_at IS NULL AND ua.deleted_at IS NULL
                   ))
              {rls_clause}
        )
        SELECT id, title, applicant, dept, code, status, time,
               mine, fk_operator AS operator_id
        FROM enriched
        ORDER BY time DESC
        LIMIT 20
        "#,
        rls_clause = rls_clause
    );

    let mut query =
        sqlx::query_as::<_, ApprovalItem>(sqlx::AssertSqlSafe(sql.as_str())).bind(user_id);
    if let Some(ids) = visible_ids {
        query = query.bind(ids.to_vec());
    }
    match query.fetch_all(pool).await {
        Ok(items) => items,
        Err(e) => {
            common::telemetry::warn!("Failed to fetch pending approvals: {}", e);
            Vec::new()
        }
    }
}

// ── T1.1 / D5: fetch_counts ───────────────────────────────────────────────────
//
// pending_total：同一过滤口径 COUNT（不受 20 条限制）
// unread_total：复用 fetch_recent_messages 的口径但只 COUNT

async fn fetch_counts(pool: &sqlx::PgPool, user_id: Option<i64>) -> Counts {
    let pending_total = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM isahl."zc_id_oper-approve" i
        WHERE i.deleted_at IS NULL
          AND (i.fk_operator = $1 OR i.created_by_id = $1
               OR EXISTS (
                   SELECT 1 FROM isahl_auth.ngac_user_rr_attribute ur
                   JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute
                   WHERE ur.fk_user = $1 AND ua.o_name = 'admin'
                     AND ur.deleted_at IS NULL AND ua.deleted_at IS NULL
               ))
          AND NOT EXISTS (
              SELECT 1 FROM isahl."zc_id_deta-opinion" o
              WHERE o.fk_list = i.id AND o.deleted_at IS NULL
                AND o.notice IN ('审批通过', '审批驳回')
          )
        "#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    {
        Ok(n) => n,
        Err(e) => {
            common::telemetry::warn!("Failed to count pending approvals: {}", e);
            0
        }
    };

    let unread_total = match sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM isahl.zc_id_message m
        LEFT JOIN isahl."zc_id_message_rr_contact-info" ci
            ON ci.ref_left = m.id AND ci.ref_right = $1 AND ci.deleted_at IS NULL
        WHERE m.deleted_at IS NULL
          AND (m.created_by_id = $1 OR $1 = ANY(m.ak_benefit_user))
          AND COALESCE(ci.feedback::text <> 'read', true)
        "#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    {
        Ok(n) => n,
        Err(e) => {
            common::telemetry::warn!("Failed to count unread messages: {}", e);
            0
        }
    };

    Counts {
        pending_total,
        unread_total,
    }
}

/// 查询最近站内信
async fn fetch_recent_messages(pool: &sqlx::PgPool, user_id: Option<i64>) -> Vec<MessageItem> {
    let sql = r#"
        SELECT
            m.id,
            COALESCE(m.notice, '系统') AS from_user,
            COALESCE(m.notice, '无标题') AS title,
            COALESCE(m.comments, '') AS content,
            COALESCE(m.created_at::text, '') AS time,
            CASE
                WHEN $1 IS NULL THEN true
                ELSE COALESCE(ci.feedback::text <> 'read', true)
            END AS unread,
            CASE
                WHEN m.tableoid = 'isahl.zc_id_message'::regclass THEN 'system'
                ELSE replace(replace(m.tableoid::regclass::text, '"zc_id_msgs-', ''), '"', '')
            END AS msg_type
        FROM isahl.zc_id_message m
        LEFT JOIN isahl."zc_id_message_rr_contact-info" ci
            ON ci.ref_left = m.id AND ci.ref_right = $1 AND ci.deleted_at IS NULL
        WHERE m.deleted_at IS NULL
          AND (m.created_by_id = $1 OR $1 = ANY(m.ak_benefit_user) OR $1 IS NULL)
        ORDER BY m.created_at DESC
        LIMIT 20
    "#;

    match sqlx::query_as::<_, MessageItem>(sql)
        .bind(user_id)
        .fetch_all(pool)
        .await
    {
        Ok(items) => items,
        Err(e) => {
            common::telemetry::warn!("Failed to fetch recent messages: {}", e);
            Vec::new()
        }
    }
}

// ============================================
// Route Configuration
// ============================================

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/global").route("/overview", web::get().to(get_global_overview)));
}
