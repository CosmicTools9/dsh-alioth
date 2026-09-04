-- ⚠️ 废弃声明：此迁移直接操作 isahl_meta schema 的表，违反 Gateway 后端不应访问 isahl_meta 的规约。
-- 相关变更应通过 Meta 后端 DDL 管理。保留此文件仅作历史参考，不应再执行。
--
-- Migration: 009_schema_isahl
-- Purpose: Migrate default schema from 'public' to 'isahl'
-- ARC-02: 默认 schema 设置为 isahl

BEGIN;

-- Step 1: 创建 isahl schema（如果不存在）
CREATE SCHEMA IF NOT EXISTS isahl;

-- Step 2: 修改 collections 表 schema 字段默认值
ALTER TABLE collections
ALTER COLUMN schema
SET DEFAULT 'isahl';

-- Step 3: 注释说明现有数据处理策略
-- 现有数据保持原值，新数据默认使用 'isahl'
-- 如需迁移现有数据，请手动执行：
-- UPDATE collections SET schema = 'isahl' WHERE schema = 'public' AND is_system = true;

-- Step 4: 更新索引（如果存在基于 schema 的索引）
-- 重新创建索引以优化 isahl schema 的查询
DROP INDEX IF EXISTS idx_collections_schema;
CREATE INDEX idx_collections_schema_isahl
ON collections(schema, name)
WHERE schema = 'isahl';

COMMIT;
