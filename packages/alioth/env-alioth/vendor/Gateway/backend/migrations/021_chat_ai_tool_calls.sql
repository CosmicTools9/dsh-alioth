-- 021_chat_ai_tool_calls.sql
-- Chat-AI 工具结果留存 (upgrade-chat-ai-tool-surface E7; isahl_auth 工程 schema).
--
-- 位置决策: isahl 冻结 (ENVIRONMENT_SPEC 11.3.1); 工具调用记录是消息的衍生数据,
-- 落 isahl_auth.chat_message_meta (019 先例) 的 add column, MUST NOT 触及 isahl.
--
-- 语义: 每轮带工具调用的 assistant 消息记录本次调用列表, 每项含
-- name / arguments / success / output; output 按既有 4000 字符口径截断
-- (chat-ai-tool-output-truncation). 用途 = 追溯与审计, 原始输出 MUST NOT
-- 回灌对话历史 (跨轮需细节时按需重新调用工具). 无工具调用的轮次该列为 NULL.
--
-- 幂等: ADD COLUMN IF NOT EXISTS, 可重放.
-- 本文件经 Gateway/backend/migrations/ 由 namespace_schema runner 与
-- scripts/db/namespace-db.sh 白名单应用; dev 库执行归 agent (isahl_auth 工程 schema 豁免).

ALTER TABLE isahl_auth.chat_message_meta
    ADD COLUMN IF NOT EXISTS tool_calls jsonb;
