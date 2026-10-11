//! Approval HTTP Handler — Gateway thin adapter
//!
//! 业务逻辑在 framework-workspace-approval crate 中。
//! 本层仅负责：提取 HttpRequest 上下文 → 调 Framework 服务 → 映射 HTTP 响应。

use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use common::search::like_pattern;
use framework_workspace_approval::{
    ApprovalActionResponse, ApprovalActor, ApprovalHook, ApprovalService,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;

/// 审批动作的可选意见 body（fix-workspace-dock-contracts P1-3）。
/// 注册审批激活/禁用（fix-register-approval-activation-chain D2）：
/// 审批实例 `code` ∈ **注册类通道集合**（`common::registration_approval::
/// activation_event_codes()` 由通道映射表单源派生；实名审核 `user-verify` 不在集合内 =
/// MUST NOT 激活账号——实名不构成访问授权）时，经
/// `fk_subject` 定位申请人，置 `status` 为 `active`/`disabled`（仅 pending/pending_approval
/// 生效，幂等守卫）；`active` 时回填实名主体。
///
/// **副作用与写径无关**（change fix-registration-activation-on-framework-write-path）：
/// 内联路径（人工端點）与平台侧 `ApprovalCompleted` 兜底订阅者共用本实现；
/// 返回值 = 本次是否真正发生状态迁移（`true` 才发结果通知）——
/// 内联路径已迁移时订阅者拿到 `false` ⇒ **结果通知恰好一次**，无重复投递。
/// 非注册审批实例 → `Ok(false)`；查询失败 → `Err`（调用方 warn 不阻断审批结果）。
pub async fn apply_registration_activation(
    pool: &sqlx::PgPool,
    approval_id: i64,
    target_status: &str,
    messaging: Option<&Arc<dyn common::messaging::MessagingService>>,
) -> Result<bool, sqlx::Error> {
    // 注册类 code 判定**由通道映射表单源派生**（wire-registration-approval-closure
    // C4 第 2 步）：MUST NOT 在此复制 `code IN (…)` 字面列表——新增注册类通道只在映射表
    // 登记即自动进入判定面（字面列表与映射表脱钩 = 漏激活缺陷，reviewer p3）。
    let activation_codes = common::registration_approval::activation_event_codes();
    let applicant: Option<i64> = sqlx::query_scalar(
        r#"SELECT fk_subject FROM isahl."zc_id_oper-approve"
           WHERE id = $1 AND deleted_at IS NULL
             AND code = ANY($2::text[])"#,
    )
    .bind(approval_id)
    .bind(&activation_codes)
    .fetch_optional(pool)
    .await?;
    let Some(user_id) = applicant else {
        return Ok(false); // 非注册类审批实例或实例不存在——不触发
    };
    let applied = sqlx::query(
        "UPDATE isahl_auth.auth_users SET status = $2::isahl_auth.auth_user_status, updated_at = NOW() \
         WHERE id = $1 AND status IN ('pending', 'pending_approval')",
    )
    .bind(user_id)
    .bind(target_status)
    .execute(pool)
    .await?
    .rows_affected()
        > 0;
    if !applied {
        return Ok(false); // 幂等：状态已迁移（内联路径或重复事件）——不重复通知
    }
    common::telemetry::info!(
        "注册审批 {} 完成：用户 {} → {}",
        approval_id,
        user_id,
        target_status
    );
    // 审批通过 → 自动绑定主体（fix-register-approval-activation-chain 续：
    // 「自动创建主体并绑定」的绑定半段——主体行由 SSO 实名提交创建，
    // 此处按实名记录回填 auth_users 主体认知字段）。
    if target_status == "active" {
        bind_subject_from_identity(pool, user_id).await;
        // 员工 UA 授予（register-approval-closure :: employee-base-access）：
        // 与状态迁移同点、同实现 ⇒ 人工单条/批量、自动通过、framework 写径、SLA 超时
        // 全部同源且恰好一次（本分支只在本次真正发生迁移时到达）。幂等由
        // `ensure_employee_ua` 自持（ON CONFLICT … DO UPDATE）；失败仅 warn 不阻断审批结果
        // （与主体绑定同族容错：授予可补，实例已落终态）。
        if approval::ngac_ensure::ensure_employee_ua(pool, user_id)
            .await
            .is_none()
        {
            common::telemetry::warn!(
                "注册审批通过后 employee UA 授予未完成（用户 {}，实例 {}）——可由 seed 自愈或管理面补挂",
                user_id,
                approval_id
            );
        }
    }
    // 结果通知（register-approval-closure::approval-result-notification）：
    // 与状态迁移同点、同实现 ⇒ 任意写径（Gateway / framework / 定时）都被覆盖且恰好一次。
    if let Some(messaging) = messaging {
        let (title, content) = if target_status == "active" {
            (
                "访问授权审批已通过",
                format!(
                    "您的注册访问授权审批已通过，可正常登录使用系统（审批实例 {}）。",
                    approval_id
                ),
            )
        } else {
            (
                "访问授权审批已驳回",
                format!(
                    "您的注册访问授权审批已驳回，账号暂不可用；可使用同一账号重新注册申请复审（审批实例 {}）。",
                    approval_id
                ),
            )
        };
        if let Err(e) = messaging
            .send_system_notification(user_id as u64, title, &content)
            .await
        {
            common::telemetry::warn!(
                "注册审批结果通知投递失败（用户 {}，实例 {}）: {}（不阻断审批结果）",
                user_id,
                approval_id,
                e
            );
        }
    }
    Ok(true)
}

/// 审批通过后的主体绑定：按申请人实名记录回填 `auth_users.entity_table/entity_id`。
///
/// 主体来源 = `isahl_auth.identity_verifications` 登记的实名实体实例
/// （SSO 实名提交创建：personal → `zc_id_empl-natural`、enterprise →
/// `zc_id_orga-non-banking-legal`）。
/// 语义：
/// - m2o COALESCE——用户已绑实体（含组织实体，`/api/binding/*` 或引导页绑定）时保留原绑定，仅补空；
/// - 幂等——`entity_id IS NULL` 才写；
/// - 白名单——仅接受 `isahl.zc_id_subjects` 继承链内的实体（`tableoid` 匹配），防越权绑定；
/// - 无实名记录/未通过实名 → 跳过 + info（外部入驻主体经 `/api/binding/*` 绑定）。
async fn bind_subject_from_identity(pool: &sqlx::PgPool, user_id: i64) {
    let entity: Option<(i64, String)> = match sqlx::query_as(
        r#"
        SELECT iv.entity_instance_id, iv.entity_instance_table
          FROM isahl_auth.identity_verifications iv
         WHERE iv.user_id = $1
           AND iv.entity_instance_id IS NOT NULL
           AND iv.entity_instance_table IS NOT NULL
           AND iv.verification_status IN ('verified', 'approved')
           AND EXISTS (
                 SELECT 1 FROM isahl.zc_id_subjects s
                  WHERE s.id = iv.entity_instance_id
                    AND s.deleted_at IS NULL
                    AND s.tableoid = to_regclass('isahl.' || quote_ident(iv.entity_instance_table))
               )
         ORDER BY iv.updated_at DESC NULLS LAST, iv.id DESC
         LIMIT 1
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            common::telemetry::warn!("主体绑定查询失败（用户 {}）: {}", user_id, e);
            return;
        }
    };

    let Some((entity_id, entity_table)) = entity else {
        common::telemetry::info!(
            "用户 {} 无已通过实名的主体记录——审批通过后不自动绑定（外部入驻主体经 /api/binding/* 绑定）",
            user_id
        );
        return;
    };

    // m2o COALESCE：entity_id 为空才写（SET 表达式读旧值语义与 employee-onboarding 同构）
    if let Err(e) = sqlx::query(
        r#"UPDATE isahl_auth.auth_users
              SET entity_id = COALESCE(entity_id, $1),
                  entity_table = CASE WHEN entity_id IS NULL THEN $2 ELSE entity_table END,
                  updated_at = NOW()
            WHERE id = $3"#,
    )
    .bind(entity_id)
    .bind(&entity_table)
    .bind(user_id)
    .execute(pool)
    .await
    {
        common::telemetry::warn!("主体绑定写入失败（用户 {}）: {}", user_id, e);
    }
}

/// 审批通过全链（人工端点与自动审批通过 handler 共用，防两条通过链路漂移）。
///
/// 链路：状态桥（`ApprovalService::execute`）→ 注册用户激活（含主体绑定）→ 流程推进。
/// `actor_id = None` 表示匿名上下文（execute 跳过 operator 鉴权）；自动审批通过
/// handler 传 `SYSTEM_USER_ID` 与系统意见文本。
pub(crate) async fn approve_instance(
    pool: &sqlx::PgPool,
    approval_id: i64,
    actor_id: Option<i64>,
    opinion: Option<String>,
    bus: Option<&Arc<dyn common::event_bus::DomainEventBus>>,
    messaging: Option<&Arc<dyn common::messaging::MessagingService>>,
) -> ApprovalActionResponse {
    let opinion_text = opinion.clone();
    let actor = actor_id.map(|uid| ApprovalActor {
        user_id: uid,
        opinion,
    });

    let resp = ApprovalService::execute(
        pool,
        approval_id,
        "approved",
        actor,
        None::<&dyn ApprovalHook>,
    )
    .await;
    if !resp.success {
        return resp;
    }

    // fix-register-approval-activation-chain：注册类审批（code ∈ 通道映射表派生集合
    // `common::registration_approval::activation_event_codes()`）通过 → 经
    // oper-approve.fk_subject 定位申请人并激活（含主体绑定），失败 warn 不阻断审批结果
    // （对齐 advance_flow 降级模式）。
    if let Err(e) = apply_registration_activation(pool, approval_id, "active", messaging).await {
        common::telemetry::warn!(
            "approval {} 通过后注册用户激活失败（不阻断审批）: {}",
            approval_id,
            e
        );
    }
    // fix-approval-endpoint-gates：审批通过后推进流程节点（fk_process → next-ops；
    // FLOW-AUTHORIZATION 的 approve→end 链闭环）——失败仅 warn 不阻断审批结果
    // （advance 幂等可重放，断点由下次操作/自检恢复）。
    if let Some(uid) = actor_id {
        if let Err(e) = approval::advance::advance_flow(pool, approval_id, uid, bus).await {
            common::telemetry::warn!(
                "approval {} 通过后流程推进失败（不阻断审批）: {}",
                approval_id,
                e
            );
        }
    }
    // fix-approval-gateway-event-publish：终态事件发布（对齐 approval crate approve_inner
    // 全链）——payable/contract/driver-onboarding/employee-onboarding 域订阅方经
    // ApprovalCompleted 驱动业务实体状态机；缺此步则审批通过后业务实体永卡中间态。
    if let Some(bus) = bus {
        approval::handlers::approve_reject::publish_approval_completed(
            bus,
            pool,
            approval_id,
            "approved",
            opinion_text.as_deref(),
        )
        .await;
    }
    resp
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
    messaging: web::Data<Arc<dyn common::messaging::MessagingService>>,
    bus: web::Data<Arc<dyn common::event_bus::DomainEventBus>>,
    path: web::Path<i64>,
    body: Option<web::Json<OpinionBody>>,
) -> HttpResponse {
    let approval_id = path.into_inner();
    let user_id = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id);
    let opinion = body.and_then(|b| b.opinion.clone());
    let resp = approve_instance(
        pool.get_ref(),
        approval_id,
        user_id,
        opinion,
        Some(bus.get_ref()),
        Some(messaging.get_ref()),
    )
    .await;
    if resp.success {
        HttpResponse::Ok().json(resp)
    } else {
        HttpResponse::BadRequest().json(resp)
    }
}

/// 审批驳回全链（人工端点与批量端点共用，防两条驳回链路漂移）。
///
/// 链路：状态桥（`ApprovalService::execute`，"rejected"）→ 注册用户禁用 →
/// 终态事件发布（订阅方回滚业务状态）。与 [`approve_instance`] 对称。
pub(crate) async fn reject_instance(
    pool: &sqlx::PgPool,
    approval_id: i64,
    actor_id: Option<i64>,
    opinion: Option<String>,
    bus: Option<&Arc<dyn common::event_bus::DomainEventBus>>,
    messaging: Option<&Arc<dyn common::messaging::MessagingService>>,
) -> ApprovalActionResponse {
    let opinion_text = opinion.clone();
    let actor = actor_id.map(|uid| ApprovalActor {
        user_id: uid,
        opinion,
    });

    let resp = ApprovalService::execute(
        pool,
        approval_id,
        "rejected",
        actor,
        None::<&dyn ApprovalHook>,
    )
    .await;
    if !resp.success {
        return resp;
    }

    // fix-register-approval-activation-chain：注册审批驳回 → 经 fk_subject 定位
    // 申请人并禁用（失败 warn 不阻断审批结果）。
    if let Err(e) = apply_registration_activation(pool, approval_id, "disabled", messaging).await {
        common::telemetry::warn!(
            "approval {} 驳回后注册用户禁用失败（不阻断审批）: {}",
            approval_id,
            e
        );
    }
    // fix-approval-gateway-event-publish：驳回终态事件——订阅方回滚业务状态
    // （如承运账单 Pending→Init）；与 approve 路径对称，best-effort 不阻断。
    if let Some(bus) = bus {
        approval::handlers::approve_reject::publish_approval_completed(
            bus,
            pool,
            approval_id,
            "rejected",
            opinion_text.as_deref(),
        )
        .await;
    }
    resp
}

/// POST /api/approvals/{id}/reject
pub async fn reject_approval(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    messaging: web::Data<Arc<dyn common::messaging::MessagingService>>,
    bus: web::Data<Arc<dyn common::event_bus::DomainEventBus>>,
    path: web::Path<i64>,
    body: Option<web::Json<OpinionBody>>,
) -> HttpResponse {
    let approval_id = path.into_inner();
    let user_id = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id);
    let opinion = body.and_then(|b| b.opinion.clone());
    let resp = reject_instance(
        pool.get_ref(),
        approval_id,
        user_id,
        opinion,
        Some(bus.get_ref()),
        Some(messaging.get_ref()),
    )
    .await;
    if resp.success {
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
    /// 转办目标用户 id——zuid 可超 JS 2^53 ⇒ 前端传字符串；
    /// serde_zuid 接受 string/number 双形态（ID_JSON_PRECISION）。
    #[serde(default, with = "common::serde_zuid")]
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

    // 行级属主：原 operator 或 admin 可转（admin 判据唯一实现 = isahl_auth.is_admin）
    let (operator, is_admin): (Option<i64>, bool) = match sqlx::query_as::<_, (Option<i64>, bool)>(
        r#"
        SELECT oa.fk_operator,
               isahl_auth.is_admin($1)
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

    // 目标必须是有效 admin（审批工作区按 operator 可见）；判据同源 = isahl_auth.is_admin
    let target_ok: bool = sqlx::query_scalar("SELECT isahl_auth.is_admin($1)")
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
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        match ontology_binding::resolve_conn(&mut tx, ("JC", "FTA", "↓_NC")).await {
            Ok(v) => v,
            Err(e) => {
                common::telemetry::warn!("approvals/transfer: 坐标解析失败: {e}");
                let _ = tx.rollback().await;
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
           (notice, opinion, fk_list, fk_biller, created_at, dk_scene, dk_factor, dk_function)
           VALUES ('审批转交', $1, $2, $3, NOW(), $4, $5, $6)"#,
    )
    .bind(body.opinion.as_deref().unwrap_or(""))
    .bind(approval_id)
    .bind(uid)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
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

/// POST /api/approvals/apply — 驳回用户手动再次发起访问授权申请
///
/// refine-rejection-not-disabled：status='rejected' 用户登录后可重新申请。
/// 本 handler = **thin adapter**（wire-registration-approval-closure D2 / C4 第 2 步）：
/// 受理序列（审批事件绑 FLOW-AUTHORIZATION + qk_sla、`register-context` 归属桥链、
/// oper-approve 实例 fk_operator=首个 admin）**单一实现** =
/// [`common::registration_approval::intake`]（与 SSO `auth/register.rs` 同源）；
/// 本层只保留 Gateway 私有逻辑：状态门禁、事务内抢占式翻转、`user` UA 指派、
/// 响应形状与失败映射。状态翻转后 → 用户状态回 pending_approval。
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
        sqlx::query_scalar("SELECT status::text FROM isahl_auth.auth_users WHERE id = $1")
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
        "SELECT COALESCE(username, '用户') FROM isahl_auth.auth_users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(pool.get_ref())
    .await
    .unwrap_or_else(|_| "用户".to_string());

    // 事务：抢占式状态翻转（Gateway 私有）+ 受理序列（`intake` 单一实现，`common`）
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
    // user UA 幂等指派（approval-endpoint-gates::user-ua-on-registration）——唯一实现
    // = common::registration_approval::ensure_user_ua（rejected 历史用户可能无 user UA——
    // PEP 对 approvals:0 create 的放行依赖 user UA 关联）。best-effort：失败不阻断申请受理。
    let _ = common::registration_approval::ensure_user_ua(&mut tx, user_id).await;

    // 受理序列（审批事件 + register-context 上下文行 + 归属桥 + 模板桥 + oper-approve 实例）
    // **单一实现** = `common::registration_approval::intake`（wire-registration-approval-closure
    // D2 / C4 第 2 步）：与 SSO `auth/register.rs` 同源，同一事务内执行（`&mut *tx`）——
    // 事件落映射表声明的叶表，坐标、文案与桥链形状全部取自该实现。本 handler MUST NOT
    // 复制受理 SQL（迁移前两份同形实现即缺陷本源）；失败映射回既有响应形状。
    let instance_id = match common::registration_approval::intake(
        &mut tx,
        &common::registration_approval::INTERNAL_REGISTRATION_CHANNEL,
        user_id,
        &username,
    )
    .await
    {
        Ok(v) => v.instance_id,
        Err(e) => {
            common::telemetry::warn!("approvals/apply: 审批受理失败: {e}");
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false, "message": "申请失败"
            }));
        }
    };

    // 状态翻转已在事务首位抢占式完成（fix-approval-endpoint-gates）——此处无需重复

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

// ── 批量审批（管理端注册审批队列，change consolidate-admin-approval-queue D3）──────
//
// 语义：逐实例走**单实例链路**（[`approve_instance`] / [`reject_instance`]——含注册激活/
// 禁用 + 主体绑定 + employee UA 授予 + 流程推进 + `ApprovalCompleted` 发布），逐实例独立提交；
// 单实例失败不阻塞其余，结果逐实例返回。**MUST NOT** 改为复用 framework
// `/approval-instances/batch/*`：该链路不含 `isahl_auth.auth_users.status` 变更，注册审批
// 实例走它会退回「用户永卡 pending_approval」（回归 fix-register-approval-activation-chain）；
// framework 写径的同族副作用由平台档位 **ApprovalCompleted 兜底订阅者**
// （`subscribe_registration_activation_events`，装配于 Gateway/host）按同一实现补齐。

/// 单次批量上限（防超长事务链；队列页按页选择，远低于该值）。
const BATCH_MAX: usize = 200;

/// 批量审批 body：`ids` 为实例 id（zuid 可超 2^53 ⇒ 接受字符串/数字双形态）。
#[derive(Debug, Default, Deserialize)]
pub struct BatchBody {
    #[serde(default, with = "common::serde_zuid::seq")]
    pub ids: Vec<i64>,
    #[serde(default)]
    pub opinion: Option<String>,
}

/// 逐实例结果（`id` 字符串化，防前端 JSON 精度截断）。
#[derive(Debug, Serialize)]
pub struct BatchItemResult {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct BatchResponse {
    pub success: bool,
    pub total: usize,
    pub succeeded: usize,
    pub results: Vec<BatchItemResult>,
}

async fn run_batch(
    pool: web::Data<sqlx::PgPool>,
    bus: web::Data<Arc<dyn common::event_bus::DomainEventBus>>,
    messaging: web::Data<Arc<dyn common::messaging::MessagingService>>,
    req: HttpRequest,
    body: Option<web::Json<BatchBody>>,
    approve: bool,
) -> HttpResponse {
    let user_id = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id);
    let Some(body) = body else {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "message": "缺少 ids"
        }));
    };
    if body.ids.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "message": "ids 不能为空"
        }));
    }
    if body.ids.len() > BATCH_MAX {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": format!("单次批量上限 {} 条", BATCH_MAX)
        }));
    }

    let mut results = Vec::with_capacity(body.ids.len());
    let mut succeeded = 0usize;
    for &id in &body.ids {
        let resp = if approve {
            approve_instance(
                pool.get_ref(),
                id,
                user_id,
                body.opinion.clone(),
                Some(bus.get_ref()),
                Some(messaging.get_ref()),
            )
            .await
        } else {
            reject_instance(
                pool.get_ref(),
                id,
                user_id,
                body.opinion.clone(),
                Some(bus.get_ref()),
                Some(messaging.get_ref()),
            )
            .await
        };
        if resp.success {
            succeeded += 1;
        }
        results.push(BatchItemResult {
            id,
            success: resp.success,
            message: resp.message,
        });
    }

    common::telemetry::info!(
        "approvals/batch/{}: 共 {} 条，成功 {} 条",
        if approve { "approve" } else { "reject" },
        results.len(),
        succeeded
    );
    HttpResponse::Ok().json(BatchResponse {
        success: true,
        total: results.len(),
        succeeded,
        results,
    })
}

/// POST /api/approvals/batch/approve
pub async fn batch_approve(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    bus: web::Data<Arc<dyn common::event_bus::DomainEventBus>>,
    messaging: web::Data<Arc<dyn common::messaging::MessagingService>>,
    body: Option<web::Json<BatchBody>>,
) -> HttpResponse {
    run_batch(pool, bus, messaging, req, body, true).await
}

/// POST /api/approvals/batch/reject
pub async fn batch_reject(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    bus: web::Data<Arc<dyn common::event_bus::DomainEventBus>>,
    messaging: web::Data<Arc<dyn common::messaging::MessagingService>>,
    body: Option<web::Json<BatchBody>>,
) -> HttpResponse {
    run_batch(pool, bus, messaging, req, body, false).await
}

// ── 认证域终态副作用兜底订阅（change fix-registration-activation-on-framework-write-path）──
//
// 为什么需要：`isahl_auth.auth_users` 激活/禁用原先只在 Gateway 单条链路**内联**执行，
// framework 写径（ns 模块「我的审批」单条 / `/approval-instances/batch/*`）与 SLA 定时路径
// 不经过该链路 ⇒ 实例转终态而用户永卡 `pending_approval`（登录 403）。
// 规约 `user-registration-approval::approval-visible-to-admin` 要求「副作用与写径无关」，
// 故平台侧装配 **ns-agnostic** 订阅者兜底；与内联路径共用 `apply_registration_activation`
// （幂等：仅 `pending*` 生效，且结果通知以「本次是否发生迁移」为准 ⇒ 恰好一次）。

/// 订阅 `ApprovalCompleted` → 认证域实例终态副作用（兜底）。装配于组合根（Gateway/host）。
pub fn subscribe_registration_activation_events(
    bus: Arc<dyn common::event_bus::DomainEventBus>,
    pool: sqlx::PgPool,
    messaging: Arc<dyn common::messaging::MessagingService>,
) {
    actix_web::rt::spawn(async move {
        let mut subscriber = match bus.subscribe("ApprovalCompleted").await {
            Ok(s) => s,
            Err(e) => {
                common::telemetry::warn!("认证域副作用订阅装配失败（ApprovalCompleted）: {e}");
                return;
            }
        };
        loop {
            match subscriber.recv().await {
                Ok(evt) => handle_registration_activation_event(&pool, &messaging, &evt).await,
                Err(_) => continue,
            }
        }
    });
}

/// 事件处理：`entity_id` = 审批实例 id（发布端 `DomainEvent::new(..., instance_id, ...)`）；
/// `approved → active`（含实名主体回填）、`rejected → disabled`，其余结果静默跳过。
/// 实例 code 收口在 `apply_registration_activation` 内（非注册域零副作用、零通知）。
pub async fn handle_registration_activation_event(
    pool: &sqlx::PgPool,
    messaging: &Arc<dyn common::messaging::MessagingService>,
    event: &common::event_bus::DomainEvent,
) {
    if event.event_type != "ApprovalCompleted" {
        return;
    }
    let result = event
        .payload
        .get("result")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let target_status = match result {
        "approved" => "active",
        "rejected" => "disabled",
        _ => return, // withdrawn / cancelled / abstained 不在认证域终态副作用面内
    };
    if let Err(e) =
        apply_registration_activation(pool, event.entity_id, target_status, Some(messaging)).await
    {
        common::telemetry::warn!(
            "认证域副作用兜底失败（实例 {} → {}）: {}（内联路径与自检仍覆盖）",
            event.entity_id,
            target_status,
            e
        );
    }
}

// ── 转办候选（队列页 ApproverPicker 数据源）─────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct ApproverCandidatesQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default, with = "common::serde_zuid::opt")]
    pub limit: Option<i64>,
}

/// 转办候选行（`id` 字符串化）。
#[derive(Debug, Serialize, FromRow)]
pub struct ApproverCandidate {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
}

/// GET /api/approvals/approver-candidates?q=&limit=
///
/// 候选 = 持 admin UA 的账号（与 [`transfer_approval`] 的目标合法性判据同源：
/// 目标必须是有效 admin，否则 400）。管理员名单属管理面数据 ⇒ 仅 admin 可读
/// （[`common::permissions::is_admin_user`]，fail-closed）。
pub async fn approver_candidates(
    req: HttpRequest,
    pool: web::Data<sqlx::PgPool>,
    query: web::Query<ApproverCandidatesQuery>,
) -> HttpResponse {
    let Some(user_id) = req
        .extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id)
    else {
        return HttpResponse::Unauthorized().json(serde_json::json!({
            "success": false, "message": "未认证"
        }));
    };
    match common::permissions::is_admin_user(pool.get_ref(), user_id).await {
        Ok(true) => {}
        Ok(false) => {
            return HttpResponse::Forbidden().json(serde_json::json!({
                "success": false, "message": "仅管理员可查看转办候选"
            }))
        }
        Err(e) => {
            common::telemetry::warn!("approvals/approver-candidates: admin 判定失败: {e}");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false, "message": "查询失败"
            }));
        }
    }

    let limit = query.limit.unwrap_or(20).clamp(1, 50);
    let pattern = query
        .q
        .as_deref()
        .map(|q| like_pattern(q.trim()))
        .filter(|p| p != "%%");

    let rows = sqlx::query_as::<_, ApproverCandidate>(
        r#"
        SELECT DISTINCT u.id, COALESCE(u.display_name, u.username) AS name, u.username, u.display_name
        FROM isahl_auth.auth_users u
        WHERE isahl_auth.is_admin(u.id)
          AND ($1::text IS NULL
               OR u.username ILIKE $1 OR u.display_name ILIKE $1)
        ORDER BY u.id
        LIMIT $2
        "#,
    )
    .bind(pattern.as_deref())
    .bind(limit)
    .fetch_all(pool.get_ref())
    .await;

    match rows {
        Ok(list) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "count": list.len(),
            "data": list,
        })),
        Err(e) => {
            common::telemetry::warn!("approvals/approver-candidates: 查询失败: {e}");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false, "message": "查询失败"
            }))
        }
    }
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/approvals")
            // 字面路径（/apply、/batch/*、/approver-candidates）必须注册在 /{id} 之前
            // （NGAC_SPEC §7.2：scope 前缀匹配独占；否则 "batch" 会被当作 {id} 段）
            .route("/apply", web::post().to(apply_approval))
            .route("/batch/approve", web::post().to(batch_approve))
            .route("/batch/reject", web::post().to(batch_reject))
            .route("/approver-candidates", web::get().to(approver_candidates))
            .route("/{id}/approve", web::post().to(approve_approval))
            .route("/{id}/reject", web::post().to(reject_approval))
            .route("/{id}/transfer", web::post().to(transfer_approval)),
    );
}
