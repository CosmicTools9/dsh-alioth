-- 010: SAML 2.0 单点登录支持
--
-- (1) 修复历史不一致：identity_providers 代码引用了 client_secret_encrypted /
--     jwks_uri / field_mapping 三列，但 live/test schema 仅存在 client_secret。
--     补列使 create_provider / get_provider_config 不再报错（不改变既有
--     OAuth 明文存储姿态，旧 client_secret 仅作兼容拷贝）。
-- (2) saml_states：SAML RelayState 绑定（防重放），镜像 oauth_states。
-- (3) user_saml_accounts：SAML IdP NameID 与本地用户的绑定，镜像 user_oauth_accounts。

ALTER TABLE isahl_auth.identity_providers
    ADD COLUMN IF NOT EXISTS client_secret_encrypted text,
    ADD COLUMN IF NOT EXISTS jwks_uri text,
    ADD COLUMN IF NOT EXISTS field_mapping jsonb DEFAULT '{}'::jsonb;

UPDATE isahl_auth.identity_providers
   SET client_secret_encrypted = client_secret
 WHERE client_secret IS NOT NULL
   AND client_secret_encrypted IS NULL;

CREATE TABLE IF NOT EXISTS isahl_auth.saml_states (
    state        text PRIMARY KEY,
    provider_id  bigint NOT NULL,
    redirect_url text,
    used         boolean NOT NULL DEFAULT false,
    used_at      timestamptz,
    expires_at   timestamptz NOT NULL DEFAULT (now() + interval '10 minutes')
);

CREATE TABLE IF NOT EXISTS isahl_auth.user_saml_accounts (
    id            bigint PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
    user_id       bigint NOT NULL,
    provider_id   bigint NOT NULL,
    name_id       text NOT NULL,
    email         text,
    display_name  text,
    raw_profile   jsonb,
    last_login_at timestamptz DEFAULT now(),
    UNIQUE (provider_id, name_id)
);

CREATE INDEX IF NOT EXISTS idx_user_saml_accounts_user
    ON isahl_auth.user_saml_accounts (user_id);
CREATE INDEX IF NOT EXISTS idx_user_saml_accounts_provider
    ON isahl_auth.user_saml_accounts (provider_id);
