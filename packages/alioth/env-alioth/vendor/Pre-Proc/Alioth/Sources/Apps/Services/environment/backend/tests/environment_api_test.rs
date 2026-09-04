//! alioth-service-environment 集成测试
//!
//! 验证 environment 因子的 Environment CRUD + settings JSONB 序列化。

use common::data::ListQuery;
use common::testing::{connect_test_db, setup_test_schema_light};
use crud::AliothRepository;
use sqlx::PgPool;

async fn insert_protocol_status(pool: &PgPool, notice: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_stus-protocol" (notice, created_by_id) VALUES ($1, 1) RETURNING id"#,
    )
    .bind(notice)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn environment_crud() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let uid: i64 = 1;

    let status_id = insert_protocol_status(&pool, "运行中").await;

    let repo = alioth_service_environment::repositories::EnvironmentRepository::from(pool.clone());

    let created = repo
        .create(
            alioth_service_environment::models::CreateEnvironmentRequest {
                name: "生产环境".to_string(),
                host: Some("prod.example.com".to_string()),
                os: Some("linux".to_string()),
                runtime: None,
                type_: Some("prod".to_string()),
                status: Some(status_id),
                services: Some(5),
                uptime: Some("72h".to_string()),
                comments: Some("主生产集群".to_string()),
            },
            uid,
        )
        .await
        .unwrap();

    assert_eq!(created.name, "生产环境");
    assert_eq!(created.host.as_deref(), Some("prod.example.com"));
    assert_eq!(created.status, Some(status_id));
    assert_eq!(created.type_.as_deref(), Some("prod"));
    assert_eq!(created.comments.as_deref(), Some("主生产集群"));

    let fetched = repo.get(created.id).await.unwrap().unwrap();
    assert_eq!(fetched.os.as_deref(), Some("linux"));
    assert_eq!(fetched.type_.as_deref(), Some("prod"));
    assert!(fetched._refs.is_some(), "_refs should be populated");
    let status_ref = fetched._refs.as_ref().unwrap()["status"].clone();
    assert_eq!(status_ref["notice"], "运行中");

    let status2 = insert_protocol_status(&pool, "维护中").await;
    let updated = repo
        .update(
            created.id,
            alioth_service_environment::models::UpdateEnvironmentRequest {
                name: Some("预发环境".to_string()),
                host: Some("staging.example.com".to_string()),
                os: None,
                runtime: None,
                type_: Some("staging".to_string()),
                status: Some(status2),
                services: None,
                uptime: None,
                comments: None,
            },
            uid,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.name, "预发环境");
    assert_eq!(updated.host.as_deref(), Some("staging.example.com"));
    assert_eq!(updated.status, Some(status2));
    assert_eq!(updated.type_.as_deref(), Some("staging"));
    assert!(
        updated._refs.is_some(),
        "_refs should be populated after update"
    );
    let status_ref2 = updated._refs.as_ref().unwrap()["status"].clone();
    assert_eq!(status_ref2["notice"], "维护中");

    repo.delete(created.id, uid).await.unwrap();
    assert!(repo.get(created.id).await.unwrap().is_none());
}

#[tokio::test]
async fn environment_seed_and_stats() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();

    let inserted = alioth_service_environment::seed::seed_environments(&pool)
        .await
        .unwrap();
    assert_eq!(inserted, 5, "seed should insert 5 environments");

    let list = alioth_service_environment::repositories::EnvironmentRepository::from(pool.clone())
        .list(&ListQuery {
            page: 1,
            page_size: 100,
            filter_field: None,
            filter_op: None,
            filter_value: None,
            sort_field: None,
            sort_order: None,
        })
        .await
        .unwrap();

    assert_eq!(list.items.len(), 5);
    for item in &list.items {
        assert!(
            item._refs.is_some(),
            "seeded environments should have _refs"
        );
        let status_notice = item._refs.as_ref().unwrap()["status"]["notice"]
            .as_str()
            .expect("status notice");
        assert!(
            ["healthy", "warning", "unknown"].contains(&status_notice),
            "unexpected status notice: {}",
            status_notice
        );
    }

    // Stats endpoint reads zc_id_even-log; verify it does not error.
    let stats: serde_json::Value = sqlx::query_as(
        r#"SELECT level::text AS level, COUNT(*)::bigint AS cnt
           FROM isahl."zc_id_even-log"
           WHERE deleted_at IS NULL
           GROUP BY level"#,
    )
    .fetch_all(&pool)
    .await
    .map(|rows: Vec<(String, i64)>| {
        serde_json::json!(rows
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>())
    })
    .unwrap();
    assert!(stats.is_object());
}

#[tokio::test]
async fn environment_list_filters_out_language_records() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();

    let seeded = alioth_service_environment::seed::seed_environments(&pool)
        .await
        .expect("seed environments");
    assert_eq!(seeded, 5, "should seed 5 environments");

    let lang_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_prot-env_config"
           (notice, code, settings, created_by_id)
           VALUES ('简体中文', 'lang:zh-CN',
                   jsonb_build_object('locale', '中国大陆', 'enabled', true, 'coverage', 1.0),
                   1)
           RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let repo = alioth_service_environment::repositories::EnvironmentRepository::from(pool.clone());
    let list = repo
        .list(&common::data::ListQuery {
            page: 1,
            page_size: 100,
            filter_field: None,
            filter_op: None,
            filter_value: None,
            sort_field: None,
            sort_order: None,
        })
        .await
        .unwrap();
    assert_eq!(
        list.total, 5,
        "list should only return environment records, not lang records"
    );
    assert_eq!(list.items.len(), 5);

    let fetched = repo.get(lang_id).await.unwrap();
    assert!(
        fetched.is_none(),
        "get on a language record should return None"
    );
}
