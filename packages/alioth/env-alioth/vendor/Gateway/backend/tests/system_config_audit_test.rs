//! 系统配置变更审计集成测试（change add-config-change-audit）
//!
//! 覆盖（单用例串行，fixture 自建自清）：
//! - create → `created_by_id` 回填操作者 + `system_config.create` 审计事件；
//! - update → `updated_by_id` 回填 + `system_config.update` 审计；
//! - delete → 软删 + `system_config.delete` 审计；
//! - 无 `RequestContext` → 401 且零行变更、零审计（fail-closed）。
//!
//! 直调 crate handler（actix 提取器手动构造），不依赖完整 app 装配。

use actix_web::{web, HttpMessage, HttpRequest};
use common::context::RequestContext;
use sqlx::{PgPool, Row};
use system_config::handlers::{create_config, delete_config, update_config};
use system_config::{CreateSystemConfigRequest, SystemConfigService, UpdateSystemConfigRequest};

use ::common::testing::connect_test_db;
use alioth_gateway::system_config_repo::SystemConfigRepo;

const CODE: &str = "test:audit-config";
const CODE_DENIED: &str = "test:audit-denied";
const TEST_USER_ID: i64 = 9_900_421;

fn authed_req(username: &str) -> HttpRequest {
    let req = actix_web::test::TestRequest::default().to_http_request();
    req.extensions_mut().insert(RequestContext::with_username(
        TEST_USER_ID,
        format!("{username}@test.local"),
        username,
    ));
    req
}

async fn cleanup(pool: &PgPool) {
    sqlx::query(r#"DELETE FROM isahl."zc_id_prot-llm_config" WHERE code LIKE 'test:audit-%'"#)
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        r#"DELETE FROM isahl_audit.audit_events
           WHERE user_id = $1 AND operation LIKE 'system_config.%'"#,
    )
    .bind(TEST_USER_ID)
    .execute(pool)
    .await
    .ok();
}

async fn assert_audit(pool: &PgPool, object_path: &str, operation: &str) {
    let n: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl_audit.audit_events
           WHERE user_id = $1 AND operation = $2 AND object_path = $3"#,
    )
    .bind(TEST_USER_ID)
    .bind(operation)
    .bind(object_path)
    .fetch_one(pool)
    .await
    .expect("audit count");
    assert!(n >= 1, "审计事件缺失：{operation} {object_path}");
}

async fn count_audit(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl_audit.audit_events
           WHERE user_id = $1 AND operation LIKE 'system_config.%'"#,
    )
    .bind(TEST_USER_ID)
    .fetch_one(pool)
    .await
    .expect("audit count")
}

#[tokio::test]
async fn config_change_audit_end_to_end() {
    let pool = connect_test_db().await;
    cleanup(&pool).await;

    let service = web::Data::new(SystemConfigService::new(SystemConfigRepo::new(
        pool.clone(),
    )));
    let pool_data = web::Data::new(pool.clone());

    // ── 1. create：created_by_id 回填 + system_config.create 审计 ──
    let resp = create_config(
        service.clone(),
        pool_data.clone(),
        authed_req("audit-admin"),
        web::Json(CreateSystemConfigRequest {
            notice: Some("审计夹具".to_string()),
            code: Some(CODE.to_string()),
            _f_: Some("llm".to_string()),
            _t_: Some("deepseek".to_string()),
            comments: None,
            credentials: None,
            settings: Some(serde_json::json!({"enabled": true, "is_default": true})),
            enabled: true,
            is_default: true,
            domain_: Some("global".to_string()),
        }),
    )
    .await
    .expect("create must succeed");
    assert_eq!(resp.status(), 200);

    let row = sqlx::query(
        r#"SELECT id, created_by_id FROM isahl."zc_id_prot-llm_config"
           WHERE code = $1 AND deleted_at IS NULL"#,
    )
    .bind(CODE)
    .fetch_one(&pool)
    .await
    .expect("created row exists");
    let id: i64 = row.get("id");
    let created_by: Option<i64> = row.get("created_by_id");
    assert_eq!(
        created_by,
        Some(TEST_USER_ID),
        "created_by_id MUST 回填操作者"
    );
    assert_audit(
        &pool,
        &format!("system_config/llm/{id}"),
        "system_config.create",
    )
    .await;

    // ── 2. update：updated_by_id 回填 + system_config.update 审计 ──
    let resp = update_config(
        service.clone(),
        pool_data.clone(),
        authed_req("audit-admin"),
        web::Path::from(id),
        web::Json(UpdateSystemConfigRequest {
            notice: Some("审计夹具-改".to_string()),
            code: None,
            _f_: Some("llm".to_string()),
            _t_: Some("deepseek".to_string()),
            comments: None,
            credentials: None,
            settings: None,
            enabled: Some(true),
            is_default: Some(true),
            domain_: None,
        }),
    )
    .await
    .expect("update must succeed");
    assert_eq!(resp.status(), 200);

    let updated_by: Option<i64> = sqlx::query_scalar(
        r#"SELECT updated_by_id FROM isahl."zc_id_prot-llm_config" WHERE id = $1"#,
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("row after update");
    assert_eq!(
        updated_by,
        Some(TEST_USER_ID),
        "updated_by_id MUST 回填操作者"
    );
    assert_audit(
        &pool,
        &format!("system_config/llm/{id}"),
        "system_config.update",
    )
    .await;

    // ── 3. delete：软删 + system_config.delete 审计 ──
    let resp = delete_config(
        service.clone(),
        pool_data.clone(),
        authed_req("audit-admin"),
        web::Path::from(id),
    )
    .await
    .expect("delete must succeed");
    assert_eq!(resp.status(), 200);

    let deleted_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar(r#"SELECT deleted_at FROM isahl."zc_id_prot-llm_config" WHERE id = $1"#)
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("row after delete");
    assert!(deleted_at.is_some(), "软删 MUST 落 deleted_at");
    assert_audit(
        &pool,
        &format!("system_config/{id}"),
        "system_config.delete",
    )
    .await;

    // ── 4. 无上下文：401 且零变更零审计（fail-closed）──
    let audit_before = count_audit(&pool).await;
    let bare = actix_web::test::TestRequest::default().to_http_request();
    let err = create_config(
        service.clone(),
        pool_data.clone(),
        bare,
        web::Json(CreateSystemConfigRequest {
            notice: Some("被拒".to_string()),
            code: Some(CODE_DENIED.to_string()),
            _f_: Some("llm".to_string()),
            _t_: Some("deepseek".to_string()),
            comments: None,
            credentials: None,
            settings: None,
            enabled: true,
            is_default: false,
            domain_: None,
        }),
    )
    .await
    .expect_err("无上下文 MUST 被拒");
    assert!(
        matches!(err, common::AliothError::Unauthorized(_)),
        "错误 MUST 为 Unauthorized：{err:?}"
    );
    let denied_rows: i64 =
        sqlx::query_scalar(r#"SELECT COUNT(*) FROM isahl."zc_id_prot-llm_config" WHERE code = $1"#)
            .bind(CODE_DENIED)
            .fetch_one(&pool)
            .await
            .expect("denied count");
    assert_eq!(denied_rows, 0, "被拒请求 MUST NOT 落行");
    assert_eq!(
        count_audit(&pool).await,
        audit_before,
        "被拒请求 MUST NOT 落审计"
    );

    cleanup(&pool).await;
}
