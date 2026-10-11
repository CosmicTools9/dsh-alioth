//! 决策审计义务落列测试（wire-obligations-into-pep-audit）。
//!
//! 判据：`record_audit_event_with_obligations` 把 PDP 下发的义务算子写入
//! `isahl_audit.audit_events.obligations_triggered`——Gateway PEP 行与 SSO 决策审计行
//! 同源对称（不再恒空）。
//!
//! 运行：`bash scripts/cargo-test.sh -p common --test ngac_audit_obligations_test -- --ignored --test-threads=1`。

use common::audit::{
    record_audit_event_with_metadata, record_audit_event_with_obligations, Decision, DecisionAudit,
};
use common::testing::connect_test_db;
use sqlx::PgPool;
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};

static SERIAL_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn lock_serial() -> MutexGuard<'static, ()> {
    SERIAL_LOCK.lock().await
}

async fn last_row_obligations(pool: &PgPool, user_id: i64) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT obligations_triggered FROM isahl_audit.audit_events \
         WHERE user_id = $1 ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "needs test DB"]
async fn audit_rows_carry_obligations_symmetrically() {
    let _g = lock_serial().await;
    let pool = connect_test_db().await;
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE id > 0 ORDER BY id LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();

    // 旧入口（业务事件）——恒空语义保持
    record_audit_event_with_metadata(
        &pool,
        user_id,
        &format!("user:{user_id}"),
        "z-test-audit-obl:0",
        "read",
        &Decision::Permit,
        serde_json::json!({}),
    )
    .await
    .unwrap();
    assert!(
        last_row_obligations(&pool, user_id).await.is_empty(),
        "业务事件（无义务面）恒空——语义保持"
    );

    // 义务携带版（PEP 决策审计）——落列
    record_audit_event_with_obligations(
        &pool,
        DecisionAudit {
            user_id,
            subject: format!("user:{user_id}"),
            object_path: "z-test-audit-obl:0".to_string(),
            operation: "read".to_string(),
            decision: Decision::Permit,
            metadata: serde_json::json!({}),
            obligations: vec!["audit:log".to_string(), "notify:admin".to_string()],
        },
    )
    .await
    .unwrap();
    let got = last_row_obligations(&pool, user_id).await;
    assert_eq!(got, vec!["audit:log", "notify:admin"], "义务随审计行落列");
}
