-- 026_add_openapi_tables.sql
-- 目的：OpenAPI 外部开放体系（openspec/changes/add-openapi-external-access/）
--   1. api_clients —— 统一调用方注册表（apikey | oauth2），每 client 挂服务用户
--   2. api_plans —— 套餐（free/basic/pro/enterprise）：限流参数 + 配额 + SLA 承诺
--   3. api_subscriptions —— client ↔ plan 订阅绑定
--   4. api_usage —— 调用计量流水（SLA 报告 / 配额窗口计数）
-- 背景：现有 oidc_clients / api_keys 双表无法承载「订阅→限流参数→SLA」体系，
--       且服务令牌（sub=client:*）在 Gateway PEP 中解析为 user_id=0 无法走 NGAC。
-- 适用范围：所有含 isahl_auth schema 的数据库。
-- 幂等：CREATE TABLE IF NOT EXISTS + 种子按 code 幂等插入。

-- 1. 统一调用方注册表
CREATE TABLE IF NOT EXISTS isahl_auth.api_clients (
    id              BIGSERIAL    PRIMARY KEY,
    client_id       VARCHAR(128) NOT NULL UNIQUE,
    client_type     VARCHAR(16)  NOT NULL DEFAULT 'oauth2'
                    CHECK (client_type IN ('apikey','oauth2')),
    client_name     VARCHAR(256) NOT NULL DEFAULT '',
    secret_hash     VARCHAR(256) NOT NULL DEFAULT '',
    scopes          TEXT[]       NOT NULL DEFAULT '{}',
    fk_service_user BIGINT       NOT NULL,          -- isahl_auth.auth_users.id
    enabled         BOOLEAN      NOT NULL DEFAULT TRUE,
    expires_at      TIMESTAMPTZ,
    last_used_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    deleted_at      TIMESTAMPTZ
);

COMMENT ON TABLE isahl_auth.api_clients
    IS 'OpenAPI 统一调用方注册表（apikey|oauth2），fk_service_user 为 NGAC 服务主体';
COMMENT ON COLUMN isahl_auth.api_clients.secret_hash
    IS 'client_secret / api_key 的 argon2id 散列；DB 不存明文';
COMMENT ON COLUMN isahl_auth.api_clients.fk_service_user
    IS '关联 isahl_auth.auth_users 服务用户行，作为 NGAC 授权主体';
COMMENT ON COLUMN isahl_auth.api_clients.expires_at
    IS '密钥/客户端过期时间；NULL = 不过期';

-- 按 client_id 前缀快速定位（认证时缩小候选行）
CREATE INDEX IF NOT EXISTS idx_api_clients_prefix
    ON isahl_auth.api_clients (left(client_id, 8));

-- 2. 套餐定义
CREATE TABLE IF NOT EXISTS isahl_auth.api_plans (
    id               BIGSERIAL     PRIMARY KEY,
    code             VARCHAR(32)   NOT NULL UNIQUE,   -- free|basic|pro|enterprise
    tier             SMALLINT      NOT NULL DEFAULT 0,
    rate_limit_rps   NUMERIC(10,2) NOT NULL DEFAULT 1.00,  -- 速率：每秒令牌
    burst            INT           NOT NULL DEFAULT 5,      -- 突发桶容量
    quota_daily      BIGINT        NOT NULL DEFAULT 0,      -- 0 = 不限
    quota_monthly    BIGINT        NOT NULL DEFAULT 0,
    sla_availability NUMERIC(5,4)  NOT NULL DEFAULT 0.990,
    sla_p95_ms       INT           NOT NULL DEFAULT 0,      -- 0 = 无承诺
    support_level    VARCHAR(16)   NOT NULL DEFAULT 'community',
    enabled          BOOLEAN       NOT NULL DEFAULT TRUE,
    created_at       TIMESTAMPTZ   NOT NULL DEFAULT NOW(),
    deleted_at       TIMESTAMPTZ
);

COMMENT ON TABLE isahl_auth.api_plans
    IS 'OpenAPI 套餐：限流参数（RPS/burst）+ 配额（日/月）+ SLA 承诺（可用性/P95/支持）';

-- 3. 订阅绑定（client ↔ plan）
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
);

COMMENT ON TABLE isahl_auth.api_subscriptions
    IS 'OpenAPI 订阅：client 绑定套餐，status 控制访问（active/suspended/canceled）';

CREATE INDEX IF NOT EXISTS idx_api_subscriptions_client
    ON isahl_auth.api_subscriptions (fk_client) WHERE deleted_at IS NULL;

-- 4. 调用计量流水
CREATE TABLE IF NOT EXISTS isahl_auth.api_usage (
    id               BIGSERIAL    PRIMARY KEY,
    fk_subscription  BIGINT       NOT NULL,
    route            VARCHAR(255) NOT NULL,
    method           VARCHAR(8)   NOT NULL,
    status           SMALLINT     NOT NULL,
    latency_ms       INT          NOT NULL DEFAULT 0,
    client_ip        INET,
    requested_at     TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE isahl_auth.api_usage
    IS 'OpenAPI 调用计量流水（SLA 报告 + 配额窗口计数）；保留 90 天 + 归档';
COMMENT ON COLUMN isahl_auth.api_usage.fk_subscription
    IS '关联 api_subscriptions.id；订阅删除后保留（计量不随订阅删除）';

CREATE INDEX IF NOT EXISTS idx_api_usage_sub_time
    ON isahl_auth.api_usage (fk_subscription, requested_at);
CREATE INDEX IF NOT EXISTS idx_api_usage_time
    ON isahl_auth.api_usage (requested_at);

-- 5. 种子套餐（幂等：按 code 检查）
DO $$
DECLARE
    v_id BIGINT;
BEGIN
    -- free
    IF NOT EXISTS (SELECT 1 FROM isahl_auth.api_plans WHERE code = 'free') THEN
        INSERT INTO isahl_auth.api_plans
            (code, tier, rate_limit_rps, burst, quota_daily, quota_monthly,
             sla_availability, sla_p95_ms, support_level)
        VALUES ('free', 0, 1.00, 5, 1000, 30000,
                0.990, 0, 'community');
    END IF;
    -- basic
    IF NOT EXISTS (SELECT 1 FROM isahl_auth.api_plans WHERE code = 'basic') THEN
        INSERT INTO isahl_auth.api_plans
            (code, tier, rate_limit_rps, burst, quota_daily, quota_monthly,
             sla_availability, sla_p95_ms, support_level)
        VALUES ('basic', 1, 10.00, 20, 50000, 1500000,
                0.995, 2000, 'standard');
    END IF;
    -- pro
    IF NOT EXISTS (SELECT 1 FROM isahl_auth.api_plans WHERE code = 'pro') THEN
        INSERT INTO isahl_auth.api_plans
            (code, tier, rate_limit_rps, burst, quota_daily, quota_monthly,
             sla_availability, sla_p95_ms, support_level)
        VALUES ('pro', 2, 50.00, 100, 500000, 15000000,
                0.999, 800, 'priority');
    END IF;
    -- enterprise
    IF NOT EXISTS (SELECT 1 FROM isahl_auth.api_plans WHERE code = 'enterprise') THEN
        INSERT INTO isahl_auth.api_plans
            (code, tier, rate_limit_rps, burst, quota_daily, quota_monthly,
             sla_availability, sla_p95_ms, support_level)
        VALUES ('enterprise', 3, 200.00, 500, 0, 0,
                0.9995, 500, 'dedicated');
    END IF;
END $$;
