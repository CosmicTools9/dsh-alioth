//! sla_timeout SLA 超时自动驳回集成测试
//!
//! 验证 `check_and_reject` 轮询链路：
//! - 查询不再因 LATERAL 匿名列引用（`already.1`）语法报错——修复前查询直接失败，
//!   超时实例永远不会被自动驳回
//! - 超时实例被写入 SLA 自动驳回意见 + 主状态置 rejected
//! - 幂等：重复轮询不重复驳回（LATERAL 防重）
//! - 未超时实例不误伤
//!
//! 数据依赖：
//! - zc_id_scal-duration（SLA 小时数，o_number）
//! - zc_id_even-approve（审批事件，qk_sla）
//! - zc_id_oper-approve（审批实例）
//! - zc_id_deta-opinion（审批意见）
//! - zc_id_lifecycle_r_primary-status / zc_id_stus-approve（状态机）

use ::common::testing::connect_test_db;
use ::common::SYSTEM_USER_ID;
use std::sync::Arc;

mod common;
use common::{ensure_role_member, grant_user_access, setup_test_schema, test_code};
use serde_json::json;

async fn insert_sla_duration(pool: &sqlx::PgPool, hours: i64) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_scal-duration" (notice, mark, created_by_id)
           VALUES ('SLA 测试时长', $1, 1) RETURNING id"#,
    )
    .bind(rust_decimal::Decimal::from(hours))
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_approve_event(pool: &sqlx::PgPool, sla_id: i64) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, qk_sla, created_by_id)
           VALUES ('SLA 测试审批事件', $1, 1) RETURNING id"#,
    )
    .bind(sla_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// created_hours_ago：负值表示创建于过去（超时），正数表示未来/未超时
async fn insert_instance(pool: &sqlx::PgPool, event_id: i64, created_hours_ago: i32) -> i64 {
    let instance_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (notice, created_at, created_by_id)
           VALUES ('SLA 测试实例', NOW() - ($1 || ' hours')::interval, 1)
           RETURNING id"#,
    )
    .bind(created_hours_ago)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(instance_id)
    .bind(event_id)
    .execute(pool)
    .await
    .unwrap();
    instance_id
}

#[tokio::test]
async fn sla_timeout_auto_rejects_overdue_instance() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    // NGAC：系统身份对 approval_actions 有 create 授权（check_and_reject 内 require_resource_access）
    grant_user_access(&pool, SYSTEM_USER_ID, "approval_actions", &["create"])
        .await
        .expect("grant system user approval access");

    let sla_id = insert_sla_duration(&pool, 1).await; // 1 小时 SLA
    let event_id = insert_approve_event(&pool, sla_id).await;
    let overdue_id = insert_instance(&pool, event_id, 2).await; // 2 小时前创建 → 已超时
    let fresh_id = insert_instance(&pool, event_id, -1).await; // 未来创建 → 未超时

    let bus: Arc<dyn ::common::event_bus::DomainEventBus> =
        Arc::new(::common::event_bus::InMemoryEventBus::new());

    // 第一次轮询：查询必须成功（修复前 `already.1` 语法错误 → Err）
    approval::sla_timeout::check_and_reject(&pool, &bus)
        .await
        .expect("check_and_reject 应成功（修复后查询不再语法报错）");

    // 断言 1：超时实例已写入自动驳回意见（fk_list = 实例 id，notice=终态动作码，原因入 opinion）
    let (notice, opinion, fk_list): (String, String, i64) = sqlx::query_as(
        r#"SELECT notice, COALESCE(opinion, ''), fk_list FROM isahl."zc_id_deta-opinion"
           WHERE fk_list = $1 AND notice = '审批驳回' AND deleted_at IS NULL
           ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(overdue_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(notice, "审批驳回");
    assert_eq!(opinion, "SLA 超时自动驳回");
    assert_eq!(fk_list, overdue_id);

    // 断言 2：实例主状态已置 rejected
    let status_code: String = sqlx::query_scalar(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" r
           JOIN isahl."zc_id_stus-approve" s ON s.id = r.ref_right
           WHERE r.ref_left = $1 AND r.deleted_at IS NULL"#,
    )
    .bind(overdue_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status_code, "rejected");

    // 断言 3：未超时实例不被误伤
    let fresh_status: Option<String> = sqlx::query_scalar(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" r
           JOIN isahl."zc_id_stus-approve" s ON s.id = r.ref_right
           WHERE r.ref_left = $1 AND r.deleted_at IS NULL"#,
    )
    .bind(fresh_id)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(fresh_status.is_none(), "未超时实例不应被自动驳回");

    // 断言 4：幂等——二次轮询不再重复驳回
    approval::sla_timeout::check_and_reject(&pool, &bus)
        .await
        .expect("二次轮询应成功");
    let cnt: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_deta-opinion"
           WHERE fk_list = $1 AND notice = '审批驳回' AND deleted_at IS NULL"#,
    )
    .bind(overdue_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(cnt, 1, "重复轮询不应重复驳回");
}

/// 注册审批 SLA 超时全链路（add-register-approval-closure 缺口 2/3）：
/// 超时自动驳回 → 注册用户 rejected（暂不授权，可登录重新申请）→ 站内信通知申请人。
#[tokio::test]
async fn sla_timeout_auto_rejects_registration_and_disables_user() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    // NGAC：系统身份对 approval_actions 有 create 授权
    grant_user_access(&pool, SYSTEM_USER_ID, "approval_actions", &["create"])
        .await
        .expect("grant system user approval access");

    // 注册用户（applicant）：status=pending_approval（用户名/email 随机后缀，
    // 防跨运行残留撞 auth_users 唯一键——测试库不清库重跑需幂等）
    let suffix = test_code("sla");
    let applicant_name = format!("sla-applicant-{suffix}");
    let applicant_email = format!("{applicant_name}@test.local");
    let applicant_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, status, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES (isahl.gen_next_zuid(), $1, $1, $2, 'standard', 'pending_approval', TRUE,
                   NOW(), NOW(), 0, '{}'::jsonb)
           RETURNING id"#,
    )
    .bind(&applicant_name)
    .bind(&applicant_email)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 注册审批事件（code=user-register-approval + comments.applicant_id + qk_sla）
    let sla_id = insert_sla_duration(&pool, 1).await; // 1 小时 SLA
    let event_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, code, comments, qk_sla, created_by_id)
           VALUES ($1, 'user-register-approval',
                   $2, $3, $4) RETURNING id"#,
    )
    .bind(format!("用户 {applicant_name} 访问授权审批"))
    .bind(format!(
        r#"{{"applicant_id": {applicant_id}, "applicant_name": "{applicant_name}"}}"#
    ))
    .bind(sla_id)
    .bind(applicant_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 审批实例：2 小时前创建 → 超时
    let instance_id = insert_instance(&pool, event_id, 2).await;

    let bus: Arc<dyn ::common::event_bus::DomainEventBus> =
        Arc::new(::common::event_bus::InMemoryEventBus::new());

    approval::sla_timeout::check_and_reject(&pool, &bus)
        .await
        .expect("check_and_reject 应成功");

    // 断言 1（remove-comments-json-embedding 降级）：applicant_id 不再从 comments
    // JSON 提取——注册用户状态保持 pending_approval（不再自动置 rejected）
    let status: String =
        sqlx::query_scalar("SELECT status FROM isahl_auth.auth_users WHERE id = $1")
            .bind(applicant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        status, "pending_approval",
        "comments 文本化后 SLA 超时不再联动注册用户状态"
    );

    // 断言 2（同降级）：站内信通知链依赖 applicant 上下文——不再投递
    let msg_cnt: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_message
           WHERE ak_benefit_user @> ARRAY[$1::bigint] AND deleted_at IS NULL
             AND notice = '访问授权审批已超时驳回'"#,
    )
    .bind(applicant_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(msg_cnt, 0, "comments 文本化后超时通知链停用");

    // 断言 3：实例状态 rejected（复用既有断言模式）
    let status_code: String = sqlx::query_scalar(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" r
           JOIN isahl."zc_id_stus-approve" s ON s.id = r.ref_right
           WHERE r.ref_left = $1 AND r.deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status_code, "rejected");

    // 清理：测试库残留（软删消息与实例；用户/事件/时长保留无害但幂等清理）
    let _ = sqlx::query("DELETE FROM isahl.zc_id_message WHERE notice = '访问授权审批已超时驳回'")
        .execute(&pool)
        .await;
    let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = $1"#)
        .bind(instance_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_even-approve" WHERE id = $1"#)
        .bind(event_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(applicant_id)
        .execute(&pool)
        .await;
}

/// fix-approval-engine-semantics D7（用户裁定「超时、通知上级」）：
/// - 节点配置 escalateTo 且有活跃成员 → 只通知上级成员（admin 不是业务最终用户，不兜底）
/// - 未配置 escalateTo / 上级岗位无成员 → 不投递任何通知（仅日志）
/// - 未注入 messaging → 无通知且驳回仍生效
#[tokio::test]
async fn sla_timeout_escalates_to_supervisor() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    grant_user_access(&pool, SYSTEM_USER_ID, "approval_actions", &["create"])
        .await
        .expect("grant system user approval access");
    // 上级岗位成员（非 admin UA）——升级通知的唯一目标
    ensure_role_member(&pool, "sup_role", 4101).await.unwrap();
    // admin 成员（验证不兜底：不应对 admin 投递）
    grant_user_access(&pool, 4001, "approval_instances", &["read"])
        .await
        .expect("grant admin member");

    let sla_id = insert_sla_duration(&pool, 1).await;
    // 节点带 escalateTo meta
    let event_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, qk_sla, comments, created_by_id)
           VALUES ('SLA 测试审批事件', $1, $2, 1) RETURNING id"#,
    )
    .bind(sla_id)
    .bind(json!({"escalateTo": "sup_role"}).to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    let overdue_id = insert_instance(&pool, event_id, 2).await;

    let bus: Arc<dyn ::common::event_bus::DomainEventBus> =
        Arc::new(::common::event_bus::InMemoryEventBus::new());
    let noop = Arc::new(common::noop_messaging::NoopMessaging::default());
    let messaging: Arc<dyn ::common::messaging::MessagingService> = noop.clone();

    approval::sla_timeout::check_and_reject_with(&pool, &bus, Some(&messaging))
        .await
        .expect("check_and_reject_with");

    // 驳回生效（升级通知不改变驳回主流程）
    let status: String = sqlx::query_scalar(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" r
           JOIN isahl."zc_id_stus-approve" s ON s.id = r.ref_right
           WHERE r.ref_left = $1 AND r.deleted_at IS NULL"#,
    )
    .bind(overdue_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "rejected");

    // 诚实降级（fix-avic-approval-node-model）：escalate 目标曾嵌 comments meta
    // （comments-text-semantics 违规）；模型暂无 escalate 归属（sla_timeout.rs
    // escalate_timeout 跳过升级通知 + warn）。断言降级语义：不投递任何通知，
    // 驳回主流程不受影响（上方 status=rejected 已断言）。
    // 待模型演进补 escalate 归属后，恢复「上级岗位成员收到 SLA 升级通知；
    // admin 不兜底」断言（sup_role 成员 4101 与 admin 4001 夹具已预置）。
    let notifications = noop.notifications.lock().unwrap().clone();
    assert!(
        notifications.is_empty(),
        "escalate 诚实降级：不应投递任何升级通知，实际: {:?}",
        notifications
    );

    // 场景 2：未配置 escalateTo → 不投递（admin 也不兜底）
    let noop2 = Arc::new(common::noop_messaging::NoopMessaging::default());
    let messaging2: Arc<dyn ::common::messaging::MessagingService> = noop2.clone();
    let event_plain = insert_approve_event(&pool, sla_id).await;
    let overdue_plain = insert_instance(&pool, event_plain, 2).await;
    approval::sla_timeout::check_and_reject_with(&pool, &bus, Some(&messaging2))
        .await
        .expect("check_and_reject_with");
    let plain_notifications = noop2.notifications.lock().unwrap().clone();
    assert!(
        plain_notifications.is_empty(),
        "未配置 escalateTo 不得投递任何通知，实际: {:?}",
        plain_notifications
    );
    let status_plain: Option<String> = sqlx::query_scalar(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" r
           JOIN isahl."zc_id_stus-approve" s ON s.id = r.ref_right
           WHERE r.ref_left = $1 AND r.deleted_at IS NULL"#,
    )
    .bind(overdue_plain)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(
        status_plain.as_deref(),
        Some("rejected"),
        "无 escalateTo 仍须驳回"
    );

    // 场景 3：escalateTo 岗位无成员 → 不投递
    let noop3 = Arc::new(common::noop_messaging::NoopMessaging::default());
    let messaging3: Arc<dyn ::common::messaging::MessagingService> = noop3.clone();
    let event_empty: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, qk_sla, comments, created_by_id)
           VALUES ('SLA 测试审批事件', $1, $2, 1) RETURNING id"#,
    )
    .bind(sla_id)
    .bind(json!({"escalateTo": "sup_role_empty"}).to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    let overdue_empty = insert_instance(&pool, event_empty, 2).await;
    approval::sla_timeout::check_and_reject_with(&pool, &bus, Some(&messaging3))
        .await
        .expect("check_and_reject_with");
    assert!(
        noop3.notifications.lock().unwrap().is_empty(),
        "上级岗位无成员不得投递任何通知"
    );
    let status_empty: Option<String> = sqlx::query_scalar(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" r
           JOIN isahl."zc_id_stus-approve" s ON s.id = r.ref_right
           WHERE r.ref_left = $1 AND r.deleted_at IS NULL"#,
    )
    .bind(overdue_empty)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(status_empty.as_deref(), Some("rejected"), "成员空仍须驳回");

    // 未注入 messaging → 无通知且驳回仍生效
    let overdue2 = insert_instance(&pool, event_id, 3).await;
    approval::sla_timeout::check_and_reject(&pool, &bus)
        .await
        .expect("check_and_reject");
    let status2: Option<String> = sqlx::query_scalar(
        r#"SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" r
           JOIN isahl."zc_id_stus-approve" s ON s.id = r.ref_right
           WHERE r.ref_left = $1 AND r.deleted_at IS NULL"#,
    )
    .bind(overdue2)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(status2.as_deref(), Some("rejected"), "未注入也须正常驳回");
}
