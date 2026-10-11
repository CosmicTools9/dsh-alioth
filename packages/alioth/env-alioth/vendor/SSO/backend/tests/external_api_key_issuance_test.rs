//! 外部主体 API-KEY 签发面集成测试
//! （change `move-apikey-issuance-to-external-portal`；前身 = openapi-self-service-portal）
//!
//! 覆盖：
//! - 鉴权：无 JWT / 无效 JWT → 401
//! - 门禁：`user_type='standard'` → 403 `EXTERNAL_ONLY`；external 但未审批 → 403 `EXTERNAL_NOT_APPROVED`
//! - 旧自助路径 `/auth/self/api-clients` → 404（MUST NOT 保留兼容别名）
//! - CRUD 生命周期：create（含前缀归属落库）→ list（前缀剥离）→ rotate（apikey 换 client_id）
//!   → delete（软删 + 订阅 canceled）
//! - 归属隔离：外部主体 A 看不到/动不了 B 的 client（列表隔离 + 越权 403）
//! - 边界：非法 client_type → 400；不存在 id → 404
//!
//! 与 openapi_metering / admin_api_test 同模式：#[tokio::test] + PgPool connect，
//! 禁 #[sqlx::test]；测试数据绝不残留（每个测试末尾清理）。

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use serde_json::Value;
use sqlx::PgPool;

async fn connect() -> PgPool {
    ::common::testing::connect_test_db().await
}

fn test_auth_state() -> AuthState {
    common::test_auth_state()
}

fn mint_token(ast: &AuthState, user_id: i64, email: &str) -> String {
    configure_token_validation(
        "http://localhost:9002".to_string(),
        "http://localhost:9002".to_string(),
    );
    encode_access_token(
        &Claims::new(&user_id.to_string(), email, false),
        &ast.jwt_private_key,
    )
    .expect("mint token")
}

/// 建测试用户（ON CONFLICT (username) 幂等），返回 user_id。`user_type`/`status` 显式传入——
/// 签发面的门禁判据（external + active）由此参数化，正例与负例共用本构造函数；
/// 冲突时按参数覆盖身份（跨轮残留不污染本轮判定）。原 `name` 值即唯一锚点、现落 `username`；
/// 邮箱唯一存储 = isahl_auth.auth_user_emails。
async fn ensure_user(pool: &PgPool, name: &str, email: &str, user_type: &str, status: &str) -> i64 {
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, username, user_type, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), $1, $2::isahl_auth.auth_user_type, $3::isahl_auth.auth_user_status, true, NOW(), NOW())
         ON CONFLICT (username) DO UPDATE
           SET user_type = EXCLUDED.user_type, status = EXCLUDED.status, updated_at = NOW()",
    )
    .bind(name)
    .bind(user_type)
    .bind(status)
    .execute(pool)
    .await
    .ok();
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE username = $1 LIMIT 1")
            .bind(name)
            .fetch_one(pool)
            .await
            .expect("user id");
    sqlx::query(
        "INSERT INTO isahl_auth.auth_user_emails (fk_user, email, is_primary, created_at, updated_at)
         VALUES ($1, $2, TRUE, NOW(), NOW())
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(email)
    .execute(pool)
    .await
    .ok();
    user_id
}

/// 前置清场：清掉该外部主体此前遗留的 client（含订阅）与服务用户。
/// 共享测试库中前轮失败会跳过尾部 cleanup，残留会把「本人全量可见」的计数断言撑爆——
/// 该主体的 client 属本测试族独占，可安全按所有者删除。
async fn purge_external_clients(pool: &PgPool, user_id: i64) {
    // 归属判据与 list_external_api_clients 同源：client_name 前缀 `user:<id>:`
    let prefix = format!("user:{}:%", user_id);
    sqlx::query(
        "DELETE FROM isahl_auth.api_subscriptions WHERE fk_client IN
           (SELECT id FROM isahl_auth.api_clients WHERE client_name LIKE $1)",
    )
    .bind(&prefix)
    .execute(pool)
    .await
    .ok();
    let svc_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT fk_service_user FROM isahl_auth.api_clients
         WHERE client_name LIKE $1 AND fk_service_user IS NOT NULL",
    )
    .bind(&prefix)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    sqlx::query("DELETE FROM isahl_auth.api_clients WHERE client_name LIKE $1")
        .bind(&prefix)
        .execute(pool)
        .await
        .ok();
    for id in svc_ids {
        sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1 AND user_type = 'service'")
            .bind(id)
            .execute(pool)
            .await
            .ok();
    }
}

/// 测试数据清理：订阅 → client → 服务用户 → 自然人（顺序满足 FK）。
async fn cleanup(pool: &PgPool, client_ids: &[i64], service_user_ids: &[i64], user_ids: &[i64]) {
    for id in client_ids {
        sqlx::query("DELETE FROM isahl_auth.api_subscriptions WHERE fk_client = $1")
            .bind(id)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM isahl_auth.api_clients WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await
            .ok();
    }
    for id in service_user_ids {
        sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1 AND user_type = 'service'")
            .bind(id)
            .execute(pool)
            .await
            .ok();
    }
    for id in user_ids {
        // auth_user_emails 入边 FK 无 ON DELETE CASCADE：先删邮箱行再删账号
        sqlx::query("DELETE FROM isahl_auth.auth_user_emails WHERE fk_user = $1")
            .bind(id)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await
            .ok();
    }
}

#[tokio::test]
async fn external_issuance_rejects_missing_or_invalid_jwt() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    let ast = test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/auth").configure(gateway_sso::auth::portal::configure_routes)),
    )
    .await;

    // 无 JWT → 401
    let req = test::TestRequest::get()
        .uri("/auth/external/api-clients")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 401, "missing JWT must be rejected");

    // 无效 JWT → 401
    let req = test::TestRequest::get()
        .uri("/auth/external/api-clients")
        .insert_header(("Authorization", "Bearer not-a-real-token"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 401, "invalid JWT must be rejected");
}

#[tokio::test]
async fn external_issuance_crud_lifecycle() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    let ast = test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/auth").configure(gateway_sso::auth::portal::configure_routes)),
    )
    .await;

    let user_id = ensure_user(
        &pool,
        "ext_crud",
        "ext_crud@alioth.test",
        "external",
        "active",
    )
    .await;
    purge_external_clients(&pool, user_id).await;
    let token = mint_token(&ast, user_id, "ext_crud@alioth.test");
    let mut client_ids: Vec<i64> = Vec::new();
    let mut svc_user_ids: Vec<i64> = Vec::new();

    // ── 创建 apikey client ──────────────────────────────────────────────────
    let req = test::TestRequest::post()
        .uri("/auth/external/api-clients")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({ "client_type": "apikey", "client_name": "my-key" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 201, "create apikey client");
    let created: Value = test::read_body_json(resp).await;
    let created_id = created["id"]
        .as_str()
        .and_then(|v| v.parse().ok())
        .expect("client id");
    let secret = created["secret"]
        .as_str()
        .expect("plaintext secret")
        .to_string();
    assert!(secret.starts_with("ak_"), "apikey secret format: {secret}");
    assert_eq!(
        created["client_id"].as_str(),
        Some(secret.as_str()),
        "apikey client_id == secret 明文"
    );
    assert_eq!(
        created["client_name"].as_str(),
        Some("my-key"),
        "响应返回显示名"
    );
    assert_eq!(created["client_type"].as_str(), Some("apikey"));
    let svc_user_id = created["fk_service_user"]
        .as_str()
        .and_then(|v| v.parse().ok())
        .expect("svc user");
    client_ids.push(created_id);
    svc_user_ids.push(svc_user_id);

    // DB：归属前缀强制写入
    let db_name: String =
        sqlx::query_scalar("SELECT client_name FROM isahl_auth.api_clients WHERE id = $1")
            .bind(created_id)
            .fetch_one(&pool)
            .await
            .expect("db client row");
    assert_eq!(
        db_name,
        format!("user:{}:my-key", user_id),
        "client_name 必须带归属前缀"
    );

    // 服务用户必须存在且为 service 类型
    let svc_type: String =
        sqlx::query_scalar("SELECT user_type::text FROM isahl_auth.auth_users WHERE id = $1")
            .bind(svc_user_id)
            .fetch_one(&pool)
            .await
            .expect("svc user row");
    assert_eq!(svc_type, "service", "自助 client 必须同步创建服务用户");

    // 默认 free 订阅落库
    let sub_status: String = sqlx::query_scalar(
        "SELECT s.status FROM isahl_auth.api_subscriptions s \
         WHERE s.fk_client = $1 AND s.deleted_at IS NULL",
    )
    .bind(created_id)
    .fetch_one(&pool)
    .await
    .expect("subscription row");
    assert_eq!(sub_status, "active", "创建时必须补种默认 free 订阅");

    // ── 非法 client_type → 400 ──────────────────────────────────────────────
    let req = test::TestRequest::post()
        .uri("/auth/external/api-clients")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({ "client_type": "saml", "client_name": "bad" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400, "unsupported client_type");

    // ── 文法外 scopes → 400 INVALID_SCOPE（scope-grammar-single-source）────
    let req = test::TestRequest::post()
        .uri("/auth/external/api-clients")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({
            "client_type": "oauth2",
            "client_name": "bad-scopes",
            "scopes": ["read", "write"],
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400, "文法外 scope 必须拒绝（零写入）");
    let err: Value = test::read_body_json(resp).await;
    assert_eq!(err["error"].as_str(), Some("INVALID_SCOPE"));
    assert_eq!(
        err["details"],
        serde_json::json!(["read", "write"]),
        "违规值清单"
    );

    // ── 自助面裸通配 → 400（通配仅管理面可授）────────────────────────────
    let req = test::TestRequest::post()
        .uri("/auth/external/api-clients")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({
            "client_type": "oauth2",
            "client_name": "wildcard",
            "scopes": ["*"],
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400, "自助面不得自授裸 *");

    // ── 创建 oauth2 client（合法 scope；client_id 自动生成）───────────────
    let req = test::TestRequest::post()
        .uri("/auth/external/api-clients")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({
            "client_type": "oauth2",
            "client_name": "web-app",
            "scopes": ["read:contract", "create:contract"],
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 201, "create oauth2 client");
    let oauth2: Value = test::read_body_json(resp).await;
    assert_eq!(
        oauth2["authorization_status"].as_str(),
        Some("granted"),
        "非空 scope ⇒ granted"
    );
    let oauth2_id = oauth2["id"]
        .as_str()
        .and_then(|v| v.parse().ok())
        .expect("oauth2 client id");
    let oauth2_cid = oauth2["client_id"].as_str().expect("client_id").to_string();
    assert!(
        oauth2_cid.starts_with("client-"),
        "oauth2 自动生成 client_id: {oauth2_cid}"
    );
    assert_eq!(oauth2["client_type"].as_str(), Some("oauth2"));
    assert_ne!(
        oauth2["client_id"].as_str(),
        oauth2["secret"].as_str(),
        "oauth2 client_id 与 secret 不同"
    );
    client_ids.push(oauth2_id);
    svc_user_ids.push(
        oauth2["fk_service_user"]
            .as_str()
            .and_then(|v| v.parse().ok())
            .expect("svc user"),
    );

    // ── 列表：仅本人 client，前缀剥离 ───────────────────────────────────────
    let req = test::TestRequest::get()
        .uri("/auth/external/api-clients")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let listed: Value = test::read_body_json(resp).await;
    let clients = listed["clients"].as_array().expect("clients array");
    assert_eq!(clients.len(), 2, "本人 client 全量可见");
    let names: Vec<&str> = clients
        .iter()
        .filter_map(|c| c["client_name"].as_str())
        .collect();
    assert!(names.contains(&"my-key"), "显示名已剥离前缀: {names:?}");
    assert!(names.contains(&"web-app"));
    for c in clients {
        let cname = c["client_name"].as_str().unwrap_or("");
        assert!(
            !cname.starts_with("user:"),
            "响应中 client_name 不得泄露归属前缀: {cname}"
        );
    }
    // oauth2 的 scopes 回显（合法文法值）
    let oauth2_listed = clients
        .iter()
        // ID_JSON_PRECISION：响应 id 为字符串，按解析值比较
        .find(|c| c["id"].as_str().and_then(|v| v.parse::<i64>().ok()) == Some(oauth2_id))
        .expect("oauth2 in list");
    assert_eq!(
        oauth2_listed["scopes"],
        serde_json::json!(["read:contract", "create:contract"]),
        "scopes 回显"
    );
    assert_eq!(
        oauth2_listed["authorization_status"].as_str(),
        Some("granted")
    );
    // 首 client（my-key，无 scopes）呈现待授权态
    let pending_listed = clients
        .iter()
        .find(|c| c["client_name"].as_str() == Some("my-key"))
        .expect("my-key in list");
    assert_eq!(
        pending_listed["authorization_status"].as_str(),
        Some("pending"),
        "空 scope ⇒ pending（待管理员授予）"
    );

    // ── rotate apikey：新明文 + client_id 同步替换 ─────────────────────────
    let req = test::TestRequest::post()
        .uri(&format!(
            "/auth/external/api-clients/{}/rotate-secret",
            created_id
        ))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200, "rotate own client");
    let rotated: Value = test::read_body_json(resp).await;
    let new_secret = rotated["data"]["secret"]
        .as_str()
        .expect("new secret")
        .to_string();
    let new_cid = rotated["data"]["client_id"]
        .as_str()
        .expect("new client_id");
    assert_ne!(new_secret, secret, "轮换必须生成新密钥");
    assert!(new_secret.starts_with("ak_"));
    assert_eq!(
        new_cid,
        new_secret.as_str(),
        "apikey 轮换后 client_id 同步为新明文"
    );

    // DB 侧 hash 已覆盖（旧明文不可再匹配）
    let stored_hash: String =
        sqlx::query_scalar("SELECT secret_hash FROM isahl_auth.api_clients WHERE id = $1")
            .bind(created_id)
            .fetch_one(&pool)
            .await
            .expect("stored hash");
    assert_ne!(stored_hash, "", "secret_hash 已更新");

    // ── rotate oauth2：client_id 稳定，仅换 secret ──────────────────────────
    let req = test::TestRequest::post()
        .uri(&format!(
            "/auth/external/api-clients/{}/rotate-secret",
            oauth2_id
        ))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let rotated_oauth2: Value = test::read_body_json(resp).await;
    assert_eq!(
        rotated_oauth2["data"]["client_id"].as_str(),
        Some(oauth2_cid.as_str()),
        "oauth2 轮换 client_id 保持稳定"
    );

    // ── delete：软删 + 订阅 canceled ────────────────────────────────────────
    let req = test::TestRequest::delete()
        .uri(&format!("/auth/external/api-clients/{}", created_id))
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200, "delete own client");
    let deleted: Value = test::read_body_json(resp).await;
    assert_eq!(deleted["deleted"], serde_json::json!(true));

    let sub_status: String = sqlx::query_scalar(
        "SELECT status FROM isahl_auth.api_subscriptions \
         WHERE fk_client = $1 AND deleted_at IS NOT NULL ORDER BY id DESC LIMIT 1",
    )
    .bind(created_id)
    .fetch_one(&pool)
    .await
    .expect("subscription row");
    assert_eq!(sub_status, "canceled", "吊销后 active 订阅必须挂起");

    // 列表只剩 oauth2
    let req = test::TestRequest::get()
        .uri("/auth/external/api-clients")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let listed: Value = test::read_body_json(resp).await;
    assert_eq!(
        listed["clients"].as_array().expect("clients array").len(),
        1,
        "软删后列表不再出现"
    );

    cleanup(&pool, &client_ids, &svc_user_ids, &[user_id]).await;
}

#[tokio::test]
async fn external_issuance_ownership_isolation() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    let ast = test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/auth").configure(gateway_sso::auth::portal::configure_routes)),
    )
    .await;

    let user_a = ensure_user(&pool, "ext_a", "ext_a@alioth.test", "external", "active").await;
    let user_b = ensure_user(&pool, "ext_b", "ext_b@alioth.test", "external", "active").await;
    purge_external_clients(&pool, user_a).await;
    purge_external_clients(&pool, user_b).await;
    let token_a = mint_token(&ast, user_a, "ext_a@alioth.test");
    let token_b = mint_token(&ast, user_b, "ext_b@alioth.test");

    // A 创建 client
    let req = test::TestRequest::post()
        .uri("/auth/external/api-clients")
        .insert_header(("Authorization", format!("Bearer {}", token_a)))
        .set_json(serde_json::json!({ "client_type": "apikey", "client_name": "a-key" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 201);
    let created: Value = test::read_body_json(resp).await;
    let client_id = created["id"]
        .as_str()
        .and_then(|v| v.parse().ok())
        .expect("client id");
    let svc_user_id = created["fk_service_user"]
        .as_str()
        .and_then(|v| v.parse().ok())
        .expect("svc user");

    // B 列表：看不到 A 的 client（归属隔离）
    let req = test::TestRequest::get()
        .uri("/auth/external/api-clients")
        .insert_header(("Authorization", format!("Bearer {}", token_b)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let listed: Value = test::read_body_json(resp).await;
    assert_eq!(
        listed["clients"].as_array().expect("clients array").len(),
        0,
        "用户 B 看不到用户 A 的 client"
    );

    // B 轮换 A 的 client → 403
    let req = test::TestRequest::post()
        .uri(&format!(
            "/auth/external/api-clients/{}/rotate-secret",
            client_id
        ))
        .insert_header(("Authorization", format!("Bearer {}", token_b)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 403, "越权轮换必须 403");

    // B 吊销 A 的 client → 403
    let req = test::TestRequest::delete()
        .uri(&format!("/auth/external/api-clients/{}", client_id))
        .insert_header(("Authorization", format!("Bearer {}", token_b)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 403, "越权吊销必须 403");

    // A 的 client 仍然存在且可用
    let req = test::TestRequest::get()
        .uri("/auth/external/api-clients")
        .insert_header(("Authorization", format!("Bearer {}", token_a)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let listed: Value = test::read_body_json(resp).await;
    assert_eq!(
        listed["clients"].as_array().expect("clients array").len(),
        1,
        "A 的 client 未被 B 破坏"
    );

    // 不存在的 id → 404（本人视角）
    let req = test::TestRequest::post()
        .uri("/auth/external/api-clients/999999999/rotate-secret")
        .insert_header(("Authorization", format!("Bearer {}", token_a)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 404, "不存在 client 必须 404");
    let req = test::TestRequest::delete()
        .uri("/auth/external/api-clients/999999999")
        .insert_header(("Authorization", format!("Bearer {}", token_a)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 404, "删除不存在 client 必须 404");

    cleanup(&pool, &[client_id], &[svc_user_id], &[user_a, user_b]).await;
}

/// 门禁负例：`user_type='standard'` 的已登录账号 MUST 被拒（403 `EXTERNAL_ONLY`），
/// 且 MUST NOT 产生任何 `api_clients` 写入（先判后写）。
#[tokio::test]
async fn external_issuance_rejects_internal_account() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    let ast = test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/auth").configure(gateway_sso::auth::portal::configure_routes)),
    )
    .await;

    let user_id = ensure_user(
        &pool,
        "ext_internal",
        "ext_internal@alioth.test",
        "standard",
        "active",
    )
    .await;
    purge_external_clients(&pool, user_id).await;
    let token = mint_token(&ast, user_id, "ext_internal@alioth.test");

    let req = test::TestRequest::get()
        .uri("/auth/external/api-clients")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 403, "内部账号列目录必须 403");
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        body["error"].as_str(),
        Some("EXTERNAL_ONLY"),
        "错误码必须为 EXTERNAL_ONLY: {body}"
    );

    let req = test::TestRequest::post()
        .uri("/auth/external/api-clients")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({ "client_type": "apikey", "client_name": "nope" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 403, "内部账号签发必须 403");

    let created: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM isahl_auth.api_clients WHERE client_name LIKE $1")
            .bind(format!("user:{}:%", user_id))
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(created, 0, "被拒请求 MUST NOT 留下 client 行");

    cleanup(&pool, &[], &[], &[user_id]).await;
}

/// 门禁负例：`user_type='external'` 但未审批（`status='pending_approval'`）MUST 被拒
/// （403 `EXTERNAL_NOT_APPROVED`）——白名单式判定，未审批状态 MUST NOT 静默放行。
#[tokio::test]
async fn external_issuance_rejects_unapproved_external() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    let ast = test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/auth").configure(gateway_sso::auth::portal::configure_routes)),
    )
    .await;

    let user_id = ensure_user(
        &pool,
        "ext_pending",
        "ext_pending@alioth.test",
        "external",
        "pending_approval",
    )
    .await;
    purge_external_clients(&pool, user_id).await;
    let token = mint_token(&ast, user_id, "ext_pending@alioth.test");

    let req = test::TestRequest::post()
        .uri("/auth/external/api-clients")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .set_json(serde_json::json!({ "client_type": "apikey", "client_name": "nope" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 403, "未审批 external 必须 403");
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(
        body["error"].as_str(),
        Some("EXTERNAL_NOT_APPROVED"),
        "错误码必须为 EXTERNAL_NOT_APPROVED: {body}"
    );

    let created: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM isahl_auth.api_clients WHERE client_name LIKE $1")
            .bind(format!("user:{}:%", user_id))
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(created, 0, "未审批主体的请求 MUST NOT 留下 client 行");

    cleanup(&pool, &[], &[], &[user_id]).await;
}

/// 旧自助路径 MUST 已移除：`/auth/self/api-clients` 返回 404，
/// MUST NOT 保留为兼容别名（改名即让 Gateway 旧调用点立即失效）。
#[tokio::test]
async fn legacy_self_service_path_is_gone() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    let ast = test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(web::scope("/auth").configure(gateway_sso::auth::portal::configure_routes)),
    )
    .await;

    let user_id = ensure_user(
        &pool,
        "ext_legacy",
        "ext_legacy@alioth.test",
        "external",
        "active",
    )
    .await;
    purge_external_clients(&pool, user_id).await;
    let token = mint_token(&ast, user_id, "ext_legacy@alioth.test");

    for uri in [
        "/auth/self/api-clients",
        "/auth/self/api-clients/1/rotate-secret",
        "/auth/self/api-clients/1",
    ] {
        let req = test::TestRequest::get()
            .uri(uri)
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404, "{uri} 必须 404（路由已移除）");
    }

    cleanup(&pool, &[], &[], &[user_id]).await;
}
