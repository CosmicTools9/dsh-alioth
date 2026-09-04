-- 025_ngac_prohibition_ak_access_rights_not_null.sql
-- 目的：
--   1. 将 isahl_auth.ngac_association / ngac_prohibition 的 ak_access_rights
--      收紧为 NOT NULL DEFAULT '{}'，对齐 docs/specs/NGAC_SPEC.md §3.1 规范 DDL。
--   2. 修复数组元素级 NULL（SQLx 按 Vec<i64> 解码时报 unexpected null）。
--   3. CHECK 约束防回归（PG 列级 NOT NULL 仍允许 '{NULL,1}' 元素）。
-- 背景：该列在全部环境（四层隔离 + namespace 库）均可空/含 NULL 元素，
--       一旦出现，SSO PDP（gateway_sso::ngac::pdp）解码失败，decide_access
--       fail-closed 返回 Deny（error occurred while decoding column
--       "ak_access_rights": unexpected null）。
-- 适用范围：所有含 isahl_auth schema 的数据库。
-- 幂等：information_schema 守门 + WHERE 条件守门，可重复执行。

DO $$
DECLARE
    v_nullable TEXT;
BEGIN
    -- Step 1: 兜底填充历史 NULL 行（列级；空数组 = 无授权，NGAC 语义等价于该规则不生效）
    UPDATE isahl_auth.ngac_association
    SET ak_access_rights = '{}'
    WHERE ak_access_rights IS NULL;
    UPDATE isahl_auth.ngac_prohibition
    SET ak_access_rights = '{}'
    WHERE ak_access_rights IS NULL;

    -- Step 2: 修复数组元素级 NULL 的 association：置 '{}'（无授权，不猜权限）。
    --   通用迁移不做语义恢复——不同模块权限模式可能不同，任意参照复制会
    --   导致权限提升/非确定性。特定行的权限恢复属人工核准的运维操作
    --   （本次 alioth 库 2 行已人工核准恢复为同 UA module 级权限集，
    --   见 openspec/changes/fix-ngac-prohibition-null-access-rights/）。
    UPDATE isahl_auth.ngac_association
    SET ak_access_rights = '{}'
    WHERE deleted_at IS NULL
      AND array_position(ak_access_rights, NULL) IS NOT NULL;

    -- Step 3: 幂等收紧列约束（已是 NOT NULL 时跳过）
    SELECT is_nullable INTO v_nullable
    FROM information_schema.columns
    WHERE table_schema = 'isahl_auth'
      AND table_name = 'ngac_association'
      AND column_name = 'ak_access_rights';
    IF v_nullable = 'YES' THEN
        ALTER TABLE isahl_auth.ngac_association
            ALTER COLUMN ak_access_rights SET DEFAULT '{}';
        ALTER TABLE isahl_auth.ngac_association
            ALTER COLUMN ak_access_rights SET NOT NULL;
    END IF;

    SELECT is_nullable INTO v_nullable
    FROM information_schema.columns
    WHERE table_schema = 'isahl_auth'
      AND table_name = 'ngac_prohibition'
      AND column_name = 'ak_access_rights';
    IF v_nullable = 'YES' THEN
        ALTER TABLE isahl_auth.ngac_prohibition
            ALTER COLUMN ak_access_rights SET DEFAULT '{}';
        ALTER TABLE isahl_auth.ngac_prohibition
            ALTER COLUMN ak_access_rights SET NOT NULL;
    END IF;
END $$;

-- Step 4: CHECK 约束防数组元素级 NULL（防回归，幂等）
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.check_constraints
        WHERE constraint_schema = 'isahl_auth'
          AND constraint_name = 'ngac_association_ak_no_null_elements'
    ) THEN
        ALTER TABLE isahl_auth.ngac_association
            ADD CONSTRAINT ngac_association_ak_no_null_elements
            CHECK (array_position(ak_access_rights, NULL) IS NULL);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM information_schema.check_constraints
        WHERE constraint_schema = 'isahl_auth'
          AND constraint_name = 'ngac_prohibition_ak_no_null_elements'
    ) THEN
        ALTER TABLE isahl_auth.ngac_prohibition
            ADD CONSTRAINT ngac_prohibition_ak_no_null_elements
            CHECK (array_position(ak_access_rights, NULL) IS NULL);
    END IF;
END $$;
