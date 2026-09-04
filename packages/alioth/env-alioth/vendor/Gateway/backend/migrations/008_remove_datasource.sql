-- ⚠️ 废弃声明：此迁移直接操作 isahl_meta schema 的表，违反 Gateway 后端不应访问 isahl_meta 的规约。
-- 相关变更应通过 Meta 后端 DDL 管理。保留此文件仅作历史参考，不应再执行。
--
-- Migration: 008_remove_datasource
-- Purpose: Remove data source management module
-- ARC-01: 移除数据源管理，但保留 Discovery 功能

BEGIN;

-- Step 1: 移除 collections 表的外键约束
ALTER TABLE collections
DROP CONSTRAINT IF EXISTS fk_collections_data_source;

-- Step 2: 移除 data_assets 表的外键约束
ALTER TABLE data_assets
DROP CONSTRAINT IF EXISTS fk_data_assets_data_source;

-- Step 3: 移除 discovery_jobs 表的外键约束
ALTER TABLE discovery_jobs
DROP CONSTRAINT IF EXISTS fk_discovery_jobs_data_source;

-- Step 4: 修改 data_source_id 列为可选（允许 NULL）
ALTER TABLE collections
ALTER COLUMN data_source_id DROP NOT NULL;

ALTER TABLE data_assets
ALTER COLUMN data_source_id DROP NOT NULL;

ALTER TABLE discovery_jobs
ALTER COLUMN data_source_id DROP NOT NULL;

-- Step 5: 为现有记录设置系统级数据源 ID（可选）
-- 如果需要在 Discovery 中保留功能，可以将这些设为特定值
-- UPDATE collections SET data_source_id = NULL WHERE data_source_id IS NOT NULL;
-- UPDATE data_assets SET data_source_id = NULL WHERE data_source_id IS NOT NULL;
-- UPDATE discovery_jobs SET data_source_id = NULL WHERE data_source_id IS NOT NULL;

-- Step 6: 删除 data_sources 表
DROP TABLE IF EXISTS data_sources CASCADE;

COMMIT;
