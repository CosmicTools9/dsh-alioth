//! identity-org 铅封批量创建集成测试（refactor-dispatch-seal-code-generation）
//!
//! 覆盖：code 前缀自动续号（空前缀首建 / 续建接号 / count 缺省 1）
//! + 显式 startCode 冲突 400。依赖：test 库存在 "isahl"."zc_id_devi-seal"。

use identity_org::models::CreateSealBatchRequest;
use identity_org::repository::SealRepository;
use sqlx::PgPool;

async fn test_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://isahl@localhost:5432/aliothstudio_test".to_string());
    let pool = PgPool::connect(&url).await.expect("connect test db");
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("current_database");
    assert!(db.contains("_test"), "REFUSED: non-test db {db}");
    pool
}

/// 动态测试前缀（纳秒派生，跨运行不冲突；仅字母数字——batch 前缀约束）
fn prefix(tag: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("TS{tag}{:x}", nanos % 0xFFFF_FFFF)
}

fn req(prefix: String, count: Option<i64>, start_code: Option<String>) -> CreateSealBatchRequest {
    CreateSealBatchRequest {
        seal_type: Some(prefix),
        start_code,
        count,
        notice: None,
        comments: None,
        waybill_id: None,
    }
}

async fn cleanup(pool: &PgPool, prefix: &str) {
    sqlx::query(r#"DELETE FROM "isahl"."zc_id_devi-seal" WHERE code LIKE $1"#)
        .bind(format!("{prefix}-%"))
        .execute(pool)
        .await
        .expect("cleanup");
}

#[tokio::test]
async fn seal_batch_auto_sequential() {
    let pool = test_pool().await;
    let repo = SealRepository::new(pool.clone());
    let pfx = prefix("A");
    cleanup(&pool, &pfx).await;

    // 首建 3 条连号：空前缀从 0001 起等宽 4 位
    let items = repo
        .batch_create(req(pfx.clone(), Some(3), None), 1)
        .await
        .expect("first batch");
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].code, Some(format!("{pfx}-0001")));
    assert_eq!(items[1].code, Some(format!("{pfx}-0002")));
    assert_eq!(items[2].code, Some(format!("{pfx}-0003")));

    // 续建 2 条：从现有最大序号 +1 续号
    let more = repo
        .batch_create(req(pfx.clone(), Some(2), None), 1)
        .await
        .expect("second batch");
    assert_eq!(more[0].code, Some(format!("{pfx}-0004")));
    assert_eq!(more[1].code, Some(format!("{pfx}-0005")));

    // count 缺省 = 1（单号）
    let single = repo
        .batch_create(req(pfx.clone(), None, None), 1)
        .await
        .expect("single");
    assert_eq!(single.len(), 1);
    assert_eq!(single[0].code, Some(format!("{pfx}-0006")));

    cleanup(&pool, &pfx).await;
}

#[tokio::test]
async fn seal_batch_explicit_start_conflict_rejected() {
    let pool = test_pool().await;
    let repo = SealRepository::new(pool.clone());
    let pfx = prefix("B");
    cleanup(&pool, &pfx).await;

    // 种子 1 条（自动续号 → {pfx}-0001）
    repo.batch_create(req(pfx.clone(), Some(1), None), 1)
        .await
        .expect("seed");

    // 显式 startCode 撞现有号 → 400 且消息含冲突号
    let err = repo
        .batch_create(req(pfx.clone(), Some(1), Some(format!("{pfx}-0001"))), 1)
        .await;
    match err {
        Err(common::AliothError::BadRequest(msg)) => {
            assert!(msg.contains("铅封号已被使用"), "unexpected msg: {msg}");
        }
        other => panic!("expected BadRequest, got {other:?}"),
    }

    cleanup(&pool, &pfx).await;
}
