//! dict 写入留痕（c1）+ /dict/tables payload 形状 + 被拒不落成功事件 — 集成测试（真实 PG，测试库）。
//!
//! 运行：`DATABASE_URL=postgres://<user>@localhost:5432/aliothstudio_test cargo test -p dict`
//! 前置：测试库已按 `005_seed_ngac_and_policy_class.sql` 种子（admin UA 在场）；本测试自建
//! fixture 用户并绑定 admin UA（前缀 `dvdo_`），使 `require_resource_access` 经 admin 治理兜底
//! 放行（与 `common/tests/permissions_runtime_test.rs` 同款夹具）。

use actix_web::{web, HttpMessage, HttpRequest};
use common::context::RequestContext;
use common::testing::connect_test_db;
use dict::handler::{
    create_dict_entry, delete_dict_entry, list_dict_tables, update_dict_entry, ItemPath,
};
use serde_json::{json, Value};
use sqlx::PgPool;

const TEST_TABLE: &str = "zc_id_cate-warehouse";

/// fixture 用户（幂等）：auth_users 行 + admin UA 绑定（prefix `dvdo_`）。
async fn fixture_user(pool: &PgPool) -> i64 {
    let username = "dvdo_audit_tester";
    let existing: Option<i64> =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE username = $1 LIMIT 1")
            .bind(username)
            .fetch_optional(pool)
            .await
            .expect("read fixture user");
    let user_id = match existing {
        Some(id) => id,
        None => sqlx::query_scalar(
            "INSERT INTO isahl_auth.auth_users (username) VALUES ($1) RETURNING id",
        )
        .bind(username)
        .fetch_one(pool)
        .await
        .expect("insert fixture user"),
    };
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
        .bind(format!("dvdo_bind_{user_id}"))
        .bind(user_id)
        .bind(admin_ua)
        .execute(pool)
        .await
        .expect("insert admin binding");
    }
    user_id
}

fn authed_req(user_id: i64) -> HttpRequest {
    let req = actix_web::test::TestRequest::default().to_http_request();
    req.extensions_mut().insert(RequestContext::with_username(
        user_id,
        format!("dvdo_{user_id}@test.local"),
        format!("dvdo_{user_id}"),
    ));
    req
}

async fn cleanup_row(pool: &PgPool, id: i64) {
    sqlx::query("DELETE FROM isahl_audit.audit_events WHERE object_path = $1")
        .bind(format!("dict:{}:{}", TEST_TABLE, id))
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type = 'dict' AND fk_resource = $1",
    )
    .bind(id)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl.\"zc_id_cate-warehouse\" WHERE id = $1")
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
    .bind(format!("dict:{}:{}", TEST_TABLE, id))
    .fetch_all(pool)
    .await
    .expect("read audit events")
}

#[tokio::test]
async fn create_update_delete_leave_audit_trail() {
    let pool = connect_test_db().await;
    let uid = fixture_user(&pool).await;

    // ── create：记新值 ──
    let resp = create_dict_entry(
        web::Path::from(TEST_TABLE.to_string()),
        web::Json(json!({ "notice": "dvdo-初始", "code": "DVDO-AUDIT-1" })),
        authed_req(uid),
        web::Data::new(pool.clone()),
    )
    .await
    .expect("create 应成功");
    assert_eq!(resp.status().as_u16(), 201);
    let id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM isahl.\"zc_id_cate-warehouse\" WHERE code = 'DVDO-AUDIT-1' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("read created row");
    let created = audit_rows(&pool, id, "dict.create").await;
    assert_eq!(created.len(), 1, "create 应落一条 dict.create 留痕");
    assert_eq!(created[0]["table"], TEST_TABLE);
    assert_eq!(created[0]["id"], id.to_string());
    assert_eq!(created[0]["actor"]["username"], format!("dvdo_{uid}"));
    assert_eq!(created[0]["after"]["notice"], "dvdo-初始");

    // ── update：记前后值 ──
    let resp = update_dict_entry(
        web::Path::from(ItemPath {
            table: TEST_TABLE.to_string(),
            id,
        }),
        web::Json(json!({ "notice": "dvdo-更新" })),
        authed_req(uid),
        web::Data::new(pool.clone()),
    )
    .await
    .expect("update 应成功");
    assert_eq!(resp.status().as_u16(), 200);
    let updated = audit_rows(&pool, id, "dict.update").await;
    assert_eq!(updated.len(), 1, "update 应落一条 dict.update 留痕");
    assert_eq!(updated[0]["before"]["notice"], "dvdo-初始");
    assert_eq!(updated[0]["after"]["notice"], "dvdo-更新");

    // ── delete（软删）：记被删行摘要 ──
    let resp = delete_dict_entry(
        web::Path::from(ItemPath {
            table: TEST_TABLE.to_string(),
            id,
        }),
        authed_req(uid),
        web::Data::new(pool.clone()),
    )
    .await
    .expect("delete 应成功");
    assert_eq!(resp.status().as_u16(), 200);
    let deleted = audit_rows(&pool, id, "dict.delete").await;
    assert_eq!(deleted.len(), 1, "delete 应落一条 dict.delete 留痕");
    assert_eq!(deleted[0]["before"]["notice"], "dvdo-更新");

    cleanup_row(&pool, id).await;
}

#[tokio::test]
async fn rejected_writes_leave_no_success_event() {
    let pool = connect_test_db().await;
    let uid = fixture_user(&pool).await;

    // 基表只读 → 拒绝
    let err = create_dict_entry(
        web::Path::from("zc_id_category".to_string()),
        web::Json(json!({ "notice": "dvdo-非法" })),
        authed_req(uid),
        web::Data::new(pool.clone()),
    )
    .await
    .expect_err("基表应拒写");
    assert!(
        matches!(err, common::AliothError::BadRequest(_)),
        "基表拒写应为 400：{err:?}"
    );

    // 白名单外 → 拒绝
    let err = create_dict_entry(
        web::Path::from("zc_id_alioth-e2e".to_string()),
        web::Json(json!({ "notice": "dvdo-非法" })),
        authed_req(uid),
        web::Data::new(pool.clone()),
    )
    .await
    .expect_err("白名单外应拒写");
    assert!(matches!(err, common::AliothError::BadRequest(_)));

    // 未鉴权 → 拒绝（无 RequestContext）
    let err = create_dict_entry(
        web::Path::from(TEST_TABLE.to_string()),
        web::Json(json!({ "notice": "dvdo-未鉴权" })),
        actix_web::test::TestRequest::default().to_http_request(),
        web::Data::new(pool.clone()),
    )
    .await
    .expect_err("未鉴权应拒写");
    assert!(matches!(err, common::AliothError::Unauthorized(_)));

    // 被拒不落「成功」事件：三类被拒路径均无 dict.create 留痕
    // （判定键 = 本测试专属表名/notice，避免与其他并行用例的合法写入相交）
    let leaked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_audit.audit_events \
         WHERE operation = 'dict.create' \
           AND (metadata->>'table' IN ('zc_id_category', 'zc_id_alioth-e2e') \
                OR metadata->'after'->>'notice' = 'dvdo-未鉴权')",
    )
    .fetch_one(&pool)
    .await
    .expect("count audit events");
    assert_eq!(leaked, 0, "被拒写入 MUST NOT 落成功事件");

    // 且未鉴权路径确实没有写入行
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl.\"zc_id_cate-warehouse\" WHERE notice = 'dvdo-未鉴权'",
    )
    .fetch_one(&pool)
    .await
    .expect("count rows");
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn tables_payload_carries_schema_version_and_column_metadata() {
    let pool = connect_test_db().await;
    let resp = list_dict_tables(web::Data::new(pool.clone()))
        .await
        .expect("list_dict_tables 应成功");
    assert_eq!(resp.status().as_u16(), 200);
    let body = actix_web::body::to_bytes(resp.into_body())
        .await
        .expect("read body");
    let v: Value = serde_json::from_slice(&body).expect("payload 应为 JSON");
    let data = &v["data"];
    let version = data["schemaVersion"]
        .as_str()
        .expect("data.schemaVersion 应为字符串");
    assert_eq!(version, dict::schema::model_version());
    let tables = data["tables"].as_array().expect("data.tables 应为数组");
    assert!(!tables.is_empty());
    let row = tables
        .iter()
        .find(|t| t["table"] == TEST_TABLE)
        .expect("cate-warehouse 应在清单内");
    let cols = row["columns"].as_array().expect("columns 应为数组");
    let notice = cols
        .iter()
        .find(|c| c["name"] == "notice")
        .expect("notice 列应在内");
    assert_eq!(notice["type"], "text");
    assert_eq!(notice["editable"], true);
    assert!(notice.get("title").is_some(), "列对象 MUST 含 title 键");
    let id_col = cols
        .iter()
        .find(|c| c["name"] == "id")
        .expect("id 列应在内");
    assert_eq!(id_col["editable"], false);
}
