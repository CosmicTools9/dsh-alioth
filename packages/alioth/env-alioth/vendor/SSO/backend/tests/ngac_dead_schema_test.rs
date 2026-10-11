//! 死面退役回归（drop-ngac-dead-schema）：ensure 后三处死面必须不存在，
//! 且决策链行为不变（由既有 NGAC 测试族覆盖——本文件只钉 schema 事实）。

use sqlx::PgPool;

async fn test_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .or_else(|_| std::env::var("SSO_TEST_DATABASE_URL"))
        .unwrap_or_else(|_| {
            let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_string());
            format!("postgres://{user}@localhost:5432/aliothstudio_test")
        });
    PgPool::connect(&url)
        .await
        .expect("测试库不可达（reset-db.sh --test）")
}

#[tokio::test]
async fn dead_schema_retired_after_ensure() {
    let pool = test_pool().await;
    // 进程级 once：测试二进制内首次调用会执行清理段
    gateway_sso::ngac::ensure::ensure_ngac_extension_tables(&pool).await;

    let table: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pg_tables WHERE schemaname='isahl_auth' AND tablename='ngac_resource_identifier'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        table.0, 0,
        "ngac_resource_identifier 表必须已被 ensure 退役"
    );

    let cols: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema='isahl_auth' \
         AND ((table_name='ngac_object_attribute' AND column_name IN ('ak_permit_user','ak_benefit_user','ak_access_user')) \
           OR (table_name='ngac_association' AND column_name='condition_expr'))",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        cols.0, 0,
        "OA ak_* 三列与 association.condition_expr 必须已被退役"
    );

    // 边界：不误伤——业务根表 ak_* 与 OA.resource_identifier 仍在
    let keep: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema='isahl' \
         AND table_name='zc_id_lifecycle' AND column_name IN ('ak_permit_user','ak_benefit_user','ak_access_user')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        keep.0, 3,
        "业务根表 ak_* 三列不受退役影响（OWNERSHIP_CTE 消费）"
    );
    let oa_rid: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema='isahl_auth' \
         AND table_name='ngac_object_attribute' AND column_name='resource_identifier'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        oa_rid.0, 1,
        "OA.resource_identifier（PEP resolve 消费）不受退役影响"
    );
}
