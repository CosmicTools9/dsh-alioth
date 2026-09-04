//! 个人仪表盘 API
//!
//! 根据 GATEWAY_DESIGN_SPEC.md §1.3 "个人化优先"：
//! 分析看板仅展示个人相关指标，每个用户的 Dashboard 内容由后端根据其个人数据生成。
//!
//! GET /api/dashboard/personal — 返回当前用户的个人数据概览

use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct PersonalDashboardResponse {
    pub success: bool,
    pub data: DashboardData,
}

#[derive(Debug, Default, Serialize)]
pub struct DashboardData {
    /// 待审批数量
    #[serde(with = "common::serde_zuid")]
    pub pending_approvals: i64,
    /// 未读消息数量
    #[serde(with = "common::serde_zuid")]
    pub unread_messages: i64,
    /// 今日日程数量
    #[serde(with = "common::serde_zuid")]
    pub today_events: i64,
    /// 我的模块数量
    pub my_modules: usize,
    /// 上月操作次数
    #[serde(with = "common::serde_zuid")]
    pub recent_activity_count: i64,
}

/// GET /api/dashboard/personal
///
/// 返回当前用户的个人仪表盘数据概览。
/// 所有指标均为当前登录用户的个人数据，无团队/管理视角。
pub async fn get_personal_dashboard(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
) -> HttpResponse {
    let user_id = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id);

    let user_id = match user_id {
        Some(uid) => uid,
        None => {
            return HttpResponse::Unauthorized().json(PersonalDashboardResponse {
                success: false,
                data: DashboardData::default(),
            })
        }
    };

    // 并行查询个人数据
    let (pending_approvals, unread_messages, today_events, recent_activity_count) = tokio::join!(
        // 待审批数
        async {
            match sqlx::query_scalar::<_, i64>(
                r#"
                SELECT COUNT(*) FROM isahl."zc_id_oper-approve" i
                LEFT JOIN isahl."zc_id_lifecycle_r_primary-status" ps
                    ON ps.ref_left = i.id AND ps.deleted_at IS NULL
                LEFT JOIN isahl."zc_id_stus-approve" st
                    ON st.id = ps.ref_right AND st.deleted_at IS NULL
                WHERE i.deleted_at IS NULL
                  AND (st.code IS NULL OR st.code != 'approved')
                  AND (i.fk_operator = $1 OR i.created_by_id = $1)
                "#,
            )
            .bind(user_id)
            .fetch_one(pool.get_ref())
            .await
            {
                Ok(n) => n,
                Err(e) => {
                    common::telemetry::warn!("dashboard pending_approvals query failed: {}", e);
                    0
                }
            }
        },
        // 未读消息数
        async {
            match sqlx::query_scalar::<_, i64>(
                r#"
                SELECT COUNT(*) FROM isahl.zc_id_message m
                LEFT JOIN isahl."zc_id_message_rr_contact-info" ci
                    ON ci.ref_left = m.id AND ci.ref_right = $1 AND ci.deleted_at IS NULL
                WHERE m.deleted_at IS NULL
                  AND (m.created_by_id = $1 OR $1 = ANY(m.ak_benefit_user))
                  AND (ci.feedback IS NULL OR ci.feedback::text <> 'read')
                "#,
            )
            .bind(user_id)
            .fetch_one(pool.get_ref())
            .await
            {
                Ok(n) => n,
                Err(e) => {
                    common::telemetry::warn!("dashboard unread_messages query failed: {}", e);
                    0
                }
            }
        },
        // 今日日程数（复用 framework-schedule 计数能力，避免重复 SQL——P2-8）
        async {
            let repo = framework_schedule::ScheduleRepository::new(pool.get_ref().clone());
            let now = chrono::Utc::now();
            match repo
                .get_plan_count_by_date_range(&now, &now, Some(user_id), None)
                .await
            {
                Ok(n) => n,
                Err(e) => {
                    common::telemetry::warn!("dashboard today_events query failed: {}", e);
                    0
                }
            }
        },
        // 近期操作次数（近30天）
        async {
            // 使用 zc_id_even-log 作为操作日志表
            match sqlx::query_scalar(
                r#"SELECT COUNT(*) FROM isahl."zc_id_even-log" WHERE created_by_id = $1 AND created_at >= CURRENT_DATE - INTERVAL '30 days'"#
            )
            .bind(user_id)
            .fetch_one(pool.get_ref())
            .await
            {
                Ok(n) => n,
                Err(e) => {
                    common::telemetry::warn!("dashboard recent_activity query failed: {}", e);
                    0
                }
            }
        },
    );

    HttpResponse::Ok().json(PersonalDashboardResponse {
        success: true,
        data: DashboardData {
            pending_approvals,
            unread_messages,
            today_events,
            my_modules: 0, // 由前端从 accessibleModules 计算
            recent_activity_count,
        },
    })
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/dashboard").route("/personal", web::get().to(get_personal_dashboard)));
}
