//! InboundOrder CRUD 集成测试（#[tokio::test] + connect_test_db()，禁 sqlx::test 宏）
mod common;

use ::common::testing::connect_test_db;
use alioth_service_inbound_order::models::{CreateInboundOrderRequest, UpdateInboundOrderRequest};
use alioth_service_inbound_order::repositories::inbound_order_repository::InboundOrderRepository;
use common::{setup_test_schema, test_code};
use crud::AliothRepository;

#[tokio::test]
async fn inbound_order_crud_roundtrip() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.expect("setup failed");
    let repo = InboundOrderRepository::new(pool.clone());
    let user_id = 1i64;
    let code = test_code("INBOUND_ORDER");

    // create
    let created = repo
        .create(
            CreateInboundOrderRequest {
                notice: Some(code.clone()),
                ..Default::default()
            },
            user_id,
        )
        .await
        .expect("create failed");
    assert_eq!(created.notice.as_deref(), Some(code.as_str()));
    let id = created.id;

    // get
    let fetched = repo.get(id).await.expect("get failed").expect("not found");
    assert_eq!(fetched.id, id);

    // update
    let updated_code = format!("{}-UPD", code);
    let updated = repo
        .update(
            id,
            UpdateInboundOrderRequest {
                notice: Some(updated_code.clone()),
                ..Default::default()
            },
            user_id,
        )
        .await
        .expect("update failed")
        .expect("update not found");
    assert_eq!(updated.notice.as_deref(), Some(updated_code.as_str()));

    // list（含本行）
    let page = repo
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
        .expect("list failed");
    assert!(page.items.iter().any(|i| i.id == id));

    // delete（软删）
    repo.delete(id, user_id).await.expect("delete failed");
    assert!(repo
        .get(id)
        .await
        .expect("get after delete failed")
        .is_none());
}
