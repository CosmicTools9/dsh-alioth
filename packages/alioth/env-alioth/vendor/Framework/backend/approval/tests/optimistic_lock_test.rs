//! 并发编辑乐观锁集成测试（fix-flow-designer-editing-gaps E2）
//!
//! 守护可观察行为：
//! 1. PUT /approval-flows/{id} 携带匹配的 expected_updated_at → 200，updated_at 前进
//!    （表无 updated_at 触发器，维护责任在本 UPDATE 语句）；
//! 2. 携带陈旧 expected_updated_at（他人已保存）→ 409 Conflict + 冲突提示，
//!    已有修改不被覆盖；
//! 3. 不带 expected_updated_at → 200（存量客户端向后兼容，无锁直写）。

use ::common::context::RequestContext;
use ::common::testing::connect_test_db;
use actix_web::{dev::Service, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
mod common;

const USER_ID: i64 = 424343;

fn test_ctx() -> RequestContext {
    RequestContext::with_username(USER_ID, "optlock@test.local", "optlock-test")
}

/// PUT /approval-flows/{id} 便捷宏（避免命名 TestService 具体类型）；
/// 返回 (status, body_json)
macro_rules! put_update {
    ($app:expr, $id:expr, $name:expr, $expected:expr) => {{
        let mut body = json!({ "name": $name, "meta": { "version": 1, "nodes": [] } });
        if let Some(exp) = $expected {
            body["expected_updated_at"] = json!(exp);
        }
        let req = test::TestRequest::put()
            .uri(&format!("/test/approval-flows/{}", $id))
            .set_json(&body)
            .to_request();
        let resp = $app.call(req).await.unwrap();
        let status = resp.status().as_u16();
        let bytes = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let val: Value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, val)
    }};
}

/// 种子：用户 + 设计·实例行（↑_NA 坐标）；返回 (flow_id, updated_at RFC3339)。
async fn seed_flow(pool: &sqlx::PgPool, code: &str) -> (i64, String) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'optlock-test', 'optlock-test', 'optlock@test.local',
                   'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(USER_ID)
    .execute(pool)
    .await
    .unwrap();

    let (scene_id, factor_id, fn_id): (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT id FROM isahl.zc_id_scene WHERE code = 'JC' AND deleted_at IS NULL LIMIT 1),
             (SELECT id FROM isahl.zc_id_factor WHERE code = 'FTA' AND deleted_at IS NULL LIMIT 1),
             (SELECT id FROM isahl.zc_id_function WHERE code = '↑_NA' AND deleted_at IS NULL LIMIT 1)"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();

    let row: (i64, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        r#"INSERT INTO isahl."zc_id_proc-approve"
           (notice, code, comments, updated_at, dk_scene, dk_factor, dk_function,
            created_by_id, _f_, _t_)
           VALUES ($1, $2, 'optlock seed', NOW(), $3, $4, $5, $6, '设计', '实例')
           RETURNING id, updated_at"#,
    )
    .bind(format!("optlock-{code}"))
    .bind(code)
    .bind(scene_id)
    .bind(factor_id)
    .bind(fn_id)
    .bind(USER_ID)
    .fetch_one(pool)
    .await
    .unwrap();
    (row.0, row.1.map(|t| t.to_rfc3339()).unwrap_or_default())
}

#[tokio::test]
async fn optimistic_lock_conflict_and_advance() {
    let pool = connect_test_db().await;
    common::setup_test_schema(&pool).await.unwrap();
    let code = common::test_code("OPTLOCK");
    let (id, initial_updated) = seed_flow(&pool, &code).await;
    common::grant_user_access(&pool, USER_ID, "approval_flows", &["update"])
        .await
        .unwrap();

    let ctx = test_ctx();
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(handlers::approval_flow::register),
        ),
    )
    .await;

    // 1. 匹配期望值 → 200，updated_at 前进（表无触发器，语句内 SET NOW()）
    let (s1, v1) = put_update!(app, id, "optlock-v1", Some(&initial_updated));
    assert_eq!(s1, 200, "匹配期望值必须放行：{v1}");
    let after_first: String =
        sqlx::query_scalar("SELECT updated_at::text FROM isahl.zc_id_process WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_ne!(
        after_first, initial_updated,
        "updated_at 必须前进（乐观锁基准）"
    );

    // 2. 陈旧期望值（initial 已被第一次保存超越）→ 409 冲突，修改不被覆盖
    let (s2, v2) = put_update!(app, id, "optlock-stale", Some(&initial_updated));
    assert_eq!(s2, 409, "陈旧期望值必须 409：{v2}");
    let name_now: String =
        sqlx::query_scalar("SELECT notice FROM isahl.zc_id_process WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(name_now, "optlock-v1", "冲突写不得覆盖已有修改");

    // 3. 不带期望值 → 200（存量客户端向后兼容）
    let (s3, v3) = put_update!(app, id, "optlock-nolock", None::<&str>);
    assert_eq!(s3, 200, "无锁直写兼容存量客户端：{v3}");

    // 清理
    sqlx::query("DELETE FROM isahl.zc_id_process WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
}
