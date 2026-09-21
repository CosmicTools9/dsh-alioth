//! 实名核验后的账号状态处理 — 自动审批通过开关（`approval:auto-approve`）集成测试
//!
//! 断言三条语义：
//! - 开关关闭 ⇒ 置 `pending_approval`（原语义）
//! - 开关开启 ⇒ **保留**现有状态（已激活用户不被降级）
//! - 配置行缺失 ⇒ 视为关闭（fail-closed，MUST NOT 默认开启兜底）
//!
//! 使用 aliothstudio_test 库、负数 ID fixture、自建自清。

use ::common::testing::connect_test_db;
use gateway_sso::auth::identity::apply_status_after_identity_verify;
use sqlx::PgPool;

const CODE: &str = "approval:auto-approve";
const USER_ID: i64 = -99301;

/// 三个用例共用同一配置行与 fixture id ⇒ 串行执行（cargo 默认并行跑测试）。
/// 用 `tokio::sync::Mutex`：std guard 跨 await 持有会被 clippy `await_holding_lock` 拦截。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn serialize() -> tokio::sync::MutexGuard<'static, ()> {
    SERIAL.lock().await
}

async fn set_switch(pool: &PgPool, enabled: Option<bool>) {
    sqlx::query(r#"DELETE FROM isahl."zc_id_prot-env_config" WHERE code = $1"#)
        .bind(CODE)
        .execute(pool)
        .await
        .expect("clear switch row");
    if let Some(v) = enabled {
        sqlx::query(
            // 类契约/坐标（§4.3.3 形态1 + §6.12）：dk_function 作派生源，三坐标按 code 解析
            // （取值与本表模型级种子 seed-auth-approval-flows.sql §5 一致）
            r#"INSERT INTO isahl."zc_id_prot-env_config"
                 (notice, code, settings, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ('实名状态测试开关', $1, jsonb_build_object('enabled', $2), 1,
                       (SELECT id FROM isahl.zc_id_scene    WHERE code = 'JE'   AND deleted_at IS NULL),
                       (SELECT id FROM isahl.zc_id_factor   WHERE code = 'GEC'  AND deleted_at IS NULL),
                       (SELECT id FROM isahl.zc_id_function WHERE code = '↑_DA' AND deleted_at IS NULL))"#,
        )
        .bind(CODE)
        .bind(v)
        .execute(pool)
        .await
        .expect("insert switch row");
    }
}

async fn reset_user(pool: &PgPool, status: &str) {
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(USER_ID)
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, status, created_at, updated_at) \
         VALUES ($1, $2, $3, NOW(), NOW())",
    )
    .bind(USER_ID)
    .bind(format!("identity-auto-approve-{USER_ID}"))
    .bind(status)
    .execute(pool)
    .await
    .expect("insert user fixture");
}

async fn status_of(pool: &PgPool) -> String {
    sqlx::query_scalar("SELECT status FROM isahl_auth.auth_users WHERE id = $1")
        .bind(USER_ID)
        .fetch_one(pool)
        .await
        .expect("query user status")
}

async fn cleanup(pool: &PgPool) {
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(USER_ID)
        .execute(pool)
        .await
        .ok();
    set_switch(pool, None).await;
}

#[tokio::test]
async fn t_identity_status_switch_off_downgrades() {
    let _guard = serialize().await;
    let pool = connect_test_db().await;
    reset_user(&pool, "active").await;
    set_switch(&pool, Some(false)).await;

    apply_status_after_identity_verify(&pool, USER_ID).await;

    assert_eq!(
        status_of(&pool).await,
        "pending_approval",
        "开关关闭应按原语义置待批"
    );

    cleanup(&pool).await;
}

#[tokio::test]
async fn t_identity_status_switch_on_keeps_active() {
    let _guard = serialize().await;
    let pool = connect_test_db().await;
    reset_user(&pool, "active").await;
    set_switch(&pool, Some(true)).await;

    apply_status_after_identity_verify(&pool, USER_ID).await;

    assert_eq!(
        status_of(&pool).await,
        "active",
        "开关开启时实名提交不得把已激活用户降级"
    );

    cleanup(&pool).await;
}

#[tokio::test]
async fn t_identity_status_missing_switch_row_is_closed() {
    let _guard = serialize().await;
    let pool = connect_test_db().await;
    reset_user(&pool, "active").await;
    set_switch(&pool, None).await; // 配置行缺失 ⇒ fail-closed

    apply_status_after_identity_verify(&pool, USER_ID).await;

    assert_eq!(
        status_of(&pool).await,
        "pending_approval",
        "配置行缺失必须视为关闭（保留原待批语义）"
    );

    cleanup(&pool).await;
}
