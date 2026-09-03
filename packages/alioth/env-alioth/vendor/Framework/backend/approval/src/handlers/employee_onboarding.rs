//! employee-onboarding 审批闭环订阅（G4 收口：审批事件消费者从发布端解耦）
//!
//! 范式：contract/airworthiness events.rs（PAGE-适航-07 先例）——
//! `ApprovalCompleted` 由发布端 `publish_approval_completed` 广播，本模块作为
//! 独立订阅者消费，替代原 approve()/reject() 内联的 employee-onboarding 同步副作用：
//! - result = `approved`  → 创建 zc_id_empl-natural 员工 + auth_users 激活
//!   + employee UA 指派（ngac_ensure::ensure_employee_ua）+ zc_id_prot-profile_config 配置
//! - result = `rejected`  → auth_users 置 `disabled`
//!
//! ## 流程识别
//! 仅 flow code = `employee-onboarding`（桥链：even-approve ← operation_rr_event ← process_rr_operation → zc_id_process.code）
//! 的审批节点事件触发；其余流程静默跳过（不报错不写状态）。
//!
//! ## entity_id 语义
//! 发布端 entity_id = 审批节点事件 id（实例经 rr_event 桥反查）；兼容历史语义
//! （entity_id = 审批实例 id，经 rr_event 桥回链）。
//!
//! ## 幂等性
//! 员工创建以 `code = emp-{applicant_id}`（确定性自然键）做存在性守卫：
//! 已存在 → 跳过创建（激活/UA 指派本身幂等，照常执行），事件重放不产生重复员工。
//!
//! ## 异步时序说明
//! 原内联副作用在 approve() 请求内执行（各语句自动提交，无包裹事务）；迁移后由
//! 订阅者后台 task 执行，事件在意见/状态写入并提交后才发布，订阅者收到时数据已可见。
//! 单实例部署（InMemoryEventBus，广播语义）下多个订阅者各自持有 receiver，互不冲突。

use common::event_bus::{DomainEvent, DomainEventBus};
use common::SYSTEM_USER_ID;
use sqlx::PgPool;
use std::sync::Arc;

/// 事件类型常量
pub mod event_types {
    /// 审批完成事件频道（与发布端 publish_approval_completed 一致）
    pub const APPROVAL_COMPLETED: &str = "ApprovalCompleted";
    /// 雇员入职审批流程 code（识别条件，与内联钩子一致）
    pub const FLOW_EMPLOYEE_ONBOARDING: &str = "employee-onboarding";
}

/// 审批完成事件数据（与发布端 payload 对齐）
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ApprovalCompletedPayload {
    pub entity_type: String,
    pub entity_id: i64,
    pub result: String,
    pub comment: Option<String>,
}

/// 注册 employee-onboarding 审批闭环订阅（Gateway 启动时调用，spawn 后台 task）。
/// 广播语义：与 contract/airworthiness 等订阅者各自持有 receiver，互不冲突。
pub fn subscribe_employee_onboarding_events(bus: Arc<dyn DomainEventBus>, pool: PgPool) {
    actix_web::rt::spawn(async move {
        let mut subscriber = match bus.subscribe(event_types::APPROVAL_COMPLETED).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to subscribe to ApprovalCompleted: {}", e);
                return;
            }
        };
        loop {
            match subscriber.recv().await {
                Ok(evt) => handle_event(&pool, evt).await,
                Err(_) => continue,
            }
        }
    });
}

/// 事件处理：employee-onboarding 流程的审批完成 → 雇员入职副作用
pub async fn handle_event(pool: &PgPool, event: DomainEvent) {
    if event.event_type != event_types::APPROVAL_COMPLETED {
        return;
    }
    let Ok(payload) = serde_json::from_value::<ApprovalCompletedPayload>(event.payload.clone())
    else {
        eprintln!("ApprovalCompleted payload 解析失败: {:?}", event.payload);
        return;
    };

    // entity_id → 审批节点事件 id（兼容历史 entity_id=审批实例 id 语义）
    let Some(even_id) = resolve_even_id(pool, payload.entity_id).await else {
        eprintln!(
            "employee-onboarding 跳过: entity_id={} 无法解析到审批节点事件",
            payload.entity_id
        );
        return;
    };

    // 流程识别：仅 employee-onboarding 触发（内联语义：桥链 even←rr_event←process_rr_operation→process.code）
    let Some(flow_code) = resolve_flow_code(pool, even_id).await else {
        return;
    };
    if flow_code != event_types::FLOW_EMPLOYEE_ONBOARDING {
        return;
    }

    // 申请人上下文：comments 为纯文本，applicant 不再可得 → 跳过 onboarding 自动化
    let Some((applicant_id, applicant_name)) = resolve_applicant(pool, even_id).await else {
        common::telemetry::warn!(
            "employee-onboarding 跳过: 审批节点事件 {} 无申请人上下文（comments 已文本化）",
            even_id
        );
        return;
    };

    match payload.result.as_str() {
        "approved" => {
            // created_by 语义对齐内联：审批操作人（意见 fk_biller）；解析失败用系统身份兜底
            let approver = resolve_approver(pool, even_id)
                .await
                .unwrap_or(SYSTEM_USER_ID);
            apply_onboarding_approved(pool, applicant_id, &applicant_name, approver).await;
        }
        "rejected" => apply_onboarding_rejected(pool, applicant_id).await,
        other => {
            common::telemetry::warn!(
                "employee-onboarding: 未知审批结果 {}（事件 {}）",
                other,
                even_id
            );
        }
    }
}

/// entity_id → 审批节点事件 id（zc_id_even-approve）
///
/// 兼容两种语义（与 airworthiness resolve_certificate_context 同型）：
/// - entity_id 即审批节点事件 id（发布端经 rr_event 桥反查）
/// - entity_id 为审批实例 id（历史语义），经 rr_event 桥回链
async fn resolve_even_id(pool: &PgPool, entity_id: i64) -> Option<i64> {
    let direct: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_even-approve"
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(entity_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    if direct.is_some() {
        return direct;
    }
    sqlx::query_scalar(
        r#"SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
           WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(entity_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// 审批节点事件 → 流程 code（桥链：even 语义行 ← rr_event ← process_rr_operation → zc_id_process.code）
async fn resolve_flow_code(pool: &PgPool, even_id: i64) -> Option<String> {
    sqlx::query_scalar(
        r#"SELECT p.code FROM isahl.zc_id_operation_rr_event oe
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.deleted_at IS NULL
           JOIN isahl.zc_id_process p ON p.id = rro.ref_left AND p.deleted_at IS NULL
           WHERE oe.ref_right = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(even_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// 审批节点事件 → 申请人（已停用）。
/// comments 为纯文本语义（remove-comments-json-embedding），申请人上下文不再可得 → 恒 None，
/// 调用方跳过 onboarding 自动化（诚实降级）。
async fn resolve_applicant(_pool: &PgPool, _even_id: i64) -> Option<(i64, String)> {
    None
}

/// 审批操作人解析：deta-opinion.fk_biller（fk_list = 实例 id，经 rr_event 桥回链）。
/// 与内联 created_by_id（操作人）语义一致；无意见行 → None（调用方用系统身份兜底）。
async fn resolve_approver(pool: &PgPool, even_id: i64) -> Option<i64> {
    sqlx::query_scalar(
        r#"SELECT o.fk_biller
           FROM isahl."zc_id_deta-opinion" o
           JOIN isahl."zc_id_oper-approve" oa ON oa.id = o.fk_list
           JOIN isahl.zc_id_operation_rr_event oe
             ON oe.ref_left = oa.id AND oe.ref_right = $1 AND oe.deleted_at IS NULL
           WHERE o.notice IN ('审批通过', '审批驳回')
             AND o.deleted_at IS NULL AND oa.deleted_at IS NULL
           ORDER BY o.created_at DESC LIMIT 1"#,
    )
    .bind(even_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// 审批通过：员工创建 + 用户激活 + UA 指派 + profile 配置
async fn apply_onboarding_approved(
    pool: &PgPool,
    applicant_id: i64,
    applicant_name: &str,
    approver_id: i64,
) {
    // 幂等守卫：code = emp-{applicant_id} 为确定性自然键，事件重放不重复创建
    let existing: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_empl-natural"
           WHERE code = $1 AND deleted_at IS NULL"#,
    )
    .bind(format!("emp-{}", applicant_id))
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let emp_id = match existing {
        Some(id) => {
            common::telemetry::info!(
                "employee-onboarding: 员工已存在（code=emp-{}，id={}），跳过创建",
                applicant_id,
                id
            );
            id
        }
        None => match sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO isahl."zc_id_empl-natural" (id, notice, code, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3) RETURNING id"#,
        )
        .bind(applicant_name)
        .bind(format!("emp-{}", applicant_id))
        .bind(approver_id)
        .fetch_one(pool)
        .await
        {
            Ok(id) => id,
            Err(e) => {
                common::telemetry::error!(
                    "employee-onboarding: 员工创建失败（applicant {}）: {}",
                    applicant_id,
                    e
                );
                return;
            }
        },
    };

    // 用户激活 + 实体绑定（m2o 语义：COALESCE——user 已绑实体（含组织实体）时
    // 保留原绑定仅激活；未绑则绑定员工 id。status 无条件激活。
    // fix-ngac-entity-binding-m2o）
    if let Err(e) = sqlx::query(
        r#"UPDATE isahl_auth.auth_users SET
             entity_id = COALESCE(entity_id, $1),
             entity_table = CASE WHEN entity_id IS NULL THEN 'zc_id_empl-natural' ELSE entity_table END,
             status='active', updated_at=NOW()
           WHERE id=$2"#,
    )
    .bind(emp_id)
    .bind(applicant_id)
    .execute(pool)
    .await
    {
        common::telemetry::warn!(
            "employee-onboarding: 激活用户 {} 失败（不影响员工创建）: {}",
            applicant_id,
            e
        );
    }

    // add-gateway-seed-self-heal：employee UA 指派收敛到公共 helper（查找/创建 + 幂等指派）
    crate::ngac_ensure::ensure_employee_ua(pool, applicant_id).await;

    // profile 配置（员工新建分支才补；重放已存在员工时不重复插）
    if existing.is_none() {
        if let Err(e) = sqlx::query(
            r#"INSERT INTO isahl."zc_id_prot-profile_config" (id, notice, fk_employee, settings, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, '{}'::jsonb, $3)"#,
        )
        .bind(applicant_name)
        .bind(emp_id)
        .bind(applicant_id)
        .execute(pool)
        .await
        {
            common::telemetry::warn!(
                "employee-onboarding: profile 配置创建失败（applicant {}）: {}",
                applicant_id,
                e
            );
        }
    }

    common::telemetry::info!(
        "employee-onboarding: 审批通过闭环完成（applicant {} → employee {}）",
        applicant_id,
        emp_id
    );
}

/// 审批驳回：禁用申请人账号（内联语义：无条件置 disabled）
async fn apply_onboarding_rejected(pool: &PgPool, applicant_id: i64) {
    match sqlx::query(
        r#"UPDATE isahl_auth.auth_users SET status = 'disabled', updated_at = NOW() WHERE id = $1"#,
    )
    .bind(applicant_id)
    .execute(pool)
    .await
    {
        Ok(_) => {
            common::telemetry::info!(
                "employee-onboarding: 审批驳回 → 用户 {} 已禁用",
                applicant_id
            );
        }
        Err(e) => {
            common::telemetry::warn!("employee-onboarding: 禁用用户 {} 失败: {}", applicant_id, e);
        }
    }
}
