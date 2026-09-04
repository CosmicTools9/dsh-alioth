//! Approval HTTP Handler — Gateway thin adapter
//!
//! 业务逻辑在 framework-workspace-approval crate 中。
//! 本层仅负责：提取 HttpRequest 上下文 → 调 Framework 服务 → 映射 HTTP 响应。

use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use framework_workspace_approval::{ApprovalActor, ApprovalHook, ApprovalService};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;

/// 审批动作的可选意见 body（fix-workspace-dock-contracts P1-3）。
/// 注册审批激活/禁用（fix-register-approval-activation-chain D2）：
/// 审批实例 `code='user-register-approval'` 时，经 `fk_subject` 定位申请人，
/// 置 `status` 为 `active`/`disabled`（仅 pending/pending_approval 生效，幂等守卫）。
/// 非注册审批实例 → 无操作；查询失败 → Err（调用方 warn 不阻断审批结果）。
pub async fn apply_registration_activation(
    pool: &sqlx::PgPool,
    approval_id: i64,
    target_status: &str,
) -> Result<(), sqlx::Error> {
    let applicant: Option<(i64, Option<String>)> = sqlx::query_as(
        r#"SELECT fk_subject, code FROM isahl."zc_id_oper-approve"
           WHERE id = $1 AND deleted_at IS NULL
             AND code IN ('user-register-approval', 'external-subject-register-approval')"#,
    )
    .bind(approval_id)
    .fetch_optional(pool)
    .await?;
    let Some((user_id, Some(_code))) = applicant else {
        return Ok(()); // 非注册审批实例或实例不存在——不触发
    };
    let _ = sqlx::query(
        "UPDATE isahl_auth.auth_users SET status = $2, updated_at = NOW() \
         WHERE id = $1 AND status IN ('pending', 'pending_approval')",
    )
    .bind(user_id)
    .bind(target_status)
    .execute(pool)
    .await?;
    common::telemetry::info!(
        "注册审批 {} 完成：用户 {} → {}",
        approval_id,
        user_id,
        target_status
    );
    Ok(())
}

/// 前端「带意见通过/驳回」POST { opinion } → 透传至 ApprovalActor.opinion。
#[derive(Debug, Default, Deserialize)]
pub struct OpinionBody {
    #[serde(default)]
    pub opinion: Option<String>,
}

/// POST /api/approvals/{id}/approve
pub async fn approve_approval(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    _messaging: web::Data<Arc<dyn common::messaging::MessagingService>>,
    path: web::Path<i64>,
    body: Option<web::Json<OpinionBody>>,
) -> HttpResponse {
    let approval_id = path.into_inner();
    let user_id = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id);
    let opinion = body.and_then(|b| b.opinion.clone());
    let actor = user_id.map(|uid| ApprovalActor {
        user_id: uid,
        opinion,
    });

    let resp = ApprovalService::execute(
        pool.get_ref(),
        approval_id,
        "approved",
        actor,
        None::<&dyn ApprovalHook>,
    )
    .await;
    if resp.success {
        // fix-register-approval-activation-chain：注册审批（user-register-approval）
        // 通过 → 经 oper-approve.fk_subject 定位申请人并激活（comments 已文本化，
        // 旧 comments JSON 解析链停用后激活无消费者——此处内联恢复，全 ns 生效）。
        // 失败 warn 不阻断审批结果（对齐 advance_flow 降级模式）。
        if let Err(e) = apply_registration_activation(pool.get_ref(), approval_id, "active").await {
            common::telemetry::warn!(
                "approval {} 通过后注册用户激活失败（不阻断审批）: {}",
                approval_id,
                e
            );
        }
        // fix-approval-endpoint-gates：审批通过后推进流程节点（fk_process → next-ops；
        // FLOW-AUTHORIZATION 的 approve→end 链闭环）——失败仅 warn 不阻断审批结果
        // （advance 幂等可重放，断点由下次操作/自检恢复）。
        if let Some(uid) = user_id {
            if let Err(e) =
                approval::advance::advance_flow(pool.get_ref(), approval_id, uid, None).await
            {
                common::telemetry::warn!(
                    "approval {} 通过后流程推进失败（不阻断审批）: {}",
                    approval_id,
                    e
                );
            }
        }
        HttpResponse::Ok().json(resp)
    } else {
        HttpResponse::BadRequest().json(resp)
    }
}

/// POST /api/approvals/{id}/reject
pub async fn reject_approval(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    _messaging: web::Data<Arc<dyn common::messaging::MessagingService>>,
    path: web::Path<i64>,
    body: Option<web::Json<OpinionBody>>,
) -> HttpResponse {
    let approval_id = path.into_inner();
    let user_id = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id);
    let opinion = body.and_then(|b| b.opinion.clone());
    let actor = user_id.map(|uid| ApprovalActor {
        user_id: uid,
        opinion,
    });

    let resp = ApprovalService::execute(
        pool.get_ref(),
        approval_id,
        "rejected",
        actor,
        None::<&dyn ApprovalHook>,
    )
    .await;
    if resp.success {
        // fix-register-approval-activation-chain：注册审批驳回 → 经 fk_subject 定位
        // 申请人并禁用（失败 warn 不阻断审批结果）。
        if let Err(e) = apply_registration_activation(pool.get_ref(), approval_id, "disabled").await
        {
            common::telemetry::warn!(
                "approval {} 驳回后注册用户禁用失败（不阻断审批）: {}",
                approval_id,
                e
            );
        }
        HttpResponse::Ok().json(resp)
    } else {
        HttpResponse::BadRequest().json(resp)
    }
}

/// POST /api/approvals/{id}/transfer — 审批转办（M2，fix-approval-endpoint-gates）
///
/// 当前处理人（fk_operator）将待审批实例转给目标 admin；意见留痕（deta-opinion）。
/// approvals:0 恒定资源（admin 全权/user create）——行级属主由 handler 校验：
/// 仅原 operator 本人或 admin 可转办。
#[derive(Debug, Default, Deserialize)]
pub struct TransferBody {
    #[serde(default)]
    pub target_id: i64,
    #[serde(default)]
    pub opinion: Option<String>,
}

pub async fn transfer_approval(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    path: web::Path<i64>,
    body: Option<web::Json<TransferBody>>,
) -> HttpResponse {
    let approval_id = path.into_inner();
    let user_id = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id);
    let Some(uid) = user_id else {
        return HttpResponse::Unauthorized().json(serde_json::json!({
            "success": false, "message": "未认证"
        }));
    };
    let Some(body) = body else {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "message": "缺少转办目标"
        }));
    };
    if body.target_id <= 0 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "message": "转办目标用户无效"
        }));
    }

    // 行级属主：原 operator 或 admin 可转
    let (operator, is_admin): (Option<i64>, bool) = match sqlx::query_as::<_, (Option<i64>, bool)>(
        r#"
        SELECT oa.fk_operator,
               EXISTS (SELECT 1 FROM isahl_auth.ngac_user_rr_attribute ur
                       JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute
                       WHERE ur.fk_user = $1 AND ua.o_name = 'admin'
                         AND ur.deleted_at IS NULL AND ua.deleted_at IS NULL)
        FROM isahl."zc_id_oper-approve" oa
        WHERE oa.id = $2 AND oa.deleted_at IS NULL
        "#,
    )
    .bind(uid)
    .bind(approval_id)
    .fetch_optional(pool.get_ref())
    .await
    {
        Ok(Some(v)) => v,
        Ok(None) => {
            return HttpResponse::NotFound().json(serde_json::json!({
                "success": false, "message": "审批实例不存在"
            }));
        }
        Err(e) => {
            common::telemetry::warn!("approvals/transfer: 查询失败: {e}");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false, "message": "转办失败"
            }));
        }
    };
    if operator != Some(uid) && !is_admin {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "success": false, "message": "仅处理人或管理员可转办"
        }));
    }

    // 目标必须是有效 admin（审批工作区按 operator 可见）
    let target_ok: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (SELECT 1 FROM isahl_auth.ngac_user_rr_attribute ur
                       JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute
                       WHERE ur.fk_user = $1 AND ua.o_name = 'admin'
                         AND ur.deleted_at IS NULL AND ua.deleted_at IS NULL)
        "#,
    )
    .bind(body.target_id)
    .fetch_one(pool.get_ref())
    .await
    .unwrap_or(false);
    if !target_ok {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "message": "转办目标必须是管理员"
        }));
    }

    // 转办：fk_operator 更新 + 意见留痕（同事务）
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => {
            common::telemetry::warn!("approvals/transfer: 事务开启失败: {e}");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false, "message": "转办失败"
            }));
        }
    };
    let updated = sqlx::query(
        r#"UPDATE isahl."zc_id_oper-approve"
           SET fk_operator = $1, updated_at = NOW()
           WHERE id = $2 AND deleted_at IS NULL"#,
    )
    .bind(body.target_id)
    .bind(approval_id)
    .execute(&mut *tx)
    .await
    .map(|r| r.rows_affected())
    .unwrap_or(0);
    if updated == 0 {
        let _ = tx.rollback().await;
        return HttpResponse::NotFound().json(serde_json::json!({
            "success": false, "message": "审批实例不存在"
        }));
    }
    let _ = sqlx::query(
        r#"INSERT INTO isahl."zc_id_deta-opinion"
           (notice, opinion, fk_list, fk_biller, created_at)
           VALUES ('审批转交', $1, $2, $3, NOW())"#,
    )
    .bind(body.opinion.as_deref().unwrap_or(""))
    .bind(approval_id)
    .bind(uid)
    .execute(&mut *tx)
    .await;
    if let Err(e) = tx.commit().await {
        common::telemetry::warn!("approvals/transfer: 事务提交失败: {e}");
        return HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false, "message": "转办失败"
        }));
    }

    common::telemetry::info!(
        "approval {} 转办：{} → {}",
        approval_id,
        uid,
        body.target_id
    );
    HttpResponse::Ok().json(serde_json::json!({ "success": true }))
}

/// GET /api/approvals?status=pending
///
/// 审批列表（工作区按 operator 可见）：fk_operator = 当前用户 + deleted_at 过滤。
/// status 派生语义对齐 get_approval_detail（最新意见 notice → pending/approved/rejected）；
/// 注册审批（user-register-approval）applicant 取 even-approve.comments.applicant_name
/// （fk_subject 为 auth_users id，非 employee，JOIN 派生为空）。
#[derive(Debug, Serialize, FromRow)]
struct ListRow {
    id: i64,
    title: String,
    applicant: String,
    code: String,
    status: String,
    time: String,
}

pub async fn list_approvals(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    let user_id = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id);

    let status_filter = query.get("status").map(String::as_str).unwrap_or("pending");

    let rows = sqlx::query_as::<_, ListRow>(
        r#"
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
            TO_CHAR(i.created_at, 'MM-DD HH24:MI') AS time
        FROM isahl."zc_id_oper-approve" i
        LEFT JOIN LATERAL (
            SELECT o.notice FROM isahl."zc_id_deta-opinion" o
            WHERE o.fk_list = i.id AND o.deleted_at IS NULL
            ORDER BY o.created_at DESC LIMIT 1
        ) act ON true
        LEFT JOIN isahl."zc_id_subj-employee" e ON e.id = i.fk_subject
        WHERE i.deleted_at IS NULL
          AND (i.fk_operator = $1
               OR (i.fk_operator IS NULL
                   AND EXISTS (
                       SELECT 1 FROM isahl_auth.ngac_user_rr_attribute ur
                       JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute
                       WHERE ur.fk_user = $1 AND ua.o_name = 'admin'
                         AND ur.deleted_at IS NULL AND ua.deleted_at IS NULL
                   )))
        "#,
    )
    .bind(user_id)
    .fetch_all(pool.get_ref())
    .await;

    match rows {
        Ok(mut list) => {
            list.retain(|r| match status_filter {
                "pending" => r.status == "pending",
                "approved" => r.status == "approved",
                "rejected" => r.status == "rejected",
                _ => true,
            });
            list.sort_by_key(|r| std::cmp::Reverse(r.id));
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "count": list.len(),
                "data": list,
            }))
        }
        Err(e) => {
            common::telemetry::warn!("查询审批列表失败: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": "查询审批列表失败"
            }))
        }
    }
}

/// GET /api/approvals/{id}
///
/// 返回审批事项详情：完整 ApprovalItem（id/title/applicant/dept/code/status/time/
/// mine/operator_id，派生语义对齐 global_overview）与意见链（opinions）。
/// - operator_id = fk_operator
/// - mine = (created_by_id = 当前用户)
/// - opinions：zc_id_deta-opinion 中 fk_list = id 的意见，按时间正序，
///   action = notice 短码（审批通过→通过 / 审批驳回→驳回，对齐前端链节点判定），
///   opinion = 意见文本，author = auth_users.name/username（审批人）
pub async fn get_approval_detail(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    path: web::Path<i64>,
) -> HttpResponse {
    let approval_id = path.into_inner();
    let user_id = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id);

    // 审批实例完整字段（对齐 global_overview 的派生语义）：
    // - status：取最新意见 notice（审批通过/审批驳回 → approved/rejected）
    // - dept：tableoid 派生（zc_id_appr-* 子表 → 子表名）
    // - time：MM-DD HH24:MI
    // - mine：created_by_id = 当前用户
    #[derive(Debug, FromRow)]
    struct InstanceRow {
        id: i64,
        title: String,
        applicant: String,
        dept: String,
        code: String,
        status: String,
        time: String,
        mine: bool,
        operator_id: Option<i64>,
    }

    let instance = match sqlx::query_as::<_, InstanceRow>(
        r#"
        SELECT
            i.id,
            COALESCE(i.notice, '未命名审批') AS title,
            COALESCE(e.notice, '未知用户') AS applicant,
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
            (i.created_by_id = $2) AS mine,
            i.fk_operator AS operator_id
        FROM isahl."zc_id_oper-approve" i
        LEFT JOIN LATERAL (
            SELECT o.notice FROM isahl."zc_id_deta-opinion" o
            WHERE o.fk_list = i.id AND o.deleted_at IS NULL
            ORDER BY o.created_at DESC LIMIT 1
        ) act ON true
        LEFT JOIN isahl."zc_id_subj-employee" e ON e.id = i.fk_subject
        WHERE i.id = $1 AND i.deleted_at IS NULL
        "#,
    )
    .bind(approval_id)
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::NotFound().json(serde_json::json!({
                "success": false,
                "message": "审批事项不存在"
            }));
        }
        Err(e) => {
            common::telemetry::warn!("查询审批实例失败: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": "查询审批实例失败"
            }));
        }
    };

    // 意见链：action 派生短码（审批通过→通过 / 审批驳回→驳回，对齐前端严格相等判定），
    // author 取 auth_users.name/username（无 display_name 列）
    #[derive(Debug, Serialize, FromRow)]
    struct OpinionItem {
        action: String,
        opinion: String,
        author: String,
    }

    let opinions = match sqlx::query_as::<_, OpinionItem>(
        r#"SELECT
                CASE
                    WHEN op.notice = '审批通过' THEN '通过'
                    WHEN op.notice = '审批驳回' THEN '驳回'
                    ELSE COALESCE(op.notice, '')
                END AS action,
                COALESCE(op.opinion, '') AS opinion,
                COALESCE(u.name, u.username, '') AS author
           FROM isahl."zc_id_deta-opinion" op
           LEFT JOIN isahl_auth.auth_users u ON u.id = op.created_by_id
           WHERE op.fk_list = $1 AND op.deleted_at IS NULL
           ORDER BY op.created_at ASC"#,
    )
    .bind(approval_id)
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            common::telemetry::warn!("查询审批意见链失败: {}", e);
            Vec::new()
        }
    };

    #[derive(Debug, Serialize)]
    struct ApprovalDetailData {
        id: i64,
        title: String,
        applicant: String,
        dept: String,
        code: String,
        status: String,
        time: String,
        mine: bool,
        operator_id: Option<i64>,
        opinions: Vec<OpinionItem>,
    }

    #[derive(Debug, Serialize)]
    struct ApprovalDetailResponse {
        success: bool,
        data: ApprovalDetailData,
    }

    HttpResponse::Ok().json(ApprovalDetailResponse {
        success: true,
        data: ApprovalDetailData {
            id: instance.id,
            title: instance.title,
            applicant: instance.applicant,
            dept: instance.dept,
            code: instance.code,
            status: instance.status,
            time: instance.time,
            mine: instance.mine,
            operator_id: instance.operator_id,
            opinions,
        },
    })
}

/// POST /api/approvals/apply — 驳回用户手动再次发起访问授权申请
///
/// refine-rejection-not-disabled：status='rejected' 用户登录后可重新申请。
/// 流程：创建审批事件（绑 FLOW-AUTHORIZATION + qk_sla；`zc_id_appr-authorization` 叶表
/// 存在则写叶表、否则写 `zc_id_even-approve` 主表——fix-approval-event-adaptive-write）
///       + oper-approve 实例（fk_operator=首个 admin）→ 用户状态回 pending_approval。
/// 门禁：仅 rejected 可申请；active（已授权）/pending_approval（审批中）/disabled（封禁）拒绝。
pub async fn apply_approval(req: HttpRequest, pool: web::Data<sqlx::PgPool>) -> HttpResponse {
    let user_id = match req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id)
    {
        Some(uid) => uid,
        None => {
            return HttpResponse::Unauthorized().json(serde_json::json!({
                "success": false, "message": "未认证"
            }))
        }
    };
    // 状态门禁（友好文案）+ 事务内抢占式 UPDATE（原子防并发，fix-approval-endpoint-gates）：
    // 并发双请求仅一次命中 WHERE status='rejected'，另一次零行 → 400，不创建实例。
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM isahl_auth.auth_users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(pool.get_ref())
            .await
            .ok()
            .flatten();
    match status.as_deref() {
        Some("rejected") => {}
        Some("active") => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false, "message": "账号已获得授权，无需重复申请"
            }))
        }
        Some("pending_approval") | Some("pending") => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false, "message": "已有审批进行中，请等待审批结果"
            }))
        }
        _ => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false, "message": "账号状态不允许发起申请"
            }))
        }
    }

    let username: String = sqlx::query_scalar(
        "SELECT COALESCE(username, name, '用户') FROM isahl_auth.auth_users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(pool.get_ref())
    .await
    .unwrap_or_else(|_| "用户".to_string());

    // 事务：抢占式状态翻转 + 叶表事件 + oper-approve 实例
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => {
            common::telemetry::warn!("approvals/apply: 事务开启失败: {e}");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false, "message": "申请失败"
            }));
        }
    };

    // 抢占式状态翻转（原子门禁）：零行 = 已被并发请求抢占
    let preempted: Option<i64> = sqlx::query_scalar(
        "UPDATE isahl_auth.auth_users SET status = 'pending_approval', updated_at = NOW() \
         WHERE id = $1 AND status = 'rejected' RETURNING id",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .ok()
    .flatten();
    if preempted.is_none() {
        let _ = tx.rollback().await;
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "message": "已有审批进行中，请等待审批结果"
        }));
    }
    // user UA 幂等指派（fix-approval-endpoint-gates：rejected 历史用户可能无 user UA——
    // PEP 对 approvals:0 create 的放行依赖 user UA 关联）
    let user_ua_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl_auth.ngac_user_attribute
           WHERE o_name = 'user' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(&mut *tx)
    .await
    .ok()
    .flatten();
    if let Some(ua_id) = user_ua_id {
        let _ = sqlx::query(
            r#"INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, o_name)
               VALUES ($1, $2, 'user')
               ON CONFLICT (fk_user, fk_user_attribute)
               DO UPDATE SET deleted_at = NULL, updated_at = NOW()"#,
        )
        .bind(user_id)
        .bind(ua_id)
        .execute(&mut *tx)
        .await;
    }

    //

    // 模板指针（FLOW-AUTHORIZATION；缺失降级 NULL）
    let flow_binding: Option<(i64, Option<i64>)> = sqlx::query_as(
        r#"
        SELECT p.id,
               (SELECT rro.ref_right FROM isahl.zc_id_process_rr_operation rro
                JOIN isahl."zc_id_oper-approve" oa
                  ON oa.id = rro.ref_right AND oa.deleted_at IS NULL
                WHERE rro.ref_left = p.id AND rro.deleted_at IS NULL LIMIT 1)
        FROM isahl.zc_id_process p
        WHERE p.code = 'FLOW-AUTHORIZATION' AND p.deleted_at IS NULL
        LIMIT 1
        "#,
    )
    .fetch_optional(&mut *tx)
    .await
    .ok()
    .flatten();
    let sla_duration_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_scal-duration"
           WHERE o_number = '72h' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(&mut *tx)
    .await
    .ok()
    .flatten();

    let approval_notice = format!("用户 {} 访问授权审批", username);
    let approval_comments =
        serde_json::json!({"applicant_id": user_id.to_string(), "applicant_name": username})
            .to_string();

    // 1. 审批事件（绑 FLOW-AUTHORIZATION）。
    //    写入目标表自适应（fix-approval-event-adaptive-write）：`zc_id_appr-authorization`
    //    叶表并非所有 namespace 存在（仅 seed-release-tables.sql 在 test/sim 建，
    //    WZ dev 有、Alioth/AVIC-CAASEC/Cosmic-Tools dev 无）。存在则写叶表（继承
    //    even-approve，查询可见），否则写 even-approve 主表——保证无叶表的
    let leaf_table_exists: bool = match sqlx::query_scalar(
        "SELECT to_regclass('isahl.\"zc_id_appr-authorization\"') IS NOT NULL",
    )
    .fetch_one(&mut *tx)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            common::telemetry::warn!(
                "approvals/apply: 检测 authorization 叶表失败（降级写 even-approve 主表）: {e}"
            );
            false
        }
    };
    let event_id: i64 = if leaf_table_exists {
        // 叶表存在（WZ 等）：写 zc_id_appr-authorization（继承 even-approve）
        match sqlx::query_scalar(
            r#"
            INSERT INTO isahl."zc_id_appr-authorization" (
                created_by_id, updated_by_id, notice, code, comments,
                tpl_id, qk_sla, created_at, updated_at
            ) VALUES ($1, $1, $2, 'user-register-approval', $3, $4, $5, NOW(), NOW())
            RETURNING id
            "#,
        )
        .bind(user_id)
        .bind(&approval_notice)
        .bind(&approval_comments)
        .bind(flow_binding.as_ref().and_then(|(_, tpl)| *tpl))
        .bind(sla_duration_id)
        .fetch_one(&mut *tx)
        .await
        {
            Ok(id) => id,
            Err(e) => {
                common::telemetry::warn!("approvals/apply: 审批事件创建失败: {e}");
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false, "message": "申请失败"
                }));
            }
        }
    } else {
        // 叶表缺失（Alioth/AVIC-CAASEC/Cosmic-Tools）：写 even-approve 主表
        match sqlx::query_scalar(
            r#"
            INSERT INTO isahl."zc_id_even-approve" (
                created_by_id, updated_by_id, notice, code, comments,
                tpl_id, qk_sla, created_at, updated_at
            ) VALUES ($1, $1, $2, 'user-register-approval', $3, $4, $5, NOW(), NOW())
            RETURNING id
            "#,
        )
        .bind(user_id)
        .bind(&approval_notice)
        .bind(&approval_comments)
        .bind(flow_binding.as_ref().and_then(|(_, tpl)| *tpl))
        .bind(sla_duration_id)
        .fetch_one(&mut *tx)
        .await
        {
            Ok(id) => id,
            Err(e) => {
                common::telemetry::warn!("approvals/apply: 审批事件创建失败: {e}");
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false, "message": "申请失败"
                }));
            }
        }
    };
    // fk_process 列已物理移除（2026-08-30 remove-event-fk-process-pollution D3 写侧）：
    // 事件↔FLOW-AUTHORIZATION 归属经桥链——'register-context' 上下文 oper 行
    // （每流程复用一行）+ process_rr_operation 归属桥 + rr_event 模板桥
    if let Some((flow_id, _)) = flow_binding {
        let ctx_oper: Option<i64> = sqlx::query_scalar(
            r#"SELECT rro.ref_right FROM isahl.zc_id_process_rr_operation rro
               JOIN isahl."zc_id_oper-approve" oa ON oa.id = rro.ref_right
                 AND oa.deleted_at IS NULL AND oa.notice = 'register-context'
               WHERE rro.ref_left = $1 AND rro.deleted_at IS NULL LIMIT 1"#,
        )
        .bind(flow_id)
        .fetch_optional(&mut *tx)
        .await
        .ok()
        .flatten();
        let ctx_oper: i64 = match ctx_oper {
            Some(v) => v,
            None => {
                let new_id: i64 = match sqlx::query_scalar(
                    r#"INSERT INTO isahl."zc_id_oper-approve" (notice, created_by_id)
                       VALUES ('register-context', $1) RETURNING id"#,
                )
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await
                {
                    Ok(v) => v,
                    Err(e) => {
                        common::telemetry::warn!(
                            "approvals/apply: register-context 行创建失败: {e}"
                        );
                        let _ = tx.rollback().await;
                        return HttpResponse::InternalServerError().json(serde_json::json!({
                            "success": false, "message": "申请失败"
                        }));
                    }
                };
                if let Err(e) = sqlx::query(
                    "INSERT INTO isahl.zc_id_process_rr_operation (ref_left, ref_right, created_by_id)
                     VALUES ($1, $2, $3)",
                )
                .bind(flow_id)
                .bind(new_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                {
                    common::telemetry::warn!("approvals/apply: 流程归属桥创建失败: {e}");
                    let _ = tx.rollback().await;
                    return HttpResponse::InternalServerError().json(serde_json::json!({
                        "success": false, "message": "申请失败"
                    }));
                }
                new_id
            }
        };
        if let Err(e) = sqlx::query(
            "INSERT INTO isahl.zc_id_operation_rr_event (ref_left, ref_right, created_by_id)
             VALUES ($1, $2, $3)",
        )
        .bind(ctx_oper)
        .bind(event_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        {
            common::telemetry::warn!("approvals/apply: 事件模板桥创建失败: {e}");
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false, "message": "申请失败"
            }));
        }
    }

    // 2. oper-approve 实例（fk_operator=首个 admin）
    let admin_id: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT ur.fk_user FROM isahl_auth.ngac_user_rr_attribute ur
        JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute
        WHERE ua.o_name = 'admin' AND ur.deleted_at IS NULL AND ua.deleted_at IS NULL
          AND (ur.expires_at IS NULL OR ur.expires_at > NOW())
        ORDER BY ur.id LIMIT 1
        "#,
    )
    .fetch_optional(&mut *tx)
    .await
    .ok()
    .flatten();
    let instance_id: i64 = match sqlx::query_scalar(
        r#"
        INSERT INTO isahl."zc_id_oper-approve" (
            notice, code, fk_subject, fk_operator, created_by_id, created_at, updated_at
        ) VALUES ($1, 'user-register-approval', $2, $3, $2, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(&approval_notice)
    .bind(user_id)
    .bind(admin_id)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            common::telemetry::warn!("approvals/apply: 审批实例创建失败: {e}");
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false, "message": "申请失败"
            }));
        }
    };

    // fix-fk-approve-residual-consumers：fk_approve 物理列已移除——
    // 实例↔事件关联经 operation_rr_event 桥行承载（同事务）
    if let Err(e) = sqlx::query(
        r#"
        INSERT INTO isahl.zc_id_operation_rr_event (ref_left, ref_right, created_by_id)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(instance_id)
    .bind(event_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    {
        common::telemetry::warn!("approvals/apply: 审批桥行写入失败: {e}");
        let _ = tx.rollback().await;
        return HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false, "message": "申请失败"
        }));
    }

    // 3. 状态翻转已在事务首位抢占式完成（fix-approval-endpoint-gates）——此处无需重复

    if let Err(e) = tx.commit().await {
        common::telemetry::warn!("approvals/apply: 事务提交失败: {e}");
        return HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false, "message": "申请失败"
        }));
    }

    common::telemetry::info!(
        "approvals/apply: 用户 {} 重新发起访问授权申请（实例 {}）",
        user_id,
        instance_id
    );
    HttpResponse::Created().json(serde_json::json!({
        "success": true,
        "instance_id": instance_id.to_string(),
        "status": "pending_approval",
    }))
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/approvals")
            .route("", web::get().to(list_approvals))
            // /apply 必须注册在 /{id} 之前（NGAC_SPEC §7.2：scope 前缀匹配独占）
            .route("/apply", web::post().to(apply_approval))
            .route("/{id}", web::get().to(get_approval_detail))
            .route("/{id}/approve", web::post().to(approve_approval))
            .route("/{id}/reject", web::post().to(reject_approval))
            .route("/{id}/transfer", web::post().to(transfer_approval)),
    );
}
