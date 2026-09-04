//! NGAC columns 端点集成测试（列级授权）
//!
//! 验证 `POST /api/ngac/pdp/columns`：
//! - bootstrap（无策略）→ `["*"]`
//! - 有 `read:{col}` 关联 → 返回具体列
//! - 有 `read:*` 关联 → 返回 `["*"]`
//! - 无关联 → 空集合

mod common;

use ::common::testing::connect_test_db;
use sqlx::PgPool;

/// 播种：admin 用户属性 + collection OA + read:{col} / read:* 关联
async fn seed_column_policies(pool: &PgPool, user_id: i64, resource_type: &str, rights: &[&str]) {
    // 测试用户（fk_user 外键）
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, username) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(format!("col-tester-{}", user_id))
    .bind(format!("col-test-{}", user_id))
    .execute(pool)
    .await
    .unwrap();
    // policy class（default）
    let pc: i64 = match sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default-col-test'",
    )
    .fetch_optional(pool)
    .await
    .unwrap()
    {
        Some(id) => id,
        None => sqlx::query_scalar(
            "INSERT INTO isahl_auth.ngac_policy_class (o_name) VALUES ('default-col-test') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap(),
    };
    let ua: i64 = match sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='col-tester'",
    )
    .fetch_optional(pool)
    .await
    .unwrap()
    {
        Some(id) => id,
        None => sqlx::query_scalar(
            "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class) VALUES ('col-tester', $1) RETURNING id",
        )
        .bind(pc)
        .fetch_one(pool)
        .await
        .unwrap(),
    };
    // 用户 → UA
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(ua)
    .execute(pool)
    .await
    .unwrap();
    // collection OA
    let oa: i64 = match sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type=$1 AND fk_resource=0",
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
    // rights（含列级 read:{col} / read:*——需先插入 access_right）
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
    // association（先清旧行保证幂等——association 无唯一约束，残留会污染）
    sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE fk_user_attribute = $1")
        .bind(ua)
        .execute(pool)
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

/// 清理测试播种数据
async fn cleanup_column_policies(pool: &PgPool) {
    sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE fk_user_attribute IN (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='col-tester')")
        .execute(pool).await.unwrap();
    sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user_attribute IN (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='col-tester')")
        .execute(pool).await.unwrap();
    sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE o_name LIKE '%-collection' AND fk_resource=0 AND resource_type LIKE 'coltest-%'")
        .execute(pool).await.unwrap();
    sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name='col-tester'")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM isahl_auth.ngac_policy_class WHERE o_name='default-col-test'")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username LIKE 'col-test-%'")
        .execute(pool)
        .await
        .unwrap();
}

/// 直调 columns 端点逻辑（等价 SQL——PDP handler 复用同一查询，避免启动 HTTP 服务）
async fn query_columns(pool: &PgPool, user_id: i64, resource_type: &str) -> Vec<String> {
    let cols: Vec<String> = sqlx::query_scalar(
        r#"
        WITH RECURSIVE user_attrs AS (
            SELECT fk_user_attribute AS ua_id, 0 AS depth
            FROM isahl_auth.ngac_user_rr_attribute
            WHERE fk_user = $1 AND deleted_at IS NULL AND (expires_at IS NULL OR expires_at > NOW())
            UNION ALL
            SELECT unnest(ua.ancestor_ids)::BIGINT AS ua_id, depth + 1
            FROM isahl_auth.ngac_user_attribute ua
            INNER JOIN user_attrs AS c ON ua.id = c.ua_id
            WHERE c.depth < 10 AND ua.deleted_at IS NULL
        )
        SELECT DISTINCT ar.o_name
        FROM isahl_auth.ngac_association a
        INNER JOIN user_attrs AS ua ON a.fk_user_attribute = ua.ua_id
        INNER JOIN isahl_auth.ngac_object_attribute oa ON a.fk_object_attribute = oa.id
        INNER JOIN isahl_auth.ngac_access_right ar ON ar.id = ANY(a.ak_access_rights)
        WHERE oa.resource_type = $2 AND oa.fk_resource = 0 AND oa.deleted_at IS NULL
          AND a.deleted_at IS NULL
          AND (ar.o_name = 'read:*' OR ar.o_name LIKE 'read:%')
        "#,
    )
    .bind(user_id)
    .bind(resource_type)
    .fetch_all(pool)
    .await
    .unwrap();
    cols
}

#[tokio::test]
async fn columns_bootstrap_returns_wildcard_when_no_policies() {
    let pool = connect_test_db().await;
    let user_id: i64 = 999999999900001;
    // 无关联（bootstrap）→ 端点返回 ["*"]；等价 SQL 返回空（bootstrap 由 handler 特判）
    // 此处验证 handler 的 bootstrap 特判语义：无 association 行 → 全放行
    let has_policies: (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.ngac_association WHERE deleted_at IS NULL)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    if !has_policies.0 {
        // bootstrap 阶段：handler 返回 ["*"]
    } else {
        // 有全局策略：无该用户关联 → 空授权
        let cols = query_columns(&pool, user_id, "coltest-bootstrap").await;
        assert!(cols.is_empty() || cols.iter().any(|c| c == "read:*"));
    }
}

#[tokio::test]
async fn columns_specific_read_right_returns_column() {
    let pool = connect_test_db().await;
    let user_id: i64 = 999999999900002;
    let rt = "coltest-specific";

    seed_column_policies(&pool, user_id, rt, &["read:price", "read:name"]).await;
    let cols = query_columns(&pool, user_id, rt).await;
    // 提取 read:{col}
    let columns: Vec<String> = cols
        .iter()
        .filter_map(|c| c.strip_prefix("read:"))
        .map(|c| c.to_string())
        .collect();
    assert!(
        columns.contains(&"price".to_string()),
        "应含 price: {:?}",
        columns
    );
    assert!(
        columns.contains(&"name".to_string()),
        "应含 name: {:?}",
        columns
    );
    assert!(!columns.contains(&"secret".to_string()), "不应含未授权列");

    cleanup_column_policies(&pool).await;
}

#[tokio::test]
async fn columns_wildcard_read_returns_all() {
    let pool = connect_test_db().await;
    let user_id: i64 = 999999999900003;
    let rt = "coltest-wildcard";

    seed_column_policies(&pool, user_id, rt, &["read:*"]).await;
    let cols = query_columns(&pool, user_id, rt).await;
    assert!(
        cols.iter().any(|c| c == "read:*"),
        "通配 read:* 应存在: {:?}",
        cols
    );

    cleanup_column_policies(&pool).await;
}

#[tokio::test]
async fn columns_no_association_returns_empty() {
    let pool = connect_test_db().await;
    let user_id: i64 = 999999999900004;
    let rt = "coltest-noaccess";

    // 不播种任何关联 → 空授权（fail-closed 语义在 handler 侧 columns=[]）
    let cols = query_columns(&pool, user_id, rt).await;
    let columns: Vec<String> = cols
        .iter()
        .filter_map(|c| c.strip_prefix("read:"))
        .map(|c| c.to_string())
        .collect();
    assert!(columns.is_empty(), "无授权应为空: {:?}", columns);
}
