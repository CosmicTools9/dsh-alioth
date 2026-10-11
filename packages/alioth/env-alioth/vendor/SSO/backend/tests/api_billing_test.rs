//! OpenAPI 计费聚合集成测试（change `add-openapi-usage-billing`）
//!
//! 覆盖：
//!   1. 定价调用 ⇒ 明细 `calls/unit_price/amount` 精确（字符串 numeric）且汇总 = 明细和；
//!   2. 未定价两态（未命中产品行 / 命中但 `qk_price` 为空）⇒ `unit_price: null` +
//!      `unpriced_calls` 计数，MUST NOT 计入总额；
//!   3. 超窗月份（早于 90 天保留期）⇒ 400 `WINDOW_OUT_OF_RETENTION`（与对账同源判定）。
//!
//! 依赖真实测试库（aliothstudio_test）。测试数据自建自清；产品行写入遵守叶表坐标契约
//! （`ontology_binding::resolve`，禁硬编码 ZUID）。

mod common;

use actix_web::{test as awtest, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
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

async fn ensure_ngac(pool: &PgPool) {
    for sql in [
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_policy_class (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL, o_name TEXT, description TEXT,
            created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
            CONSTRAINT ngac_policy_class_pkey PRIMARY KEY (id))",
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_user_attribute (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL, o_name TEXT, fk_policy_class BIGINT NOT NULL,
            ancestor_ids BIGINT[] DEFAULT '{}', children_ids BIGINT[] DEFAULT '{}', property JSONB DEFAULT '{}',
            created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
            created_by_id BIGINT, updated_by_id BIGINT, deleted_at TIMESTAMPTZ,
            CONSTRAINT ngac_user_attribute_pkey PRIMARY KEY (id))",
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_user_rr_attribute (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL, created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
            updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, created_by_id BIGINT, updated_by_id BIGINT, o_name TEXT,
            fk_user BIGINT, fk_user_attribute BIGINT, assigned_at TIMESTAMPTZ DEFAULT NOW(), expires_at TIMESTAMPTZ,
            conditions JSONB DEFAULT '{}', deleted_at TIMESTAMPTZ,
            CONSTRAINT ngac_user_rr_attribute_pkey PRIMARY KEY (id))",
        "INSERT INTO isahl_auth.ngac_policy_class (id, o_name)
            SELECT isahl.gen_next_zuid(), 'default'
            WHERE NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_policy_class WHERE o_name = 'default')",
    ] {
        sqlx::query(sql).execute(pool).await.ok();
    }
}

async fn ensure_admin(pool: &PgPool) -> i64 {
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, username, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), 'admin_billing', 'active', true, NOW(), NOW())
         ON CONFLICT (username) DO NOTHING",
    )
    .execute(pool)
    .await
    .ok();
    let admin: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE username = 'admin_billing' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("admin user");
    sqlx::query(
        "INSERT INTO isahl_auth.auth_user_emails (fk_user, email, is_primary, created_at, updated_at)
         VALUES ($1, 'admin_billing@test.local', TRUE, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(admin)
    .execute(pool)
    .await
    .ok();
    let pc: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("policy class");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
         VALUES ('admin', $1, NOW(), NOW()) ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING",
    )
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' AND fk_policy_class = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(pc)
    .fetch_one(pool)
    .await
    .expect("admin UA");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
         VALUES ($1, $2, NOW(), NOW())",
    )
    .bind(admin)
    .bind(ua)
    .execute(pool)
    .await
    .ok();
    admin
}

/// 价标量行（唯一价源 `isahl."zc_id_scal-price".mark`）。
async fn insert_price(pool: &PgPool, marker: &str, mark: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_scal-price\" (notice, code, mark) VALUES ($1, $2, $3::numeric) RETURNING id",
    )
    .bind(format!("billing-price-{}", marker))
    .bind(marker)
    .bind(mark)
    .fetch_one(pool)
    .await
    .expect("insert price scalar")
}

/// 产品行（坐标契约：值经 `ontology_binding::resolve` 解析）。
async fn insert_product(
    pool: &PgPool,
    marker: &str,
    projection: &str,
    price_id: Option<i64>,
) -> i64 {
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("FF", "FJA", "↓.HA"))
        .await
        .expect("resolve coordinates");
    sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_prod-openapi-sales\"
           (notice, code, projection, qk_price, dk_scene, dk_factor, dk_function)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(format!("billing-product-{}", marker))
    .bind(marker)
    .bind(projection)
    .bind(price_id)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("insert product row")
}

struct Fixture {
    client_id: String,
    subscription_id: i64,
    service_user: i64,
}

async fn create_client(pool: &PgPool, marker: &str) -> Fixture {
    let client_id = format!("billing_test_{}", marker);
    let svc_user: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.auth_users
           (username, password_hash, user_type, is_active, status,
            created_at, updated_at, failed_login_attempts, notification_preferences)
         VALUES ($1, NULL, 'service', TRUE, 'active', NOW(), NOW(), 0, '{}'::jsonb)
         ON CONFLICT (username) DO UPDATE SET updated_at = NOW() RETURNING id",
    )
    .bind(format!("svc:{}", client_id))
    .fetch_one(pool)
    .await
    .expect("service user");
    let client_row: (i64,) = sqlx::query_as(
        "INSERT INTO isahl_auth.api_clients
           (client_id, client_type, client_name, secret_hash, scopes, fk_service_user, enabled)
         VALUES ($1, 'oauth2', 'billing-test', '', ARRAY[]::TEXT[], $2, TRUE) RETURNING id",
    )
    .bind(&client_id)
    .bind(svc_user)
    .fetch_one(pool)
    .await
    .expect("client");
    let plan: i64 = sqlx::query_scalar("SELECT id FROM isahl_auth.api_plans LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("a plan exists（套餐种子）");
    let sub: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.api_subscriptions (fk_client, fk_plan, status) \
         VALUES ($1, $2, 'active') RETURNING id",
    )
    .bind(client_row.0)
    .bind(plan)
    .fetch_one(pool)
    .await
    .expect("subscription");
    Fixture {
        client_id,
        subscription_id: sub,
        service_user: svc_user,
    }
}

/// 追加计量行（显式 `requested_at`：计费用例在**指定月份**造量，避免共享测试库 NOW() 污染）。
async fn add_usage(
    pool: &PgPool,
    sub_id: i64,
    route: &str,
    times: i32,
    at: chrono::DateTime<chrono::Utc>,
) {
    for _ in 0..times {
        sqlx::query(
            "INSERT INTO isahl_auth.api_usage (fk_subscription, route, method, status, latency_ms, requested_at) \
             VALUES ($1, $2, 'GET', 200, 5, $3)",
        )
        .bind(sub_id)
        .bind(route)
        .bind(at)
        .execute(pool)
        .await
        .expect("insert usage");
    }
}

async fn cleanup(pool: &PgPool, fixture: &Fixture, marker: &str) {
    let _ = sqlx::query("DELETE FROM isahl_auth.api_usage WHERE fk_subscription = $1")
        .bind(fixture.subscription_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.api_subscriptions WHERE id = $1")
        .bind(fixture.subscription_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.api_clients WHERE client_id = $1")
        .bind(&fixture.client_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(fixture.service_user)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_prod-openapi-sales\" WHERE code = $1")
        .bind(marker)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_prod-openapi-sales\" WHERE code = $1")
        .bind(format!("{}-np", marker))
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_scal-price\" WHERE code = $1")
        .bind(marker)
        .execute(pool)
        .await;
}

/// 从计费响应取汇总数值（测试侧解析，仅用于断言相对增量）。
fn total(resp: &serde_json::Value, key: &str) -> f64 {
    resp["totals"][key]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| resp["totals"][key].as_f64())
        .unwrap_or(0.0)
}

/// 计费聚合：定价明细 + 未定价两态 + **相对增量**汇总口径（共享测试库 ⇒ 断言用 delta）
#[actix_web::test]
async fn billing_aggregates_usage_with_product_prices() {
    use chrono::Datelike;
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    ensure_ngac(&pool).await;
    let admin = ensure_admin(&pool).await;
    let ast = test_auth_state();
    let app = awtest::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::admin::configure),
    )
    .await;
    let hdr = format!(
        "Bearer {}",
        mint_token(&ast, admin, "admin_billing@test.local")
    );

    let marker = format!(
        "b{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        std::process::id()
    );
    let fx = create_client(&pool, &marker).await;
    let price = insert_price(&pool, &marker, "12.50").await;
    let priced_projection = format!("billing-{}/priced", marker);
    insert_product(&pool, &marker, &priced_projection, Some(price)).await;
    let unpriced_projection = format!("billing-{}/unpriced", marker);
    insert_product(&pool, &format!("{}-np", marker), &unpriced_projection, None).await;

    // 计费窗口 = **上一个月**（共享测试库中其它用例只写 NOW() ⇒ 该月无外部污染）；
    // 汇总断言用「加量前 vs 加量后」的**相对增量**，对任何残留同样免疫。
    let now = chrono::Utc::now();
    let prev_first = chrono::NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .expect("当月 1 日")
        .pred_opt()
        .expect("上月最后一天");
    let target_month = format!("{:04}-{:02}", prev_first.year(), prev_first.month());
    let at = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
        prev_first
            .with_day(15)
            .expect("15 日")
            .and_hms_opt(12, 0, 0)
            .expect("12:00"),
        chrono::Utc,
    );

    let uri = format!("/api/admin/api-billing?month={}", target_month);
    // 取一次计费响应（macro 而非闭包：避免 `&app` 被 move 进闭包后不可再用）
    macro_rules! billing_get {
        ($uri:expr) => {{
            let resp = awtest::call_service(
                &app,
                awtest::TestRequest::get()
                    .uri(&$uri)
                    .insert_header(("Authorization", hdr.clone()))
                    .to_request(),
            )
            .await;
            assert_eq!(resp.status().as_u16(), 200, "计费端点应 200");
            let body: serde_json::Value = awtest::read_body_json(resp).await;
            body
        }};
    }

    let before = billing_get!(uri);

    add_usage(
        &pool,
        fx.subscription_id,
        &format!("/api/service/{}", priced_projection),
        3,
        at,
    )
    .await;
    add_usage(
        &pool,
        fx.subscription_id,
        &format!("/api/service/{}", unpriced_projection),
        1,
        at,
    )
    .await;
    add_usage(
        &pool,
        fx.subscription_id,
        &format!("/api/service/unmatched-{}", marker),
        2,
        at,
    )
    .await;

    let after = billing_get!(uri);

    // 明细（按 client × 投影隔离 ⇒ 与共享库无关）
    let priced = after["items"]
        .as_array()
        .expect("items 数组")
        .iter()
        .find(|i| {
            i["client_id"].as_str() == Some(fx.client_id.as_str())
                && i["projection"].as_str() == Some(priced_projection.as_str())
        })
        .unwrap_or_else(|| panic!("定价明细缺失：{after:?}"));
    assert_eq!(priced["calls"].as_i64(), Some(3));
    assert_eq!(
        priced["unit_price"].as_str(),
        Some("12.5"),
        "单价以字符串承载（numeric 去尾零）"
    );
    assert_eq!(
        priced["amount"].as_str(),
        Some("37.5"),
        "金额 = 调用量 × 单价"
    );

    let unpriced = after["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| {
            i["client_id"].as_str() == Some(fx.client_id.as_str())
                && i["projection"].as_str() == Some(unpriced_projection.as_str())
        })
        .unwrap_or_else(|| panic!("未定价明细缺失：{after:?}"));
    assert!(unpriced["unit_price"].is_null(), "未定价 MUST 为 null");
    assert!(unpriced["amount"].is_null(), "未定价金额 MUST 为 null");
    assert_eq!(after["overlap_policy"].as_str(), Some("per-product"));

    // 汇总相对增量：定价 +3 / 未定价 +3（1 命中无价 + 2 未命中）/ 总额 +37.5
    assert_eq!(
        total(&after, "priced_calls") - total(&before, "priced_calls"),
        3.0,
        "priced delta（before={before:?}）"
    );
    assert_eq!(
        total(&after, "unpriced_calls") - total(&before, "unpriced_calls"),
        3.0,
        "unpriced delta（after={after:?}）"
    );
    assert_eq!(
        total(&after, "total_amount") - total(&before, "total_amount"),
        37.5,
        "金额增量（after={after:?}）"
    );

    // 超窗月份 ⇒ 400 WINDOW_OUT_OF_RETENTION（与对账同源判定）
    let old = now - chrono::Duration::days(150);
    let old_month = format!("{:04}-{:02}", old.year(), old.month());
    let resp = awtest::call_service(
        &app,
        awtest::TestRequest::get()
            .uri(&format!("/api/admin/api-billing?month={}", old_month))
            .insert_header(("Authorization", hdr.clone()))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 400, "超窗月份 MUST 400");
    let body: serde_json::Value = awtest::read_body_json(resp).await;
    assert_eq!(body["error"], "WINDOW_OUT_OF_RETENTION", "body={body:?}");

    cleanup(&pool, &fx, &marker).await;
}
