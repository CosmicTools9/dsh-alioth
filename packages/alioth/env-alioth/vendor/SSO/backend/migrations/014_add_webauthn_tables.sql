-- WebAuthn / Passkey 凭据与挑战状态表
-- 注意：本迁移编号 014（013 已被 API Key 管理占用）。

CREATE TABLE IF NOT EXISTS isahl_auth.webauthn_credentials (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES isahl_auth.auth_users(id) ON DELETE CASCADE,
    credential_id BYTEA NOT NULL UNIQUE,
    public_key_cose BYTEA NOT NULL,
    sign_count BIGINT NOT NULL DEFAULT 0,
    credential_type TEXT NOT NULL DEFAULT 'passkey',
    --  transports 以 JSON 数组文本存储（["usb","nfc","internal"]），避免 sqlx 数组类型推断歧义
    transports TEXT NOT NULL DEFAULT '[]',
    aaguid TEXT,
    device_name TEXT,
    last_used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_webauthn_credentials_user
    ON isahl_auth.webauthn_credentials(user_id) WHERE deleted_at IS NULL;

-- 注册 / 登录 challenge 状态（5 分钟 TTL），用于 begin/complete 防重放配对
CREATE TABLE IF NOT EXISTS isahl_auth.webauthn_challenges (
    challenge TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    purpose TEXT NOT NULL,
    state TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_webauthn_challenges_expires
    ON isahl_auth.webauthn_challenges(expires_at);
