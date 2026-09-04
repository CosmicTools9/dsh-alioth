-- 032: NGAC 策略变更审计日志分区兜底（add-ngac-audit-trail-view D1 前置）
--
-- 背景（spec-audit V-1 实测）：
--   isahl_auth.ngac_policy_audit_log 为 PARTITION BY RANGE (created_at) 的
--   声明式分区表，但子分区数为 0（无 DEFAULT 分区）——当前任何 INSERT 直接
--   报 "no partition of relation found for row"，策略变更审计面从未可写。
--   另存在孤儿表 ngac_policy_audit_log_2026_04（relkind='r'、未挂载、0 行、
--   命名与月度分区冲突）。
--
-- 本迁移（幂等可重放、跨环境安全）：
--   1. DROP 孤儿表 ngac_policy_audit_log_2026_04；
--   2. 创建当月 + 次月 RANGE 月度分区（动态名 ngac_policy_audit_log_YYYY_MM）；
--   3. 创建 ngac_policy_audit_log_default DEFAULT 分区兜底任意时间戳
--      （防复现 0 分区事故；月度滚动续期与保留策略另立 change）。
--
-- 名称冲突处置（跨环境安全，advisor 复审后加固）：
--   to_regclass 非空不等于"已是分区"——旧环境可能存在同名普通月表（如
--   _2026_04 同款孤儿）。本迁移对每个目标名先查 pg_inherits 父子关系：
--   - 已是本父表分区 → 跳过（幂等）；
--   - 同名但不是本父表子表 → RENAME 为 <name>_orphan_<时间戳>（保留数据、
--     禁止 DROP），并 RAISE WARNING 提示人工处置后，再创建分区。
--
-- 级联义务（spec-audit 实测）：reset-db.sh --test 仅应用 Backup/latest/schema.sql，
-- 本迁移落盘后必须刷新 Backup 快照（change tasks 0.2），否则 test 库缺分区。

DO $$
DECLARE
    v_parent CONSTANT REGCLASS := 'isahl_auth.ngac_policy_audit_log';
    v_cur_start DATE := date_trunc('month', NOW())::DATE;
    v_next_start DATE := (date_trunc('month', NOW()) + INTERVAL '1 month')::DATE;
    v_next_end DATE := (date_trunc('month', NOW()) + INTERVAL '2 months')::DATE;
    v_names TEXT[] := ARRAY[
        'ngac_policy_audit_log_' || to_char(v_cur_start, 'YYYY_MM'),
        'ngac_policy_audit_log_' || to_char(v_next_start, 'YYYY_MM'),
        'ngac_policy_audit_log_default'
    ];
    v_froms DATE[] := ARRAY[v_cur_start, v_next_start, NULL];
    v_tos DATE[] := ARRAY[v_next_start, v_next_end, NULL];
    v_name TEXT;
    v_orphan_name TEXT;
    i INT;
BEGIN
    -- 1. 孤儿表（未挂载分区、0 行）
    DROP TABLE IF EXISTS isahl_auth.ngac_policy_audit_log_2026_04;

    FOR i IN 1..3 LOOP
        v_name := v_names[i];

        IF to_regclass('isahl_auth.' || v_name) IS NOT NULL THEN
            -- 已是本父表的分区 → 幂等跳过
            IF EXISTS (
                SELECT 1 FROM pg_inherits
                WHERE inhrelid = ('isahl_auth.' || v_name)::regclass
                  AND inhparent = v_parent
            ) THEN
                CONTINUE;
            END IF;
            -- 同名但不是本父表子表 → 改名保留（禁 DROP），人工处置
            v_orphan_name := v_name || '_orphan_' || to_char(NOW(), 'YYYYMMDD_HH24MISS');
            EXECUTE format('ALTER TABLE isahl_auth.%I RENAME TO %I', v_name, v_orphan_name);
            RAISE WARNING '032: % exists but is not a partition of %; renamed to % — manual review required',
                v_name, v_parent, v_orphan_name;
        END IF;

        IF v_froms[i] IS NULL THEN
            EXECUTE format(
                'CREATE TABLE isahl_auth.%I PARTITION OF isahl_auth.ngac_policy_audit_log DEFAULT',
                v_name
            );
        ELSE
            EXECUTE format(
                'CREATE TABLE isahl_auth.%I PARTITION OF isahl_auth.ngac_policy_audit_log
                 FOR VALUES FROM (%L) TO (%L)',
                v_name, v_froms[i], v_tos[i]
            );
        END IF;
    END LOOP;
END $$;
