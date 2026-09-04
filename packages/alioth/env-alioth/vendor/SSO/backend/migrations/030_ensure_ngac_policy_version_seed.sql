-- ============================================================================
-- 030: ngac_policy_version 幂等初始化（NGAC_SPEC §8 策略版本缓存契约）
--
-- 背景：PDP 决策路径以 ngac_policy_version 作为 PolicyGraph 缓存失效信号；
-- 历史 seed 迁移（005/008/019/020/021/024）只在表已有行时执行
-- `UPDATE ... version=version+1`，表为空时是空操作 —— dev/test 库该表 0 行，
-- 版本信号从未产生。本迁移：
--   1) 表缺失时兜底创建（外部 007_ngac_extension_tables.sql 未覆盖的环境，
--      DDL 与既有备份 schema 完全一致：id/version/updated_at + PK + 序列默认值）；
--   2) 空表时插入首行 version=1，已有行则不动。
-- 全部语句幂等（CREATE IF NOT EXISTS / ON CONFLICT / WHERE NOT EXISTS），
-- 可经 init_db.rs（sso_migrations 追踪）或 psql -f 安全重跑。
-- ============================================================================

CREATE TABLE IF NOT EXISTS isahl_auth.ngac_policy_version (
    id bigint NOT NULL,
    version bigint NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE SEQUENCE IF NOT EXISTS isahl_auth.ngac_policy_version_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE;

ALTER SEQUENCE isahl_auth.ngac_policy_version_id_seq OWNED BY isahl_auth.ngac_policy_version.id;

ALTER TABLE ONLY isahl_auth.ngac_policy_version
    ALTER COLUMN id SET DEFAULT nextval('isahl_auth.ngac_policy_version_id_seq'::regclass);

-- PK 约束幂等补建（既有环境由 007_ngac_extension_tables.sql 已建，跳过）
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'ngac_policy_version_pkey'
          AND connamespace = 'isahl_auth'::regnamespace
    ) THEN
        ALTER TABLE ONLY isahl_auth.ngac_policy_version
            ADD CONSTRAINT ngac_policy_version_pkey PRIMARY KEY (id);
    END IF;
END $$;

INSERT INTO isahl_auth.ngac_policy_version (id, version)
SELECT 1, 1
WHERE NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_policy_version);

-- ============================================================================
-- 版本 bump 触发器（review 修复：SSO 此前无任何路径 bump ngac_policy_version，
-- 策略变更后 PDP 缓存永不失效）。
--
-- 定义与外部 007_ngac_extension_tables.sql 中同名对象一致（适配 isahl_auth
-- schema）：association / prohibition / access_right 任一 INSERT/UPDATE/DELETE
-- 语句执行后自动 version+1。全部语句幂等（CREATE OR REPLACE / DROP IF EXISTS），
-- 可经 init_db.rs（sso_migrations 追踪）或 psql -f 安全重跑。
-- ============================================================================

CREATE OR REPLACE FUNCTION isahl_auth.ngac_bump_policy_version() RETURNS TRIGGER AS $$
BEGIN
    UPDATE isahl_auth.ngac_policy_version
    SET version = version + 1, updated_at = NOW()
    WHERE id = (SELECT id FROM isahl_auth.ngac_policy_version ORDER BY id LIMIT 1);
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_ngac_association_version ON isahl_auth.ngac_association;
CREATE TRIGGER trg_ngac_association_version
    AFTER INSERT OR UPDATE OR DELETE ON isahl_auth.ngac_association
    FOR EACH STATEMENT
    EXECUTE FUNCTION isahl_auth.ngac_bump_policy_version();

DROP TRIGGER IF EXISTS trg_ngac_prohibition_version ON isahl_auth.ngac_prohibition;
CREATE TRIGGER trg_ngac_prohibition_version
    AFTER INSERT OR UPDATE OR DELETE ON isahl_auth.ngac_prohibition
    FOR EACH STATEMENT
    EXECUTE FUNCTION isahl_auth.ngac_bump_policy_version();

DROP TRIGGER IF EXISTS trg_ngac_access_right_version ON isahl_auth.ngac_access_right;
CREATE TRIGGER trg_ngac_access_right_version
    AFTER INSERT OR UPDATE OR DELETE ON isahl_auth.ngac_access_right
    FOR EACH STATEMENT
    EXECUTE FUNCTION isahl_auth.ngac_bump_policy_version();
