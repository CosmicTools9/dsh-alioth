//! deny-overrides 全局合并集成测试（fix-ngac-decision-consistency）
//!
//! 验证（change specs/ngac-pdp deny-overrides-combining / rls-decision-parity /
//! admin-governance-exemption）：
//! - 跨 (UA,OA) 对 allow + prohibition 冲突 → decide/explain 均 Deny（顺序无关）
//! - explain 与 decide 严格一致，steps 含被 deny 盖住的 matched allow 边
//! - admin 治理豁免优先于 prohibition（遍历前 Permit）
//! - RLS visible_ids 扣减 prohibition 行；conditions 未满足的 prohibition 不扣减

mod common;

use ::common::testing::connect_test_db;
use actix_web::body::MessageBody;
use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::ngac::pdp::list_resource_access;
use serde_json::json;
use sqlx::PgPool;

const RT: &str = "dotest-res";
const RT_RLS: &str = "dotest-rls";
const RT_ADMIN: &str = "dotest-admin";
const ROW_ID: i64 = 987654321;

struct Seed {
    pc: i64,
    user: i64,
}

async fn ensure_pc(pool: &PgPool) -> i64 {
    // 复用全局 'default' 策略类（ngac_policy_class.id 无序列默认值，新建需
    // gen_next_zuid；setup_schema / ensure_cognition_uas 已幂等保证 'default' 存在）
    sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default' LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("default policy class")
}

async fn ensure_ua(pool: &PgPool, pc: i64, name: &str) -> i64 {
    // 测试库 ngac_* 表可能经无约束重建（缺 id 序列默认值，ensure.rs 注释实证）——
    // 显式 isahl.gen_next_zuid() 主键（isahl_auth 属 zuid 域）。
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (id, o_name, fk_policy_class) \
         VALUES (isahl.gen_next_zuid(), $1, $2) \
         ON CONFLICT (o_name, fk_policy_class) WHERE deleted_at IS NULL DO NOTHING",
    )
    .bind(name)
    .bind(pc)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name=$1 AND fk_policy_class=$2 AND deleted_at IS NULL",
    )
    .bind(name)
    .bind(pc)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn ensure_oa(pool: &PgPool, pc: i64, resource_type: &str, fk_resource: i64) -> i64 {
    if let Some(id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type=$1 AND fk_resource=$2 AND deleted_at IS NULL",
    )
    .bind(resource_type)
    .bind(fk_resource)
    .fetch_optional(pool)
    .await
    .unwrap()
    {
        return id;
    }
    sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_object_attribute (id, o_name, fk_policy_class, resource_type, fk_resource) \
         VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4) RETURNING id",
    )
    .bind(format!(
        "{}-{}",
        resource_type,
        if fk_resource == 0 {
            "collection".to_string()
        } else {
            fk_resource.to_string()
        }
    ))
    .bind(pc)
    .bind(resource_type)
    .bind(fk_resource)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn ensure_ar(pool: &PgPool, name: &str) -> i64 {
    sqlx::query("INSERT INTO isahl_auth.ngac_access_right (id, o_name) VALUES (isahl.gen_next_zuid(), $1) ON CONFLICT DO NOTHING")
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_access_right WHERE o_name=$1")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn bind_user(pool: &PgPool, user_id: i64, ua: i64) {
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, username) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(format!("do-tester-{}", user_id))
    .bind(format!("do-test-{}", user_id))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (id, fk_user, fk_user_attribute) VALUES (isahl.gen_next_zuid(), $1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(ua)
    .execute(pool)
    .await
    .unwrap();
}

/// 播种：用户同时持有 allow UA（association 授予 read）与 deny UA（prohibition 禁止 read）。
async fn seed_conflict(pool: &PgPool, user_id: i64) -> Seed {
    let pc = ensure_pc(pool).await;
    let allow_ua = ensure_ua(pool, pc, "do-ua-allow").await;
    let deny_ua = ensure_ua(pool, pc, "do-ua-deny").await;
    let coll_oa = ensure_oa(pool, pc, RT, 0).await;
    let read_ar = ensure_ar(pool, "read").await;
    bind_user(pool, user_id, allow_ua).await;
    bind_user(pool, user_id, deny_ua).await;

    sqlx::query(
        "INSERT INTO isahl_auth.ngac_association (id, fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class) \
         VALUES (isahl.gen_next_zuid(), $1, $2, ARRAY[$3], $4) ON CONFLICT DO NOTHING",
    )
    .bind(allow_ua)
    .bind(coll_oa)
    .bind(read_ar)
    .bind(pc)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM isahl_auth.ngac_prohibition WHERE o_name = 'do-prohibit-read'")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_prohibition (id, o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active) \
         VALUES (isahl.gen_next_zuid(), $1, $2, $3, ARRAY[$4], TRUE)",
    )
    .bind("do-prohibit-read")
    .bind(deny_ua)
    .bind(coll_oa)
    .bind(read_ar)
    .execute(pool)
    .await
    .unwrap();

    Seed { pc, user: user_id }
}

async fn cleanup(pool: &PgPool, user_ids: &[i64]) {
    let ua_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name IN ('do-ua-allow','do-ua-deny','do-ua-admin-deny','do-rls-ua')",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM isahl_auth.ngac_prohibition WHERE fk_user_attribute = ANY($1)")
        .bind(&ua_ids)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE fk_user_attribute = ANY($1)")
        .bind(&ua_ids)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = ANY($1)")
        .bind(user_ids)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = ANY($1)")
        .bind(user_ids)
        .execute(pool)
        .await
        .unwrap();
}

async fn call_decide(pool: &PgPool, user_id: i64, resource: &str, action: &str) -> bool {
    let req = test::TestRequest::default().to_http_request();
    let resp = gateway_sso::ngac::pdp::ngac_decide(
        req,
        web::Data::new(pool.clone()),
        web::Json(ngac_contract::PdpCheckRequest {
            user_id,
            resource: resource.to_string(),
            action: action.to_string(),
        }),
    )
    .await;
    let bytes = resp.into_body().try_into_bytes().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    v["permitted"].as_bool().unwrap()
}

/// 跨对冲突：allow association（UA-A）+ prohibition（UA-B）→ Deny（遍历顺序无关）。
#[tokio::test]
async fn decide_deny_overrides_cross_pair() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let user_id = 999999999900201;
    let s = seed_conflict(&pool, user_id).await;

    assert!(
        !call_decide(&pool, s.user, &format!("{}:0", RT), "read").await,
        "allow + prohibition 跨对冲突必须 Deny（deny-overrides）"
    );

    cleanup(&pool, &[user_id]).await;
}

/// 对照：仅 association 无 prohibition → Permit。
#[tokio::test]
async fn decide_permit_without_prohibition() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let user_id = 999999999900202;
    let pc = ensure_pc(&pool).await;
    let allow_ua = ensure_ua(&pool, pc, "do-ua-allow").await;
    let coll_oa = ensure_oa(&pool, pc, RT, 0).await;
    let read_ar = ensure_ar(&pool, "read").await;
    bind_user(&pool, user_id, allow_ua).await;
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_association (id, fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class) \
         VALUES (isahl.gen_next_zuid(), $1, $2, ARRAY[$3], $4) ON CONFLICT DO NOTHING",
    )
    .bind(allow_ua)
    .bind(coll_oa)
    .bind(read_ar)
    .bind(pc)
    .execute(&pool)
    .await
    .unwrap();

    assert!(
        call_decide(&pool, user_id, &format!("{}:0", RT), "read").await,
        "仅 association 应 Permit"
    );

    cleanup(&pool, &[user_id]).await;
}

/// explain 与 decide 严格一致 + steps 完备（含被 deny 盖住的 matched allow 边）。
#[tokio::test]
async fn explain_matches_decide_with_full_steps() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let user_id = 999999999900203;
    let s = seed_conflict(&pool, user_id).await;

    // admin 调用方（explain 端点 require_admin）
    let admin_email = "do-explain-admin@test.local";
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, username, email, status, is_active, created_at, updated_at) \
         VALUES (999999999900204, 'do-explain-admin', 'do-explain-admin', $1, 'active', true, NOW(), NOW()) \
         ON CONFLICT (email) DO NOTHING",
    )
    .bind(admin_email)
    .execute(&pool)
    .await
    .unwrap();
    let admin_ua = ensure_ua(&pool, s.pc, "admin").await;
    bind_user(&pool, 999999999900204, admin_ua).await;

    let state = common::test_auth_state();
    configure_token_validation(
        "http://localhost:9002".to_string(),
        "http://localhost:9002".to_string(),
    );
    let token = encode_access_token(
        &Claims::new("999999999900204", admin_email, false),
        &state.jwt_private_key,
    )
    .expect("encode token");

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(common::test_auth_state()))
            .route(
                "/api/ngac/decide/explain",
                web::post().to(gateway_sso::ngac::pdp::ngac_decide_explain),
            ),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/decide/explain")
            .insert_header(("Authorization", format!("Bearer {}", token)))
            .set_json(json!({
                "user_id": user_id.to_string(),
                "resource": format!("{}:0", RT),
                "action": "read"
            }))
            .to_request(),
    )
    .await;
    assert!(resp.status().is_success(), "explain: {}", resp.status());
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["outcome"], "deny", "explain 与 decide 一致 Deny");
    assert_eq!(body["permitted"], false);
    let steps = body["steps"].as_array().expect("steps array");
    let has_matched_allow = steps
        .iter()
        .any(|s| s["rule_type"] == "association" && s["matched"] == true);
    let has_matched_deny = steps
        .iter()
        .any(|s| s["rule_type"] == "prohibition" && s["matched"] == true);
    assert!(has_matched_deny, "steps 必须含 matched prohibition");
    assert!(
        has_matched_allow,
        "explain 不早停：steps 必须含被 deny 盖住的 matched association"
    );

    cleanup(&pool, &[user_id, 999999999900204]).await;
}

/// admin 治理豁免 = 遍历后兜底（fix-ngac-decision-consistency）：
/// 显式 prohibition 对 admin 同样生效（deny-overrides）；仅无规则命中的资源放行。
#[tokio::test]
async fn admin_exemption_only_when_not_applicable() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let user_id = 999999999900205;
    let pc = ensure_pc(&pool).await;
    let admin_ua = ensure_ua(&pool, pc, "admin").await;
    let deny_ua = ensure_ua(&pool, pc, "do-ua-admin-deny").await;
    let coll_oa = ensure_oa(&pool, pc, RT_ADMIN, 0).await;
    let read_ar = ensure_ar(&pool, "read").await;
    bind_user(&pool, user_id, admin_ua).await;
    bind_user(&pool, user_id, deny_ua).await;
    // 保证非 bootstrap（全局至少一条 association）
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_association (id, fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class) \
         VALUES (isahl.gen_next_zuid(), $1, $2, ARRAY[$3], $4) ON CONFLICT DO NOTHING",
    )
    .bind(admin_ua)
    .bind(coll_oa)
    .bind(read_ar)
    .bind(pc)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM isahl_auth.ngac_prohibition WHERE o_name = 'do-prohibit-admin-read'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_prohibition (id, o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active) \
         VALUES (isahl.gen_next_zuid(), $1, $2, $3, ARRAY[$4], TRUE)",
    )
    .bind("do-prohibit-admin-read")
    .bind(deny_ua)
    .bind(coll_oa)
    .bind(read_ar)
    .execute(&pool)
    .await
    .unwrap();

    // 显式 prohibition 命中 → admin 也被拒（deny-overrides 优先于治理豁免）
    assert!(
        !call_decide(&pool, user_id, &format!("{}:0", RT_ADMIN), "read").await,
        "prohibition 必须约束 admin（deny-overrides）"
    );
    // 无规则命中的资源（无 OA）→ admin 遍历后兜底 Permit
    assert!(
        call_decide(&pool, user_id, "dotest-no-policy:0", "read").await,
        "admin 对无策略资源应豁免 Permit"
    );

    cleanup(&pool, &[user_id]).await;
}

/// RLS 同源：prohibition 行从 visible_ids 扣减；conditions 未满足不扣减。
#[tokio::test]
async fn rls_visible_ids_subtracts_prohibition() {
    let pool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let user_id = 999999999900206;
    let pc = ensure_pc(&pool).await;
    let ua = ensure_ua(&pool, pc, "do-rls-ua").await;
    let row_oa = ensure_oa(&pool, pc, RT_RLS, ROW_ID).await;
    let read_ar = ensure_ar(&pool, "read").await;
    bind_user(&pool, user_id, ua).await;
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_association (id, fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class) \
         VALUES (isahl.gen_next_zuid(), $1, $2, ARRAY[$3], $4) ON CONFLICT DO NOTHING",
    )
    .bind(ua)
    .bind(row_oa)
    .bind(read_ar)
    .bind(pc)
    .execute(&pool)
    .await
    .unwrap();

    async fn call_list(pool: &PgPool, user_id: i64, resource_type: &str) -> serde_json::Value {
        let resp = list_resource_access(
            web::Data::new(pool.clone()),
            web::Json(ngac_contract::PdpListRequest {
                user_id,
                resource_type: resource_type.to_string(),
                action: "read".to_string(),
            }),
        )
        .await;
        let bytes = resp.into_body().try_into_bytes().unwrap();
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    }

    // 基线：仅 association → 行可见
    let body = call_list(&pool, user_id, RT_RLS).await;
    let visible: Vec<i64> = body["visible_ids"]
        .as_array()
        .expect("visible_ids array")
        .iter()
        .filter_map(|v| {
            v.as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| v.as_i64())
        })
        .collect();
    assert!(visible.contains(&ROW_ID), "基线行应可见: {:?}", visible);

    // 加 prohibition → 行被扣减
    sqlx::query("DELETE FROM isahl_auth.ngac_prohibition WHERE o_name = 'do-prohibit-rls-read'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_prohibition (id, o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active) \
         VALUES (isahl.gen_next_zuid(), $1, $2, $3, ARRAY[$4], TRUE)",
    )
    .bind("do-prohibit-rls-read")
    .bind(ua)
    .bind(row_oa)
    .bind(read_ar)
    .execute(&pool)
    .await
    .unwrap();
    let body = call_list(&pool, user_id, RT_RLS).await;
    let visible: Vec<i64> = body["visible_ids"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| {
                    v.as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| v.as_i64())
                })
                .collect()
        })
        .unwrap_or_default();
    assert!(
        !visible.contains(&ROW_ID),
        "prohibition 命中的行 MUST NOT 出现在 visible_ids: {:?}",
        visible
    );

    // conditions 未满足（not_before 在未来）→ prohibition 不生效 → 行恢复可见
    sqlx::query(
        "UPDATE isahl_auth.ngac_prohibition SET conditions = '{\"not_before\": \"2999-01-01T00:00:00Z\"}'::jsonb \
         WHERE o_name = 'do-prohibit-rls-read'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let body = call_list(&pool, user_id, RT_RLS).await;
    let visible: Vec<i64> = body["visible_ids"]
        .as_array()
        .expect("visible_ids array")
        .iter()
        .filter_map(|v| {
            v.as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| v.as_i64())
        })
        .collect();
    assert!(
        visible.contains(&ROW_ID),
        "conditions 未满足的 prohibition 不得扣减: {:?}",
        visible
    );

    cleanup(&pool, &[user_id]).await;
}
