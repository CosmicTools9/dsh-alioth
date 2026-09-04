-- 009_add_oidc_clients.sql
-- OIDC 多租户客户端注册表
--
-- 支持多个 RP（Relying Party）注册各自的 client_id、redirect_uri 和 client_secret。
-- 单租户配置（OIDC_CLIENT_ID / OIDC_REDIRECT_URIS 环境变量）仍作为兼容 fallback。

CREATE TABLE IF NOT EXISTS isahl_auth.oidc_clients (
    id              BIGSERIAL       PRIMARY KEY,
    client_id       VARCHAR(128)    NOT NULL UNIQUE,
    client_name     VARCHAR(256)    NOT NULL DEFAULT '',
    -- client_secret 的 SHA-256 散列；空值＝无 secret（public client，如 SPA）
    client_secret_hash VARCHAR(256) NOT NULL DEFAULT '',
    redirect_uris   TEXT[]          NOT NULL DEFAULT '{}',
    enabled         BOOLEAN         NOT NULL DEFAULT TRUE,
    created_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    deleted_at      TIMESTAMPTZ
);

COMMENT ON TABLE isahl_auth.oidc_clients IS 'OIDC RP 客户端注册表，支持多租户';
COMMENT ON COLUMN isahl_auth.oidc_clients.client_id IS 'RP 标识符，授权请求中必须匹配';
COMMENT ON COLUMN isahl_auth.oidc_clients.client_secret_hash IS 'client_secret 的 SHA-256 散列，空＝public client';
COMMENT ON COLUMN isahl_auth.oidc_clients.redirect_uris IS '允许的重定向 URI 白名单';
