-- 013: API 密钥表（服务到服务认证 / Machine-to-Machine）
--
-- 后端服务（CI/CD、定时同步、消息队列消费者）使用 API 密钥换取短时效 JWT。
-- 明文密钥仅创建时返回一次；DB 仅存 argon2id 散列与展示用前缀。

CREATE TABLE IF NOT EXISTS isahl_auth.api_keys (
    id           BIGSERIAL      PRIMARY KEY,
    client_name  TEXT           NOT NULL,
    key_hash     TEXT           NOT NULL,
    key_prefix   VARCHAR(8)     NOT NULL,
    scopes       TEXT[]         NOT NULL DEFAULT '{}',
    enabled      BOOLEAN        NOT NULL DEFAULT TRUE,
    created_by   BIGINT         NOT NULL DEFAULT 0,
    expires_at   TIMESTAMPTZ    NULL,
    last_used_at TIMESTAMPTZ    NULL,
    created_at   TIMESTAMPTZ    NOT NULL DEFAULT NOW(),
    deleted_at   TIMESTAMPTZ
);

COMMENT ON TABLE isahl_auth.api_keys
    IS '服务到服务 API 密钥（argon2id hash 存储，明文仅创建时返回一次）';
COMMENT ON COLUMN isahl_auth.api_keys.key_hash
    IS 'API 密钥的 argon2id 散列；DB 不存明文';
COMMENT ON COLUMN isahl_auth.api_keys.key_prefix
    IS '密钥前 8 字符（展示用，便于识别 ak_ 前缀），用于认证时缩小候选行';
COMMENT ON COLUMN isahl_auth.api_keys.scopes
    IS '该密钥被授予的 OAuth scope 集合';

-- 按前缀快速定位候选行（认证时先按前缀过滤，再 argon2id 校验）
CREATE INDEX IF NOT EXISTS idx_api_keys_prefix
    ON isahl_auth.api_keys (key_prefix);
