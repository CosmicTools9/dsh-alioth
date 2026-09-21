-- 020_chat_ai_subject_memory.sql
-- Chat-AI 主体化双层记忆 (refactor-chat-ai-subject-identity-memory, isahl_auth 工程 schema).
--
-- 位置决策: isahl 冻结 (ENVIRONMENT_SPEC 11.3.1); 记忆为消息/会话的衍生数据,
-- 落 isahl_auth (Gateway 完全访问域, 015/019 先例). MUST NOT 建在 isahl/isahl_meta.
--
-- 双层语义 (用户裁决 2026-09-13):
--   L1 主体层 chat_ai_subject_memory: 键 = 智能体主体 id (isahl.zc_id_empl-agent.id),
--      跨对话方共享累积.
--   L2 对话方层 chat_ai_counterpart_memory: 键 = 主体 id x 联系人 id (isahl.zc_id_contacts.id),
--      对话方私有.
-- 覆盖语义: 全量替换 (新的覆盖旧的), 无合并层, 无审阅层.
--
-- 不建 FK: subject_id / counterpart_id 为跨 schema (isahl) 逻辑引用, 行由 namespace 级
-- 种子创建; FK 会在种子顺序与历史数据上引入无谓耦合 (记忆只是索引, 不是强一致关系).
--
-- 迁移策略: isahl_auth.gateway_user_memory (015) 保留不删以留回滚余地, 但停止读写.
-- 既有记忆数据不迁移 (键空间无法映射到主体维度, 用户裁决 "新的覆盖旧的").
-- 本文件经 Gateway/backend/migrations/ 由 namespace_schema runner 应用.
-- dev 库建表执行归 agent (isahl_auth 工程 schema 豁免, ensure 幂等模式).

CREATE TABLE IF NOT EXISTS isahl_auth.chat_ai_subject_memory (
  subject_id bigint PRIMARY KEY,
  memory     jsonb NOT NULL DEFAULT '{}'::jsonb,
  version    bigint NOT NULL DEFAULT 1,
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS isahl_auth.chat_ai_counterpart_memory (
  subject_id     bigint NOT NULL,
  counterpart_id bigint NOT NULL,
  memory         jsonb NOT NULL DEFAULT '{}'::jsonb,
  version        bigint NOT NULL DEFAULT 1,
  updated_at     timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (subject_id, counterpart_id)
);
