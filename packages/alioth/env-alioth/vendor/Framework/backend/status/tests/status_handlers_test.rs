//! status typed 维护面（crate 级 handler）— CRUD 四径 + 鉴权/NGAC 拒绝 + 留痕 — 集成测试（真实 PG，测试库）。
//!
//! 运行：`cargo test -p status`
//! 前置：测试库已按 005_seed_ngac_and_policy_class.sql 种子（admin UA 在场）；本测试自建
//! fixture 用户（前缀 `stdo_`），admin 夹具绑定 admin UA 使 `require_resource_access`
//! 经 admin 治理兜底放行；非 admin 夹具验证 NGAC 拒绝路径（与 dict 写入留痕测试同款夹具）。

use actix_web::{web, HttpMessage, HttpRequest};
use common::context::RequestContext;
use common::data::ListQuery;
use common::testing::{connect_test_db, setup_test_schema_light};
use serde_json::{json, Value};
use sqlx::PgPool;
use status::handlers::status::{
    create_status, delete_status, get_status, list_statuses, update_status, CreateStatusReq,
    UpdateStatusReq,
};

const TABLE: &str = "zc_id_stus-project";

/// fixture 用户（幂等）：auth_users 行；`bind_admin` 时绑定 admin UA（prefix `stdo_`）。
async fn fixture_user(pool: &PgPool, name: &str, bind_admin: bool) -> i64 {
    let username = format!("stdo_{name}");
    // 并发用例共用 fixture 名：INSERT ... ON CONFLICT DO NOTHING（不写 UPDATE，免授权假设）+ 回读
    let user_id = match sqlx::query_scalar(
        "INSERT INTO isahl_auth.auth_users (username) VALUES ($1) \
         ON CONFLICT (username) DO NOTHING RETURNING id",
    )
    .bind(&username)
    .fetch_optional(pool)
    .await
    .expect("insert fixture user")
    {
        Some(id) => id,
        None => sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE username = $1")
            .bind(&username)
            .fetch_one(pool)
            .await
            .expect("read fixture user"),
    };
    if bind_admin {
        let admin_ua: i64 = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' AND deleted_at IS NULL ORDER BY id LIMIT 1",
        )
        .fetch_one(pool)
        .await
        .expect("admin UA 须已种子（005_seed_ngac_and_policy_class.sql / bootstrap）");
        let bound: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_user_rr_attribute \
             WHERE fk_user = $1 AND fk_user_attribute = $2 AND deleted_at IS NULL LIMIT 1",
        )
        .bind(user_id)
        .bind(admin_ua)
        .fetch_optional(pool)
        .await
        .expect("read admin binding");
        if bound.is_none() {
            sqlx::query(
                "INSERT INTO isahl_auth.ngac_user_rr_attribute (o_name, fk_user, fk_user_attribute) VALUES ($1, $2, $3)",
            )
            .bind(format!("stdo_bind_{user_id}"))
            .bind(user_id)
            .bind(admin_ua)
            .execute(pool)
            .await
            .expect("insert admin binding");
        }
    }
    user_id
}

fn authed_req(user_id: i64) -> HttpRequest {
    let req = actix_web::test::TestRequest::default().to_http_request();
    req.extensions_mut().insert(RequestContext::with_username(
        user_id,
        format!("stdo_{user_id}@test.local"),
        format!("stdo_{user_id}"),
    ));
    req
}

async fn cleanup_row(pool: &PgPool, id: i64) {
    sqlx::query("DELETE FROM isahl_audit.audit_events WHERE object_path = $1")
        .bind(format!("status:{}:{}", TABLE, id))
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type = 'status' AND fk_resource = $1",
    )
    .bind(id)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl.\"zc_id_stus-project\" WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .ok();
}

async fn audit_rows(pool: &PgPool, id: i64, operation: &str) -> Vec<Value> {
    sqlx::query_scalar::<_, Value>(
        "SELECT metadata FROM isahl_audit.audit_events \
         WHERE operation = $1 AND object_path = $2 ORDER BY created_at",
    )
    .bind(operation)
    .bind(format!("status:{}:{}", TABLE, id))
    .fetch_all(pool)
    .await
    .expect("read audit events")
}

async fn body_json(resp: actix_web::HttpResponse) -> Value {
    let bytes = actix_web::body::to_bytes(resp.into_body())
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("payload 应为 JSON")
}

#[tokio::test]
async fn crud_roundtrip_with_audit_trail() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let uid = fixture_user(&pool, "crud_tester", true).await;
    let marker = format!("stdo-crud-{}", std::process::id());

    // ── create：NGAC create 检查 + 行级锚点注册 + 记新值 ──
    let resp = create_status(
        web::Data::new(pool.clone()),
        authed_req(uid),
        web::Json(CreateStatusReq {
            name: Some(marker.clone()),
            flag: Some("start".to_string()),
            comments: Some("留痕用例".to_string()),
        }),
    )
    .await
    .expect("create 应成功");
    assert_eq!(resp.status().as_u16(), 201);
    let id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl.\"zc_id_stus-project\" WHERE notice = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(&marker)
    .fetch_one(&pool)
    .await
    .expect("read created row");
    let created = audit_rows(&pool, id, "status.create").await;
    assert_eq!(created.len(), 1, "create 应落一条 status.create 留痕");
    assert_eq!(created[0]["table"], TABLE);
    assert_eq!(created[0]["id"], id.to_string());
    assert_eq!(created[0]["actor"]["username"], format!("stdo_{uid}"));
    assert_eq!(created[0]["after"]["name"], marker);

    // ── list / get：读径（name 别名投影 = notice） ──
    let resp = list_statuses(
        web::Data::new(pool.clone()),
        authed_req(uid),
        web::Query(ListQuery {
            page: 1,
            page_size: 200,
            ..Default::default()
        }),
    )
    .await
    .expect("list 应成功");
    assert_eq!(resp.status().as_u16(), 200);
    let v = body_json(resp).await;
    let items = v["data"]["items"].as_array().expect("data.items 应为数组");
    let mine = items
        .iter()
        .find(|it| it["id"].as_str() == Some(id.to_string().as_str()))
        .expect("list 应含新建行");
    assert_eq!(mine["name"], marker, "name 应为 notice 别名投影");

    let resp = get_status(
        web::Data::new(pool.clone()),
        authed_req(uid),
        web::Path::from(id),
    )
    .await
    .expect("get 应成功");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(body_json(resp).await["data"]["name"], marker);

    // ── update：记前后值 ──
    let resp = update_status(
        web::Data::new(pool.clone()),
        web::Path::from(id),
        authed_req(uid),
        web::Json(UpdateStatusReq {
            name: Some(format!("{marker}-u")),
            flag: Some("doing".to_string()),
            comments: None,
        }),
    )
    .await
    .expect("update 应成功");
    assert_eq!(resp.status().as_u16(), 200);
    let updated = audit_rows(&pool, id, "status.update").await;
    assert_eq!(updated.len(), 1, "update 应落一条 status.update 留痕");
    assert_eq!(updated[0]["before"]["name"], marker);
    assert_eq!(updated[0]["after"]["name"], format!("{marker}-u"));

    // ── delete（软删）：记被删行摘要 ──
    let resp = delete_status(
        web::Data::new(pool.clone()),
        web::Path::from(id),
        authed_req(uid),
    )
    .await
    .expect("delete 应成功");
    assert_eq!(resp.status().as_u16(), 200);
    let deleted_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT deleted_at FROM isahl.\"zc_id_stus-project\" WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("read deleted_at");
    assert!(deleted_at.is_some(), "delete 应为软删");
    let deleted = audit_rows(&pool, id, "status.delete").await;
    assert_eq!(deleted.len(), 1, "delete 应落一条 status.delete 留痕");
    assert_eq!(deleted[0]["before"]["name"], format!("{marker}-u"));

    cleanup_row(&pool, id).await;
}

#[tokio::test]
async fn rejected_writes_leave_no_success_event() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let marker = format!("stdo-rej-{}", std::process::id());

    // 未鉴权（无 RequestContext）→ 401
    let err = create_status(
        web::Data::new(pool.clone()),
        actix_web::test::TestRequest::default().to_http_request(),
        web::Json(CreateStatusReq {
            name: Some(marker.clone()),
            flag: None,
            comments: None,
        }),
    )
    .await
    .expect_err("未鉴权应拒写");
    assert!(
        matches!(err, common::AliothError::Unauthorized(_)),
        "未鉴权应为 401：{err:?}"
    );

    // NGAC 拒绝：非 admin、无 grants → 403
    let nobody = fixture_user(&pool, "nobody", false).await;
    let err = create_status(
        web::Data::new(pool.clone()),
        authed_req(nobody),
        web::Json(CreateStatusReq {
            name: Some(marker.clone()),
            flag: None,
            comments: None,
        }),
    )
    .await
    .expect_err("无授权用户应被 NGAC 拒绝");
    assert!(
        matches!(err, common::AliothError::Forbidden(_)),
        "NGAC 拒绝应为 403：{err:?}"
    );

    // 被拒不落「成功」事件，且无写入行（判定键 = 本用例专属 notice）
    let leaked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_audit.audit_events \
         WHERE operation = 'status.create' AND metadata->'after'->>'name' = $1",
    )
    .bind(&marker)
    .fetch_one(&pool)
    .await
    .expect("count audit events");
    assert_eq!(leaked, 0, "被拒写入 MUST NOT 落成功事件");
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM isahl.\"zc_id_stus-project\" WHERE notice = $1")
            .bind(&marker)
            .fetch_one(&pool)
            .await
            .expect("count rows");
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn missing_row_update_and_delete_are_not_found() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    let uid = fixture_user(&pool, "crud_tester", true).await;

    let missing: i64 = 9_000_000_000_000_001;
    let err = update_status(
        web::Data::new(pool.clone()),
        web::Path::from(missing),
        authed_req(uid),
        web::Json(UpdateStatusReq {
            name: Some("stdo-ghost".to_string()),
            flag: None,
            comments: None,
        }),
    )
    .await
    .expect_err("不存在的行应 404");
    assert!(matches!(err, common::AliothError::NotFound(_)), "{err:?}");
    let err = delete_status(
        web::Data::new(pool.clone()),
        web::Path::from(missing),
        authed_req(uid),
    )
    .await
    .expect_err("不存在的行应 404");
    assert!(matches!(err, common::AliothError::NotFound(_)), "{err:?}");

    // 未命中路径不落任何成功事件
    let leaked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_audit.audit_events WHERE object_path LIKE $1",
    )
    .bind(format!("status:{}:{}", TABLE, missing))
    .fetch_one(&pool)
    .await
    .expect("count audit events");
    assert_eq!(leaked, 0);
}

#[tokio::test]
async fn register_mounts_statuses_scope() {
    use actix_web::App;
    let app = actix_web::test::init_service(
        App::new()
            .app_data(web::Data::new(connect_test_db().await))
            .service(web::scope("/service/status").configure(status::register)),
    )
    .await;
    // 路由表语义：/service/status/statuses 可达（未鉴权 → 401，非 404）
    let req = actix_web::test::TestRequest::post()
        .uri("/service/status/statuses")
        .set_json(json!({"name": "stdo-route-probe"}))
        .to_request();
    let resp = actix_web::test::call_service(&app, req).await;
    assert_eq!(
        resp.status().as_u16(),
        401,
        "POST /service/status/statuses 须命中（未鉴权 401，非 404）"
    );
}
