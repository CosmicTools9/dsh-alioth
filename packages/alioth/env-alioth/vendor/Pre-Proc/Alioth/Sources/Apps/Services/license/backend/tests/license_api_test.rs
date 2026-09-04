//! alioth-service-license 集成测试
//!
//! 验证 license 因子的 License CRUD + 标量引用 (qk_capacity/qk_duration) 处理。

use chrono::{Duration, Utc};
use common::testing::{connect_test_db, setup_test_schema_light};
use crud::AliothRepository;
use rust_decimal::Decimal;

#[tokio::test]
async fn license_crud() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let uid: i64 = 1;

    let repo = alioth_service_license::repositories::LicenseRepository::from(pool.clone());

    let created = repo
        .create(
            alioth_service_license::models::CreateLicenseRequest {
                name: "企业版许可证".to_string(),
                key: Some("LIC-2026-0001".to_string()),
                vendor: Some(1),
                kind: Some(2),
                vendor_name: None,
                kind_name: None,
                seats: Some(100),
                expires: Some(Utc::now() + Duration::days(365)),
                status: Some(3),
            },
            uid,
        )
        .await
        .unwrap();

    assert_eq!(created.name, "企业版许可证");
    assert_eq!(created.key.as_deref(), Some("LIC-2026-0001"));
    assert_eq!(created.vendor, Some(1));
    assert_eq!(created.seats, Some(Decimal::new(100, 0)));
    assert!(created.expires.is_some());

    let fetched = repo.get(created.id).await.unwrap().unwrap();
    assert_eq!(fetched.kind, Some(2));
    assert_eq!(fetched.status, Some(3));

    let updated = repo
        .update(
            created.id,
            alioth_service_license::models::UpdateLicenseRequest {
                name: None,
                key: None,
                vendor: None,
                kind: None,
                vendor_name: None,
                kind_name: None,
                seats: Some(200),
                expires: None,
                status: Some(4),
            },
            uid,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.seats, Some(Decimal::new(200, 0)));
    assert_eq!(updated.status, Some(4));

    repo.delete(created.id, uid).await.unwrap();
    assert!(repo.get(created.id).await.unwrap().is_none());
}

#[tokio::test]
async fn license_create_resolves_vendor_and_kind_names_and_returns_refs() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    alioth_service_license::seed::seed_licenses(&pool)
        .await
        .expect("seed should succeed");

    let repo = alioth_service_license::repositories::LicenseRepository::from(pool.clone());
    let created = repo
        .create(
            alioth_service_license::models::CreateLicenseRequest {
                name: "Name Resolution Test".to_string(),
                key: Some("LIC-RES-001".to_string()),
                vendor: None,
                kind: None,
                vendor_name: Some("JetBrains".to_string()),
                kind_name: Some("永久".to_string()),
                seats: Some(10),
                expires: Some(Utc::now() + Duration::days(30)),
                status: None,
            },
            1,
        )
        .await
        .unwrap();

    assert_eq!(created.name, "Name Resolution Test");
    assert_eq!(created.used, Some(0));
    assert!(created.vendor.is_some());
    assert!(created.kind.is_some());

    let refs = created._refs.expect("_refs should be present");
    assert_eq!(
        refs.get("vendor")
            .and_then(|v| v.get("notice"))
            .and_then(|v| v.as_str()),
        Some("JetBrains")
    );
    assert_eq!(
        refs.get("type")
            .and_then(|v| v.get("notice"))
            .and_then(|v| v.as_str()),
        Some("永久")
    );
}
