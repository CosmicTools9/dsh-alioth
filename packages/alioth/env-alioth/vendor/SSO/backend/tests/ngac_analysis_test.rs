//! NGAC 图分析面集成测试（openspec change `establish-age-graph-analysis-plane` 任务 3.x/5.3）
//!
//! 覆盖：
//!   1. SoD 冲突检测：双侧闭包命中 / 单侧不误报；`mode` 与 fail-visible 字段（projection/stale）
//!   2. 可达性枚举：授权边命中（含 UA 名） / 无授权 action ⇒ 空
//!   3. explain-paths：响应契约（paths/paths_degraded/projection）
//!   4. dry-run（任务 5.2/5.3）：克隆图前后翻转（before/after/flipped）+ 零写库（版本与表行不变）
//!      + **等价性守门**：dry-run 预测的 after 与真实写库后 `require_resource_access` 决策一致
//!
//! 数据约定：`anx-` 前缀，测试前后双清理（与 tests/common 的"测试数据绝不残留"原则一致）。

mod common;

use ::common::testing::connect_test_db;
use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};

/// 文件级串行化（共享 `anx-*` 场景夹具；并行互相清空 ⇒ 非确定性红）。
static SERIAL_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn lock_serial() -> MutexGuard<'static, ()> {
    SERIAL_LOCK.lock().await
}

struct Seed {
    admin_user: i64,
    ua_a: i64,
    ua_b: i64,
    user_both: i64,
    user_single: i64,
    oa_id: i64,
    email: String,
}

const ADMIN_EMAIL: &str = "anx-admin@test.local";
const BOTH_EMAIL: &str = "anx-both@test.local";
const SINGLE_EMAIL: &str = "anx-single@test.local";

async fn ensure_user(pool: &PgPool, email: &str) -> i64 {
    let username = email.split('@').next().unwrap_or("anx").replace('-', "_");
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users (id, username, status, is_active, created_at, updated_at)
           VALUES (isahl.gen_next_zuid(), $1, 'active', true, NOW(), NOW())
           ON CONFLICT (username) DO NOTHING"#,
    )
    .bind(&username)
    .execute(pool)
    .await
    .expect("ensure user");
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE username = $1 LIMIT 1")
            .bind(&username)
            .fetch_one(pool)
            .await
            .expect("fetch user");
    sqlx::query(
        "INSERT INTO isahl_auth.auth_user_emails (fk_user, email, is_primary, created_at, updated_at) \
         VALUES ($1, $2, TRUE, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(email)
    .execute(pool)
    .await
    .expect("ensure email");
    user_id
}

async fn ensure_ua(pool: &PgPool, pc: i64, name: &str) -> i64 {
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (id, o_name, fk_policy_class) \
         VALUES (isahl.gen_next_zuid(), $1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(name)
    .bind(pc)
    .execute(pool)
    .await
    .expect("ensure ua");
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .expect("fetch ua")
}

async fn assign_ua(pool: &PgPool, user: i64, ua: i64) {
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (id, o_name, fk_user, fk_user_attribute, assigned_at) \
         VALUES (isahl.gen_next_zuid(), $1, $2, $3, NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(format!("anx-assign-{user}-{ua}"))
    .bind(user)
    .bind(ua)
    .execute(pool)
    .await
    .expect("assign ua");
}

async fn cleanup(pool: &PgPool) {
    for sql in [
        "DELETE FROM isahl_auth.ngac_sod_pair WHERE o_name LIKE 'anx-%'",
        "DELETE FROM isahl_auth.ngac_association WHERE o_name LIKE 'anx-%'",
        "DELETE FROM isahl_auth.ngac_prohibition WHERE o_name LIKE 'anx-%'",
        "DELETE FROM isahl_auth.ngac_object_attribute WHERE o_name LIKE 'anx-%'",
        "DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE o_name LIKE 'anx-%' OR o_name LIKE 'anx-assign-%'",
        "DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'anx-%'",
        "DELETE FROM isahl_auth.auth_user_emails WHERE email LIKE 'anx-%@test.local'",
        "DELETE FROM isahl_auth.auth_users WHERE username LIKE 'anx_%'",
    ] {
        let _ = sqlx::query(sql).execute(pool).await;
    }
}

async fn seed(pool: &PgPool) -> Seed {
    // default 策略类（seed 链已建；防御性 ensure）
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_policy_class (id, o_name) \
         VALUES (isahl.gen_next_zuid(), 'default') ON CONFLICT (o_name) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("ensure policy class");
    let pc: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default'")
            .fetch_one(pool)
            .await
            .expect("fetch policy class");

    let admin_user = ensure_user(pool, ADMIN_EMAIL).await;
    let user_both = ensure_user(pool, BOTH_EMAIL).await;
    let user_single = ensure_user(pool, SINGLE_EMAIL).await;

    let ua_a = ensure_ua(pool, pc, "anx-teller").await;
    let ua_b = ensure_ua(pool, pc, "anx-auditor").await;
    let admin_ua = ensure_ua(pool, pc, "admin").await;
    assign_ua(pool, admin_user, admin_ua).await;
    assign_ua(pool, user_both, ua_a).await;
    assign_ua(pool, user_both, ua_b).await;
    assign_ua(pool, user_single, ua_a).await;

    // 实例 OA + read 授权边（可达性与 dry-run 用）
    let oa_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_object_attribute (id, o_name, fk_policy_class, resource_type, fk_resource) \
         VALUES (isahl.gen_next_zuid(), 'anx-oa', $1, 'anx-widget', 4242) RETURNING id",
    )
    .bind(pc)
    .fetch_one(pool)
    .await
    .expect("seed oa");
    let read_ar: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'read'")
            .fetch_one(pool)
            .await
            .expect("read right");
    let _assoc_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_association (id, o_name, fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights) \
         VALUES (isahl.gen_next_zuid(), 'anx-assoc', $1, $2, $3, ARRAY[$4]::bigint[]) RETURNING id",
    )
    .bind(ua_a)
    .bind(oa_id)
    .bind(pc)
    .bind(read_ar)
    .fetch_one(pool)
    .await
    .expect("seed association");

    Seed {
        admin_user,
        ua_a,
        ua_b,
        user_both,
        user_single,
        oa_id,
        email: ADMIN_EMAIL.to_string(),
    }
}

fn test_auth_state() -> AuthState {
    common::test_auth_state()
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

async fn app(
    pool: PgPool,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse,
    Error = actix_web::Error,
> {
    test::init_service(
        App::new()
            .app_data(web::Data::new(pool))
            .app_data(web::Data::new(test_auth_state()))
            // 生产同构挂载（fix-ngac-admin-route-mount）：`lib.rs::configure_protected_routes`
            // 直接 `.configure(admin::configure)`；NGAC 家族已注册在 admin::configure 内部的
            // `/api/admin` scope 内 ⇒ 这里 MUST NOT 再包一层 `/api/admin`（历史双前缀已移除）。
            .configure(gateway_sso::admin::configure),
    )
    .await
}

async fn post_json<S>(app: &S, uri: &str, token: &str, body: Value) -> (u16, Value)
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse,
        Error = actix_web::Error,
    >,
{
    let req = test::TestRequest::post()
        .uri(uri)
        .insert_header(("Authorization", format!("Bearer {token}")))
        .set_json(body)
        .to_request();
    let resp = test::call_service(app, req).await;
    let status = resp.status().as_u16();
    let bytes = test::read_body(resp).await;
    let body: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("deserialize body")
    };
    (status, body)
}

/// SoD：双侧闭包命中 / 单侧不误报 / fail-visible 字段在场。
#[tokio::test]
async fn sod_hits_both_sides_only() {
    let _g = lock_serial().await;
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    cleanup(&pool).await;
    let s = seed(&pool).await;

    // 配置冲突对
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_sod_pair (id, o_name, fk_ua_a, fk_ua_b) \
         VALUES (isahl.gen_next_zuid(), 'anx-柜员×审计', $1, $2)",
    )
    .bind(s.ua_a)
    .bind(s.ua_b)
    .execute(&pool)
    .await
    .expect("seed sod pair");

    let svc = app(pool.clone()).await;
    let token = admin_token(s.admin_user, &s.email).await;
    let (status, body) = post_json(&svc, "/api/admin/ngac/analysis/sod", &token, json!({})).await;
    assert_eq!(status, 200, "sod 分析响应：{body}");
    assert!(
        body["projection"].is_object(),
        "fail-visible：projection 在场"
    );
    assert!(body["stale"].is_boolean());
    let mode = body["mode"].as_str().unwrap_or_default();
    assert!(
        mode == "sql",
        "mode 恒 sql（关系单源；NGAC 侧图投影已退役）（实际 {mode}）"
    );
    let hits = body["hits"].as_array().expect("hits 数组");
    let both_hit = hits.iter().any(|h| {
        h["user_id"]
            .as_i64()
            .or(h["user_id"].as_str().and_then(|v| v.parse().ok()))
            == Some(s.user_both)
    });
    let single_hit = hits.iter().any(|h| {
        h["user_id"]
            .as_i64()
            .or(h["user_id"].as_str().and_then(|v| v.parse().ok()))
            == Some(s.user_single)
    });
    assert!(both_hit, "双侧闭包用户必须命中（hits={hits:?}）");
    assert!(!single_hit, "单侧持有不得误报（hits={hits:?}）");

    cleanup(&pool).await;
}

/// 可达性：授权边命中 + 无授权 action ⇒ 空。
#[tokio::test]
async fn reachability_enumerates_via_association() {
    let _g = lock_serial().await;
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    cleanup(&pool).await;
    let s = seed(&pool).await;

    let svc = app(pool.clone()).await;
    let token = admin_token(s.admin_user, &s.email).await;

    let (status, body) = post_json(
        &svc,
        "/api/admin/ngac/analysis/reachability",
        &token,
        json!({ "resource_type": "anx-widget", "fk_resource": 4242, "action": "read" }),
    )
    .await;
    assert_eq!(status, 200, "可达性响应：{body}");
    let reachable = body["reachable"].as_array().expect("reachable 数组");
    let ids: Vec<Option<i64>> = reachable
        .iter()
        .map(|r| {
            r["user_id"]
                .as_i64()
                .or(r["user_id"].as_str().and_then(|v| v.parse().ok()))
        })
        .collect();
    assert!(
        ids.contains(&Some(s.user_single)),
        "持 ua_a 用户可达（ids={ids:?}）"
    );
    assert!(
        ids.contains(&Some(s.user_both)),
        "双侧用户经 ua_a 可达（ids={ids:?}）"
    );

    let (status, body) = post_json(
        &svc,
        "/api/admin/ngac/analysis/reachability",
        &token,
        json!({ "resource_type": "anx-widget", "fk_resource": 4242, "action": "delete" }),
    )
    .await;
    assert_eq!(status, 200);
    assert!(
        body["reachable"]
            .as_array()
            .map(|a| a.is_empty())
            .unwrap_or(false),
        "无授权 action ⇒ 空可达集"
    );

    cleanup(&pool).await;
}

/// explain-paths：响应契约（paths / paths_degraded / projection）。
#[tokio::test]
async fn explain_paths_contract() {
    let _g = lock_serial().await;
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    cleanup(&pool).await;
    let s = seed(&pool).await;

    let svc = app(pool.clone()).await;
    let token = admin_token(s.admin_user, &s.email).await;
    let (status, body) = post_json(
        &svc,
        "/api/admin/ngac/analysis/explain-paths",
        &token,
        json!({ "resource": "anx-widget:4242", "action": "read" }),
    )
    .await;
    assert_eq!(status, 200, "explain-paths 响应：{body}");
    assert!(body["paths_degraded"].is_boolean(), "降级显式标注");
    assert!(body["projection"].is_object());
    if body["paths_degraded"] == json!(false) {
        let paths = body["paths"].as_array().expect("paths 数组");
        assert!(
            paths.iter().any(|p| p["kind"] == json!("ASSOCIATION")),
            "AGE 就绪时种子边应出现在路径证据（paths={paths:?}）"
        );
    }

    cleanup(&pool).await;
}

/// dry-run（任务 5.2/5.3 等价性守门）：克隆图翻转 + 零写库 + 真实写库后决策一致。
#[tokio::test]
async fn dry_run_flips_and_matches_real_write() {
    let _g = lock_serial().await;
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    cleanup(&pool).await;
    let s = seed(&pool).await;

    let svc = app(pool.clone()).await;
    let token = admin_token(s.admin_user, &s.email).await;

    let version_before: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM isahl_auth.ngac_policy_version")
            .fetch_one(&pool)
            .await
            .expect("version before");

    // 前提：当前 user_single 对 anx-widget:4242 read = 允许（ua_a 授权边）
    let permitted =
        common::permissions_probe(&pool, s.user_single, "anx-widget", 4242, "read").await;
    assert!(permitted, "种子授权边生效（dry-run 的 before 前提）");

    // dry-run：补丁 = 对 (ua_a, oa) 加 read prohibition（id 9001 克隆图专用，不落库）
    let (status, body) = post_json(
        &svc,
        "/api/admin/ngac/analysis/dry-run",
        &token,
        json!({
            "patches": [{ "kind": "prohibition", "remove": false, "id": 9001,
                          "fk_user_attribute": s.ua_a, "fk_object_attribute": s.oa_id,
                          "rights": ["read"], "is_active": true }],
            "checks": [{ "user_id": s.user_single, "resource_type": "anx-widget",
                         "fk_resource": 4242, "action": "read" }],
        }),
    )
    .await;
    assert_eq!(status, 200, "dry-run 响应：{body}");
    assert_eq!(body["writes"], json!(0), "克隆图求值零写库");
    let result = &body["results"][0];
    assert_eq!(result["before"], json!("permit"), "补丁前允许：{result:?}");
    assert_eq!(result["after"], json!("deny"), "补丁后拒绝：{result:?}");
    assert_eq!(result["flipped"], json!(true));

    // 零写库复核：版本与 prohibition 表均未变化
    let version_after: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM isahl_auth.ngac_policy_version")
            .fetch_one(&pool)
            .await
            .expect("version after");
    assert_eq!(version_before, version_after, "dry-run 不 bump 版本");
    let proh: i64 =
        sqlx::query_scalar("SELECT count(*) FROM isahl_auth.ngac_prohibition WHERE id = 9001")
            .fetch_one(&pool)
            .await
            .expect("prohibition count");
    assert_eq!(proh, 0, "克隆图补丁 id 不落库");

    // 等价性（任务 5.3）：真实写库同款 prohibition → 真决策 = dry-run 预测的 after
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_prohibition (id, o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active) \
         VALUES (isahl.gen_next_zuid(), 'anx-proh', $1, $2, ARRAY[(SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='read')]::bigint[], true)",
    )
    .bind(s.ua_a)
    .bind(s.oa_id)
    .execute(&pool)
    .await
    .expect("真实写入 prohibition");
    sqlx::query("UPDATE isahl_auth.ngac_policy_version SET version = version + 1 WHERE id = 1")
        .execute(&pool)
        .await
        .expect("bump 版本");
    let permitted_after =
        common::permissions_probe(&pool, s.user_single, "anx-widget", 4242, "read").await;
    assert!(
        !permitted_after,
        "真实写库后决策与 dry-run 预测一致（deny）"
    );

    cleanup(&pool).await;
}
