-- 028: 移除 SAML 2.0 支持（remove-saml-sso）
--
-- SAML 支持整体下线后，saml_states / user_saml_accounts 两张专用表与
-- provider_type='saml' 的 provider 均为死数据。
-- 010/011 迁移文件保留（迁移历史不可变），此处做反向清理。

DROP TABLE IF EXISTS isahl_auth.saml_states;
DROP TABLE IF EXISTS isahl_auth.user_saml_accounts;

DELETE FROM isahl_auth.identity_providers
 WHERE provider_type = 'saml';
