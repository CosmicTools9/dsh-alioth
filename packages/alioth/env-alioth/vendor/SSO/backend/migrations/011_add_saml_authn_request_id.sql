-- 011: SAML AuthnRequest ID 跟踪（用于 SubjectConfirmationData InResponseTo 校验）
--
-- 在 saml_states 中记录发起登录时的 AuthnRequest ID，以便 ACS 回调时
-- 验证 SAML Response 中的 SubjectConfirmationData/InResponseTo 与该 ID 匹配。

ALTER TABLE isahl_auth.saml_states
    ADD COLUMN IF NOT EXISTS authn_request_id text;
