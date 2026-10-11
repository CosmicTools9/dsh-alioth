//! 义务配置面（admin CRUD）集成测试（change `wire-org-policy-declaration-fields` 切片 C）。
//!
//! 覆盖：
//!   1. 往返：create → list（含 64bit id 字符串化）→ update（COALESCE 局部）→ delete（软删）→ list 不可见；
//!   2. 同事务策略审计：insert/update/delete 各 1 行（`entity_type = ngac_obligation`）；
//!   3. 写径校验 fail-closed：非法 Rhai 标识符 / 非字符串数组元素 / 非法 phase ⇒ 400 且零落库。
//!
//! 数据约定：`oblx-` 前缀；测试前后双清理（含审计行），不残留。

mod common;

use ::common::testing::connect_test_db;
use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use serde_json::{json, Value};
use sqlx::PgPool;

/// 文件内串行化（共享测试库；cleanup 会互删同前缀数据）。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_auth_state() -> AuthState {
    common::test_auth_state()
}

/// `ngac_obligation` 由 SSO ensure 建立；共享测试库缺表时自建最小契约面（幂等）。
async fn ensure_obligation_ddl(pool: &PgPool) {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.ngac_obligation (
            id bigserial PRIMARY KEY,
            o_name text,
            event_pattern text,
            response_ops jsonb DEFAULT '[]'::jsonb,
            is_active boolean DEFAULT TRUE,
            priority integer DEFAULT 0,
            conditions jsonb DEFAULT '{}'::jsonb,
            phase text NOT NULL DEFAULT 'post',
            created_at timestamptz NOT NULL DEFAULT NOW(),
            updated_at timestamptz NOT NULL DEFAULT NOW(),
            created_by_id bigint,
            updated_by_id bigint,
            deleted_at timestamptz
        )"#,
    )
    .execute(pool)
    .await
    .expect("bootstrap ngac_obligation DDL");
}

async fn cleanup(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_policy_audit_log WHERE new_values::text LIKE '%oblx-%'",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.ngac_obligation WHERE o_name LIKE 'oblx-%'")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_user_emails WHERE email LIKE 'oblx-%@test.local'")
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username LIKE 'oblx\\_%'")
        .execute(pool)
        .await
        .ok();
}

async fn ensure_user(pool: &PgPool, email: &str) -> i64 {
    let username = email.split('@').next().unwrap_or("oblx").replace('-', "_");
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users (id, username, status, is_active, created_at, updated_at)
           VALUES (isahl.gen_next_zuid(), $1, 'active', true, NOW(), NOW())
           ON CONFLICT (username) DO NOTHING"#,
    )
    .bind(&username)
    .execute(pool)
    .await
    .ok();
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE username=$1 LIMIT 1")
            .bind(&username)
            .fetch_one(pool)
            .await
            .expect("test user");
    sqlx::query(
        "INSERT INTO isahl_auth.auth_user_emails (fk_user, email, is_primary, created_at, updated_at) \
         VALUES ($1, $2, TRUE, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(email)
    .execute(pool)
    .await
    .ok();
    user_id
}

/// seed：default 策略类 + admin 用户与其 admin UA 绑定。
async fn seed_admin(pool: &PgPool) -> (i64, String) {
    sqlx::query("INSERT INTO isahl_auth.ngac_policy_class (o_name) VALUES ('default') ON CONFLICT DO NOTHING")
        .execute(pool)
        .await
        .ok();
    let pc: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("policy class");
    let email = "oblx-admin@test.local".to_string();
    let admin_user = ensure_user(pool, &email).await;
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
           VALUES ('admin', $1, NOW(), NOW())
           ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING"#,
    )
    .bind(pc)
    .execute(pool)
    .await
    .ok();
    let admin_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute \
         WHERE o_name='admin' AND fk_policy_class = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(pc)
    .fetch_one(pool)
    .await
    .expect("admin UA");
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING"#,
    )
    .bind(admin_user)
    .bind(admin_ua)
    .execute(pool)
    .await
    .ok();
    (admin_user, email)
}

async fn admin_token(admin: i64, email: &str) -> String {
    let state = test_auth_state();
    configure_token_validation(
        "http://localhost:9002".to_string(),
        "http://localhost:9002".to_string(),
    );
    encode_access_token(
        &Claims::new(&admin.to_string(), email, false),
        &state.jwt_private_key,
    )
    .expect("encode token")
}

async fn audit_count(pool: &PgPool, action: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.ngac_policy_audit_log \
         WHERE entity_type = 'ngac_obligation' AND action = $1 \
           AND new_values::text LIKE '%oblx-%'",
    )
    .bind(action)
    .fetch_one(pool)
    .await
    .expect("audit count")
}

#[tokio::test]
async fn obligation_crud_roundtrip_with_same_tx_audit() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    ensure_obligation_ddl(&pool).await;
    let _serial = SERIAL.lock().await;
    cleanup(&pool).await;
    let (admin, email) = seed_admin(&pool).await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(test_auth_state()))
            .configure(gateway_sso::admin::configure),
    )
    .await;
    let token = admin_token(admin, &email).await;

    // ① create（Rhai 字符串形态 + 静态算子数组形态并存）
    let created: Value = {
        let resp = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/admin/ngac/obligations")
                .insert_header(("Authorization", format!("Bearer {token}")))
                .set_json(json!({
                    "o_name": "oblx-roundtrip",
                    "event_pattern": "resource == \"oblxmod:1\"",
                    "response_ops": ["audit:log"],
                    "phase": "post",
                    "priority": 3,
                }))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 201, "create obligation");
        test::read_body_json(resp).await
    };
    // 64bit id MUST 字符串化（ID_JSON_PRECISION）
    let id_str = created["id"].as_str().expect("id MUST be string");
    let id: i64 = id_str.parse().expect("id parses");
    assert_eq!(created["phase"], "post");
    assert_eq!(created["priority"], 3);
    assert_eq!(created["response_ops"], json!(["audit:log"]));

    // ② list 可见
    let listed: Value = {
        let resp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/api/admin/ngac/obligations")
                .insert_header(("Authorization", format!("Bearer {token}")))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 200);
        test::read_body_json(resp).await
    };
    let rows = listed["obligations"].as_array().expect("rows array");
    let mine = rows
        .iter()
        .find(|r| r["o_name"] == "oblx-roundtrip")
        .expect("created row listed");
    assert_eq!(mine["id"].as_str(), Some(id_str));

    // ③ update（COALESCE 局部：只改 is_active/priority，其余保持）
    let updated: Value = {
        let resp = test::call_service(
            &app,
            test::TestRequest::put()
                .uri(&format!("/api/admin/ngac/obligations/{id}"))
                .insert_header(("Authorization", format!("Bearer {token}")))
                .set_json(json!({ "is_active": false, "priority": 9 }))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 200, "update obligation");
        test::read_body_json(resp).await
    };
    assert_eq!(updated["is_active"], false);
    assert_eq!(updated["priority"], 9);
    assert_eq!(
        updated["event_pattern"], "resource == \"oblxmod:1\"",
        "COALESCE：未提供字段 MUST 保持现值"
    );

    // ④ delete（软删）⇒ list 不再可见
    {
        let resp = test::call_service(
            &app,
            test::TestRequest::delete()
                .uri(&format!("/api/admin/ngac/obligations/{id}"))
                .insert_header(("Authorization", format!("Bearer {token}")))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 200, "delete obligation");
    }
    let after: Value = {
        let resp = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/api/admin/ngac/obligations")
                .insert_header(("Authorization", format!("Bearer {token}")))
                .to_request(),
        )
        .await;
        test::read_body_json(resp).await
    };
    assert!(
        !after["obligations"]
            .as_array()
            .expect("rows")
            .iter()
            .any(|r| r["o_name"] == "oblx-roundtrip"),
        "软删行 MUST NOT 出现在清单"
    );

    // ⑤ 同事务审计：insert/update/delete 各 1
    assert_eq!(audit_count(&pool, "insert").await, 1, "create 审计行");
    assert_eq!(audit_count(&pool, "update").await, 1, "update 审计行");
    assert_eq!(audit_count(&pool, "delete").await, 1, "delete 审计行");

    cleanup(&pool).await;
}

#[tokio::test]
async fn obligation_write_path_validates_expressions_fail_closed() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    ensure_obligation_ddl(&pool).await;
    let _serial = SERIAL.lock().await;
    cleanup(&pool).await;
    let (admin, email) = seed_admin(&pool).await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(test_auth_state()))
            .configure(gateway_sso::admin::configure),
    )
    .await;
    let token = admin_token(admin, &email).await;

    let post = |body: Value| {
        let token = token.clone();
        let app = &app;
        async move {
            test::call_service(
                app,
                test::TestRequest::post()
                    .uri("/api/admin/ngac/obligations")
                    .insert_header(("Authorization", format!("Bearer {token}")))
                    .set_json(body)
                    .to_request(),
            )
            .await
        }
    };

    // ① 非白名单标识符的 Rhai 源 ⇒ 400
    let r = post(json!({
        "o_name": "oblx-bad-ident",
        "event_pattern": "bogus_key == 1",
    }))
    .await;
    assert_eq!(r.status(), 400, "非法标识符 MUST 400");

    // ② response_ops 非法形态（对象）⇒ 400
    let r = post(json!({
        "o_name": "oblx-bad-ops",
        "response_ops": { "kind": "audit" },
    }))
    .await;
    assert_eq!(r.status(), 400, "非字符串/数组形态 MUST 400");

    // ③ 空字符串数组 ⇒ 400
    let r = post(json!({
        "o_name": "oblx-empty-ops",
        "response_ops": [],
    }))
    .await;
    assert_eq!(r.status(), 400, "空算子数组 MUST 400");

    // ④ 非法 phase ⇒ 400
    let r = post(json!({
        "o_name": "oblx-bad-phase",
        "phase": "middle",
    }))
    .await;
    assert_eq!(r.status(), 400, "非法 phase MUST 400");

    // ⑤ conditions 含未知键（含拼写错误）⇒ 400（tasks 1.5：核心对未知键 fail-closed ⇒ 写径 MUST 早拒）
    let r = post(json!({
        "o_name": "oblx-bad-cond",
        "conditions": { "bogus": 1 },
    }))
    .await;
    assert_eq!(r.status(), 400, "conditions 未知键 MUST 400");
    let r = post(json!({
        "o_name": "oblx-mis-typed",
        "conditions": { "not_befor": "2020-01-01T00:00:00Z" },
    }))
    .await;
    assert_eq!(
        r.status(),
        400,
        "conditions 键名拼写错误 MUST 400（MUST NOT 静默退化为无条件）"
    );

    // ⑥ 对照：合法键（v1 时间窗 + v2 属性闭包 + v3 rhai 组合）⇒ 通过（收口 MUST NOT 窄化合法语义）
    let r = post(json!({
        "o_name": "oblok-cond-ok",
        "event_pattern": "resource.starts_with(\"rt:\")",
        "conditions": {
            "not_before": "2020-01-01T00:00:00Z",
            "user_attr_in": ["ua-x"],
            "rhai": "true",
        },
    }))
    .await;
    assert_eq!(r.status(), 201, "合法 conditions MUST 通过（新建 201）");
    sqlx::query("DELETE FROM isahl_auth.ngac_obligation WHERE o_name = 'oblok-cond-ok'")
        .execute(&pool)
        .await
        .expect("cleanup ok-case");

    // ⑦ 全部拒绝 ⇒ 零落库、零审计
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.ngac_obligation WHERE o_name LIKE 'oblx-%'",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(n, 0, "校验失败 MUST NOT 落库");
    assert_eq!(
        audit_count(&pool, "insert").await,
        0,
        "校验失败 MUST NOT 写审计"
    );

    cleanup(&pool).await;
}
