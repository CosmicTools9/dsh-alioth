//! NGAC PDP list 端点集成测试（行级可见性 + admin 豁免）
//!
//! 验证 `POST /api/ngac/pdp/list`：
//! - admin UA 用户 → permitted=true, visible_ids=None（NGAC_SPEC §6.2 全量不过滤）
//! - admin UA 经 ancestor 继承 → 同样 None
//! - 普通用户有 collection association → visible_ids=Some(ids)
//! - 普通用户无 association → permitted=false（fail-closed 403）

mod common;

use ::common::testing::connect_test_db;
use actix_web::body::MessageBody;
use actix_web::web;
use gateway_sso::ngac::pdp::list_resource_access;
use ngac_contract::PdpListRequest;
use sqlx::PgPool;

/// 播种：policy_class + UA + 用户绑定 + collection OA + association（read right）
/// 幂等：policy_class/UA 已存在则复用（并发测试安全）。
async fn seed_list_policies(
    pool: &PgPool,
    user_id: i64,
    ua_name: &str,
    resource_type: &str,
    rights: &[&str],
) {
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, username) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(format!("list-tester-{}-{}", ua_name, user_id))
    .bind(format!("list-test-{}", user_id))
    .execute(pool)
    .await
    .unwrap();

    // policy_class 幂等（ON CONFLICT DO NOTHING 后回查）
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_policy_class (o_name) VALUES ('default-list-test') ON CONFLICT (o_name) DO NOTHING",
    )
    .execute(pool)
    .await
    .unwrap();
    let pc: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default-list-test'",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    // UA 幂等：ON CONFLICT DO NOTHING 后回查（并发安全）
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class) VALUES ($1, $2) \
         ON CONFLICT (o_name, fk_policy_class) WHERE deleted_at IS NULL DO NOTHING",
    )
    .bind(ua_name)
    .bind(pc)
    .execute(pool)
    .await
    .unwrap();
    let ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name=$1 AND fk_policy_class=$2 AND deleted_at IS NULL",
    )
    .bind(ua_name)
    .bind(pc)
    .fetch_one(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(ua)
    .execute(pool)
    .await
    .unwrap();

    let oa: i64 = match sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type=$1 AND fk_resource=0 AND deleted_at IS NULL",
    )
    .bind(resource_type)
    .fetch_optional(pool)
    .await
    .unwrap()
    {
        Some(id) => id,
        None => sqlx::query_scalar(
            "INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource) VALUES ($1, $2, $3, 0) RETURNING id",
        )
        .bind(format!("{}-collection", resource_type))
        .bind(pc)
        .bind(resource_type)
        .fetch_one(pool)
        .await
        .unwrap(),
    };

    for r in rights {
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ($1) ON CONFLICT DO NOTHING",
        )
        .bind(r)
        .execute(pool)
        .await
        .unwrap();
    }
    let right_ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = ANY($1)")
            .bind(rights)
            .fetch_all(pool)
            .await
            .unwrap();

    sqlx::query(
        "INSERT INTO isahl_auth.ngac_association (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class) VALUES ($1, $2, $3, $4)",
    )
    .bind(ua)
    .bind(oa)
    .bind(&right_ids)
    .bind(pc)
    .execute(pool)
    .await
    .unwrap();
}

/// 清理测试播种数据（按 user_id 精确清理，避免并发测试误删他人数据）。
/// OA / UA / policy_class 保留（幂等复用，删除会破坏并发测试）。
async fn cleanup_list_policies(pool: &PgPool, user_ids: &[i64]) {
    let ua_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT ura.fk_user_attribute FROM isahl_auth.ngac_user_rr_attribute ura \
         WHERE ura.fk_user = ANY($1)",
    )
    .bind(user_ids)
    .fetch_all(pool)
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

/// 直调 list 端点 handler
async fn call_list(
    pool: &PgPool,
    user_id: i64,
    resource_type: &str,
) -> ngac_contract::PdpListResponse {
    let body = web::Json(PdpListRequest {
        user_id,
        resource_type: resource_type.to_string(),
        action: "read".to_string(),
    });
    let resp = list_resource_access(web::Data::new(pool.clone()), body).await;
    let bytes = resp.into_body().try_into_bytes().unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn list_admin_user_returns_none_visible_ids() {
    let pool = connect_test_db().await;
    let user_id: i64 = 999999999900101;
    // 用户持有名为 `admin` 的 UA（豁免判定以 o_name='admin' 为准）
    seed_list_policies(&pool, user_id, "admin", "listtest-admin", &["read"]).await;

    let resp = call_list(&pool, user_id, "listtest-admin").await;
    assert!(resp.permitted, "admin 应 permitted: {:?}", resp.reason);
    assert!(
        resp.visible_ids.is_none(),
        "admin 应 visible_ids=None（全量不过滤）: {:?}",
        resp.visible_ids
    );

    cleanup_list_policies(&pool, &[user_id]).await;
}

#[tokio::test]
async fn list_regular_user_returns_collection_ids() {
    let pool = connect_test_db().await;
    let user_id: i64 = 999999999900102;
    let rt = "listtest-regular";
    seed_list_policies(&pool, user_id, "list-ua-regular", rt, &["read"]).await;

    let resp = call_list(&pool, user_id, rt).await;
    assert!(
        resp.permitted,
        "有 read 关联应 permitted: {:?}",
        resp.reason
    );
    assert_eq!(
        resp.visible_ids,
        Some(vec![0]),
        "collection OA（fk_resource=0）应可见"
    );

    cleanup_list_policies(&pool, &[user_id]).await;
}

#[tokio::test]
async fn list_user_without_association_denied() {
    let pool = connect_test_db().await;
    let user_id: i64 = 999999999900103;
    // 播种另一资源的 association（保持全局有策略，避免 bootstrap 分支）
    seed_list_policies(&pool, user_id, "list-ua-other", "listtest-other", &["read"]).await;

    let resp = call_list(&pool, user_id, "listtest-noaccess").await;
    assert!(
        !resp.permitted,
        "无关联资源类型应 denied（fail-closed）: {:?}",
        resp.reason
    );

    cleanup_list_policies(&pool, &[user_id]).await;
}

#[tokio::test]
async fn list_admin_with_inherited_ua_returns_none() {
    let pool = connect_test_db().await;
    let user_id: i64 = 999999999900104;
    // 先建 admin UA（作为继承源），再建 child UA 指向它
    seed_list_policies(
        &pool,
        999999999900105,
        "admin",
        "listtest-inherit-admin",
        &["read"],
    )
    .await;
    seed_list_policies(
        &pool,
        user_id,
        "list-ua-child",
        "listtest-inherit",
        &["read"],
    )
    .await;
    // 建立继承链：child.ancestor_ids 含 admin UA
    sqlx::query(
        "UPDATE isahl_auth.ngac_user_attribute SET ancestor_ids = ARRAY(
            SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='admin'
        ) WHERE o_name='list-ua-child'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let resp = call_list(&pool, user_id, "listtest-inherit").await;
    assert!(resp.permitted, "继承 admin 应 permitted: {:?}", resp.reason);
    assert!(
        resp.visible_ids.is_none(),
        "继承 admin 应 visible_ids=None: {:?}",
        resp.visible_ids
    );

    cleanup_list_policies(&pool, &[user_id, 999999999900105]).await;
}
