//! App 可见性自愈集成测试（generalize-app-visibility-seed）
//!
//! 验证 Gateway 启动自愈链：发现面（DEPLOY_PATH apps.json 聚合形态）驱动的
//! App OA + admin 关联建立、幂等重放、零硬编码（夹具 app code 为测试专有）。

use common::testing::connect_test_db;
use sqlx::PgPool;

const TEST_APP: &str = "seed-vis-test-app-x1";

async fn setup_fixture(pool: &PgPool) {
    // 基座：复用真实链路（policy_class default / UA / access_right 幂等自愈）
    alioth_gateway::seed::ngac_seed::ensure(pool).await;
    // 防御性清理：历史残留测试行
    let _ = sqlx::query(
        r#"DELETE FROM isahl_auth.ngac_association a
           USING isahl_auth.ngac_object_attribute oa
           WHERE a.fk_object_attribute = oa.id AND oa.resource_type = 'app' AND oa.resource_identifier = $1"#,
    )
    .bind(TEST_APP)
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type = 'app' AND resource_identifier = $1",
    )
    .bind(TEST_APP)
    .execute(pool)
    .await;
}

async fn webdir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("alioth-app-vis-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("临时目录创建失败");
    std::fs::write(
        dir.join("apps.json"),
        format!(r#"{{"appInstances":[{{"code":"{TEST_APP}","namespace":"test"}}]}}"#),
    )
    .expect("夹具 apps.json 写入失败");
    dir
}

async fn oa_and_assoc(pool: &PgPool) -> (i64, i64) {
    let oa: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type = 'app' AND resource_identifier = $1 AND deleted_at IS NULL",
    )
    .bind(TEST_APP)
    .fetch_optional(pool)
    .await
    .expect("查 OA 失败");
    let assoc: Option<i64> = sqlx::query_scalar(
        r#"SELECT a.id FROM isahl_auth.ngac_association a
           JOIN isahl_auth.ngac_user_attribute ua ON ua.id = a.fk_user_attribute
           JOIN isahl_auth.ngac_object_attribute oa ON oa.id = a.fk_object_attribute
           WHERE ua.o_name = 'admin' AND oa.resource_type = 'app' AND oa.resource_identifier = $1
             AND a.deleted_at IS NULL"#,
    )
    .bind(TEST_APP)
    .fetch_optional(pool)
    .await
    .expect("查关联失败");
    (oa.unwrap_or(0), assoc.unwrap_or(0))
}

#[tokio::test]
async fn app_visibility_seed_builds_and_is_idempotent() {
    let pool = connect_test_db().await;
    setup_fixture(&pool).await;
    let dir = webdir().await;
    std::env::set_var("DEPLOY_PATH", &dir);

    let (oa0, assoc0) = oa_and_assoc(&pool).await;
    assert_eq!((oa0, assoc0), (0, 0), "前置清理后应无残留");

    // 首跑：发现面驱动建立 OA + admin 关联
    let s1 = alioth_gateway::seed::app_visibility_seed::ensure(&pool).await;
    let (oa1, assoc1) = oa_and_assoc(&pool).await;
    assert!(oa1 > 0, "应建立 App OA");
    assert!(assoc1 > 0, "应建立 admin → App OA 关联");
    let created_first = s1.created;
    assert!(
        created_first >= 2,
        "首跑应至少新建 OA+关联各一，实际 {created_first}"
    );

    // 重放：幂等（不新增）
    let s2 = alioth_gateway::seed::app_visibility_seed::ensure(&pool).await;
    assert_eq!(s2.created, 0, "重放不得新增行");
    let (oa2, assoc2) = oa_and_assoc(&pool).await;
    assert_eq!((oa2, assoc2), (oa1, assoc1), "重放后实体不变");

    // 清理
    let _ = sqlx::query(
        r#"DELETE FROM isahl_auth.ngac_association a
           USING isahl_auth.ngac_object_attribute oa
           WHERE a.fk_object_attribute = oa.id AND oa.resource_type = 'app' AND oa.resource_identifier = $1"#,
    )
    .bind(TEST_APP)
    .execute(&pool)
    .await;
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type = 'app' AND resource_identifier = $1",
    )
    .bind(TEST_APP)
    .execute(&pool)
    .await;
    std::env::remove_var("DEPLOY_PATH");
    let _ = std::fs::remove_dir_all(&dir);
}
