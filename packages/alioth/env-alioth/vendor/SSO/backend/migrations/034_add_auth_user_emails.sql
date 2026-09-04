-- 034: auth_user_emails 表——用户多邮箱（email 为可选认证链路，非身份唯一基点）
-- 背景：注册曾以 email 为必填唯一基点 + 强制邮箱验证门禁（错误设计）。身份唯一基点
-- 应为账号 username；email 是 1:N 的可选认证/联系方式。本表承载「一个用户多个邮箱」。
-- auth_users.email 保留为可空「主邮箱」镜像（兼容既有读取方），不再作为注册唯一基点。

CREATE TABLE IF NOT EXISTS isahl_auth.auth_user_emails (
    id            bigint PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
    fk_user       bigint NOT NULL REFERENCES isahl_auth.auth_users(id),
    email         text NOT NULL,
    is_primary    boolean NOT NULL DEFAULT false,
    verified      boolean NOT NULL DEFAULT false,
    created_at    timestamp with time zone DEFAULT now() NOT NULL,
    updated_at    timestamp with time zone DEFAULT now() NOT NULL,
    deleted_at    timestamp with time zone,
    deleted_by_id bigint
);

COMMENT ON TABLE isahl_auth.auth_user_emails IS
  '用户邮箱（1:N）：email 为可选认证链路，非身份唯一基点；全系统一邮箱地址仅归属一个账号';
COMMENT ON COLUMN isahl_auth.auth_user_emails.email IS '邮箱地址，全局唯一（一邮箱一账号）';
COMMENT ON COLUMN isahl_auth.auth_user_emails.is_primary IS '是否主邮箱（auth_users.email 镜像源）';
COMMENT ON COLUMN isahl_auth.auth_user_emails.verified IS '是否已验证（仅当用户选择邮箱认证时）';

-- email 全局唯一（软删过滤，允许多次删除后复用）
CREATE UNIQUE INDEX IF NOT EXISTS uq_auth_user_emails_email
    ON isahl_auth.auth_user_emails (email) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_auth_user_emails_user
    ON isahl_auth.auth_user_emails (fk_user);
