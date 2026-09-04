//! DbMessagingService 集成测试（fix-gateway-infra-gaps G1）
//!
//! 覆盖：站内信通道真实落库（send_direct / send_group / broadcast）、
//! 设备/告警通道显式 NotImplemented（禁止静默 Ok）。
//!
//! 使用 aliothstudio_test 数据库，自建自清 fixture（负 ID 用户）。

use ::common::testing::connect_test_db;
use alioth_gateway::notification::db_messaging::DbMessagingService;
use common::error::AliothError;
use common::messaging::{AlertLevel, DeviceCommand, MessagingService};
use sqlx::PgPool;

// ── Fixtures ─────────────────────────────────────────────────────────────────

/// 创建测试用户（负 ID，便于清理），返回 user_id
async fn ensure_test_user(pool: &PgPool, uid: i64) -> i64 {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users (id, name, username, email, status, is_active, created_at, updated_at)
           VALUES ($1, $2, $3, $4, 'active', true, NOW(), NOW())
           ON CONFLICT (id) DO UPDATE SET is_active = true
           RETURNING id"#,
    )
    .bind(uid)
    .bind(format!("User {}", uid.abs()))
    .bind(format!("user_{}", uid.abs()))
    .bind(format!("user_{}@test.local", uid.abs()))
    .fetch_one(pool)
    .await
    .expect("insert test user");
    uid
}

/// 删除测试用户及其站内信
async fn cleanup_test_user(pool: &PgPool, uid: i64) {
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(uid)
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM isahl.zc_id_message WHERE created_by_id = $1 OR $1 = ANY(ak_benefit_user)",
    )
    .bind(uid)
    .execute(pool)
    .await
    .ok();
}

const USER_A: i64 = -910001;
const USER_B: i64 = -910002;

// ── T1: send_direct 真实落库 ─────────────────────────────────────────────────

#[tokio::test]
async fn t1_send_direct_persists_message() {
    let pool = connect_test_db().await;
    ensure_test_user(&pool, USER_A).await;
    ensure_test_user(&pool, USER_B).await;
    cleanup_test_user(&pool, USER_A).await;
    cleanup_test_user(&pool, USER_B).await;
    ensure_test_user(&pool, USER_A).await;
    ensure_test_user(&pool, USER_B).await;

    let svc = DbMessagingService::new(pool.clone());
    svc.send_direct(USER_A as u64, USER_B as u64, "单聊内容")
        .await
        .expect("send_direct should succeed");

    // 断言：一条消息，notice=comments=content，受益用户 = USER_B
    let (notice, comments, benefit): (String, String, Vec<i64>) = sqlx::query_as(
        r#"SELECT notice, comments, ak_benefit_user
           FROM isahl.zc_id_message
           WHERE created_by_id = $1 AND deleted_at IS NULL
           ORDER BY id DESC LIMIT 1"#,
    )
    .bind(USER_A)
    .fetch_one(&pool)
    .await
    .expect("message should exist");

    assert_eq!(notice, "单聊内容");
    assert_eq!(comments, "单聊内容");
    assert!(
        benefit.contains(&USER_B),
        "benefit users should contain recipient"
    );

    cleanup_test_user(&pool, USER_A).await;
    cleanup_test_user(&pool, USER_B).await;
}

// ── T2: broadcast 对全部活跃用户落库 ────────────────────────────────────────

#[tokio::test]
async fn t2_broadcast_persists_to_active_users() {
    let pool = connect_test_db().await;
    ensure_test_user(&pool, USER_A).await;
    ensure_test_user(&pool, USER_B).await;

    let svc = DbMessagingService::new(pool.clone());
    svc.broadcast(0, "全站广播")
        .await
        .expect("broadcast should succeed");

    // 断言：USER_A 与 USER_B 都能读到这条广播
    for uid in [USER_A, USER_B] {
        let count: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*) FROM isahl.zc_id_message
               WHERE (created_by_id = $1 OR $1 = ANY(ak_benefit_user))
                 AND notice = '全站广播' AND deleted_at IS NULL"#,
        )
        .bind(uid)
        .fetch_one(&pool)
        .await
        .expect("count query");
        assert!(count >= 1, "user {} should receive broadcast", uid);
    }

    cleanup_test_user(&pool, USER_A).await;
    cleanup_test_user(&pool, USER_B).await;
}

// ── T3: send_group 关联会话线程 ──────────────────────────────────────────────

#[tokio::test]
async fn t3_send_group_links_thread() {
    let pool = connect_test_db().await;
    ensure_test_user(&pool, USER_A).await;
    cleanup_test_user(&pool, USER_A).await;
    ensure_test_user(&pool, USER_A).await;

    let svc = DbMessagingService::new(pool.clone());
    svc.send_group(USER_A as u64, 123456789, "群聊内容")
        .await
        .expect("send_group should succeed");

    let (thread, content): (Option<i64>, String) = sqlx::query_as(
        r#"SELECT fk_thread, comments FROM isahl.zc_id_message
           WHERE created_by_id = $1 AND deleted_at IS NULL
           ORDER BY id DESC LIMIT 1"#,
    )
    .bind(USER_A)
    .fetch_one(&pool)
    .await
    .expect("group message should exist");

    assert_eq!(thread, Some(123456789));
    assert_eq!(content, "群聊内容");

    cleanup_test_user(&pool, USER_A).await;
}

// ── T4: 设备/告警通道显式 NotImplemented ─────────────────────────────────────

#[tokio::test]
async fn t4_device_and_alert_channels_fail_explicitly() {
    let pool = connect_test_db().await;
    let svc = DbMessagingService::new(pool.clone());

    // send_alert → NotImplemented
    let alert_res = svc.send_alert(AlertLevel::Warning, "告警", "内容").await;
    assert!(
        matches!(alert_res, Err(AliothError::NotImplemented(_))),
        "send_alert must be NotImplemented, got {:?}",
        alert_res
    );

    // send_device_command → NotImplemented
    let dev_cmd = DeviceCommand::new("dev-cmd-1", "restart");
    let dev_res = svc.send_device_command("dev-1", dev_cmd).await;
    assert!(
        matches!(dev_res, Err(AliothError::NotImplemented(_))),
        "send_device_command must be NotImplemented, got {:?}",
        dev_res
    );

    // broadcast_device_command → NotImplemented
    let bdc_res = svc
        .broadcast_device_command(DeviceCommand::new("dev-cmd-2", "restart"))
        .await;
    assert!(
        matches!(bdc_res, Err(AliothError::NotImplemented(_))),
        "broadcast_device_command must be NotImplemented, got {:?}",
        bdc_res
    );

    // send_raw → NotImplemented
    let raw_res = svc.send_raw("device/all", vec![], 1).await;
    assert!(
        matches!(raw_res, Err(AliothError::NotImplemented(_))),
        "send_raw must be NotImplemented, got {:?}",
        raw_res
    );
}
