-- 033_add_api_idempotency_keys.sql
--
-- 目的：OpenAPI 幂等键（openspec/changes/add-openapi-idempotency-keys/）——
--   api_idempotency_keys：第三方写请求（POST/PUT/PATCH /api/service/*）的
--   服务端幂等记录（首次执行存储响应快照 / 网络重试重放快照）。
-- 背景：开放 API 面向不可靠网络的第三方，重试导致重复创建实体/重复扣配额。
--       行业最佳实践（Stripe/Moesif）以 Idempotency-Key header 为写路径
--       correctness 底线；存储选 DB（唯一约束提供跨实例正确性，无 Redis 依赖）。
-- 适用范围：所有含 isahl_auth schema 的数据库。
-- 幂等：IF NOT EXISTS / ON CONFLICT，可重复执行。

CREATE TABLE IF NOT EXISTS isahl_auth.api_idempotency_keys (
    id                    BIGINT        PRIMARY KEY,
    fk_client             BIGINT        NOT NULL REFERENCES isahl_auth.api_clients(id),
    api_version           VARCHAR(32)   NOT NULL DEFAULT 'v1',
    idem_key              VARCHAR(255)  NOT NULL,
    method                VARCHAR(8)    NOT NULL,
    path                  VARCHAR(255)  NOT NULL,
    request_fingerprint   VARCHAR(64)   NOT NULL,
    state                 VARCHAR(16)   NOT NULL DEFAULT 'in_progress'
                          CHECK (state IN ('in_progress','completed')),
    response_status       SMALLINT,
    response_content_type VARCHAR(128),
    response_body         TEXT,
    created_at            TIMESTAMPTZ   NOT NULL DEFAULT NOW(),
    completed_at          TIMESTAMPTZ
);

COMMENT ON TABLE isahl_auth.api_idempotency_keys
    IS 'OpenAPI 幂等键记录：同 (client, api_version, key) 写请求重放首次响应快照；保留 24h';
COMMENT ON COLUMN isahl_auth.api_idempotency_keys.id
    IS 'isahl.gen_next_zuid() 生成（isahl_auth 链规则，与 api_clients/api_plans 一致）';
COMMENT ON COLUMN isahl_auth.api_idempotency_keys.fk_client
    IS '关联 api_clients.id；幂等作用域主体（服务令牌）';
COMMENT ON COLUMN isahl_auth.api_idempotency_keys.api_version
    IS '版本专项字段（预留版本机制）：来自 X-Api-Version header，默认 v1；参与唯一约束——同 key 跨版本互不冲突';
COMMENT ON COLUMN isahl_auth.api_idempotency_keys.request_fingerprint
    IS '请求指纹 sha256(method+path+body)；同 key 异指纹 → 409（防 key 重用攻击）';
COMMENT ON COLUMN isahl_auth.api_idempotency_keys.state
    IS 'in_progress=leader 执行中（并发 follower → 409）；completed=快照可重放';
COMMENT ON COLUMN isahl_auth.api_idempotency_keys.response_body
    IS '响应 body 快照，cap 256KB；超过则 NULL（重放仅返回 status，诚实降级不伪造）';

-- 协议核心：并发抢占靠唯一约束（INSERT ... ON CONFLICT DO NOTHING）
CREATE UNIQUE INDEX IF NOT EXISTS uq_api_idempotency_scope
    ON isahl_auth.api_idempotency_keys (fk_client, api_version, idem_key);

-- TTL 清理扫描（24h 过期，后台 10min 周期删除）
CREATE INDEX IF NOT EXISTS idx_api_idempotency_created
    ON isahl_auth.api_idempotency_keys (created_at);
