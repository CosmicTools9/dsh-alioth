//! OpenAPI 订阅与对账（P4）集成测试
//!
//! 覆盖：
//!   1. 订阅列表：client 创建后自动有默认 free 订阅
//!   2. 变更容量档位：切换 plan 后 list_subscriptions 反映新档位
//!   3. 暂停/恢复：status 流转
//!   4. 对账导出：按月聚合 api_usage（含错误率/P95）
//!
//! 依赖真实测试库（aliothstudio_test）。测试数据自建自清。

use sqlx::PgPool;

use ::common::testing::connect_test_db;

/// 创建测试 client + 服务用户 + 默认 free 订阅，返回 (client_id, subscription_id, svc_user_id)。
async fn create_test_client(pool: &PgPool, suffix: &str) -> (String, i64, i64) {
    let client_id = format!("sub_test_{}", suffix);
    let svc_user: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.auth_users \
         (name, username, email, password_hash, user_type, is_active, status, \
          created_at, updated_at, failed_login_attempts, notification_preferences) \
         VALUES ($1, $2, NULL, NULL, 'service', TRUE, 'active', NOW(), NOW(), 0, '{}'::jsonb) \
         RETURNING id",
    )
    .bind(format!("svc-{}", client_id))
    .bind(format!("svc:{}", client_id))
    .fetch_one(pool)
    .await
    .expect("create service user");

    let client_row: (i64,) = sqlx::query_as(
        "INSERT INTO isahl_auth.api_clients \
         (client_id, client_type, client_name, secret_hash, scopes, fk_service_user, enabled) \
         VALUES ($1, 'oauth2', 'sub-test', '', $2::TEXT[], $3, TRUE) \
         RETURNING id",
    )
    .bind(&client_id)
    .bind(&["read:units".to_string()])
    .bind(svc_user)
    .fetch_one(pool)
    .await
    .expect("create client");

    // 默认 free 订阅（与 create_api_client 行为一致）
    let plan_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.api_plans WHERE code = 'free'")
            .fetch_one(pool)
            .await
            .expect("find free plan");
    let sub_row: (i64,) = sqlx::query_as(
        "INSERT INTO isahl_auth.api_subscriptions (fk_client, fk_plan, status) \
         VALUES ($1, $2, 'active') RETURNING id",
    )
    .bind(client_row.0)
    .bind(plan_id)
    .fetch_one(pool)
    .await
    .expect("create subscription");

    (client_id, sub_row.0, svc_user)
}

async fn cleanup_test_client(pool: &PgPool, client_id: &str) {
    let _ = sqlx::query(
        r#"DELETE FROM isahl_auth.api_usage u
           USING isahl_auth.api_subscriptions s, isahl_auth.api_clients c
           WHERE u.fk_subscription = s.id AND s.fk_client = c.id AND c.client_id = $1"#,
    )
    .bind(client_id)
    .execute(pool)
    .await;
    let _ = sqlx::query(
        r#"DELETE FROM isahl_auth.api_subscriptions s
           USING isahl_auth.api_clients c
           WHERE s.fk_client = c.id AND c.client_id = $1"#,
    )
    .bind(client_id)
    .execute(pool)
    .await;
    let svc: Option<i64> = sqlx::query_scalar(
        "SELECT fk_service_user FROM isahl_auth.api_clients WHERE client_id = $1",
    )
    .bind(client_id)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);
    if let Some(uid) = svc {
        let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
            .bind(uid)
            .execute(pool)
            .await;
    }
    let _ = sqlx::query("DELETE FROM isahl_auth.api_clients WHERE client_id = $1")
        .bind(client_id)
        .execute(pool)
        .await;
}

// ============================================================================
// 测试组 1: 订阅管理 SQL 语义（不经 handler，验证查询/更新逻辑）
// ============================================================================

/// SUB-001: client 创建后有默认 free 订阅
#[tokio::test]
async fn sub_001_default_free_subscription() {
    let pool = connect_test_db().await;
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (client_id, sub_id, _) = create_test_client(&pool, &suffix).await;

    // 验证订阅存在且指向 free
    let (plan_code, status): (String, String) = sqlx::query_as(
        r#"SELECT p.code, s.status
           FROM isahl_auth.api_subscriptions s
           JOIN isahl_auth.api_plans p ON p.id = s.fk_plan
           WHERE s.id = $1"#,
    )
    .bind(sub_id)
    .fetch_one(&pool)
    .await
    .expect("fetch subscription");
    assert_eq!(plan_code, "free");
    assert_eq!(status, "active");

    cleanup_test_client(&pool, &client_id).await;
}

/// SUB-002: 变更容量档位（free → pro）后反映新档位
#[tokio::test]
async fn sub_002_change_plan() {
    let pool = connect_test_db().await;
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (client_id, sub_id, _) = create_test_client(&pool, &suffix).await;

    // 变更 → pro（pro 档种子可能缺失于测试库，按「测试数据自建自清」约定自建；自建则自清）
    let (pro_id, pro_provisioned): (i64, bool) =
        match sqlx::query_scalar("SELECT id FROM isahl_auth.api_plans WHERE code = 'pro'")
            .fetch_optional(&pool)
            .await
            .expect("query pro plan")
        {
            Some(id) => (id, false),
            None => (
                sqlx::query_scalar(
                    "INSERT INTO isahl_auth.api_plans (code) VALUES ('pro') RETURNING id",
                )
                .fetch_one(&pool)
                .await
                .expect("provision pro plan"),
                true,
            ),
        };
    sqlx::query("UPDATE isahl_auth.api_subscriptions SET fk_plan = $1 WHERE id = $2")
        .bind(pro_id)
        .bind(sub_id)
        .execute(&pool)
        .await
        .expect("change plan");

    let plan_code: String = sqlx::query_scalar(
        "SELECT p.code FROM isahl_auth.api_subscriptions s \
         JOIN isahl_auth.api_plans p ON p.id = s.fk_plan WHERE s.id = $1",
    )
    .bind(sub_id)
    .fetch_one(&pool)
    .await
    .expect("fetch plan");
    assert_eq!(plan_code, "pro", "变更后应为 pro");

    cleanup_test_client(&pool, &client_id).await;
    if pro_provisioned {
        let _ = sqlx::query("DELETE FROM isahl_auth.api_plans WHERE id = $1")
            .bind(pro_id)
            .execute(&pool)
            .await;
    }
}

/// SUB-003: 暂停 → 恢复状态流转
#[tokio::test]
async fn sub_003_status_transition() {
    let pool = connect_test_db().await;
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (client_id, sub_id, _) = create_test_client(&pool, &suffix).await;

    // 暂停
    sqlx::query("UPDATE isahl_auth.api_subscriptions SET status = 'suspended' WHERE id = $1")
        .bind(sub_id)
        .execute(&pool)
        .await
        .expect("suspend");
    let status: String =
        sqlx::query_scalar("SELECT status FROM isahl_auth.api_subscriptions WHERE id = $1")
            .bind(sub_id)
            .fetch_one(&pool)
            .await
            .expect("fetch status");
    assert_eq!(status, "suspended");

    // 恢复
    sqlx::query("UPDATE isahl_auth.api_subscriptions SET status = 'active' WHERE id = $1")
        .bind(sub_id)
        .execute(&pool)
        .await
        .expect("resume");
    let status: String =
        sqlx::query_scalar("SELECT status FROM isahl_auth.api_subscriptions WHERE id = $1")
            .bind(sub_id)
            .fetch_one(&pool)
            .await
            .expect("fetch status");
    assert_eq!(status, "active");

    cleanup_test_client(&pool, &client_id).await;
}

/// SUB-004: 对账导出 SQL 聚合（按月）
#[tokio::test]
async fn sub_004_reconcile_aggregation() {
    let pool = connect_test_db().await;
    let suffix = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (client_id, sub_id, _) = create_test_client(&pool, &suffix).await;

    // 造 2 条计量（1 成功 1 错误）
    for (status, latency) in [(200i16, 100i16), (500, 300)] {
        sqlx::query(
            "INSERT INTO isahl_auth.api_usage (fk_subscription, route, method, status, latency_ms) \
             VALUES ($1, '/api/service/measurement/units', 'GET', $2, $3)",
        )
        .bind(sub_id)
        .bind(status)
        .bind(latency)
        .execute(&pool)
        .await
        .expect("insert usage");
    }

    // 与 reconcile_export 相同的聚合 SQL
    let rows: Vec<(String, String, i64, i64, f64, i64, i16)> = sqlx::query_as(
        r#"
        SELECT c.client_id, p.code AS plan_code,
               COUNT(*)::bigint AS total,
               COUNT(*) FILTER (WHERE u.status >= 400)::bigint AS errors,
               COALESCE(AVG(u.latency_ms), 0)::float8 AS avg_latency_ms,
               PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY u.latency_ms)::bigint AS p95_ms,
               p.tier AS plan_tier
        FROM isahl_auth.api_usage u
        JOIN isahl_auth.api_subscriptions s ON s.id = u.fk_subscription
        JOIN isahl_auth.api_clients c ON c.id = s.fk_client
        JOIN isahl_auth.api_plans p ON p.id = s.fk_plan
        WHERE u.requested_at >= date_trunc('month', NOW())
          AND u.requested_at < date_trunc('month', NOW()) + INTERVAL '1 month'
        GROUP BY c.client_id, p.code, p.tier
        ORDER BY c.client_id
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("reconcile query");

    let mine = rows.iter().find(|r| r.0 == client_id);
    assert!(mine.is_some(), "对账应包含测试 client");
    let (_, plan, total, errors, _, _, tier) = mine.unwrap();
    assert_eq!(total, &2, "total=2");
    assert_eq!(errors, &1, "errors=1");
    assert_eq!(plan, "free");
    assert_eq!(tier, &0);

    cleanup_test_client(&pool, &client_id).await;
}
