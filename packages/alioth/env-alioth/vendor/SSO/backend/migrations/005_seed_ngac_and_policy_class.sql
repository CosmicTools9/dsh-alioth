-- 005_seed_ngac_and_policy_class.sql
-- NGAC 种子数据：policy_class、基础 UA、AccessRight、初始 admin 用户绑定
-- 关联: docs/superpowers/specs/2026-07-02-gateway-user-system-upgrade-design.md §Phase 1a
--
-- 注意: ngac_policy_version 由 007_ngac_extension_tables.sql 创建，本迁移不重复

-- === 1. policy_class "default" ===
-- 使用固定 ID 确保后续 seed 可引用 (避免依赖 isahl.gen_next_zuid() 的运行时行为)
DO $$
DECLARE
    v_pc_id BIGINT;
BEGIN
    -- 仅当 policy_class "default" 不存在时创建
    IF NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_policy_class WHERE o_name = 'default') THEN
        INSERT INTO isahl_auth.ngac_policy_class (o_name, description, is_active, created_at)
        VALUES ('default', '默认策略类，所有现有用户归属', TRUE, NOW())
        RETURNING id INTO v_pc_id;
    ELSE
        SELECT id INTO v_pc_id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1;
    END IF;
END $$;

-- === 2. 基础 AccessRight ===
INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES
    ('read'),
    ('write'),
    ('delete'),
    ('approve'),
    ('admin')
ON CONFLICT (o_name) DO NOTHING;

-- === 3. 基础 User Attribute (UA) — 使用 fk_policy_class NOT NULL ===
DO $$
DECLARE
    v_pc_id BIGINT;
    v_admin_id BIGINT;
    v_operator_id BIGINT;
BEGIN
    SELECT id INTO v_pc_id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1;

    -- admin (继承 operator)
    INSERT INTO isahl_auth.ngac_user_attribute
        (o_name, fk_policy_class, ancestor_ids, children_ids, property, created_at)
    SELECT 'admin', v_pc_id, '{}', '{}'::BIGINT[], '{"description":"平台管理员"}'::jsonb, NOW()
    WHERE NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin');
    SELECT id INTO v_admin_id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' LIMIT 1;

    -- operator
    INSERT INTO isahl_auth.ngac_user_attribute
        (o_name, fk_policy_class, ancestor_ids, children_ids, property, created_at)
    SELECT 'operator', v_pc_id, '{}', '{}'::BIGINT[], '{"description":"业务操作员"}'::jsonb, NOW()
    WHERE NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_user_attribute WHERE o_name = 'operator');
    SELECT id INTO v_operator_id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'operator' LIMIT 1;

    -- auditor
    INSERT INTO isahl_auth.ngac_user_attribute
        (o_name, fk_policy_class, ancestor_ids, children_ids, property, created_at)
    SELECT 'auditor', v_pc_id, '{}', '{}'::BIGINT[], '{"description":"审计员 (只读)"}'::jsonb, NOW()
    WHERE NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_user_attribute WHERE o_name = 'auditor');

    -- user
    INSERT INTO isahl_auth.ngac_user_attribute
        (o_name, fk_policy_class, ancestor_ids, children_ids, property, created_at)
    SELECT 'user', v_pc_id, '{}', '{}'::BIGINT[], '{"description":"普通用户 (注册默认)"}'::jsonb, NOW()
    WHERE NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_user_attribute WHERE o_name = 'user');

    -- 维护 admin 继承自 operator (bigint[])
    UPDATE isahl_auth.ngac_user_attribute
    SET ancestor_ids = ARRAY[v_operator_id]
    WHERE o_name = 'admin'
      AND (ancestor_ids = '{}'::BIGINT[] OR array_length(ancestor_ids, 1) IS NULL);
END $$;

-- === 4. 触发 ngac_policy_version +1（标记种子完成）===
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema='isahl_auth' AND table_name='ngac_policy_version') THEN
        UPDATE isahl_auth.ngac_policy_version
        SET version = version + 1, updated_at = NOW()
        WHERE id = (SELECT id FROM isahl_auth.ngac_policy_version ORDER BY id LIMIT 1);
    END IF;
END $$;
