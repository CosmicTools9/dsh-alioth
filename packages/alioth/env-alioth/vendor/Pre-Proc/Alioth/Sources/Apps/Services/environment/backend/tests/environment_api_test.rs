//! alioth-service-environment 集成测试
//!
//! 验证 environment 因子的 Environment CRUD + settings JSONB 序列化。

use common::data::ListQuery;
use common::testing::{connect_test_db, setup_test_schema_light};
use crud::AliothRepository;
use sqlx::PgPool;

async fn insert_protocol_status(pool: &PgPool, notice: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_stus-protocol" (notice, created_by_id, flag) VALUES ($1, 1, 'doing') RETURNING id"#,
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

    alioth_service_environment::seed::reset_seed_environments(&pool)
        .await
        .unwrap();
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

    // 只断言本种子 5 行（共享库他源行的状态词表不在本契约内）；
    // 状态词 = 种子 status_map 的 notice 键（healthy/warning/unknown）
    let seed_hosts = [
        "localhost:3000",
        "ci.alioth.dev",
        "staging.alioth.dev",
        "prod.alioth.dev",
        "dr.alioth.dev",
    ];
    let seed_rows: Vec<_> = list
        .items
        .iter()
        .filter(|e| seed_hosts.contains(&e.host.as_deref().unwrap_or("")))
        .collect();
    assert_eq!(seed_rows.len(), 5, "种子 5 行应在列表中");
    for item in seed_rows {
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
        r#"SELECT COALESCE(cc.code, 'uncategorized')::text AS level, COUNT(*)::bigint AS cnt
           FROM isahl."zc_id_even-log" e
           LEFT JOIN isahl."zc_id_cate-log" cc ON cc.id = e.ck_category AND cc.deleted_at IS NULL
           WHERE e.deleted_at IS NULL
           GROUP BY 1"#,
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

    alioth_service_environment::seed::reset_seed_environments(&pool)
        .await
        .expect("reset seed environments");
    let seeded = alioth_service_environment::seed::seed_environments(&pool)
        .await
        .expect("seed environments");
    assert_eq!(seeded, 5, "should seed 5 environments");

    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JE", "GEC", "↑_DA"))
            .await
            .unwrap();
    let lang_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_prot-env_config"
           (notice, code, settings, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES ('简体中文', 'lang:zh-CN',
                   jsonb_build_object('locale', '中国大陆', 'enabled', true, 'coverage', 1.0),
                   1, '实现', '实例', $1, $2, $3)
           RETURNING id"#,
    )
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
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
    // 共享测试库他源行会使 total > 5——契约断言：lang 行不出现 + 种子 5 行齐
    let items = list.items;
    assert!(
        items
            .iter()
            .all(|e| !e.host.as_deref().unwrap_or("").starts_with("lang:")),
        "lang 记录不应出现在环境列表: {:?}",
        items.iter().map(|e| &e.host).collect::<Vec<_>>()
    );
    let seeded_hosts = [
        "localhost:3000",
        "ci.alioth.dev",
        "staging.alioth.dev",
        "prod.alioth.dev",
        "dr.alioth.dev",
    ];
    for h in seeded_hosts {
        assert!(
            items.iter().any(|e| e.host.as_deref() == Some(h)),
            "种子环境 {h} 应在列表中"
        );
    }

    let fetched = repo.get(lang_id).await.unwrap();
    assert!(
        fetched.is_none(),
        "get on a language record should return None"
    );
}
