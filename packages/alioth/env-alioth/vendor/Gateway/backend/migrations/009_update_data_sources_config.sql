-- Migration: 009_update_data_sources_config
-- 注意：本迁移已废弃。isahl_meta 表的修改应在 Meta 后端 DDL 中执行。
-- Gateway 禁止直接操作 Meta 应用的 schema（应用独立性规约）。
-- 原内容已迁移至 DB schema 对应版本。

-- 保留空迁移以确保 migration 编号连续性
SELECT 1;
