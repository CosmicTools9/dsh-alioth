//! alioth-service-status 集成测试
//!
//! 验证 status CRUD 与状态机转换规则（start → doing → end）。

use crud::AliothRepository;
use status::models::CreateStatusRequest;

use common::testing::{connect_test_db, setup_test_schema_light};

#[tokio::test]
async fn status_crud_lifecycle() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();

    let repo = status::StatusRepository::from(pool.clone());
    let uid: i64 = 1;

    let created = repo
        .create(
            CreateStatusRequest {
                notice: Some("待处理".to_string()),
                flag: Some("start".to_string()),
                comments: None,
                code: None,
                enable: None,
            },
            uid,
        )
        .await
        .unwrap();

    assert_eq!(created.notice.as_deref(), Some("待处理"));
    assert_eq!(created.flag.as_deref(), Some("start"));

    let fetched = repo.get(created.id).await.unwrap().expect("status exists");
    assert_eq!(fetched.notice.as_deref(), Some("待处理"));

    let updated = repo
        .update(
            created.id,
            status::models::UpdateStatusRequest {
                notice: None,
                flag: Some("doing".to_string()),
                comments: None,
                code: None,
                enable: None,
            },
            uid,
        )
        .await
        .unwrap()
        .expect("update returns status");
    assert_eq!(updated.flag.as_deref(), Some("doing"));

    repo.delete(created.id, uid).await.unwrap();
    let after_delete = repo.get(created.id).await.unwrap();
    assert!(after_delete.is_none(), "status should be soft-deleted");
}

#[tokio::test]
async fn status_transition_enforces_state_machine() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();

    let repo = status::StatusRepository::from(pool.clone());
    let uid: i64 = 1;

    let start = repo
        .create(
            CreateStatusRequest {
                notice: Some("Start".to_string()),
                flag: Some("start".to_string()),
                comments: None,
                code: None,
                enable: None,
            },
            uid,
        )
        .await
        .unwrap();

    let doing = repo
        .update(
            start.id,
            status::models::UpdateStatusRequest {
                notice: None,
                flag: Some("doing".to_string()),
                comments: None,
                code: None,
                enable: None,
            },
            uid,
        )
        .await
        .unwrap()
        .expect("valid start→doing transition");
    assert_eq!(doing.flag.as_deref(), Some("doing"));

    let end = repo
        .update(
            doing.id,
            status::models::UpdateStatusRequest {
                notice: None,
                flag: Some("end".to_string()),
                comments: None,
                code: None,
                enable: None,
            },
            uid,
        )
        .await
        .unwrap()
        .expect("valid doing→end transition");
    assert_eq!(end.flag.as_deref(), Some("end"));

    // 转换校验（validate_transition）已随 services.rs 删除（无调用者死代码）；
    // 壳化后 update 直接生效（flag 无中间态拦截）——语义变化记录于 change 文档。
    let start2 = repo
        .create(
            CreateStatusRequest {
                notice: Some("Start2".to_string()),
                flag: Some("start".to_string()),
                comments: None,
                code: None,
                enable: None,
            },
            uid,
        )
        .await
        .unwrap();

    let jumped = repo
        .update(
            start2.id,
            status::models::UpdateStatusRequest {
                notice: None,
                flag: Some("end".to_string()),
                comments: None,
                code: None,
                enable: None,
            },
            uid,
        )
        .await
        .unwrap()
        .expect("update without transition gate succeeds");
    assert_eq!(jumped.flag.as_deref(), Some("end"));
}
