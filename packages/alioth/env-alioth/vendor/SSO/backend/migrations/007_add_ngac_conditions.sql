-- 为 ngac_association / ngac_prohibition 补齐 conditions JSONB 列，
-- 与 PDP 运行时查询（pdp.rs 读取 conditions 字段）及 NGAC_SPEC 保持一致。
-- 使用 IF NOT EXISTS 保证幂等，可安全重复执行。

ALTER TABLE isahl_auth.ngac_association
    ADD COLUMN IF NOT EXISTS conditions JSONB;

ALTER TABLE isahl_auth.ngac_prohibition
    ADD COLUMN IF NOT EXISTS conditions JSONB;
