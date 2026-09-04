-- 008_seed_ngac_module_permissions.sql
-- Module 级别 NGAC 权限种子
--
-- 为 admin/operator 角色创建"所有模块"对象属性(OA)并建立关联，
-- 确保 admin/operator 用户有显式的模块级读写权限（而非依赖 PDP 兜底）。

-- === 1. 创建"所有模块"对象属性 ===
DO $$
DECLARE
    v_pc_id BIGINT;
    v_admin_ua_id BIGINT;
    v_operator_ua_id BIGINT;
    v_om_oa_id BIGINT;
    v_read_ar_id BIGINT;
    v_write_ar_id BIGINT;
    v_admin_ar_id BIGINT;
BEGIN
    SELECT id INTO v_pc_id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1;
    SELECT id INTO v_admin_ua_id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' AND deleted_at IS NULL LIMIT 1;
    SELECT id INTO v_operator_ua_id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'operator' AND deleted_at IS NULL LIMIT 1;
    SELECT id INTO v_read_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'read' LIMIT 1;
    SELECT id INTO v_write_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'write' LIMIT 1;
    SELECT id INTO v_admin_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'admin' LIMIT 1;

    -- "all-modules" OA: 代表 Gateway 中所有已发现模块
    -- 判重按 UNIQUE(resource_type, fk_resource)（uq_ngac_oa_resource）维度：
    -- 任一 module:0 集合 OA（如 005/019 seed 的 module-collection）存在即跳过，
    -- 避免按 resource_identifier 判重漏判导致唯一约束冲突（修复 init_db 008 中止）。
    INSERT INTO isahl_auth.ngac_object_attribute
        (o_name, fk_policy_class, resource_type, fk_resource, resource_identifier, property, created_at)
    SELECT 'all-modules', v_pc_id, 'module', 0, 'all-modules',
           '{"description":"所有模块"}'::jsonb, NOW()
    WHERE NOT EXISTS (
        SELECT 1 FROM isahl_auth.ngac_object_attribute
        WHERE resource_type = 'module' AND fk_resource = 0 AND deleted_at IS NULL
    )
    RETURNING id INTO v_om_oa_id;

    -- 若已存在则查询（复用既有 module:0 集合 OA）
    IF v_om_oa_id IS NULL THEN
        SELECT id INTO v_om_oa_id FROM isahl_auth.ngac_object_attribute
        WHERE resource_type = 'module' AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1;
    END IF;

    -- === 2. 建立 admin → all-modules 关联（admin 权限） ===
    IF v_admin_ua_id IS NOT NULL AND v_om_oa_id IS NOT NULL THEN
        INSERT INTO isahl_auth.ngac_association
            (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at)
        SELECT v_admin_ua_id, v_om_oa_id,
               ARRAY[v_read_ar_id, v_write_ar_id, v_admin_ar_id], v_pc_id, NOW()
        WHERE NOT EXISTS (
            SELECT 1 FROM isahl_auth.ngac_association
            WHERE fk_user_attribute = v_admin_ua_id AND fk_object_attribute = v_om_oa_id AND deleted_at IS NULL
        );
    END IF;

    -- === 3. 建立 operator → all-modules 关联（读写权限） ===
    IF v_operator_ua_id IS NOT NULL AND v_om_oa_id IS NOT NULL THEN
        INSERT INTO isahl_auth.ngac_association
            (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at)
        SELECT v_operator_ua_id, v_om_oa_id,
               ARRAY[v_read_ar_id, v_write_ar_id], v_pc_id, NOW()
        WHERE NOT EXISTS (
            SELECT 1 FROM isahl_auth.ngac_association
            WHERE fk_user_attribute = v_operator_ua_id AND fk_object_attribute = v_om_oa_id AND deleted_at IS NULL
        );
    END IF;

    -- === 4. 用户自己的数据：创建 baseline user OA ===
    -- 'user' 属性没有默认 module 关联，因为它需要管理员手动或审批后自动分配（见 service.rs post-approval hook）
    -- 用户通过审批激活后会自动获得 'user' UA 绑定，但无默认 module OA 关联，
    -- 意味着默认用户没有模块级访问权限（需要管理员通过 bind_user_attribute + association 配置）

    -- === 5. 触发 ngac_policy_version +1 ===
    IF v_om_oa_id IS NOT NULL THEN
        UPDATE isahl_auth.ngac_policy_version
        SET version = version + 1, updated_at = NOW()
        WHERE id = (SELECT id FROM isahl_auth.ngac_policy_version ORDER BY id LIMIT 1);
    END IF;
END $$;
