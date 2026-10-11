//! 通用 NGAC 委托集成测试（add-ngac-delegation）
//!
//! 覆盖：
//! - 发起委托（直接持有 UA）→ 被委托人决策放行 → 撤销 → 即时失效
//! - 时间窗外不生效
//! - 链式委托（委托来源再委托）→ 400
//! - 重复 active 时间窗重叠 → 409
//! - me 列表双向隔离 + 服务端过滤
//! - 表 `isahl_auth.ngac_delegation` 由本文件幂等自建（测试基建模式；
//!   生产走运行时幂等 ensure（lib `ngac/ensure.rs`））
#![allow(clippy::type_complexity)]

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use serde_json::json;
use sqlx::PgPool;

async fn connect() -> PgPool {
    ::common::testing::connect_test_db().await
}

async fn ensure_table(pool: &PgPool) {
    // NGAC 运行时自愈（lib 唯一实现）
    gateway_sso::ngac::ensure::ensure_ngac_extension_tables(pool).await;
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

struct Fixture {
    u1: i64,
    u1_email: String,
    u2: i64,
    u2_email: String,
    u3: i64,
    u3_email: String,
    ua_id: i64,
    /// 本测试专属集合级资源类型（`{rt}:0`）——MUST NOT 跨测试共享：两个测试曾共用
    /// `deleggres:0`，一方 cleanup 删除该 OA 会令另一方 decide 失败（实测非确定性红，302 行）。
    rt: String,
    /// 本测试夹具后缀（UA/委托行命名基准；cleanup 按它限定，MUST NOT 用全局通配）。
    suffix: String,
}

async fn seed(pool: &PgPool, suffix: &str) -> Fixture {
    ensure_table(pool).await;
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_policy_class (o_name) VALUES ('default') ON CONFLICT DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("seed step");
    let pc: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("policy class");

    let mut ids = [0i64; 3];
    let mut emails = [String::new(), String::new(), String::new()];
    for (i, tag) in ["d1", "d2", "d3"].iter().enumerate() {
        let email = format!("deleg_{}_{}@alioth.test", tag, suffix);
        let username = format!("deleg_{}_{}", tag, suffix);
        sqlx::query(
            "INSERT INTO isahl_auth.auth_users (id, username, status, is_active, created_at, updated_at)
             VALUES (isahl.gen_next_zuid(), $1, 'active', true, NOW(), NOW())
             ON CONFLICT (username) DO NOTHING",
        )
        .bind(&username)
        .execute(pool)
        .await
        .expect("insert user");
        ids[i] =
            sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE username = $1 LIMIT 1")
                .bind(&username)
                .fetch_one(pool)
                .await
                .expect("fetch user");
        sqlx::query(
            "INSERT INTO isahl_auth.auth_user_emails (fk_user, email, is_primary, created_at, updated_at) \
             VALUES ($1, $2, TRUE, NOW(), NOW()) ON CONFLICT DO NOTHING",
        )
        .bind(ids[i])
        .bind(&email)
        .execute(pool)
        .await
        .ok();
        emails[i] = email;
    }

    // UA X + 关联（X → {rt}/read；资源类型按 suffix 唯一，避免跨测试共享）
    let rt = format!("deleggres_{suffix}");
    let ua_name = format!("deleg-target-{}", suffix);
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
         VALUES ($1, $2, NOW(), NOW())
         ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING",
    )
    .bind(&ua_name)
    .bind(pc)
    .execute(pool)
    .await
    .expect("seed step");
    let ua_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(&ua_name)
    .fetch_one(pool)
    .await
    .expect("target UA");
    // U1 直接持有 X
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at)
         VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(ids[0])
    .bind(ua_id)
    .execute(pool)
    .await
    .expect("seed step");
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at)
         VALUES ($1, $2, $3, 0, NOW(), NOW())
         ON CONFLICT (resource_type, fk_resource) DO UPDATE SET deleted_at = NULL",
    )
    .bind(format!("{rt}-oa"))
    .bind(pc)
    .bind(&rt)
    .execute(pool)
    .await
    .expect("seed step");
    let oa: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type=$1 AND fk_resource=0 LIMIT 1",
    )
    .bind(&rt)
    .fetch_one(pool)
    .await
    .expect("OA");
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ('read') ON CONFLICT (o_name) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("seed step");
    let ar: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='read' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("AR");
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_association (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at, updated_at)
         VALUES ($1, $2, ARRAY[$3], $4, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(ua_id)
    .bind(oa)
    .bind(ar)
    .bind(pc)
    .execute(pool)
    .await
    .expect("seed step");

    Fixture {
        u1: ids[0],
        u1_email: emails[0].clone(),
        u2: ids[1],
        u2_email: emails[1].clone(),
        u3: ids[2],
        u3_email: emails[2].clone(),
        ua_id,
        rt,
        suffix: suffix.to_string(),
    }
}

/// 按 **suffix** 清理该测试的全部夹具行——幂等且覆盖上轮崩溃残留。
///
/// 为什么必须按名字而非 id：夹具用户 id 每 run 用 `gen_next_zuid()` 新生成，但用户名
/// （`deleg_d{1,2,3}_{suffix}`）、UA 名（`deleg-target-{suffix}`）、资源类型（`deleggres_{suffix}`）
/// 跨 run 恒定。上轮崩溃残留的**同名用户/UA/委托行**会被本轮 `ON CONFLICT DO NOTHING` + 按名 SELECT
/// 复用 ⇒ 残留的活跃委托直接派生 UA，令「委托前 U2 无权限」这类断言**假放行**
///（实测：按 id 清理时确定性红于 267 行 `permitted:true`）。
async fn purge_fixture(pool: &PgPool, suffix: &str) {
    let usernames = vec![
        format!("deleg_d1_{suffix}"),
        format!("deleg_d2_{suffix}"),
        format!("deleg_d3_{suffix}"),
    ];
    let ua_name = format!("deleg-target-{suffix}");
    let rt = format!("deleggres_{suffix}");
    const STEPS: [&str; 6] = [
        // 委托（委托方/被委托方按用户名解析；另按目标 UA 兜底）
        "DELETE FROM isahl_auth.ngac_delegation WHERE fk_delegator IN \
           (SELECT id FROM isahl_auth.auth_users WHERE username = ANY($1)) \
         OR fk_delegatee IN (SELECT id FROM isahl_auth.auth_users WHERE username = ANY($1)) \
         OR fk_user_attribute IN (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $2)",
        // 绑定（按用户名解析 + 目标 UA）
        "DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user IN \
           (SELECT id FROM isahl_auth.auth_users WHERE username = ANY($1)) \
         OR fk_user_attribute IN (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $2)",
        // 关联（目标 UA）
        "DELETE FROM isahl_auth.ngac_association WHERE fk_user_attribute IN \
           (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $2)",
        // 目标 UA（按名——覆盖旧 id 残留）
        "DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name = $2",
        // 集合级 OA（按本测试资源类型）
        "DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type = $3 AND fk_resource = 0",
        // 用户 + 邮箱（按名）
        "DELETE FROM isahl_auth.auth_user_emails WHERE fk_user IN \
           (SELECT id FROM isahl_auth.auth_users WHERE username = ANY($1))",
    ];
    for sql in STEPS {
        let _ = sqlx::query(sql)
            .bind(&usernames)
            .bind(&ua_name)
            .bind(&rt)
            .execute(pool)
            .await;
    }
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username = ANY($1)")
        .bind(&usernames)
        .execute(pool)
        .await;
}

async fn cleanup(pool: &PgPool, f: &Fixture) {
    purge_fixture(pool, &f.suffix).await;
}

#[tokio::test]
async fn delegation_grant_revoke_and_boundaries() {
    let pool = connect().await;
    {
        // 开头清理：上轮崩溃留下的活跃委托仍在时间窗内会派生 UA（预检假放行实测）。
        // MUST 按 suffix 清（同名行跨 run 恒定），旧实现只清 "c1-pre" 前缀 ⇒ 清不到 "c1" 残留。
        purge_fixture(&pool, "c1").await;
    }
    let f = seed(&pool, "c1").await;
    let ast = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(
                web::scope("/api/ngac")
                    // 生产同构：RequireAuth 注入 Claims（fix-auth-token-precedence 收紧后必需）
                    .wrap(gateway_sso::auth::middleware::RequireAuth::new())
                    .configure(gateway_sso::ngac::pdp::configure_routes),
            ),
    )
    .await;
    let t1 = mint_token(&ast, f.u1, &f.u1_email);
    let t2 = mint_token(&ast, f.u2, &f.u2_email);
    let t3 = mint_token(&ast, f.u3, &f.u3_email);

    // 委托前 U2 无权限
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/pdp/decide")
            // 收紧后 decide 需 Bearer 且 sub==被问主体（enforce_decision_subject）
            .insert_header(("Authorization", format!("Bearer {}", t2)))
            .set_json(json!({"user_id": f.u2, "resource": format!("{}:0", f.rt), "action": "read"}))
            .to_request(),
    )
    .await;
    let st0 = resp.status();
    let raw = test::read_body(resp).await;
    let d: serde_json::Value = serde_json::from_slice(&raw).unwrap_or(serde_json::Value::Null);
    assert_eq!(
        d["permitted"],
        false,
        "委托前 U2 应无权限: status={} body={}",
        st0,
        String::from_utf8_lossy(&raw)
    );

    // U1 委托 X 给 U2（现在 → +1h）
    let now = chrono::Utc::now();
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/delegations")
            .insert_header(("Authorization", format!("Bearer {}", t1)))
            .set_json(json!({
                "fk_user_attribute": f.ua_id,
                "fk_delegatee": f.u2,
                "date_st": now,
                "date_ed": now + chrono::Duration::hours(1),
            }))
            .to_request(),
    )
    .await;
    let resp_status = resp.status();
    let resp_body: serde_json::Value = {
        let b = test::read_body(resp).await;
        serde_json::from_slice::<serde_json::Value>(&b).unwrap_or(serde_json::Value::Null)
    };
    assert_eq!(resp_status, 201, "发起委托应 201: body={:?}", resp_body);
    let created = resp_body;
    let del_id: i64 = created["id"].as_str().unwrap().parse().expect("id");

    // 委托后 U2 放行（PIP 派生）
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/pdp/decide")
            .insert_header(("Authorization", format!("Bearer {}", t2)))
            .set_json(json!({"user_id": f.u2, "resource": format!("{}:0", f.rt), "action": "read"}))
            .to_request(),
    )
    .await;
    let d: serde_json::Value = {
        let b = test::read_body(resp).await;
        serde_json::from_slice::<serde_json::Value>(&b).unwrap_or(serde_json::Value::Null)
    };
    assert_eq!(d["permitted"], true, "委托生效 U2 应放行: {:?}", d);

    // 链式委托：U2（委托来源）再委托给 U3 → 400
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/delegations")
            .insert_header(("Authorization", format!("Bearer {}", t2)))
            .set_json(json!({
                "fk_user_attribute": f.ua_id,
                "fk_delegatee": f.u3,
                "date_st": now,
                "date_ed": now + chrono::Duration::hours(1),
            }))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 400, "链式委托应 400");

    // 重复 active 重叠 → 409
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/delegations")
            .insert_header(("Authorization", format!("Bearer {}", t1)))
            .set_json(json!({
                "fk_user_attribute": f.ua_id,
                "fk_delegatee": f.u2,
                "date_st": now,
                "date_ed": now + chrono::Duration::hours(2),
            }))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 409, "重叠委托应 409");

    // me 列表双向隔离
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/ngac/delegations/me?direction=out")
            .insert_header(("Authorization", format!("Bearer {}", t1)))
            .to_request(),
    )
    .await;
    let out: serde_json::Value = {
        let b = test::read_body(resp).await;
        serde_json::from_slice::<serde_json::Value>(&b).unwrap_or(serde_json::Value::Null)
    };
    assert!(
        out.as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"].as_str() == Some(&del_id.to_string())),
        "U1 out 应含委托"
    );
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/ngac/delegations/me?direction=in")
            .insert_header(("Authorization", format!("Bearer {}", t2)))
            .to_request(),
    )
    .await;
    let in_: serde_json::Value = {
        let b = test::read_body(resp).await;
        serde_json::from_slice::<serde_json::Value>(&b).unwrap_or(serde_json::Value::Null)
    };
    assert!(
        in_.as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"].as_str() == Some(&del_id.to_string())),
        "U2 in 应含委托"
    );
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/ngac/delegations/me?direction=out")
            .insert_header(("Authorization", format!("Bearer {}", t3)))
            .to_request(),
    )
    .await;
    let out3: serde_json::Value = {
        let b = test::read_body(resp).await;
        serde_json::from_slice::<serde_json::Value>(&b).unwrap_or(serde_json::Value::Null)
    };
    assert!(
        !out3
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"].as_str() == Some(&del_id.to_string())),
        "U3 不应见该委托"
    );

    // 撤销 → 即时失效
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/api/ngac/delegations/{}/revoke", del_id))
            .insert_header(("Authorization", format!("Bearer {}", t1)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "撤销应 200");
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/pdp/decide")
            .insert_header(("Authorization", format!("Bearer {}", t2)))
            .set_json(json!({"user_id": f.u2, "resource": format!("{}:0", f.rt), "action": "read"}))
            .to_request(),
    )
    .await;
    let d: serde_json::Value = {
        let b = test::read_body(resp).await;
        serde_json::from_slice::<serde_json::Value>(&b).unwrap_or(serde_json::Value::Null)
    };
    assert_eq!(d["permitted"], false, "撤销后 U2 应无权限: {}", d);

    // 二次撤销 → 409
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/api/ngac/delegations/{}/revoke", del_id))
            .insert_header(("Authorization", format!("Bearer {}", t1)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 409, "重复撤销应 409");

    cleanup(&pool, &f).await;
}

#[tokio::test]
async fn delegation_outside_window_inactive() {
    let pool = connect().await;
    {
        // 同 c1：按 suffix 清本测试残留（旧实现清 "c2-pre" ⇒ 清不到 "c2"）
        purge_fixture(&pool, "c2").await;
    }
    let f = seed(&pool, "c2").await;
    let ast = common::test_auth_state();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .service(
                web::scope("/api/ngac")
                    .wrap(gateway_sso::auth::middleware::RequireAuth::new())
                    .configure(gateway_sso::ngac::pdp::configure_routes),
            ),
    )
    .await;

    // 过去时间窗（已过期）
    let now = chrono::Utc::now();
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_delegation \
         (fk_delegator, fk_delegatee, fk_user_attribute, date_st, date_ed) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(f.u1)
    .bind(f.u2)
    .bind(f.ua_id)
    .bind(now - chrono::Duration::hours(2))
    .bind(now - chrono::Duration::hours(1))
    .execute(&pool)
    .await
    .expect("expired delegation");

    let t_u2 = mint_token(&ast, f.u2, &f.u2_email);
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/ngac/pdp/decide")
            .insert_header(("Authorization", format!("Bearer {}", t_u2)))
            .set_json(json!({"user_id": f.u2, "resource": format!("{}:0", f.rt), "action": "read"}))
            .to_request(),
    )
    .await;
    let d: serde_json::Value = {
        let b = test::read_body(resp).await;
        serde_json::from_slice::<serde_json::Value>(&b).unwrap_or(serde_json::Value::Null)
    };
    assert_eq!(d["permitted"], false, "过期委托不应生效: {}", d);

    cleanup(&pool, &f).await;
}
