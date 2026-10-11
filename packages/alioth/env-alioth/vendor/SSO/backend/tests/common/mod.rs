//! SSO 集成测试公共辅助函数
//!
//! 核心原则：测试数据绝不残留。
#![allow(dead_code)]
use gateway_sso::auth::AuthState;
use sqlx::{Executor, PgPool};
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

/// 构造测试用的 `Config`（`email_code_login_enabled` = env `SSO_EMAIL_CODE_LOGIN` 语义值）。
///
/// 为什么用字面量而非 `Config::from_env()`：开关值需逐测试确定，且进程 env 在并行测试间
/// 相互污染（同 `src/config.rs` 测试模块注释所述 set_var/remove_var 缺陷）——字面量构造
/// 让「默认关闭 / 显式启用」两条路径都确定性可测。
pub fn test_config(email_code_login_enabled: bool) -> gateway_sso::Config {
    gateway_sso::Config {
        server_addr: "0.0.0.0:9002".into(),
        database_url: String::new(),
        sso_jwt_private_key: String::new(),
        encryption_key: "test-encryption-key".into(),
        ngac_preview_dir: None,
        jwt_access_expiry: 900,
        jwt_refresh_expiry: 604800,
        oidc_issuer: "http://localhost:9002".into(),
        oidc_client_id: None,
        oidc_redirect_uris: vec![],
        log_level: "info".into(),
        identity_verify_mode: "local".into(),
        identity_external_verify_url: None,
        email_mode: "smtp".into(),
        email_code_login_enabled,
        sso_jwt_public_key_prev: None,
        db_pool_max: 30,
        workers: None,
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

    // 3. 创建 identity_providers 表（sso_sessions 依赖它）
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

    // 4. 创建邮箱/手机验证码表
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

    // auth_users 结构收敛 MUST 与生产同源（单一实现）：直接调用 crate 侧 ensure——
    // 覆盖通知偏好列/settings、账号枚举类型（status/user_type）、ldap jsonb 与
    // 冗余显示名/邮箱/LDAP 标记/绑定 DN 四列下线（openspec/specs/auth-users-schema）。
    // MUST NOT 在夹具里手抄 ALTER（REUSE_FIRST：第二实现会随后续结构变更漂移）。
    gateway_sso::ngac::ensure::ensure_ngac_extension_tables(pool).await;

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
    // email 唯一存储 = auth_user_emails（auth_users.email 冗余列已下线）：待清理用户 id MUST 在本
    // 语句块内先物化——邮箱行删除后无法再按 email 反查；且 auth_users（被引用行）MUST 后于其
    // 邮箱行/依赖行删除（入边 FK 无 ON DELETE CASCADE）。单事务保持原 raw_sql 的原子性。
    let mut tx = pool.begin().await?;
    let user_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT e.fk_user FROM isahl_auth.auth_user_emails e \
         JOIN isahl_auth.auth_users u ON u.id = e.fk_user \
         WHERE (e.email LIKE '%@alioth.test' OR e.email LIKE '%@test.local') \
           AND u.username IS DISTINCT FROM 'system'",
    )
    .fetch_all(&mut *tx)
    .await?;
    if !user_ids.is_empty() {
        sqlx::query("DELETE FROM isahl_auth.auth_user_emails WHERE fk_user = ANY($1)")
            .bind(&user_ids)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM isahl_auth.session_revocations WHERE user_id = ANY($1)")
            .bind(&user_ids)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM isahl_auth.ngac_access_request WHERE fk_user = ANY($1)")
            .bind(&user_ids)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM isahl_auth.ngac_binding_request WHERE fk_user = ANY($1)")
            .bind(&user_ids)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM isahl_auth.ngac_delegation \
             WHERE fk_delegator = ANY($1) OR fk_delegatee = ANY($1)",
        )
        .bind(&user_ids)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = ANY($1)")
            .bind(&user_ids)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
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

/// 按 email 精确清理用户（依赖行先行——auth_user_emails 入边 FK 无
/// ON DELETE CASCADE，先删子行再删主行，否则 DELETE 静默失败残留 → 固定邮箱重跑 409）。
///
/// email 唯一存储 = auth_user_emails（auth_users.email 冗余列已下线）：待清理用户 id MUST
/// 在删邮箱行之前物化（事后无法再按 email 反查），删除按 id 收口（不再读 auth_users.email）。
pub async fn cleanup_user_by_email(pool: &PgPool, email: &str) -> Result<(), sqlx::Error> {
    let user_ids: Vec<i64> =
        sqlx::query_scalar("SELECT fk_user FROM isahl_auth.auth_user_emails WHERE email = $1")
            .bind(email)
            .fetch_all(pool)
            .await?;
    // 子行全清（含该用户其余邮箱行，与 email 冗余列时代的 `fk_user IN (…)` 语义一致）
    sqlx::query(
        "DELETE FROM isahl_auth.auth_user_emails \
         WHERE email = $1 OR fk_user = ANY($2)",
    )
    .bind(email)
    .bind(&user_ids)
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = ANY($1)")
        .bind(&user_ids)
        .execute(pool)
        .await?;
    Ok(())
}

/// Framework 内层门探针（establish-age-graph-analysis-plane 5.3 等价性守门）：
/// `require_resource_access` 唯一实现的直调面——dry-run 预测 vs 真实写库后决策对比用。
pub async fn permissions_probe(
    pool: &PgPool,
    user_id: i64,
    resource_type: &str,
    resource_id: i64,
    action: &str,
) -> bool {
    ::common::permissions::require_resource_access(
        pool,
        user_id,
        resource_type,
        resource_id,
        action,
    )
    .await
    .is_ok()
}
