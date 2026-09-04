//! /auth/me permissions 矩阵集成测试
//!
//! 验证 `me` handler 返回的 `permissions` 字段：
//! - UA 指派 + association → OA resource_type × AccessRight 聚合
//! - 过期指派 / 时间条件不满足的关联 → 不授予（fail-closed）
//! - 无关联用户 → 空对象
#![allow(clippy::type_complexity)]

mod common;

use actix_web::{test, web, App};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use sqlx::PgPool;

async fn connect() -> PgPool {
    // 统一走共享测试库连接（含 OS 用户注入），避免 postgres://localhost/... 被
    // sqlx 解析为 anonymous 角色导致连接失败（与 admin_api_test 一致）。
    ::common::testing::connect_test_db().await
}

fn test_auth_state() -> AuthState {
    common::test_auth_state()
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

/// NGAC 表兜底（迁移已建则 no-op）——对齐 sso_gap_fixes_test 的 ensure_ngac 模式。
async fn ensure_ngac(pool: &PgPool) {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_policy_class (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL,
            o_name TEXT, description TEXT,
            created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
            CONSTRAINT ngac_policy_class_pkey PRIMARY KEY (id))",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_user_attribute (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL, o_name TEXT, fk_policy_class BIGINT NOT NULL,
            ancestor_ids BIGINT[] DEFAULT '{}', children_ids BIGINT[] DEFAULT '{}', property JSONB DEFAULT '{}',
            created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
            created_by_id BIGINT, updated_by_id BIGINT, deleted_at TIMESTAMPTZ,
            CONSTRAINT ngac_user_attribute_pkey PRIMARY KEY (id))",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_user_rr_attribute (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL, created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
            updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, created_by_id BIGINT, updated_by_id BIGINT, o_name TEXT,
            fk_user BIGINT, fk_user_attribute BIGINT, assigned_at TIMESTAMPTZ DEFAULT NOW(), expires_at TIMESTAMPTZ,
            conditions JSONB DEFAULT '{}', deleted_at TIMESTAMPTZ,
            CONSTRAINT ngac_user_rr_attribute_pkey PRIMARY KEY (id),
            CONSTRAINT ngac_user_rr_attribute_fk_user_fkey FOREIGN KEY (fk_user) REFERENCES isahl_auth.auth_users(id) ON DELETE CASCADE,
            CONSTRAINT ngac_user_rr_attribute_fk_user_attribute_fkey FOREIGN KEY (fk_user_attribute) REFERENCES isahl_auth.ngac_user_attribute(id) ON DELETE CASCADE)",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_object_attribute (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL, o_name TEXT, fk_policy_class BIGINT,
            ancestor_ids BIGINT[] DEFAULT '{}', children_ids BIGINT[] DEFAULT '{}',
            resource_type VARCHAR(64) NOT NULL, fk_resource BIGINT NOT NULL DEFAULT 0,
            resource_identifier VARCHAR(255), property JSONB DEFAULT '{}',
            created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
            created_by_id BIGINT, updated_by_id BIGINT, deleted_at TIMESTAMPTZ,
            CONSTRAINT ngac_object_attribute_pkey PRIMARY KEY (id),
            CONSTRAINT uq_ngac_oa_resource UNIQUE (resource_type, fk_resource))",
    )
    .execute(pool)
    .await
    .ok();
    // reset-db 预建的 ngac_object_attribute 可能无 uq_ngac_oa_resource（Gateway 012 迁移
    // 不在 SSO 测试建表路径），CREATE IF NOT EXISTS 不会重建 → 显式幂等补约束。
    sqlx::query(
        r#"
        DO $$ BEGIN
            IF NOT EXISTS (
                SELECT 1 FROM pg_constraint
                WHERE conname = 'uq_ngac_oa_resource'
                  AND conrelid = 'isahl_auth.ngac_object_attribute'::regclass
            ) THEN
                ALTER TABLE isahl_auth.ngac_object_attribute
                    ADD CONSTRAINT uq_ngac_oa_resource UNIQUE (resource_type, fk_resource);
            END IF;
        END $$;
        "#,
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_association (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL, fk_user_attribute BIGINT NOT NULL,
            fk_object_attribute BIGINT NOT NULL, ak_access_rights BIGINT[] NOT NULL DEFAULT '{}',
            conditions JSONB, created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, deleted_at TIMESTAMPTZ,
            CONSTRAINT ngac_association_pkey PRIMARY KEY (id))",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_access_right (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL, o_name VARCHAR(64) NOT NULL UNIQUE,
            CONSTRAINT ngac_access_right_pkey PRIMARY KEY (id))",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_policy_class (id, o_name) \
         SELECT isahl.gen_next_zuid(), 'default' \
         WHERE NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_policy_class WHERE o_name = 'default')",
    )
    .execute(pool)
    .await
    .ok();
    for right in ["read", "create", "update", "delete"] {
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ($1) ON CONFLICT (o_name) DO NOTHING",
        )
        .bind(right)
        .execute(pool)
        .await
        .ok();
    }
    // 测试库可能被无约束重建（id 序列默认丢失，致 INSERT...RETURNING 报 null id）——
    // 幂等补 gen_next_zuid() 默认（isahl_auth 属 zuid 域）。
    for stmt in [
        "ALTER TABLE isahl_auth.ngac_user_attribute ALTER COLUMN id SET DEFAULT isahl.gen_next_zuid()",
        "ALTER TABLE isahl_auth.ngac_user_rr_attribute ALTER COLUMN id SET DEFAULT isahl.gen_next_zuid()",
        "ALTER TABLE isahl_auth.ngac_object_attribute ALTER COLUMN id SET DEFAULT isahl.gen_next_zuid()",
        "ALTER TABLE isahl_auth.ngac_association ALTER COLUMN id SET DEFAULT isahl.gen_next_zuid()",
        "ALTER TABLE isahl_auth.ngac_access_right ALTER COLUMN id SET DEFAULT isahl.gen_next_zuid()",
        "ALTER TABLE isahl_auth.ngac_prohibition ALTER COLUMN id SET DEFAULT isahl.gen_next_zuid()",
    ] {
        sqlx::query(stmt).execute(pool).await.ok();
    }
}

async fn access_right_id(pool: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = $1")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("access right")
}

async fn default_policy_class(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("default policy class")
}

/// 唯一名称（测试 DB 有种子 UA，固定名会撞唯一约束）
fn unique(name: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{}_{}", name, nanos)
}

/// 创建 UA + OA + association，并把 UA 指派给用户。
/// 返回 (ua_id, oa_id, assoc_id)。
#[allow(clippy::too_many_arguments)] // 测试辅助：授权关系参数固定
async fn grant_relation(
    pool: &PgPool,
    user_id: i64,
    pc_id: i64,
    ua_name: &str,
    resource_type: &str,
    rights: &[&str],
    expires_at: Option<&str>,
    conditions: Option<&serde_json::Value>,
) -> (i64, i64, i64) {
    let ua_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at)
         VALUES ($1, $2, NOW(), NOW())
         RETURNING id",
    )
    .bind(ua_name)
    .bind(pc_id)
    .fetch_one(pool)
    .await
    .expect("create UA");

    let oa_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at)
         VALUES ($1, $2, $3, 0, NOW(), NOW())
         ON CONFLICT (resource_type, fk_resource) DO UPDATE SET o_name = EXCLUDED.o_name
         RETURNING id",
    )
    .bind(format!("oa_{}", resource_type))
    .bind(pc_id)
    .bind(resource_type)
    .fetch_one(pool)
    .await
    .expect("create OA");

    let right_ids: Vec<i64> = {
        let mut ids = Vec::new();
        for r in rights {
            ids.push(access_right_id(pool, r).await);
        }
        ids
    };

    let assoc_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_association (fk_user_attribute, fk_object_attribute, ak_access_rights, conditions, fk_policy_class, created_at)
         VALUES ($1, $2, $3, $4, $5, NOW())
         RETURNING id",
    )
    .bind(ua_id)
    .bind(oa_id)
    .bind(&right_ids)
    .bind(conditions.unwrap_or(&serde_json::json!({})))
    .bind(pc_id)
    .fetch_one(pool)
    .await
    .expect("create association");

    let expires: Option<chrono::DateTime<chrono::Utc>> = match expires_at {
        Some(s) => Some(
            chrono::DateTime::parse_from_rfc3339(s)
                .expect("expires_at RFC3339")
                .with_timezone(&chrono::Utc),
        ),
        None => None,
    };
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at, expires_at)
         VALUES ($1, $2, NOW(), NOW(), $3)",
    )
    .bind(user_id)
    .bind(ua_id)
    .bind(expires)
    .execute(pool)
    .await
    .expect("assign UA");

    (ua_id, oa_id, assoc_id)
}

async fn cleanup(pool: &PgPool, user_id: i64, ua_ids: &[i64], oa_ids: &[i64]) {
    sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .ok();
    for ua in ua_ids {
        sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE id = $1")
            .bind(ua)
            .execute(pool)
            .await
            .ok();
    }
    for oa in oa_ids {
        sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE id = $1")
            .bind(oa)
            .execute(pool)
            .await
            .ok();
    }
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn me_returns_permission_matrix_from_associations() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    ensure_ngac(&pool).await;
    let ast = test_auth_state();

    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), 'perm_user', 'perm_user@alioth.test', 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .execute(&pool)
    .await
    .ok();
    let user_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE email = 'perm_user@alioth.test' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let pc_id = default_policy_class(&pool).await;

    // 有效关联：engineers → read + create
    let (ua1, oa1, _) = grant_relation(
        &pool,
        user_id,
        pc_id,
        &unique("operator"),
        "engineers",
        &["read", "create"],
        None,
        None,
    )
    .await;
    // 过期指派：inventory → delete（不应出现在矩阵）
    let (ua2, oa2, _) = grant_relation(
        &pool,
        user_id,
        pc_id,
        &unique("expired_role"),
        "inventory",
        &["delete"],
        Some("2000-01-01T00:00:00Z"),
        None,
    )
    .await;
    // 时间条件不满足：not_before 在未来 → 不授予
    let (ua3, oa3, _) = grant_relation(
        &pool,
        user_id,
        pc_id,
        &unique("future_role"),
        "warehouse",
        &["update"],
        None,
        Some(&serde_json::json!({ "not_before": "2999-01-01T00:00:00Z" })),
    )
    .await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::auth::login::configure),
    )
    .await;

    let token = mint_token(&ast, user_id, "perm_user@alioth.test");
    let req = test::TestRequest::get()
        .uri("/me")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let body: serde_json::Value = test::read_body_json(test::call_service(&app, req).await).await;

    let perms = &body["user"]["permissions"];
    // 有效关联生效
    let engineers = perms["engineers"].as_array().expect("engineers matrix");
    let actions: Vec<&str> = engineers.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        actions.contains(&"read"),
        "engineers should include read: {:?}",
        actions
    );
    assert!(
        actions.contains(&"create"),
        "engineers should include create: {:?}",
        actions
    );
    // 过期指派与未来条件不生效
    assert!(
        perms["inventory"].is_null(),
        "expired assignment must not grant: {perms}"
    );
    assert!(
        perms["warehouse"].is_null(),
        "future not_before must not grant: {perms}"
    );
    // 无关联资源不出现
    assert!(perms["orders"].is_null());

    cleanup(&pool, user_id, &[ua1, ua2, ua3], &[oa1, oa2, oa3]).await;
}

#[tokio::test]
async fn me_permissions_empty_for_user_without_relations() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    let ast = test_auth_state();

    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), 'perm_none', 'perm_none@alioth.test', 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .execute(&pool)
    .await
    .ok();
    let user_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE email = 'perm_none@alioth.test' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::auth::login::configure),
    )
    .await;

    let token = mint_token(&ast, user_id, "perm_none@alioth.test");
    let req = test::TestRequest::get()
        .uri("/me")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let body: serde_json::Value = test::read_body_json(test::call_service(&app, req).await).await;

    assert_eq!(
        body["user"]["permissions"],
        serde_json::json!({}),
        "no relations → empty matrix"
    );

    cleanup(&pool, user_id, &[], &[]).await;
}

/// prohibition 扣减（fix-ngac-decision-consistency D6）：用户有效 UA 集命中的
/// active prohibition（conditions fail-closed 求值）从 permissions 矩阵剔除；
/// conditions 不满足的 prohibition 不扣减。
#[tokio::test]
async fn me_matrix_subtracts_prohibition() {
    let pool = connect().await;
    common::setup_schema(&pool).await.expect("schema");
    ensure_ngac(&pool).await;
    // prohibition 表兜底（扩展表，运行时 ensure 同源）
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.ngac_prohibition (
            id BIGINT DEFAULT isahl.gen_next_zuid() NOT NULL,
            o_name TEXT, fk_user_attribute BIGINT NOT NULL, fk_object_attribute BIGINT NOT NULL,
            ak_access_rights BIGINT[] NOT NULL DEFAULT '{}', is_active BOOLEAN DEFAULT TRUE,
            conditions JSONB, created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL, deleted_at TIMESTAMPTZ,
            CONSTRAINT ngac_prohibition_pkey PRIMARY KEY (id))",
    )
    .execute(&pool)
    .await
    .ok();
    let ast = test_auth_state();

    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, name, email, status, is_active, created_at, updated_at)
         VALUES (isahl.gen_next_zuid(), 'perm_proh', 'perm_proh@alioth.test', 'active', true, NOW(), NOW())
         ON CONFLICT (email) DO NOTHING",
    )
    .execute(&pool)
    .await
    .ok();
    let user_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.auth_users WHERE email = 'perm_proh@alioth.test' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let pc_id = default_policy_class(&pool).await;

    // 关联：engineers → read + create + delete
    let (ua, oa, _) = grant_relation(
        &pool,
        user_id,
        pc_id,
        &unique("proh_role"),
        "engineers_proh",
        &["read", "create", "delete"],
        None,
        None,
    )
    .await;

    // 无条件 prohibition：禁止 create → 矩阵剔除
    let create_ar = access_right_id(&pool, "create").await;
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_prohibition (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active)
         VALUES ($1, $2, $3, ARRAY[$4], TRUE)",
    )
    .bind(unique("proh_create"))
    .bind(ua)
    .bind(oa)
    .bind(create_ar)
    .execute(&pool)
    .await
    .unwrap();

    // 条件 prohibition（not_before 在未来）→ 不扣减 delete
    let delete_ar = access_right_id(&pool, "delete").await;
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_prohibition (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active, conditions)
         VALUES ($1, $2, $3, ARRAY[$4], TRUE, '{\"not_before\": \"2999-01-01T00:00:00Z\"}'::jsonb)",
    )
    .bind(unique("proh_delete_future"))
    .bind(ua)
    .bind(oa)
    .bind(delete_ar)
    .execute(&pool)
    .await
    .unwrap();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(ast.clone()))
            .configure(gateway_sso::auth::login::configure),
    )
    .await;

    let token = mint_token(&ast, user_id, "perm_proh@alioth.test");
    let req = test::TestRequest::get()
        .uri("/me")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();
    let body: serde_json::Value = test::read_body_json(test::call_service(&app, req).await).await;

    let perms = &body["user"]["permissions"];
    let engineers = perms["engineers_proh"]
        .as_array()
        .expect("engineers_proh matrix");
    let actions: Vec<&str> = engineers.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        actions.contains(&"read"),
        "未被禁止的 read 应保留: {:?}",
        actions
    );
    assert!(
        !actions.contains(&"create"),
        "conditions 满足的 prohibition 必须剔除 create: {:?}",
        actions
    );
    assert!(
        actions.contains(&"delete"),
        "conditions 不满足的 prohibition 不得扣减 delete: {:?}",
        actions
    );

    // 清理 prohibition + 关系
    sqlx::query("DELETE FROM isahl_auth.ngac_prohibition WHERE fk_user_attribute = $1")
        .bind(ua)
        .execute(&pool)
        .await
        .ok();
    cleanup(&pool, user_id, &[ua], &[oa]).await;
}
