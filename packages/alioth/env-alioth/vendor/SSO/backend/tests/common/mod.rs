//! SSO 集成测试公共辅助函数
//!
//! 核心原则：测试数据绝不残留。
#![allow(dead_code)]
use gateway_sso::auth::AuthState;
use sqlx::{AssertSqlSafe, Executor, PgPool};
use std::path::PathBuf;
use tokio::sync::OnceCell;

/// 测试用 EC P-256 私钥（PKCS#8 PEM）
const TEST_JWT_PRIVATE_KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----
\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgD/UpJ7dxbI+3BhJs
\
dDIxSFS+tdT9wSzVVS8z+Au6MRahRANCAATEcFhYPhVkFdIGNAiBwxQpu0cYRXc0
\
roJB3RHF1LfIsaCxcnVep0snC4+8StUixIjfLAZ8Mc8+uqa43ndeNEFm
\
-----END PRIVATE KEY-----";

/// 测试用 EC P-256 公钥（PKCS#8 PEM）
const TEST_JWT_PUBLIC_KEY: &[u8] = b"-----BEGIN PUBLIC KEY-----
\
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAExHBYWD4VZBXSBjQIgcMUKbtHGEV3
\
NK6CQd0RxdS3yLGgsXJ1XqdLJwuPvErVIsSI3ywGfDHPPrqmuN53XjRBZg==
\
-----END PUBLIC KEY-----";

/// 构造测试用的 AuthState（ES256 密钥对）
pub fn test_auth_state() -> AuthState {
    AuthState {
        jwt_private_key: TEST_JWT_PRIVATE_KEY.to_vec(),
        jwt_public_key: TEST_JWT_PUBLIC_KEY.to_vec(),
        jwt_public_keys_prev: vec![],
        encryption_key: b"test-encryption-key-16bytes".to_vec(),
        ngac_preview_dir: None,
        jwt_access_expiry_secs: 900,
        jwt_refresh_expiry_secs: 604800,
        identity_verify_mode: "local".to_string(),
        identity_external_verify_url: None,
    }
}

/// 全局幂等标志：每个测试 binary 只执行一次 schema 初始化，避免
/// `CREATE TABLE/INDEX/TYPE` 在已存在的对象上产生并发冲突。
static SETUP_DONE: OnceCell<()> = OnceCell::const_new();

/// 加载 SSO 测试所需的 schema 和表
pub async fn setup_schema(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    // OnceCell::get_or_try_init 自带并发去重：第一个调用执行初始化，后续直接返回成功
    SETUP_DONE
        .get_or_try_init(|| async { do_setup_schema(pool).await.map(|_| ()) })
        .await
        .map(|_| ())
}

async fn do_setup_schema(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    // 1. 手动创建基础 schema（避免 CREATE EXTENSION 的权限问题）
    pool.execute("CREATE SCHEMA IF NOT EXISTS isahl").await?;
    pool.execute("CREATE SCHEMA IF NOT EXISTS isahl_auth")
        .await?;
    pool.execute("CREATE SCHEMA IF NOT EXISTS isahl_audit")
        .await?;

    // 2. 创建 ZUID 函数（必须在 auth 表之前）
    // 使用内联简化版，避免文件解析问题
    pool.execute(r#"
        CREATE SEQUENCE IF NOT EXISTS isahl.zuid_sequence
            MINVALUE 0 MAXVALUE 2047 CYCLE;

        CREATE OR REPLACE FUNCTION isahl.gen_next_zuid()
        RETURNS BIGINT
        LANGUAGE plpgsql
        AS $$
        DECLARE
            epoch_millis BIGINT := 1622505600000;
            timestamp_mask BIGINT := (1::BIGINT << 40) - 1;
            sequence_mask BIGINT := 2047;
            now_ms BIGINT;
            timestamp_val BIGINT;
            seq_val BIGINT;
        BEGIN
            now_ms := (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT;
            timestamp_val := now_ms - epoch_millis;
            IF timestamp_val > timestamp_mask THEN
                RAISE EXCEPTION 'Timestamp overflow: ZUID timestamp (40-bit) exhausted. now_ms=%, epoch=%, val=%, mask=%',
                    now_ms, epoch_millis, timestamp_val, timestamp_mask;
            END IF;
            seq_val := nextval('isahl.zuid_sequence') & sequence_mask;
            RETURN ((timestamp_val & timestamp_mask) << 11) | (seq_val & sequence_mask);
        END;
        $$;
    "#).await?;

    // 3. 加载 001_auth_tables.sql（不依赖其他表）
    let migrations_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/db/migrations");

    let auth_tables_path = migrations_dir.join("001_auth_tables.sql");
    if auth_tables_path.exists() {
        let sql = tokio::fs::read_to_string(&auth_tables_path).await?;
        pool.execute(AssertSqlSafe(sql)).await?;
    }

    // 4. 预先创建 identity_providers 表（因为 002 中的 sso_sessions 依赖它）
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.identity_providers (
            id BIGINT PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
            name TEXT NOT NULL,
            provider_type TEXT NOT NULL,
            client_id TEXT,
            client_secret_encrypted TEXT,
            jwks_uri TEXT,
            authorization_endpoint TEXT,
            token_endpoint TEXT,
            userinfo_endpoint TEXT,
            scopes TEXT[],
            enabled BOOLEAN DEFAULT true,
            config JSONB DEFAULT '{}',
            field_mapping JSONB DEFAULT '{}',
            created_at TIMESTAMPTZ DEFAULT NOW(),
            updated_at TIMESTAMPTZ DEFAULT NOW()
        )
    "#,
    )
    .await?;

    // 4c. OpenAPI 调用方注册表（迁移 026，api_key_create_and_authenticate 等测试依赖；
    //     与生产 schema 对齐——api_clients 承载 apikey/oauth2 调用方 + 订阅绑定）
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.api_clients (
            id              BIGSERIAL    PRIMARY KEY,
            client_id       VARCHAR(128) NOT NULL UNIQUE,
            client_type     VARCHAR(16)  NOT NULL DEFAULT 'oauth2'
                            CHECK (client_type IN ('apikey','oauth2')),
            client_name     VARCHAR(256) NOT NULL DEFAULT '',
            secret_hash     VARCHAR(256) NOT NULL DEFAULT '',
            scopes          TEXT[]       NOT NULL DEFAULT '{}',
            fk_service_user BIGINT       NOT NULL,
            enabled         BOOLEAN      NOT NULL DEFAULT TRUE,
            expires_at      TIMESTAMPTZ,
            last_used_at    TIMESTAMPTZ,
            created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
            deleted_at      TIMESTAMPTZ
        )
    "#,
    )
    .await?;
    pool.execute(
        "CREATE INDEX IF NOT EXISTS idx_api_clients_prefix ON isahl_auth.api_clients (left(client_id, 8))",
    )
    .await?;
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.api_plans (
            id               BIGSERIAL     PRIMARY KEY,
            code             VARCHAR(32)   NOT NULL UNIQUE,
            tier             SMALLINT      NOT NULL DEFAULT 0,
            rate_limit_rps   NUMERIC(10,2) NOT NULL DEFAULT 1.00,
            burst            INT           NOT NULL DEFAULT 5,
            quota_daily      BIGINT        NOT NULL DEFAULT 0,
            quota_monthly    BIGINT        NOT NULL DEFAULT 0,
            sla_availability NUMERIC(5,4)  NOT NULL DEFAULT 0.990,
            sla_p95_ms       INT           NOT NULL DEFAULT 0,
            support_level    VARCHAR(16)   NOT NULL DEFAULT 'community',
            enabled          BOOLEAN       NOT NULL DEFAULT TRUE,
            created_at       TIMESTAMPTZ   NOT NULL DEFAULT NOW(),
            deleted_at       TIMESTAMPTZ
        )
    "#,
    )
    .await?;
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.api_subscriptions (
            id          BIGSERIAL   PRIMARY KEY,
            fk_client   BIGINT      NOT NULL REFERENCES isahl_auth.api_clients(id),
            fk_plan     BIGINT      NOT NULL REFERENCES isahl_auth.api_plans(id),
            status      VARCHAR(16) NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active','suspended','canceled')),
            starts_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            expires_at  TIMESTAMPTZ,
            created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            deleted_at  TIMESTAMPTZ
        )
    "#,
    )
    .await?;
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.api_usage (
            id               BIGSERIAL    PRIMARY KEY,
            fk_subscription  BIGINT       NOT NULL,
            route            VARCHAR(255) NOT NULL,
            method           VARCHAR(8)   NOT NULL,
            status           SMALLINT     NOT NULL,
            latency_ms       INT          NOT NULL DEFAULT 0,
            client_ip        INET,
            requested_at     TIMESTAMPTZ  NOT NULL DEFAULT NOW()
        )
    "#,
    )
    .await?;

    // 5. 加载 002_auth_session_tables.sql
    let session_tables_path = migrations_dir.join("002_auth_session_tables.sql");
    if session_tables_path.exists() {
        let sql = tokio::fs::read_to_string(&session_tables_path).await?;
        pool.execute(AssertSqlSafe(sql)).await?;
    }

    // 6. 创建邮箱/手机验证码表
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.auth_email_verifications (
            id BIGSERIAL PRIMARY KEY,
            email TEXT NOT NULL,
            code TEXT NOT NULL,
            purpose TEXT NOT NULL DEFAULT 'register',
            expires_at TIMESTAMPTZ NOT NULL,
            verified BOOLEAN DEFAULT FALSE,
            created_at TIMESTAMPTZ DEFAULT NOW(),
            updated_at TIMESTAMPTZ DEFAULT NOW()
        )
    "#,
    )
    .await?;
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.auth_phone_verifications (
            id BIGSERIAL PRIMARY KEY,
            phone TEXT NOT NULL,
            code TEXT NOT NULL,
            purpose TEXT NOT NULL DEFAULT 'register',
            expires_at TIMESTAMPTZ NOT NULL,
            verified BOOLEAN DEFAULT FALSE,
            created_at TIMESTAMPTZ DEFAULT NOW(),
            updated_at TIMESTAMPTZ DEFAULT NOW()
        )
    "#,
    )
    .await?;

    // WebAuthn / Passkey 表（凭据与挑战状态）
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.webauthn_credentials (
            id BIGSERIAL PRIMARY KEY,
            user_id BIGINT NOT NULL REFERENCES isahl_auth.auth_users(id),
            credential_id BYTEA NOT NULL UNIQUE,
            public_key_cose BYTEA NOT NULL,
            sign_count BIGINT NOT NULL DEFAULT 0,
            credential_type TEXT NOT NULL DEFAULT 'passkey',
            transports TEXT NOT NULL DEFAULT '[]',
            aaguid TEXT,
            device_name TEXT,
            last_used_at TIMESTAMPTZ,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            deleted_at TIMESTAMPTZ
        )
    "#,
    )
    .await?;
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.webauthn_challenges (
            challenge TEXT PRIMARY KEY,
            user_id BIGINT NOT NULL,
            purpose TEXT NOT NULL,
            state TEXT NOT NULL,
            expires_at TIMESTAMPTZ NOT NULL
        )
    "#,
    )
    .await?;

    // 通知偏好列（迁移 016_add_notification_preferences.sql）。幂等，避免
    // 在已迁移的测试库上重复执行失败；同时保证全新测试库拥有该列。
    pool.execute(
        "ALTER TABLE IF EXISTS isahl_auth.auth_users \
         ADD COLUMN IF NOT EXISTS notification_preferences JSONB NOT NULL DEFAULT '{}'::jsonb",
    )
    .await?;

    // SCIM external_id 存储列（auth_users.settings，dev 库 ad-hoc 增列未回写迁移；
    // scim create_user/get_user 读写 settings->>'scim_external_id'）。幂等补列。
    pool.execute(
        "ALTER TABLE IF EXISTS isahl_auth.auth_users \
         ADD COLUMN IF NOT EXISTS settings JSONB DEFAULT '{}'::jsonb",
    )
    .await?;

    // 诊断：查看 gen_zuid 函数的 epoch_millis
    let func_src: Result<(String,), sqlx::Error> = sqlx::query_as(
        "SELECT prosrc FROM pg_proc WHERE proname = 'gen_zuid' AND pronamespace = (SELECT oid FROM pg_namespace WHERE nspname = 'isahl')"
    )
    .fetch_one(pool)
    .await;
    match &func_src {
        Ok((src,)) => {
            let epoch_line = src.lines().find(|l| l.contains("epoch_millis"));
            eprintln!("🔍 gen_zuid source epoch line: {:?}", epoch_line);
        }
        Err(e) => eprintln!("⚠️  Failed to get gen_zuid source: {}", e),
    }

    // 诊断：验证 ZUID 函数
    let diag: Result<(i64, i64, i64, i64), sqlx::Error> = sqlx::query_as(
        "SELECT \
         (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT as now_ms, \
         1622505600000::BIGINT as epoch_millis, \
         ((EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT - 1622505600000::BIGINT) as timestamp_val, \
         ((1::BIGINT << 40) - 1) as timestamp_mask"
    )
    .fetch_one(pool)
    .await;
    match &diag {
        Ok((now_ms, epoch_millis, timestamp_val, timestamp_mask)) => {
            eprintln!(
                "🔍 ZUID diag: now_ms={}, epoch_millis={}, val={}, mask={}",
                now_ms, epoch_millis, timestamp_val, timestamp_mask
            );
        }
        Err(e) => eprintln!("⚠️  ZUID diag query failed: {}", e),
    }

    let zuid_diag: Result<(i64,), sqlx::Error> = sqlx::query_as("SELECT isahl.gen_next_zuid()")
        .fetch_one(pool)
        .await;
    match &zuid_diag {
        Ok((zuid,)) => eprintln!("✅ ZUID diagnostic: {}", zuid),
        Err(e) => eprintln!("⚠️  ZUID diagnostic failed: {}", e),
    }

    // 诊断：查看 sso_sessions 实际在哪个 schema
    let schema_check: Result<(String,), sqlx::Error> = sqlx::query_as(
        "SELECT table_schema FROM information_schema.tables WHERE table_name = 'sso_sessions'",
    )
    .fetch_one(pool)
    .await;
    match &schema_check {
        Ok((schema,)) => eprintln!("🔍 sso_sessions is in schema: {}", schema),
        Err(e) => eprintln!("⚠️  sso_sessions schema check failed: {}", e),
    }

    // 诊断：验证 sso_sessions 能否写入（user_id 用真实存在的用户，避免 FK 误报）
    let diag = sqlx::query(
        "INSERT INTO isahl_auth.sso_sessions (user_id, session_token, expires_at) \
         SELECT id, 'diag-test', NOW() + INTERVAL '1 day' FROM isahl_auth.auth_users LIMIT 1",
    )
    .execute(pool)
    .await;
    if let Err(e) = &diag {
        eprintln!("⚠️  sso_sessions diagnostic insert failed: {}", e);
    } else {
        sqlx::query("DELETE FROM isahl_auth.sso_sessions WHERE session_token = 'diag-test'")
            .execute(pool)
            .await?;
    }

    // 同源结构自愈（fix-sso-id-default-heal）：复用 gateway_sso::ngac::ensure 的
    // 运行时自愈（扩展表 + id 默认全量 + 核心约束 + 审计分区），禁止测试侧维护
    // 第二份自愈 SQL。失败语义与生产一致（warn 不阻断），缺失结构由后续断言暴露。
    gateway_sso::ngac::ensure::ensure_ngac_extension_tables(pool).await;
    Ok(())
}

/// 清理 SSO 测试用户（按 email 后缀）。依赖行先行——`auth_user_emails`/
/// `session_revocations`/`ngac_access_request` 等入边 FK 无 ON DELETE CASCADE，
/// 残留依赖会让本 DELETE 静默失败（调用方 `.ok()` 吞错）→ 固定邮箱重跑 409。
/// 排除 `username='system'`（seed 契约基础设施账号，bootstrap 测试依赖）。
pub async fn cleanup_test_users(pool: &PgPool) -> Result<(), sqlx::Error> {
    // 覆盖范围：@alioth.test（多数集成测试）与 @test.local（reset_password_test / me_enrichment_test 等）。
    sqlx::raw_sql(sqlx::AssertSqlSafe(
        r#"
        DELETE FROM isahl_auth.auth_user_emails WHERE fk_user IN (
            SELECT id FROM isahl_auth.auth_users
            WHERE (email LIKE '%@alioth.test' OR email LIKE '%@test.local')
              AND username IS DISTINCT FROM 'system');
        DELETE FROM isahl_auth.session_revocations WHERE user_id IN (
            SELECT id FROM isahl_auth.auth_users
            WHERE (email LIKE '%@alioth.test' OR email LIKE '%@test.local')
              AND username IS DISTINCT FROM 'system');
        DELETE FROM isahl_auth.ngac_access_request WHERE fk_user IN (
            SELECT id FROM isahl_auth.auth_users
            WHERE (email LIKE '%@alioth.test' OR email LIKE '%@test.local')
              AND username IS DISTINCT FROM 'system');
        DELETE FROM isahl_auth.ngac_binding_request WHERE fk_user IN (
            SELECT id FROM isahl_auth.auth_users
            WHERE (email LIKE '%@alioth.test' OR email LIKE '%@test.local')
              AND username IS DISTINCT FROM 'system');
        DELETE FROM isahl_auth.ngac_delegation WHERE fk_delegator IN (
            SELECT id FROM isahl_auth.auth_users
            WHERE (email LIKE '%@alioth.test' OR email LIKE '%@test.local')
              AND username IS DISTINCT FROM 'system')
            OR fk_delegatee IN (
            SELECT id FROM isahl_auth.auth_users
            WHERE (email LIKE '%@alioth.test' OR email LIKE '%@test.local')
              AND username IS DISTINCT FROM 'system');
        DELETE FROM isahl_auth.auth_users
         WHERE (email LIKE '%@alioth.test' OR email LIKE '%@test.local')
           AND username IS DISTINCT FROM 'system';
        "#,
    ))
    .execute(pool)
    .await?;
    Ok(())
}

/// 幂等确保 `isahl_auth.auth_users` 表可用（ngac_graph_snapshot_test 前置依赖，
/// 9a816ba39 引入时悬空引用——测试库未初始化 schema 时自足建表）。
pub async fn ensure_auth_users(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    setup_schema(pool).await?;
    pool.execute("SELECT 1 FROM isahl_auth.auth_users LIMIT 1")
        .await?;
    Ok(())
}

/// 按 email 精确清理用户
pub async fn cleanup_user_by_email(pool: &PgPool, email: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE email = $1")
        .bind(email)
        .execute(pool)
        .await?;
    Ok(())
}

/// 直接在数据库中写入一条"已验证"的邮箱验证码记录，绕过 send-code/verify-code 流程。
///
/// 适用于 register handler 要求 `email_verified = true` 的测试前置准备。
/// 注意：register handler 内部会用 lower(email) 查询，写入时统一使用 lower() 避免大小写问题。
pub async fn pre_verify_email(pool: &PgPool, email: &str) -> Result<(), sqlx::Error> {
    let email_lower = email.to_lowercase();
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.auth_email_verifications
            (email, code, purpose, expires_at, verified)
        VALUES ($1, '000000', 'register', NOW() + INTERVAL '1 day', TRUE)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(&email_lower)
    .execute(pool)
    .await?;
    // 删除可能残留的过期未验证记录，避免冲突
    sqlx::query(
        r#"
        DELETE FROM isahl_auth.auth_email_verifications
        WHERE email = $1 AND purpose = 'register' AND verified = FALSE
        "#,
    )
    .bind(&email_lower)
    .execute(pool)
    .await?;
    Ok(())
}
