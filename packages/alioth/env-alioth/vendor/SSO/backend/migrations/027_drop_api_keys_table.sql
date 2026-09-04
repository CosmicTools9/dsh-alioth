-- 027_drop_api_keys_table.sql
-- 目的：移除旧 API 密钥表 `isahl_auth.api_keys`。
--
-- 背景：OpenAPI 统一调用方注册表 `isahl_auth.api_clients`（migration 026）
-- 已完全替代 api_keys（认证查 api_clients、admin CRUD 重定向、0 行数据）。
-- 旧表保留仅造成双写面死代码，按「删优于留」原则移除。
--
-- 幂等：存在性守门，可重复执行。

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.tables
        WHERE table_schema = 'isahl_auth' AND table_name = 'api_keys'
    ) THEN
        DROP TABLE isahl_auth.api_keys;
    END IF;
END $$;
