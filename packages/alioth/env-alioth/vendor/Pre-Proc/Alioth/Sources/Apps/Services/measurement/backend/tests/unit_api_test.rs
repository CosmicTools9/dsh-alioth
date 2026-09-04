//! alioth-service-measurement 集成测试
//!
//! 验证 measurement 因子的单位、单位换算率、汇率 CRUD 及 detail multiplier 语义。

use common::testing::{connect_test_db, setup_test_schema_light};
use crud::AliothRepository;

/// 清理计量表，消除 WZ/Alioth 共用测试库的顺序耦合（seed 幂等断言依赖空表起点）
async fn setup_clean_pool() -> sqlx::PgPool {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    sqlx::query("DELETE FROM isahl.zc_id_unit*")
        .execute(&pool)
        .await
        .expect("清理测试库计量表失败");
    sqlx::query("DELETE FROM isahl.zc_id_rate*")
        .execute(&pool)
        .await
        .expect("清理测试库计量表失败");
    sqlx::query("DELETE FROM isahl.\"zc_id_scal-price\"")
        .execute(&pool)
        .await
        .expect("清理测试库计量表失败");
    pool
}

#[tokio::test]
async fn unit_list_returns_business_semantic_fields() {
    let pool = setup_clean_pool().await;
    let uid: i64 = 1;

    let repo = measurement::biz_repositories::MeasurementUnitRepository::from(pool.clone());
    let unit = repo
        .create(
            measurement::biz_models::CreateMeasurementUnitRequest {
                name: "米".to_string(),
                code: Some("m".to_string()),
                symbol: Some("m".to_string()),

                system: Some("公制".to_string()),
                dimension_key: Some("distance".to_string()),
                base: Some(true),
                t_color_: None,
            },
            uid,
        )
        .await
        .unwrap();

    assert_eq!(unit.name, "米");
    assert_eq!(unit.code.as_deref(), Some("m"));
    assert_eq!(unit.dimension.as_deref(), Some("distance"));
    assert_eq!(unit.system.as_deref(), Some("公制"));
    assert_eq!(unit.base, Some(true));
}

#[tokio::test]
async fn unit_detail_returns_multiplier_via_rate_table() {
    let pool = setup_clean_pool().await;
    let uid: i64 = 1;

    let unit_repo = measurement::biz_repositories::MeasurementUnitRepository::from(pool.clone());
    let rate_repo = measurement::biz_repositories::UnitConversionRateRepository::from(pool.clone());

    let base_unit = unit_repo
        .create(
            measurement::biz_models::CreateMeasurementUnitRequest {
                name: "米".to_string(),
                code: Some("m".to_string()),
                symbol: Some("m".to_string()),

                system: Some("公制".to_string()),
                dimension_key: Some("distance".to_string()),
                base: Some(true),
                t_color_: None,
            },
            uid,
        )
        .await
        .unwrap();

    let cm_unit = unit_repo
        .create(
            measurement::biz_models::CreateMeasurementUnitRequest {
                name: "厘米".to_string(),
                code: Some("cm".to_string()),
                symbol: Some("cm".to_string()),

                system: Some("公制".to_string()),
                dimension_key: Some("distance".to_string()),
                base: Some(false),
                t_color_: None,
            },
            uid,
        )
        .await
        .unwrap();

    let _rate = rate_repo
        .create(
            measurement::biz_models::CreateUnitConversionRateRequest {
                name: "厘米到米".to_string(),
                left: Some(cm_unit.id),
                right: Some(base_unit.id),
                multiply: Some("0.01".parse().unwrap()),
                division: None,
                precision_: None,
                intrinsic: None,
                dimension_key: Some("distance".to_string()),
            },
            uid,
        )
        .await
        .unwrap();

    let svc = measurement::service::MeasurementService::new(pool.clone());
    let detail = svc.to_unit_detail(cm_unit).await.unwrap();

    assert_eq!(detail.multiplier, Some("0.01".parse().unwrap()));
}

#[tokio::test]
async fn unit_list_returns_multiplier() {
    let pool = setup_clean_pool().await;
    let uid: i64 = 1;

    let unit_repo = measurement::biz_repositories::MeasurementUnitRepository::from(pool.clone());
    let rate_repo = measurement::biz_repositories::UnitConversionRateRepository::from(pool.clone());

    let base_unit = unit_repo
        .create(
            measurement::biz_models::CreateMeasurementUnitRequest {
                name: "米".to_string(),
                code: Some("m".to_string()),
                symbol: Some("m".to_string()),

                system: Some("公制".to_string()),
                dimension_key: Some("distance".to_string()),
                base: Some(true),
                t_color_: None,
            },
            uid,
        )
        .await
        .unwrap();

    let cm_unit = unit_repo
        .create(
            measurement::biz_models::CreateMeasurementUnitRequest {
                name: "厘米".to_string(),
                code: Some("cm".to_string()),
                symbol: Some("cm".to_string()),

                system: Some("公制".to_string()),
                dimension_key: Some("distance".to_string()),
                base: Some(false),
                t_color_: None,
            },
            uid,
        )
        .await
        .unwrap();

    let _rate = rate_repo
        .create(
            measurement::biz_models::CreateUnitConversionRateRequest {
                name: "厘米到米".to_string(),
                left: Some(cm_unit.id),
                right: Some(base_unit.id),
                multiply: Some("0.01".parse().unwrap()),
                division: None,
                precision_: None,
                intrinsic: None,
                dimension_key: Some("distance".to_string()),
            },
            uid,
        )
        .await
        .unwrap();

    let svc = measurement::service::MeasurementService::new(pool.clone());
    let list = svc
        .list_units(
            &common::data::ListQuery {
                page: 1,
                page_size: 100,
                filter_field: None,
                filter_op: None,
                filter_value: None,
                sort_field: None,
                sort_order: None,
            },
            None,
            None,
        )
        .await
        .unwrap();

    assert!(!list.items.is_empty(), "list should contain seeded units");
    let cm_item = list
        .items
        .iter()
        .find(|u| u.code == "cm")
        .expect("cm unit in list");
    assert_eq!(
        cm_item.multiplier,
        "0.01".parse::<rust_decimal::Decimal>().unwrap()
    );
    assert!(!cm_item.base);

    let m_item = list
        .items
        .iter()
        .find(|u| u.code == "m")
        .expect("m unit in list");
    assert!(m_item.base);
    assert_eq!(m_item.multiplier, rust_decimal::Decimal::ONE);
}

#[tokio::test]
async fn unit_detail_without_rate_has_no_multiplier() {
    let pool = setup_clean_pool().await;
    let uid: i64 = 1;

    let unit_repo = measurement::biz_repositories::MeasurementUnitRepository::from(pool.clone());

    let weight_unit = unit_repo
        .create(
            measurement::biz_models::CreateMeasurementUnitRequest {
                name: "千克".to_string(),
                code: Some("kg".to_string()),
                symbol: Some("kg".to_string()),

                system: Some("公制".to_string()),
                dimension_key: Some("weight".to_string()),
                base: Some(true),
                t_color_: None,
            },
            uid,
        )
        .await
        .unwrap();

    let svc = measurement::service::MeasurementService::new(pool.clone());
    let detail = svc.to_unit_detail(weight_unit).await.unwrap();

    assert_eq!(detail.multiplier, None);
}

#[tokio::test]
async fn exchange_rate_create_and_get() {
    let pool = setup_clean_pool().await;
    let uid: i64 = 1;

    let repo = measurement::biz_repositories::ExchangeRateRepository::from(pool.clone());
    let created = repo
        .create(
            measurement::biz_models::CreateExchangeRateRequest {
                name: "USD/CNY".to_string(),
                left_currency: Some(1),
                right_currency: Some(2),
                rate: Some("7.2345".parse().unwrap()),
                ask_price: Some("7.2456".parse().unwrap()),
                source: Some("xe".to_string()),
                date: Some(chrono::Utc::now()),
            },
            uid,
        )
        .await
        .unwrap();

    assert_eq!(created.name, "USD/CNY");
    assert_eq!(created.rate, Some("7.2345".parse().unwrap()));
    assert_eq!(created.ask_price, Some("7.2456".parse().unwrap()));

    let fetched = repo.get(created.id).await.unwrap().unwrap();
    assert_eq!(fetched.source.as_deref(), Some("xe"));
}

#[tokio::test]
async fn exchange_rate_create_with_currency_codes_resolves_ids_and_returns_refs() {
    let pool = setup_clean_pool().await;
    let uid: i64 = 1;

    // 先注入货币种子
    alioth_service_measurement::seed::seed_currencies_and_rates(&pool)
        .await
        .unwrap();

    let repo = measurement::biz_repositories::ExchangeRateRepository::from(pool.clone());
    // 字符串货币代码解析已迁移到壳 handler 层（resolve_currency_id）；
    // repository 层契约为 i64 引用——测试先解析货币 id 再创建。
    let usd_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_unit-currency" WHERE code = 'USD' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    let cny_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_unit-currency" WHERE code = 'CNY' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    let usd_id = usd_id.expect("USD 货币种子存在");
    let cny_id = cny_id.expect("CNY 货币种子存在");

    let created = repo
        .create(
            measurement::biz_models::CreateExchangeRateRequest {
                name: "USD/CNY".to_string(),
                left_currency: Some(usd_id),
                right_currency: Some(cny_id),
                rate: Some("7.2345".parse().unwrap()),
                ask_price: Some("7.2456".parse().unwrap()),
                source: Some("xe".to_string()),
                date: Some(chrono::Utc::now()),
            },
            uid,
        )
        .await
        .unwrap();

    assert_eq!(created.name, "USD/CNY");
    assert_eq!(created.left_currency, Some(usd_id));
    assert_eq!(created.right_currency, Some(cny_id));
    assert_eq!(created.rate, Some("7.2345".parse().unwrap()));

    let fetched = repo.get(created.id).await.unwrap().unwrap();
    assert_eq!(fetched.left_currency, Some(usd_id));
    assert_eq!(fetched.right_currency, Some(cny_id));
}

#[tokio::test]
async fn conversion_rate_create_and_get() {
    let pool = setup_clean_pool().await;
    let uid: i64 = 1;

    let repo = measurement::biz_repositories::UnitConversionRateRepository::from(pool.clone());
    let created = repo
        .create(
            measurement::biz_models::CreateUnitConversionRateRequest {
                name: "米到千米".to_string(),
                left: Some(1),
                right: Some(2),
                multiply: Some("0.001".parse().unwrap()),
                division: None,
                precision_: Some(6),
                intrinsic: Some(false),
                dimension_key: Some("distance".to_string()),
            },
            uid,
        )
        .await
        .unwrap();

    assert_eq!(created.name, "米到千米");
    assert_eq!(created.multiply, Some("0.001".parse().unwrap()));
    assert_eq!(created.dimension.as_deref(), Some("distance"));

    let fetched = repo.get(created.id).await.unwrap().unwrap();
    assert_eq!(fetched.left, Some(1));
}
