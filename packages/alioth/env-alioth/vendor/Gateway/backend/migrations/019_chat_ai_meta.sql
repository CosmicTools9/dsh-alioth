-- 019_chat_ai_meta.sql
-- 消息级元数据 + 消息反馈（fix-chat-ai-feature-gaps D2.5，isahl_auth 工程 schema）。
--
-- 位置决策：isahl 冻结（ENVIRONMENT_SPEC §11.3.1）——isahl."zc_id_msgs-chat_ai"
-- 仅存消息正文/发送方，消息级衍生数据（agent_code/structured/usage/附件/
-- 知识引用）与用户反馈不得改 isahl 结构，落 isahl_auth（Gateway 完全访问域，
-- 015 gateway_user_memory 先例）。
-- 本文件经 Gateway/backend/migrations/ 由 namespace_schema runner 应用；
-- dev 库建表执行归 agent（isahl_auth 工程 schema 豁免，ensure 幂等模式，
-- 见 openspec change fix-chat-ai-feature-gaps design.md D2.5）。

CREATE TABLE IF NOT EXISTS isahl_auth.chat_message_meta (
  msg_id         bigint PRIMARY KEY REFERENCES isahl."zc_id_msgs-chat_ai"(id) ON DELETE CASCADE,
  session_id     bigint NOT NULL,
  agent_code     text NOT NULL DEFAULT '',
  structured     jsonb,
  usage          jsonb,
  attachments    jsonb,          -- [{type:"image",mime,data_base64|url}]
  knowledge_refs jsonb,          -- [{key,title}]（前端注入命中的知识块）
  created_at     timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS isahl_auth.chat_message_feedback (
  msg_id     bigint NOT NULL REFERENCES isahl."zc_id_msgs-chat_ai"(id) ON DELETE CASCADE,
  user_id    bigint NOT NULL,
  rating     text NOT NULL CHECK (rating IN ('up','down')),
  comment    text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (msg_id, user_id)
);
