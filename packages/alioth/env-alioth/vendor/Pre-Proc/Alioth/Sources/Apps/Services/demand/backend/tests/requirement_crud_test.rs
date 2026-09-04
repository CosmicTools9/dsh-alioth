//! Requirement CRUD 集成测试（TEST_INFRASTRUCTURE.md：`#[tokio::test]` + `connect_test_db()`，
//! 禁止 sqlx::test 宏）。
//!
//! 覆盖：CRUD 闭环、`_refs` 名称解析（category/place）、类目单值替换语义。
//!
//! 依赖测试库预置（`scripts/reset-db.sh --test`）：isahl schema（zc_id_event /
//! zc_id_lifecycle_r_category / zc_id_category / zc_id_lifecycle）。

mod common;

use ::common::testing::connect_test_db;
use alioth_service_demand::models::{CreateRequirementRequest, UpdateRequirementRequest};
use alioth_service_demand::repositories::requirement_repository::RequirementRepository;
use common::{setup_test_schema, test_code};
use crud::AliothRepository;

/// 建一个测试类目，返回 (id, name)
async fn insert_category(pool: &sqlx::PgPool, name: &str) -> (i64, String) {
    let id: i64 =
        sqlx::query_scalar(r#"INSERT INTO isahl.zc_id_category (notice) VALUES ($1) RETURNING id"#)
            .bind(name)
            .fetch_one(pool)
            .await
            .expect("insert category");
    (id, name.to_string())
}

/// 建一个测试场所（zc_id_lifecycle 叶表），返回 (id, name)
async fn insert_place(pool: &sqlx::PgPool, name: &str) -> (i64, String) {
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_lifecycle (notice) VALUES ($1) RETURNING id"#,
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .expect("insert place");
    (id, name.to_string())
}

#[tokio::test]
async fn requirement_crud_with_refs() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.expect("setup failed");
    let repo = RequirementRepository::new(pool.clone());
    let user_id = 1i64;
    let code = test_code("REQ");

    let (cat_id, cat_name) = insert_category(&pool, "功能需求").await;
    let (place_id, place_name) = insert_place(&pool, "项目A").await;

    // ── create：code/name/类目/场所全部落库 ──
    let created = repo
        .create(
            CreateRequirementRequest {
                code: code.clone(),
                name: "支持批量导入需求条目".to_string(),
                comments: Some("详细描述".to_string()),
                category: Some(cat_id),
                place: Some(place_id),
            },
            user_id,
        )
        .await
        .expect("create requirement");
    assert_eq!(created.name, "支持批量导入需求条目");
    assert_eq!(created.code.as_deref(), Some(code.as_str()));
    assert_eq!(created.category, Some(cat_id));
    assert_eq!(created.place, Some(place_id));

    // ── get：_refs 解析名称（禁 raw ID）──
    let got = repo.get(created.id).await.expect("get").expect("exists");
    let refs = got._refs.expect("_refs present");
    assert_eq!(
        refs.pointer("/category/notice").and_then(|v| v.as_str()),
        Some(cat_name.as_str()),
        "_refs.category.notice 应解析类目名称"
    );
    assert_eq!(
        refs.pointer("/place/notice").and_then(|v| v.as_str()),
        Some(place_name.as_str()),
        "_refs.place.notice 应解析场所名称"
    );

    // ── list：分页包含新记录 ──
    let list = repo
        .list(&crud::ListQuery {
            page: 1,
            page_size: 10,
            filter_field: None,
            filter_op: None,
            filter_value: None,
            sort_field: None,
            sort_order: None,
        })
        .await
        .expect("list");
    assert!(list.total >= 1);
    let found = list
        .items
        .iter()
        .find(|r| r.id == created.id)
        .expect("in list");
    assert!(found._refs.is_some(), "列表项应含 _refs");

    // ── update：改 name/类目（单值替换）──
    let (cat2_id, cat2_name) = insert_category(&pool, "非功能需求").await;
    let updated = repo
        .update(
            created.id,
            UpdateRequirementRequest {
                code: None,
                name: Some("支持批量导入需求条目（修订）".to_string()),
                comments: None,
                category: Some(cat2_id),
                place: None,
            },
            user_id,
        )
        .await
        .expect("update")
        .expect("exists");
    assert_eq!(updated.name, "支持批量导入需求条目（修订）");
    assert_eq!(updated.category, Some(cat2_id), "类目应单值替换");
    let refs2 = updated._refs.expect("_refs present");
    assert_eq!(
        refs2.pointer("/category/notice").and_then(|v| v.as_str()),
        Some(cat2_name.as_str()),
        "更新后 _refs.category 应指向新类目名称"
    );

    // 旧类目关联应已软删（单值语义：桥接表仅一行活跃）
    let active_rows: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl.zc_id_lifecycle_r_category
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(created.id)
    .fetch_one(&pool)
    .await
    .expect("count bridge");
    assert_eq!(active_rows, 1, "类目单值：桥接表只保留一行活跃关联");

    // ── delete：软删除后 get 返回 None ──
    repo.delete(created.id, user_id).await.expect("delete");
    assert!(repo
        .get(created.id)
        .await
        .expect("get after delete")
        .is_none());
}

#[tokio::test]
async fn requirement_dimensions_sources() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.expect("setup failed");
    let repo = RequirementRepository::new(pool.clone());

    let (cat_id, _) = insert_category(&pool, "技术需求").await;
    let (place_id, _) = insert_place(&pool, "子系统B").await;

    let categories = repo.list_categories().await.expect("categories");
    let places = repo.list_places().await.expect("places");
    assert!(
        categories
            .iter()
            .any(|c| c.id == cat_id && c.name == "技术需求"),
        "类目维度应包含新插入项"
    );
    assert!(
        places
            .iter()
            .any(|p| p.id == place_id && p.name == "子系统B"),
        "场所维度应包含新插入项"
    );
}
