-- 018: 统一 isahl_audit 日志/审计表 id 默认值为 isahl.gen_next_zuid()。
-- 背景：audit_events/audit_log_archive/audit_logs/log_db_query/log_db_stats
-- 五表 id:bigint 主键曾无数据库侧默认值（dev 库实测），不带 id 的 INSERT 直接
-- 失败。017 仅为 audit_events 引入私有序列，未覆盖其余四表，且私有序列与
-- 平台 ID 体系（AGENTS.md：isahl_auth/isahl_audit 允许 gen_next_zuid()）不一致。
-- 本 migration 以 gen_next_zuid() 统一五表，取代 017 的序列方案：
-- gen_next_zuid() 输出 JS 安全（& 2^53-1）且全局唯一的 zuid，
-- 优于每表私有序列（仅表内唯一、跨表可碰撞）。

ALTER TABLE isahl_audit.audit_events
    ALTER COLUMN id SET DEFAULT isahl.gen_next_zuid();
ALTER TABLE isahl_audit.audit_log_archive
    ALTER COLUMN id SET DEFAULT isahl.gen_next_zuid();
ALTER TABLE isahl_audit.audit_logs
    ALTER COLUMN id SET DEFAULT isahl.gen_next_zuid();
ALTER TABLE isahl_audit.log_db_query
    ALTER COLUMN id SET DEFAULT isahl.gen_next_zuid();
ALTER TABLE isahl_audit.log_db_stats
    ALTER COLUMN id SET DEFAULT isahl.gen_next_zuid();

-- 序列处置：017 的 audit_events_id_seq 与历史遗留的 log_db_query_id_seq /
-- log_db_stats_id_seq 在 zuid 化后成为死对象。已在 dev 库核实：当前
-- audit_events_id_seq 不可见（不据此推断 017 的应用历史），其余两条序列
-- 经 pg_depend 双向依赖与 pg_proc 函数体扫描确认零引用，DROP 安全。
-- 先 SET DEFAULT 后 DROP SEQUENCE，避免旧 default 悬空引用。
-- 注：017 本身保留不改（历史 migration 不可变）；未应用过 017 的环境中
-- DROP SEQUENCE IF EXISTS 为空操作。
DROP SEQUENCE IF EXISTS isahl_audit.audit_events_id_seq;
DROP SEQUENCE IF EXISTS isahl_audit.log_db_query_id_seq;
DROP SEQUENCE IF EXISTS isahl_audit.log_db_stats_id_seq;
