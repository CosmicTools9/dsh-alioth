//! 产品投影（L4 接线）集成测试（change `wire-openapi-product-projection`）
//!
//! 三组：
//! 1. 投影文法/判定（无 DB）：段边界、`*` 全量、空集=全量、空串忽略、文档裁剪。
//! 2. 产品表加载（DB）：插入产品行 ⇒ 投影集生效；清除后回全量（自建自清）。
//! 3. PEP 授权面（DB）：投影集非空时投影外业务端点 403 `NOT_IN_PRODUCT_SET`，
//!    投影内放行（scope 均满足 ⇒ 差异只能来自投影判定）。
//!
//! 依赖真实测试库（aliothstudio_test）。测试数据自建自清。

use actix_web::{test as awtest, web, App, HttpResponse};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::json;
use sqlx::PgPool;

use ::common::testing::connect_test_db;
use alioth_gateway::openapi::projection::{
    global_cache, load_product_projection, ProductProjection,
};
use alioth_gateway::pep::NgacEnforcer;

/// 测试 EC P-256 密钥对（复用治理套件 fixtures；非生产密钥）。
const TEST_PRIVATE_KEY: &[u8] = include_bytes!("fixtures/openapi_governance_ec_private.pem");
const TEST_PUBLIC_KEY: &[u8] = include_bytes!("fixtures/openapi_governance_ec_public.pem");
const TEST_ISSUER: &str = "http://localhost:9002";

fn issue_token(sub: &str, svc_user_id: i64, scope: &str) -> String {
    let now = chrono::Utc::now().timestamp() as usize;
    let claims = serde_json::json!({
        "sub": sub,
        "exp": now + 3600,
        "iat": now,
        "email": "",
        "username": "",
        "sid": "",
        "iss": TEST_ISSUER,
        "aud": TEST_ISSUER,
        "scope": scope,
        "svc_user_id": svc_user_id,
    });
    let header = Header::new(Algorithm::ES256);
    encode(
        &header,
        &claims,
        &EncodingKey::from_ec_pem(TEST_PRIVATE_KEY).unwrap(),
    )
    .unwrap()
}

// ============================================================================
// 组 1：投影文法与判定（无 DB）
// ============================================================================

/// PROJ-001：段边界匹配（`waybills` 不匹配 `waybill` / `waybills-x`）
#[test]
fn proj_001_segment_boundary_semantics() {
    let p = ProductProjection::from_projections(["transport-operations/waybills"]);
    assert!(!p.is_full());
    assert!(p.allows("/api/service/transport-operations/waybills"));
    assert!(p.allows("/api/service/transport-operations/waybills/{id}"));
    assert!(p.allows("/api/service/transport-operations/waybills/{id}/loading"));
    assert!(!p.allows("/api/service/transport-operations/waybill"));
    assert!(!p.allows("/api/service/transport-operations/waybills-x"));
    assert!(!p.allows("/api/service/transport-operations/kanban"));
    // 非业务数据面（文档/认证/NGAC）不受投影约束
    assert!(p.allows("/api/openapi.json"));
    assert!(p.allows("/api/auth/token"));
    // 管理面（NGAC 门控）不受投影约束
    assert!(p.allows("/api/service/openapi/configs"));
}

/// PROJ-002：空集 / `*` / 空串语义（向后兼容不变式）
#[test]
fn proj_002_full_and_empty_semantics() {
    // 空集 ⇒ 全量
    let empty = ProductProjection::from_projections(Vec::<String>::new());
    assert!(empty.is_full() && empty.allows("/api/service/anything"));
    // `*` ⇒ 全量
    let star = ProductProjection::from_projections(["*", "contracts"]);
    assert!(star.is_full() && star.allows("/api/service/anything"));
    // 空串/NULL 行不参与 ⇒ 不产生「空投影集」以外的语义（此处全为空白 ⇒ 全量）
    let blanks = ProductProjection::from_projections(["", "  ", "/"]);
    assert!(blanks.is_full());
    // 有真实前缀时空白行被忽略而非清空
    let mixed = ProductProjection::from_projections(["", "contracts", "  "]);
    assert!(!mixed.is_full());
    assert_eq!(mixed.prefix_count(), 1);
    assert!(mixed.allows("/api/service/contracts/{id}"));
    assert!(!mixed.allows("/api/service/transport-operations/kanban"));
}

/// PROJ-003：前缀归一（前导/尾随斜杠、重复值去重）与状态行
#[test]
fn proj_003_normalization_and_status_line() {
    let p = ProductProjection::from_projections(["/contracts/", "contracts", "bin-locations"]);
    assert_eq!(p.prefix_count(), 2, "归一后去重");
    assert!(p.allows("/api/service/bin-locations"));
    assert!(p.status_line().contains("2 条前缀"));
    assert!(ProductProjection::full().status_line().contains("全量候选"));
}

/// PROJ-004：文档 paths 裁剪（出向契约段不裁剪）
#[test]
fn proj_004_filter_doc_paths() {
    let mut doc = json!({
        "paths": {
            "/api/service/measurement/units": { "get": {} },
            "/api/service/measurement/units/{id}": { "get": {} },
            "/api/service/measurement/other": { "get": {} }
        },
        "x-outbound-contracts": [{ "path": "/whatever" }]
    });
    let p = ProductProjection::from_projections(["measurement/units"]);
    let visible = p.filter_doc(&mut doc);
    assert_eq!(visible, 2, "仅投影前缀内路径保留");
    assert!(doc["paths"]["/api/service/measurement/other"].is_null());
    assert!(
        doc["x-outbound-contracts"].is_array(),
        "出向契约段 MUST NOT 被裁剪"
    );

    // 全量 ⇒ 不裁剪
    let mut full_doc = json!({ "paths": { "/api/service/a": {}, "/api/service/b": {} } });
    assert_eq!(ProductProjection::full().filter_doc(&mut full_doc), 2);
}

// ============================================================================
// 组 2：产品表加载（DB）
// ============================================================================

/// 插入一行产品（自清理前置）。返回行 id。
///
/// 坐标列 MUST 经 `ontology_binding::resolve` 解析（禁硬编码 ZUID）——与内核
/// `Framework/backend/openapi/src/repositories.rs::OpenApiSalesRepository::create` 同契约
/// （叶表写入契约 §4.3.3 类写入 / §6.12 坐标写入）。
async fn insert_product(pool: &PgPool, marker: &str, projection: &str) -> i64 {
    cleanup_product(pool, marker).await;
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("FF", "FJA", "↓.HA"))
        .await
        .expect("resolve coordinates（契约：值经 ontology_binding 解析）");
    sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_prod-openapi-sales\" \
         (notice, code, projection, dk_scene, dk_factor, dk_function) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(format!("proj-test-{}", marker))
    .bind(marker)
    .bind(projection)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("insert product row")
}

async fn cleanup_product(pool: &PgPool, marker: &str) {
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_prod-openapi-sales\" WHERE code = $1")
        .bind(marker)
        .execute(pool)
        .await;
}

/// PROJ-005：产品行 ⇒ 投影集含该前缀；清除后 ⇒ 不再含（**marker 作用域**——
/// 同二进制内其它用例可并发插入产品行，MUST NOT 断言全局空集）
#[tokio::test]
async fn proj_005_loader_reflects_product_rows() {
    let pool = connect_test_db().await;
    let marker = format!(
        "proj005-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    global_cache().invalidate();

    // 前置：本 marker 未插入 ⇒ 其前缀不在投影集（全局可能非空——不做全局断言）
    let before = load_product_projection(&pool).await;
    assert!(
        !before.prefixes().iter().any(|p| p == "proj005/units"),
        "本用例前缀不该在插入前出现"
    );

    insert_product(&pool, &marker, "proj005/units").await;
    let loaded = load_product_projection(&pool).await;
    assert!(!loaded.is_full(), "存在投影行 ⇒ 收窄");
    assert!(
        loaded.prefixes().iter().any(|p| p == "proj005/units"),
        "插入的 projection MUST 进入投影集（实得 {:?}）",
        loaded.prefixes()
    );
    assert!(loaded.allows("/api/service/proj005/units"));
    assert!(!loaded.allows("/api/service/other-service/units"));

    cleanup_product(&pool, &marker).await;
    global_cache().invalidate();
    let after = load_product_projection(&pool).await;
    assert!(
        !after.prefixes().iter().any(|p| p == "proj005/units"),
        "清除后本前缀 MUST 消失（实得 {:?}）",
        after.prefixes()
    );
}

// ============================================================================
// 组 3：PEP 授权面（DB）
// ============================================================================

/// 建夹具：服务用户 + client（scope 覆盖两路径）+ 产品行（仅投影 units）。
async fn create_fixture(pool: &PgPool, marker: &str) -> (String, i64, i64) {
    let client_id = format!("proj_test_{}", marker);
    let _ = sqlx::query("DELETE FROM isahl_auth.api_clients WHERE client_id = $1")
        .bind(&client_id)
        .execute(pool)
        .await;
    let svc_user: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.auth_users \
         (username, password_hash, user_type, is_active, status, \
          created_at, updated_at, failed_login_attempts, notification_preferences) \
         VALUES ($1, NULL, 'service', TRUE, 'active', NOW(), NOW(), 0, '{}'::jsonb) \
         ON CONFLICT (username) DO UPDATE SET updated_at = NOW() RETURNING id",
    )
    .bind(format!("svc:{}", client_id))
    .fetch_one(pool)
    .await
    .expect("service user");
    let client_row: (i64,) = sqlx::query_as(
        "INSERT INTO isahl_auth.api_clients \
         (client_id, client_type, client_name, secret_hash, scopes, fk_service_user, enabled) \
         VALUES ($1, 'oauth2', 'proj-test', '', $2::TEXT[], $3, TRUE) RETURNING id",
    )
    .bind(&client_id)
    .bind(&["read:units".to_string(), "read:other".to_string()])
    .bind(svc_user)
    .fetch_one(pool)
    .await
    .expect("client row");
    (client_id, client_row.0, svc_user)
}

async fn cleanup_fixture(pool: &PgPool, client_id: &str) {
    let svc: Option<i64> = sqlx::query_scalar(
        "SELECT fk_service_user FROM isahl_auth.api_clients WHERE client_id = $1",
    )
    .bind(client_id)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);
    let _ = sqlx::query("DELETE FROM isahl_auth.api_clients WHERE client_id = $1")
        .bind(client_id)
        .execute(pool)
        .await;
    if let Some(uid) = svc {
        let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
            .bind(uid)
            .execute(pool)
            .await;
    }
}

/// PROJ-006：投影集非空时，投影外业务端点 403 NOT_IN_PRODUCT_SET；投影内放行
#[actix_web::test]
async fn proj_006_pep_denies_outside_projection() {
    let pool = connect_test_db().await;
    let marker = format!(
        "proj006-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (client_id, _client_row, svc_user) = create_fixture(&pool, &marker).await;
    insert_product(&pool, &marker, "measurement/units").await;
    global_cache().invalidate();
    // 授权透传（PDP 无端点形态；与治理套件同法）
    std::env::set_var("NGAC_FAIL_OPEN", "true");

    let app = awtest::init_service(
        App::new().service(
            web::scope("/api")
                .wrap(
                    NgacEnforcer::new(
                        pool.clone(),
                        TEST_PUBLIC_KEY.to_vec(),
                        Vec::new(),
                        String::new(),
                    )
                    .with_token_binding(TEST_ISSUER.to_string(), TEST_ISSUER.to_string()),
                )
                .route(
                    "/service/measurement/units",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"data": []})) }),
                )
                .route(
                    "/service/measurement/other",
                    web::get().to(|| async { HttpResponse::Ok().json(json!({"data": []})) }),
                ),
        ),
    )
    .await;

    let token = issue_token(
        &format!("client:{}", client_id),
        svc_user,
        "read:units read:other",
    );

    // 投影外（scope 已覆盖 ⇒ 403 只能来自投影判定）
    let denied = awtest::call_service(
        &app,
        awtest::TestRequest::get()
            .uri("/api/service/measurement/other")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(denied.status().as_u16(), 403, "投影外端点 MUST 403");
    let body: serde_json::Value = awtest::read_body_json(denied).await;
    assert_eq!(body["error"], "NOT_IN_PRODUCT_SET", "body={body:?}");

    // 投影内 ⇒ 放行（scope 满足 + 授权透传）
    let allowed = awtest::call_service(
        &app,
        awtest::TestRequest::get()
            .uri("/api/service/measurement/units")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request(),
    )
    .await;
    assert_eq!(
        allowed.status().as_u16(),
        200,
        "投影内端点 MUST 放行（投影判定不误伤）"
    );

    cleanup_product(&pool, &marker).await;
    cleanup_fixture(&pool, &client_id).await;
    global_cache().invalidate();
    std::env::remove_var("NGAC_FAIL_OPEN");
}
