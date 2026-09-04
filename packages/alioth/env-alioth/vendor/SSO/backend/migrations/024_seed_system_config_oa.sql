-- 024: system-config 资源级 PEP 种子
--
-- 背景：Gateway PEP（pep/middleware.rs + ResourceRegistry）将 /api/system-config/*
-- 统一解析为 system_config:0（ResourceRegistry 注册 string_id 类型，与 SSO
-- /api/admin/* → sso_admin:0 恒定资源模式一致）。此前 isahl_auth 仅有
-- all-modules（module:0）/ sso-admin / sso-audit OA，无 system_config OA →
-- 非 NGAC_FAIL_OPEN 的 namespace（如 Alioth）下基础设施配置 API 全 403。
-- 本迁移 seed system_config OA 并绑定 admin UA（全权）：
--   system_config → admin UA（read/write/create/update/delete/admin）
-- operator 不关联（基础设施配置为 admin-only，与 sso_admin/sso_audit 同策略）。

DO $$
DECLARE
    v_pc_id BIGINT;
    v_admin_ua_id BIGINT;
    v_read_ar_id BIGINT;
    v_write_ar_id BIGINT;
    v_create_ar_id BIGINT;
    v_update_ar_id BIGINT;
    v_delete_ar_id BIGINT;
    v_admin_ar_id BIGINT;
    v_sc_oa_id BIGINT;
BEGIN
    SELECT id INTO v_pc_id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1;
    SELECT id INTO v_admin_ua_id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' AND deleted_at IS NULL LIMIT 1;
    SELECT id INTO v_read_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'read' LIMIT 1;
    SELECT id INTO v_write_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'write' LIMIT 1;
    SELECT id INTO v_create_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'create' LIMIT 1;
    SELECT id INTO v_update_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'update' LIMIT 1;
    SELECT id INTO v_delete_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'delete' LIMIT 1;
    SELECT id INTO v_admin_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'admin' LIMIT 1;

    -- system_config OA（/api/system-config/* 基础设施配置资源）
    INSERT INTO isahl_auth.ngac_object_attribute
        (o_name, fk_policy_class, resource_type, fk_resource, resource_identifier, property, created_at)
    SELECT 'system-config', v_pc_id, 'system_config', 0, 'system-config',
           '{"description":"基础设施配置（/api/system-config/*）"}'::jsonb, NOW()
    WHERE NOT EXISTS (
        SELECT 1 FROM isahl_auth.ngac_object_attribute
        WHERE resource_type = 'system_config' AND fk_resource = 0 AND deleted_at IS NULL
    )
    RETURNING id INTO v_sc_oa_id;

    -- 若已存在则查询
    IF v_sc_oa_id IS NULL THEN
        SELECT id INTO v_sc_oa_id FROM isahl_auth.ngac_object_attribute
        WHERE resource_type = 'system_config' AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1;
    END IF;

    -- admin UA → system_config OA（全权：read/write/create/update/delete/admin）
    IF v_admin_ua_id IS NOT NULL AND v_sc_oa_id IS NOT NULL THEN
        INSERT INTO isahl_auth.ngac_association
            (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at)
        SELECT v_admin_ua_id, v_sc_oa_id,
               ARRAY[v_read_ar_id, v_write_ar_id, v_create_ar_id, v_update_ar_id, v_delete_ar_id, v_admin_ar_id],
               v_pc_id, NOW()
        WHERE NOT EXISTS (
            SELECT 1 FROM isahl_auth.ngac_association
            WHERE fk_user_attribute = v_admin_ua_id AND fk_object_attribute = v_sc_oa_id AND deleted_at IS NULL
        );
    END IF;

    -- 触发 ngac_policy_version +1
    UPDATE isahl_auth.ngac_policy_version
    SET version = version + 1, updated_at = NOW()
    WHERE id = (SELECT id FROM isahl_auth.ngac_policy_version ORDER BY id LIMIT 1);
END $$;
