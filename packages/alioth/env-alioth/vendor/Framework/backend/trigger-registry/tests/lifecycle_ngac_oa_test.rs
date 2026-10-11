//! trigger-registry 集成测试：`SideEffect::EnsureResourceOa` 的执行面（授权写入单源）
//!
//! 验证 `SideEffectExecutor` 的 OA 注册**复用** `common::ngac_grants`（NGAC_SPEC §2.2.5，
//! M-NGAC-B 触发器数据面单源）：
//! 1. 命名与标识：`o_name` = `{resource_type}-{id}`、`resource_identifier` 落库、策略类 = `default`
//! 2. 幂等：同 `(resource_type, fk_resource)` 重复执行 ⇒ 单行、零改写（首写者胜）
//! 3. 软删占位行不复活：软删后重放 ⇒ 零新增、仍软删
//! 4. 事务入口：`execute_all_in_transaction` 内的分支同样落单源（提交后可见）
//!
//! 需要 DATABASE_URL 指向 aliothstudio_test（`#[ignore]`，仿 mv_inventory_self_heal_test 先例）。
//! 夹具自建自清：**每个用例只清自己的资源类型**（`RT` / `RT_TX`）——两个用例共享同一测试库，
//! 若各自清 `[RT, RT_TX]` 会互删对方行（实测：并发下「软删不复活」用例被另一用例的清理硬删占位行，
//! 重放遂插入新行 ⇒ 假失败；`--test-threads=1` 下恒绿）。

use common::testing::connect_test_db;
use sqlx::PgPool;
use trigger_registry::executor::SideEffectExecutor;
use trigger_registry::SideEffect;

const RT: &str = "z-test-trigger-oa";
const RT_TX: &str = "z-test-trigger-oa-tx";

fn effect(resource_type: &str, resource_id: i64, identifier: Option<&str>) -> SideEffect {
    SideEffect::EnsureResourceOa {
        resource_type: resource_type.to_string(),
        resource_id,
        resource_identifier: identifier.map(str::to_string),
    }
}

async fn cleanup(pool: &PgPool, rt: &str) {
    for rt in [rt] {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type = $1",
        )
        .bind(rt)
        .fetch_all(pool)
        .await
        .expect("清理：读 OA id");
        for id in ids {
            sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE fk_object_attribute = $1")
                .bind(id)
                .execute(pool)
                .await
                .expect("清理：删 association");
        }
        sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type = $1")
            .bind(rt)
            .execute(pool)
            .await
            .expect("清理：删 OA");
    }
}

async fn open_test_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    match url {
        Some(_) => Some(connect_test_db().await),
        None => {
            eprintln!("skipped: DATABASE_URL 未设置");
            None
        }
    }
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn ensure_resource_oa_is_idempotent_and_not_revived() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool, RT).await;
    let executor = SideEffectExecutor::new(pool.clone());

    // 1) 首次执行 ⇒ 建行；命名/标识/策略类按单源约定
    executor
        .execute(&effect(RT, 9_900_101, Some("BILL-9900101")))
        .await
        .expect("首次 EnsureResourceOa");
    let (o_name, ident, pc_name): (String, Option<String>, String) = sqlx::query_as(
        "SELECT oa.o_name, oa.resource_identifier, pc.o_name \
           FROM isahl_auth.ngac_object_attribute oa \
           JOIN isahl_auth.ngac_policy_class pc ON pc.id = oa.fk_policy_class \
          WHERE oa.resource_type = $1 AND oa.fk_resource = $2 AND oa.deleted_at IS NULL",
    )
    .bind(RT)
    .bind(9_900_101i64)
    .fetch_one(&pool)
    .await
    .expect("首写后应有 OA 行");
    assert_eq!(o_name, format!("{RT}-9900101"));
    assert_eq!(ident.as_deref(), Some("BILL-9900101"));
    assert_eq!(pc_name, "default");

    // 2) 幂等：重复执行 ⇒ 单行、零改写
    executor
        .execute(&effect(RT, 9_900_101, Some("BILL-其它")))
        .await
        .expect("重放 EnsureResourceOa");
    let (total, alive): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COUNT(*) FILTER (WHERE deleted_at IS NULL) \
           FROM isahl_auth.ngac_object_attribute WHERE resource_type = $1",
    )
    .bind(RT)
    .fetch_one(&pool)
    .await
    .expect("计数");
    assert_eq!(total, 1, "重放 MUST NOT 新增行");
    assert_eq!(alive, 1);
    let ident_after: Option<String> = sqlx::query_scalar(
        "SELECT resource_identifier FROM isahl_auth.ngac_object_attribute \
          WHERE resource_type = $1 AND fk_resource = $2",
    )
    .bind(RT)
    .bind(9_900_101i64)
    .fetch_one(&pool)
    .await
    .expect("标识");
    assert_eq!(
        ident_after.as_deref(),
        Some("BILL-9900101"),
        "幂等注册 MUST NOT 改写既有行（首写者胜）"
    );

    // 3) 软删占位行不复活：软删后重放 ⇒ 零新增、仍软删
    sqlx::query(
        "UPDATE isahl_auth.ngac_object_attribute SET deleted_at = NOW() \
          WHERE resource_type = $1 AND fk_resource = $2",
    )
    .bind(RT)
    .bind(9_900_101i64)
    .execute(&pool)
    .await
    .expect("软删");
    executor
        .execute(&effect(RT, 9_900_101, Some("BILL-复活尝试")))
        .await
        .expect("软删后重放");
    let (total, alive): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COUNT(*) FILTER (WHERE deleted_at IS NULL) \
           FROM isahl_auth.ngac_object_attribute WHERE resource_type = $1",
    )
    .bind(RT)
    .fetch_one(&pool)
    .await
    .expect("计数");
    assert_eq!(total, 1, "软删占位行 MUST NOT 被新增行顶替");
    assert_eq!(alive, 0, "软删占位行 MUST NOT 复活");

    cleanup(&pool, RT).await;
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn ensure_resource_oa_in_transaction_uses_single_source() {
    let Some(pool) = open_test_pool().await else {
        return;
    };
    cleanup(&pool, RT_TX).await;
    let executor = SideEffectExecutor::new(pool.clone());

    let effects = vec![
        effect(RT_TX, 9_900_201, Some("TX-A")),
        effect(RT_TX, 9_900_202, None),
    ];
    executor
        .execute_all_in_transaction(&effects)
        .await
        .expect("事务内 EnsureResourceOa");

    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT o_name, resource_identifier FROM isahl_auth.ngac_object_attribute \
          WHERE resource_type = $1 AND deleted_at IS NULL ORDER BY fk_resource",
    )
    .bind(RT_TX)
    .fetch_all(&pool)
    .await
    .expect("事务提交后应可见");
    assert_eq!(
        rows,
        vec![
            (format!("{RT_TX}-9900201"), Some("TX-A".to_string())),
            (format!("{RT_TX}-9900202"), None),
        ]
    );

    cleanup(&pool, RT_TX).await;
}
