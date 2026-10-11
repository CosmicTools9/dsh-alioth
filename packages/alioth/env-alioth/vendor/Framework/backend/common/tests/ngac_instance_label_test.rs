//! 实例行 OA 分级标签写径测试（wire-instance-oa-labels，§2.7 实例级写径② + 判定参与）。
//!
//! 判据：
//! 1. **继承**：集合 OA 带 `property.label` 时，`ensure_resource_oa` 创建的**实例行 OA**
//!    自动继承同标签（`oa_insert` 内联子查询）；
//! 2. **判定参与**：白名单资源类型的实例行，`label_clearance_denies` 读实例 OA 标签
//!    ⇒ 无 clearance 主体对**实例行**判拒（此前实例级无生产方 ⇒ 不介入）；
//! 3. **非白名单不介入**：未列入 scope 的资源类型实例行不受影响。
//!
//! 运行：`bash scripts/cargo-test.sh -p common --test ngac_instance_label_test -- --ignored --test-threads=1`。

use common::ngac_grants::{ensure_resource_oa, ResourceOaSpec};
use common::testing::connect_test_db;
use ngac_host::label_clearance_denies;
use sqlx::PgPool;
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};

static SERIAL_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn lock_serial() -> MutexGuard<'static, ()> {
    SERIAL_LOCK.lock().await
}

const RT: &str = "z-test-inst-rt";
const RT_OTHER: &str = "z-test-inst-other";
const LABEL: &str = "z-test-inst-label";
const ROW_ID: i64 = 987_654_321;

async fn cleanup(pool: &PgPool) {
    for rt in [RT, RT_OTHER] {
        sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type = $1")
            .bind(rt)
            .execute(pool)
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM isahl_auth.org_policy_class WHERE code = $1")
        .bind("z-test-inst-class")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM isahl_auth.org_policy_label WHERE code = $1")
        .bind(LABEL)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs test DB"]
async fn instance_oa_inherits_collection_label_and_denies() {
    let _g = lock_serial().await;
    let pool = connect_test_db().await;
    cleanup(&pool).await;

    // 夹具：分级字典 + 白名单类（scope 含 RT，不含 RT_OTHER）+ 测试用户（无 clearance）
    sqlx::query(
        r#"INSERT INTO isahl_auth.org_policy_label (code, rank, domain, notice, is_active)
           VALUES ($1, 5, 'test', '实例级写径测试标签', TRUE)"#,
    )
    .bind(LABEL)
    .execute(&pool)
    .await
    .unwrap();
    let class_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.org_policy_class
             (code, notice, state, scope, effective_from)
           VALUES ('z-test-inst-class', '实例级测试白名单', 'active',
                   jsonb_build_object('resource_types', jsonb_build_array($1)),
                   NOW() - INTERVAL '1 hour')
           RETURNING id"#,
    )
    .bind(RT)
    .fetch_one(&pool)
    .await
    .unwrap();
    let _ = class_id;
    // 无 clearance 主体 = 既有任一用户（clearance 只来自 UA property——种子用户无本测试标签 clearance）
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users ORDER BY id LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    // 集合 OA 带 label（模拟派生器正本写入产物）
    ensure_resource_oa(
        &pool,
        &ResourceOaSpec {
            o_name: format!("{RT}-collection"),
            resource_type: RT,
            resource_id: 0,
            resource_identifier: None,
            created_by: None,
            policy_class_name: Some("default"),
        },
    )
    .await
    .unwrap();
    sqlx::query(
        r#"UPDATE isahl_auth.ngac_object_attribute
           SET property = jsonb_build_object('label', $1)
           WHERE resource_type = $2 AND fk_resource = 0"#,
    )
    .bind(LABEL)
    .bind(RT)
    .execute(&pool)
    .await
    .unwrap();

    // ── ① 继承：实例行 OA 创建即带集合标签 ──
    ensure_resource_oa(
        &pool,
        &ResourceOaSpec {
            o_name: format!("{RT}-{ROW_ID}"),
            resource_type: RT,
            resource_id: ROW_ID,
            resource_identifier: Some("z-test-inst-ident"),
            created_by: None,
            policy_class_name: Some("default"),
        },
    )
    .await
    .unwrap();
    let inherited: Option<String> = sqlx::query_scalar(
        "SELECT property ->> 'label' FROM isahl_auth.ngac_object_attribute \
         WHERE resource_type = $1 AND fk_resource = $2 AND deleted_at IS NULL",
    )
    .bind(RT)
    .bind(ROW_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        inherited.as_deref(),
        Some(LABEL),
        "实例 OA 继承集合标签（写径②）"
    );

    // ── ② 判定参与：无 clearance 主体对白名单实例行判拒（fail-closed）──
    let denied = label_clearance_denies(&pool, RT, ROW_ID, user_id)
        .await
        .unwrap();
    assert!(denied, "实例行标签参与密级上限：无 clearance ⇒ 拒");

    // ── ③ 非白名单资源类型不介入（对照）──
    ensure_resource_oa(
        &pool,
        &ResourceOaSpec {
            o_name: format!("{RT_OTHER}-{ROW_ID}"),
            resource_type: RT_OTHER,
            resource_id: ROW_ID,
            resource_identifier: None,
            created_by: None,
            policy_class_name: Some("default"),
        },
    )
    .await
    .unwrap();
    let other_denied = label_clearance_denies(&pool, RT_OTHER, ROW_ID, user_id)
        .await
        .unwrap();
    assert!(!other_denied, "非白名单类型：上限不介入");

    cleanup(&pool).await;
}
