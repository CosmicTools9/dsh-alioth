-- 012: OIDC 客户端授予的 scope 集合（client_credentials grant 范围限制）
--
-- client_credentials 流签发的 service token 需按客户端被授予的 scope 子集约束，
-- 因此记录每个 client 的授权 scope。空数组表示未配置 scope（兼容旧客户端，
-- 此时放行客户端请求的全部 scope）。

ALTER TABLE isahl_auth.oidc_clients
    ADD COLUMN IF NOT EXISTS scopes TEXT[] NOT NULL DEFAULT '{}';

COMMENT ON COLUMN isahl_auth.oidc_clients.scopes
    IS '该客户端被授予的 OAuth scope 集合；client_credentials 签发的 token 仅能携带其子集';
