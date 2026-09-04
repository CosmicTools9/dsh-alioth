-- 036_repair_api_idempotency_alignment.sql
--
-- 目的：修补 033 之前手工建表（2026-08-18 运维直建）造成的 dev/test/pre/prod
--       api_idempotency_keys 漂移，四层统一对齐 033 规范：
--         1. 列类型 text/integer → varchar(n)/smallint（033 定义）
--         2. api_version 补 DEFAULT 'v1'；id 删 default（代码 INSERT 自带 gen_next_zuid()）
--         3. 索引名 ux_api_idem_scope/ix_api_idem_created_at → uq_api_idempotency_scope/idx_api_idempotency_created
--         4. CHECK/FK 约束名对齐 PG 自动命名（state_check / fk_client_fkey）
-- 前置：026（api_clients 父表族）必须已在目标库执行——迁移序号天然保证。
-- 适用范围：所有含 isahl_auth.api_idempotency_keys 的数据库。
-- 幂等：全部 IF [NOT] EXISTS / 条件 DO block，可重复执行。
-- 数据安全：四层实测 0 行（2026-08-18），类型 ALTER 无损。

-- ── 1. 列类型对齐 033 ──
ALTER TABLE isahl_auth.api_idempotency_keys
    ALTER COLUMN api_version           TYPE VARCHAR(32),
    ALTER COLUMN api_version           SET DEFAULT 'v1',
    ALTER COLUMN idem_key              TYPE VARCHAR(255),
    ALTER COLUMN method                TYPE VARCHAR(8),
    ALTER COLUMN path                  TYPE VARCHAR(255),
    ALTER COLUMN request_fingerprint   TYPE VARCHAR(64),
    ALTER COLUMN state                 TYPE VARCHAR(16),
    ALTER COLUMN response_status       TYPE SMALLINT,
    ALTER COLUMN response_content_type TYPE VARCHAR(128);

-- id 默认值删除：033 无 default；Gateway openapi/idempotency.rs:307 INSERT 显式
-- VALUES (isahl.gen_next_zuid(), ...) 自带 id。
ALTER TABLE isahl_auth.api_idempotency_keys ALTER COLUMN id DROP DEFAULT;

-- ── 2. 索引名对齐 033 ──
DROP INDEX IF EXISTS isahl_auth.ux_api_idem_scope;
DROP INDEX IF EXISTS isahl_auth.ix_api_idem_created_at;
CREATE UNIQUE INDEX IF NOT EXISTS uq_api_idempotency_scope
    ON isahl_auth.api_idempotency_keys (fk_client, api_version, idem_key);
CREATE INDEX IF NOT EXISTS idx_api_idempotency_created
    ON isahl_auth.api_idempotency_keys (created_at);

-- ── 3. 约束名对齐（033 内联约束的 PG 自动命名）──
DO $$
BEGIN
  -- CHECK：033 内联 state CHECK → 自动名 api_idempotency_keys_state_check
  IF EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'ck_api_idem_state'
             AND conrelid = 'isahl_auth.api_idempotency_keys'::regclass) THEN
    ALTER TABLE isahl_auth.api_idempotency_keys DROP CONSTRAINT ck_api_idem_state;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'api_idempotency_keys_state_check'
                 AND conrelid = 'isahl_auth.api_idempotency_keys'::regclass) THEN
    ALTER TABLE isahl_auth.api_idempotency_keys
        ADD CONSTRAINT api_idempotency_keys_state_check
        CHECK (state IN ('in_progress','completed'));
  END IF;
  -- FK：手工运维建的 fk_api_idem_client → 033 内联 REFERENCES 自动名 fk_client_fkey
  IF EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'fk_api_idem_client'
             AND conrelid = 'isahl_auth.api_idempotency_keys'::regclass) THEN
    ALTER TABLE isahl_auth.api_idempotency_keys DROP CONSTRAINT fk_api_idem_client;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'api_idempotency_keys_fk_client_fkey'
                 AND conrelid = 'isahl_auth.api_idempotency_keys'::regclass) THEN
    ALTER TABLE isahl_auth.api_idempotency_keys
        ADD CONSTRAINT api_idempotency_keys_fk_client_fkey
        FOREIGN KEY (fk_client) REFERENCES isahl_auth.api_clients (id);
  END IF;
END $$;

GRANT SELECT ON TABLE isahl_auth.api_idempotency_keys TO alioth_readonly;
