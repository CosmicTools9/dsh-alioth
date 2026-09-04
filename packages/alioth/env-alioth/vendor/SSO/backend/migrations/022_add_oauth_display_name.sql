-- 022: user_oauth_accounts 补 display_name 列
--
-- 背景：social.rs list_social_accounts 查询 oa.display_name，但基础 schema
-- 的 user_oauth_accounts 无此列（dev/test 实测均缺失）→ GET /auth/social/accounts
-- 运行时 42703 错误。user_saml_accounts（010）已有 display_name，此处对齐。
-- 与 010 为 identity_providers 补列的先例一致：SSO 迁移对基础 schema 做增量。

ALTER TABLE isahl_auth.user_oauth_accounts
    ADD COLUMN IF NOT EXISTS display_name text;
