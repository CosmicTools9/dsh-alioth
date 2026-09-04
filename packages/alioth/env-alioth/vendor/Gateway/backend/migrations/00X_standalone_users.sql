-- Gateway standalone users table
-- 用于无密码登录（standalone 模式），独立于 isahl_auth.auth_users（不共享 NGAC/MFA/OAuth FK）
-- id 使用 isahl.gen_next_zuid()（遵守 §11.3：isahl_auth schema 表用 zuid）
-- 其余字段与 AppCreator app_creator.users 同构

CREATE TABLE IF NOT EXISTS isahl_auth.standalone_users (
    id             bigint  PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
    username       text    NOT NULL,
    username_norm  text    NOT NULL,  -- lower(trim(username)) 生成列
    namespace      text    NOT NULL,  -- 编码 NS-<PascalUsername>
    created_at     timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_standalone_users_username_norm
    ON isahl_auth.standalone_users (username_norm);
CREATE UNIQUE INDEX IF NOT EXISTS uq_standalone_users_namespace
    ON isahl_auth.standalone_users (namespace);
