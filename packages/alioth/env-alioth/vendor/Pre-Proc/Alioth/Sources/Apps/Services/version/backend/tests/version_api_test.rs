//! alioth-service-version 集成测试
//!
//! 验证 VersionRecord CRUD 与版本链维护（fk_previous）。

use common::testing::{connect_test_db, setup_test_schema_light};
use crud::AliothRepository;

#[tokio::test]
async fn version_crud_and_chain_maintenance() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    // ONLY：zc_id_version 是模型继承族之根（384 表，含 zc_id_law/stan-*）——
    // 不带 ONLY 的清场会连坐删除全族行（模型级种子数据）
    sqlx::query("DELETE FROM ONLY isahl.zc_id_version")
        .execute(&pool)
        .await
        .unwrap();
    let uid: i64 = 1;

    let repo = version::entity::VersionRepository::from(pool.clone());

    let head = repo
        .create(
            version::entity::CreateVersionRequest {
                tpl_id: Some(1),
                tk_version: Some(1),
                notice: None,
                code: None,
                comments: None,
                tk_batch_no: None,
                ck_branch: None,
                fk_previous: None,
            },
            uid,
        )
        .await
        .unwrap();
    assert_eq!(head.tpl_id, Some(1));
    assert_eq!(head.tk_version, Some(1));
    assert_eq!(head.fk_previous, None);

    let next = repo
        .create(
            version::entity::CreateVersionRequest {
                tpl_id: Some(1),
                tk_version: Some(2),
                notice: None,
                code: None,
                comments: None,
                tk_batch_no: None,
                ck_branch: None,
                fk_previous: None,
            },
            uid,
        )
        .await
        .unwrap();
    assert_eq!(next.fk_previous, None);

    // 壳 create 的链维护（Alioth 语义：旧链头指向新版本）
    repo.link_chain(next.id, next.tpl_id).await.unwrap();

    let head_again = repo.get(head.id).await.unwrap().unwrap();
    assert_eq!(head_again.fk_previous, Some(next.id));

    let updated = repo
        .update(
            next.id,
            version::entity::UpdateVersionRequest {
                tpl_id: None,
                tk_version: None,
                comments: Some("rev-5".into()),
                notice: None,
                code: None,
                tk_batch_no: None,
                ck_branch: None,
                fk_previous: None,
            },
            uid,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.comments.as_deref(), Some("rev-5"));

    let list = repo
        .list(&common::data::ListQuery {
            page: 1,
            page_size: 10,
            filter_field: None,
            filter_op: None,
            filter_value: None,
            sort_field: None,
            sort_order: None,
        })
        .await
        .unwrap();
    assert!(list.items.len() >= 2);

    repo.delete(next.id, uid).await.unwrap();
    assert!(repo.get(next.id).await.unwrap().is_none());
}

#[tokio::test]
async fn seed_versions_is_idempotent() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    // ONLY：zc_id_version 是模型继承族之根（384 表，含 zc_id_law/stan-*）——
    // 不带 ONLY 的清场会连坐删除全族行（模型级种子数据）
    sqlx::query("DELETE FROM ONLY isahl.zc_id_version")
        .execute(&pool)
        .await
        .unwrap();
    // 精确清本种子自有行（叶表 zc_id_bom-file）：不动族内模型级行
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_bom-file"
           WHERE (tpl_id, tk_version) IN ((1,1), (1,2), (1,3), (2,1), (2,2))"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let _first = alioth_service_version::seed::seed_versions(&pool)
        .await
        .expect("first seed should succeed");
    let second = alioth_service_version::seed::seed_versions(&pool)
        .await
        .expect("second seed should succeed");

    // 共享测试库可能已有同 (tpl_id, tk_version) 行（幂等即跳过），故断言包含式：
    // 5 组种子版本全部在册 + 二次调用零新增
    assert_eq!(second, 0, "re-seeding should be idempotent");
    let seeded: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_bom-file"
           WHERE deleted_at IS NULL
             AND (tpl_id, tk_version) IN ((1,1), (1,2), (1,3), (2,1), (2,2))"#,
    )
    .fetch_one(&pool)
    .await
    .expect("count seeded versions");
    assert_eq!(seeded, 5, "种子 5 组版本应全部在册");
}
