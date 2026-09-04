-- 035_drop_system_configs.sql
-- 下线 isahl_auth.system_configs 表（配置迁移至活库 isahl.zc_id_prot-*_config 族）。
--
-- 背景：文件存储配置 → isahl.zc_id_prot-oss_config；LLM → zc_id_prot-llm_config；
--       业务参数（platform_waybill_only、月度对账确认）→ zc_id_prot-env_config。
--       数据迁移见 scripts/db/migrate-system-configs-to-live-db.sql（必须先执行）。
--       通用 CRUD（/api/system-config）Repository 已改读活库族表。
--
-- 幂等：表不存在时跳过。

DROP TABLE IF EXISTS isahl_auth.system_configs;
