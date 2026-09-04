//! alioth-service-isahl-db 集成测试
//!
//! 验证 entity 因子的 Identity CRUD。

use common::testing::{connect_test_db, setup_test_schema_light};
use crud::AliothRepository;
#[tokio::test]
async fn identity_list_with_rls() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    sqlx::query("DELETE FROM isahl.zc_id_lifecycle")
        .execute(&pool)
        .await
        .unwrap();

    let first = alioth_service_isahl_db::seed::seed_identities(&pool)
        .await
        .unwrap();
    assert_eq!(first, 2, "should seed 2 identities");

    let repo = identity_org::repository::IdentityRepository::from(pool.clone());

    let query = common::data::ListQuery {
        page: 1,
        page_size: 10,
        filter_field: None,
        filter_op: None,
        filter_value: None,
        sort_field: None,
        sort_order: None,
    };

    let all = repo.list_with_rls(&query, None, None).await.unwrap();
    assert_eq!(all.total, 2, "should list 2 seeded identities");
    assert_eq!(all.items.len(), 2);

    let visible_id = all.items[0].id;
    let filtered = repo
        .list_with_rls(&query, Some(&[visible_id]), None)
        .await
        .unwrap();
    assert_eq!(filtered.total, 1, "RLS filter should return 1 identity");
    assert_eq!(filtered.items[0].id, visible_id);
}

#[tokio::test]
async fn identity_list_with_null_notice_record() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    sqlx::query("DELETE FROM isahl.zc_id_lifecycle")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query(
        r#"INSERT INTO isahl.zc_id_subjects (notice, code, created_by_id) VALUES (NULL, NULL, 1)"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let repo = identity_org::repository::IdentityRepository::from(pool.clone());
    let query = common::data::ListQuery {
        page: 1,
        page_size: 10,
        filter_field: None,
        filter_op: None,
        filter_value: None,
        sort_field: None,
        sort_order: None,
    };

    let all = repo.list_with_rls(&query, None, None).await.unwrap();
    assert_eq!(all.total, 1, "should list 1 identity");
    assert_eq!(
        all.items[0].name, "",
        "null notice should decode as empty name"
    );
    assert_eq!(all.items[0].code, None, "null code should decode as None");
}

#[tokio::test]
async fn identity_crud() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    sqlx::query("DELETE FROM isahl.zc_id_lifecycle")
        .execute(&pool)
        .await
        .unwrap();
    let uid: i64 = 1;

    let repo = identity_org::repository::IdentityRepository::from(pool.clone());
    let created = repo
        .create(
            identity_org::models::CreateIdentityRequest {
                name: "测试身份".to_string(),
                subject_type: "group".to_string(),
                code: Some("TEST-001".to_string()),
                notice: Some("这是一个测试身份".to_string()),
            },
            uid,
        )
        .await
        .unwrap();

    assert_eq!(created.name, "测试身份");
    assert_eq!(created.code.as_deref(), Some("TEST-001"));

    let fetched = repo.get(created.id).await.unwrap().unwrap();
    assert_eq!(fetched.name, "测试身份");
    assert_eq!(fetched.code.as_deref(), Some("TEST-001"));

    let updated = repo
        .update(
            created.id,
            identity_org::models::UpdateIdentityRequest {
                name: Some("更新身份".to_string()),
                code: None,
                notice: None,
                comments: None,
                mdm_code: None,
            },
            uid,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.name, "更新身份");
    assert_eq!(updated.code.as_deref(), Some("TEST-001"));

    repo.delete(created.id, uid).await.unwrap();
    assert!(repo.get(created.id).await.unwrap().is_none());
}

#[tokio::test]
async fn identity_seed_idempotent() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    sqlx::query("DELETE FROM isahl.zc_id_lifecycle")
        .execute(&pool)
        .await
        .unwrap();

    let first = alioth_service_isahl_db::seed::seed_identities(&pool)
        .await
        .unwrap();
    let second = alioth_service_isahl_db::seed::seed_identities(&pool)
        .await
        .unwrap();

    assert_eq!(first, 2, "should insert 2 identities on first run");
    assert_eq!(second, 0, "should be idempotent on second run");
}
