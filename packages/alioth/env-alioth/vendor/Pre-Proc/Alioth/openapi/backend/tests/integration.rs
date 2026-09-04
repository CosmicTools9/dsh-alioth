//! OpenAPI 数据服务产品 Service 集成测试
//!
//! 验证 isahl 4 张 openapi 表的 CRUD（经 GenericRepository/crud_routes 语义）：
//!   1. 配置表（zc_id_prot-openapi_config）：create → get → soft delete
//!   2. 产品表（sales/purchase/made）：create → list
//!
//! 依赖真实测试库（aliothstudio_test）。测试数据自建自清。

use crud::repository::AliothRepository;
use crud::{ListQuery, PaginatedResponse};

use ::common::testing::connect_test_db;

use alioth_service_openapi::models::*;
use alioth_service_openapi::repositories::*;

const TEST_USER: i64 = 1002;

/// CRUD-001: 对接配置 create → get → update → soft delete
#[tokio::test]
async fn openapi_crud_001_config_lifecycle() {
    let pool = connect_test_db().await;
    let repo = OpenApiConfigRepository::from(pool.clone());
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    // create
    let created = repo
        .create(
            CreateOpenApiConfigRequest {
                name: format!("test-config-{}", suffix),
                code: Some(format!("CFG-{}", suffix)),
                comments: Some("integration test".into()),
                settings: Some(serde_json::json!({"enabled": true, "rps": 10})),
                enc_fields: Some(serde_json::json!({"api_key_enc": "enc:test"})),
            },
            TEST_USER,
        )
        .await
        .expect("create config");
    assert!(created.id > 0);
    assert_eq!(created.name, format!("test-config-{}", suffix));
    assert_eq!(
        created.settings.as_ref().unwrap()["rps"],
        serde_json::json!(10)
    );

    // get
    let got = repo.get(created.id).await.expect("get config");
    assert!(got.is_some());
    assert_eq!(
        got.unwrap().code.as_deref(),
        Some(format!("CFG-{}", suffix).as_str())
    );

    // update
    let updated = repo
        .update(
            created.id,
            UpdateOpenApiConfigRequest {
                name: Some(format!("test-config-updated-{}", suffix)),
                code: None,
                comments: None,
                settings: Some(serde_json::json!({"enabled": false})),
                enc_fields: None,
            },
            TEST_USER,
        )
        .await
        .expect("update config");
    assert!(updated.is_some());
    assert_eq!(
        updated.unwrap().name,
        format!("test-config-updated-{}", suffix)
    );

    // soft delete
    repo.delete(created.id, TEST_USER)
        .await
        .expect("delete config");
    let gone = repo.get(created.id).await.expect("get after delete");
    assert!(gone.is_none(), "soft delete 后不应查到");
}

/// CRUD-002: 销售侧产品 create → list（含 settings jsonb）
#[tokio::test]
async fn openapi_crud_002_sales_product_crud() {
    let pool = connect_test_db().await;
    let repo = OpenApiSalesRepository::from(pool.clone());
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    let created = repo
        .create(
            CreateOpenApiSalesRequest {
                name: format!("sales-{}", suffix),
                code: Some(format!("S-{}", suffix)),
                comments: Some("sales product".into()),
                projection: Some("zc_id_contract".into()),
                tpl_id: None,
                p_number: None,
                fk_subj_demand: None,
                fk_subj_provider: None,
                qk_price: None,
                fk_process: None,
                sk_currency: None,
                qk_size: None,
            },
            TEST_USER,
        )
        .await
        .expect("create sales");
    assert!(created.id > 0);
    assert_eq!(created.projection.as_deref(), Some("zc_id_contract"));

    // list 含新记录
    let page: PaginatedResponse<OpenApiSales> = repo
        .list(&ListQuery {
            page: 1,
            page_size: 20,
            filter_field: None,
            filter_op: None,
            filter_value: None,
            sort_field: None,
            sort_order: None,
        })
        .await
        .expect("list sales");
    assert!(page.total > 0);

    repo.delete(created.id, TEST_USER)
        .await
        .expect("delete sales");
}

/// CRUD-003: 采购/制造产品表存在且可写
#[tokio::test]
async fn openapi_crud_003_purchase_and_made() {
    let pool = connect_test_db().await;
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    // purchase
    let prepo = OpenApiPurchaseRepository::from(pool.clone());
    let p = prepo
        .create(
            CreateOpenApiPurchaseRequest {
                name: format!("purchase-{}", suffix),
                code: None,
                comments: None,
                projection: None,
                tpl_id: None,
                p_number: None,
                fk_subj_demand: None,
                fk_subj_provider: None,
                qk_price: None,
                fk_process: None,
                sk_currency: None,
                qk_size: None,
            },
            TEST_USER,
        )
        .await
        .expect("create purchase");
    assert!(p.id > 0);

    // made
    let mrepo = OpenApiMadeRepository::from(pool.clone());
    let m = mrepo
        .create(
            CreateOpenApiMadeRequest {
                name: format!("made-{}", suffix),
                code: None,
                comments: None,
                projection: None,
                tpl_id: None,
                p_number: None,
                fk_subj_demand: None,
                fk_subj_provider: None,
                qk_price: None,
                fk_process: None,
                sk_currency: None,
                qk_size: None,
            },
            TEST_USER,
        )
        .await
        .expect("create made");
    assert!(m.id > 0);

    prepo
        .delete(p.id, TEST_USER)
        .await
        .expect("delete purchase");
    mrepo.delete(m.id, TEST_USER).await.expect("delete made");
}
