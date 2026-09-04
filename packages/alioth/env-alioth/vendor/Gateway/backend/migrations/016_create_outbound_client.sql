-- isahl_auth 出向调用方注册表与计量（gateway-openapi-outbound-unify）
-- 跨 namespace 通用：任意 NS 对接第三方（FSSC 等）复用。
-- 幂等可重放；WZ 旧表（wz_fssc.outbound_*）数据迁移（若存在）。

-- ── 出向调用方注册表 ──
CREATE TABLE IF NOT EXISTS isahl_auth.outbound_client (
    id bigserial NOT NULL PRIMARY KEY,
    code text NOT NULL UNIQUE,
    provider text NOT NULL DEFAULT 'fssc',
    base_url text,
    app_id text,
    app_secret_enc text,               -- enc: 密文（AES-256-GCM，密钥 OUTBOUND_ENC_KEY）
    tenant_id text,
    account_id text,
    language text,
    "user" text,
    usertype text,
    workflow_view_user text,
    version integer NOT NULL DEFAULT 1,
    enabled boolean NOT NULL DEFAULT TRUE,
    created_at timestamptz DEFAULT now() NOT NULL,
    updated_at timestamptz DEFAULT now() NOT NULL,
    deleted_at timestamptz
);
CREATE INDEX IF NOT EXISTS idx_outbound_client_code_isahl ON isahl_auth.outbound_client(code)
    WHERE deleted_at IS NULL;

-- ── 出向调用计量（不含 payload/凭据）──
CREATE TABLE IF NOT EXISTS isahl_auth.outbound_call_log (
    id bigserial NOT NULL PRIMARY KEY,
    provider text NOT NULL,
    interface text NOT NULL,
    method text NOT NULL DEFAULT 'POST',
    status text,
    latency_ms bigint NOT NULL DEFAULT 0,
    request_id text,
    requested_at timestamptz DEFAULT now() NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_outbound_call_log_time_isahl ON isahl_auth.outbound_call_log(requested_at DESC);

-- ── WZ 旧表数据迁移（幂等；wz_fssc schema 存在且有行时复制）──
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.tables
        WHERE table_schema = 'wz_fssc' AND table_name = 'outbound_client'
    ) THEN
        INSERT INTO isahl_auth.outbound_client
            (id, code, provider, base_url, app_id, app_secret_enc, tenant_id, account_id,
             language, "user", usertype, workflow_view_user, version, enabled, created_at, updated_at, deleted_at)
        SELECT id, code, provider, base_url, app_id, app_secret_enc, tenant_id, account_id,
               language, "user", usertype, workflow_view_user, version, enabled, created_at, updated_at, deleted_at
        FROM wz_fssc.outbound_client
        WHERE NOT EXISTS (SELECT 1 FROM isahl_auth.outbound_client d WHERE d.code = wz_fssc.outbound_client.code);
    END IF;
    IF EXISTS (
        SELECT 1 FROM information_schema.tables
        WHERE table_schema = 'wz_fssc' AND table_name = 'outbound_call_log'
    ) THEN
        INSERT INTO isahl_auth.outbound_call_log
            (id, provider, interface, method, status, latency_ms, request_id, requested_at)
        SELECT id, provider, interface, method, status, latency_ms, request_id, requested_at
        FROM wz_fssc.outbound_call_log
        WHERE NOT EXISTS (SELECT 1 FROM isahl_auth.outbound_call_log d WHERE d.id = wz_fssc.outbound_call_log.id);
    END IF;
END $$;
