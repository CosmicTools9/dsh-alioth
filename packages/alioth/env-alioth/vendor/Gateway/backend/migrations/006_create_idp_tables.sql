-- ⚠️ 废弃声明：此迁移已被 Alioth 标准 DDL 取代。
-- 身份提供商、OAuth 账户绑定及状态表的标准定义已迁移至 DB schema（isahl_auth 表）
-- 主键应使用 BIGINT DEFAULT isahl.gen_next_zuid()。
--
-- 保留以下内容仅作历史参考，不应再执行或作为新标准使用。

-- 身份提供商表
-- 存储支持的 OAuth2/OIDC 身份提供商配置

-- 身份提供商表
CREATE TABLE IF NOT EXISTS identity_providers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    provider_type TEXT NOT NULL CHECK (provider_type IN ('oauth2', 'oidc')),
    
    -- OAuth2/OIDC 配置
    authorization_endpoint TEXT NOT NULL,
    token_endpoint TEXT NOT NULL,
    userinfo_endpoint TEXT,
    jwks_uri TEXT,
    
    -- 客户端凭证 (加密存储)
    client_id TEXT NOT NULL,
    client_secret_encrypted TEXT NOT NULL,
    
    -- 作用域配置
    scopes TEXT NOT NULL DEFAULT 'openid email profile',
    
    -- 字段映射 (JSON 格式)
    field_mapping JSONB DEFAULT '{
        "id": "sub",
        "email": "email",
        "name": "name",
        "picture": "picture"
    }'::jsonb,
    
    -- 状态
    enabled BOOLEAN NOT NULL DEFAULT true,
    
    -- 元数据
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- 用户 OAuth 账户绑定表
CREATE TABLE IF NOT EXISTS user_oauth_accounts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider_id UUID NOT NULL REFERENCES identity_providers(id) ON DELETE CASCADE,
    
    -- 外部用户标识
    provider_user_id TEXT NOT NULL,
    
    -- 外部用户信息 (缓存)
    email TEXT,
    display_name TEXT,
    avatar_url TEXT,
    raw_profile JSONB,
    
    -- 访问令牌 (加密存储)
    access_token_encrypted TEXT,
    refresh_token_encrypted TEXT,
    token_expires_at TIMESTAMP WITH TIME ZONE,
    
    -- 元数据
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    
    -- 唯一约束：一个用户在一个提供商下只能有一个账户
    UNIQUE(user_id, provider_id),
    -- 唯一约束：一个提供商账户只能绑定一个用户
    UNIQUE(provider_id, provider_user_id)
);

-- OAuth 状态表 (用于 state 和 PKCE 验证)
CREATE TABLE IF NOT EXISTS oauth_states (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    state TEXT NOT NULL UNIQUE,
    pkce_code_verifier_hash TEXT NOT NULL, -- SHA256 hash
    
    -- 可选：预绑定用户 (用于已登录用户绑定新 OAuth 账户)
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    
    -- 提供商
    provider_id UUID NOT NULL REFERENCES identity_providers(id) ON DELETE CASCADE,
    
    -- 重定向 URL (登录成功后跳转)
    redirect_url TEXT,
    
    -- 过期时间 (10 分钟)
    expires_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT (NOW() + INTERVAL '10 minutes'),
    
    -- 使用状态
    used BOOLEAN NOT NULL DEFAULT false,
    used_at TIMESTAMP WITH TIME ZONE,
    
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- 索引
CREATE INDEX IF NOT EXISTS idx_oauth_accounts_user_id ON user_oauth_accounts(user_id);
CREATE INDEX IF NOT EXISTS idx_oauth_accounts_provider ON user_oauth_accounts(provider_id, provider_user_id);
CREATE INDEX IF NOT EXISTS idx_oauth_states_state ON oauth_states(state);
CREATE INDEX IF NOT EXISTS idx_oauth_states_expires ON oauth_states(expires_at) WHERE used = false;

-- 插入默认身份提供商配置 (需要手动配置 client_id 和 client_secret)
INSERT INTO identity_providers (
    name, display_name, provider_type,
    authorization_endpoint, token_endpoint, userinfo_endpoint, jwks_uri,
    client_id, client_secret_encrypted, scopes, field_mapping
) VALUES 
-- Google
(
    'google',
    'Google',
    'oidc',
    'https://accounts.google.com/o/oauth2/v2/auth',
    'https://oauth2.googleapis.com/token',
    'https://openidconnect.googleapis.com/v1/userinfo',
    'https://www.googleapis.com/oauth2/v3/certs',
    'PLACEHOLDER_CLIENT_ID',
    'PLACEHOLDER_CLIENT_SECRET',
    'openid email profile',
    '{"id": "sub", "email": "email", "name": "name", "picture": "picture"}'::jsonb
),
-- GitHub
(
    'github',
    'GitHub',
    'oauth2',
    'https://github.com/login/oauth/authorize',
    'https://github.com/login/oauth/access_token',
    'https://api.github.com/user',
    NULL,
    'PLACEHOLDER_CLIENT_ID',
    'PLACEHOLDER_CLIENT_SECRET',
    'read:user user:email',
    '{"id": "id", "email": "email", "name": "name", "picture": "avatar_url"}'::jsonb
),
-- Microsoft
(
    'microsoft',
    'Microsoft',
    'oidc',
    'https://login.microsoftonline.com/common/oauth2/v2.0/authorize',
    'https://login.microsoftonline.com/common/oauth2/v2.0/token',
    'https://graph.microsoft.com/oidc/userinfo',
    'https://login.microsoftonline.com/common/discovery/v2.0/keys',
    'PLACEHOLDER_CLIENT_ID',
    'PLACEHOLDER_CLIENT_SECRET',
    'openid email profile',
    '{"id": "sub", "email": "email", "name": "name", "picture": "picture"}'::jsonb
)
ON CONFLICT (name) DO NOTHING;
