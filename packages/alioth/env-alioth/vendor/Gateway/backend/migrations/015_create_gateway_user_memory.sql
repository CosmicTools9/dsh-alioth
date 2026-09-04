-- 015_create_gateway_user_memory.sql
-- 用户级 AI memory 空间（add-agent-pool-user-memory）。
-- gateway.chat-ai 内置 Agent 的跨 session 用户记忆：按 user_id 绑定，
-- 所有该用户 session 共享；prompt 注入个性化（UserMemoryStore 读写）。
--
-- 位置决策：isahl 冻结（ENVIRONMENT_SPEC §11.3.1 仅豁免 id DEFAULT）；
-- isahl_meta.agent_memory 存在但 SECURITY_SPEC §6 禁 Gateway 业务路径访问；
-- isahl_auth 为 Gateway 完全访问域（standalone_users/api_keys 先例）。
-- 本文件经 Gateway/backend/migrations/ 由 namespace_schema runner 应用。

CREATE TABLE IF NOT EXISTS isahl_auth.gateway_user_memory (
    user_id     BIGINT PRIMARY KEY,
    memory      JSONB NOT NULL DEFAULT '{}',
    version     BIGINT NOT NULL DEFAULT 1,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
