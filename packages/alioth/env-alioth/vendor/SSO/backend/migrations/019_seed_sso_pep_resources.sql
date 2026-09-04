-- 019: SSO 管理面资源级 PEP 种子
--
-- 背景：NgacPep 中间件（auth/middleware.rs）对 /api/admin/* 与 /api/audit/*
-- 做资源级 NGAC 决策，资源映射为 (sso_admin, 0) / (sso_audit, 0)。此前
-- 仅有 all-modules OA（008），无对应 OA/关联 → Enforce 模式下管理面全 403。
-- 本迁移 seed 两个资源 OA 并绑定 admin UA（全权）：
--   sso_admin → admin UA（read/write/delete/admin）
--   sso_audit → admin UA（read/write/delete/admin）
-- operator 不关联（SSO 管理面/审计读取为 admin-only，与 require_admin 角色级一致）。

DO $$
DECLARE
    v_pc_id BIGINT;
    v_admin_ua_id BIGINT;
    v_read_ar_id BIGINT;
    v_write_ar_id BIGINT;
    v_delete_ar_id BIGINT;
    v_admin_ar_id BIGINT;
    v_sso_admin_oa_id BIGINT;
    v_sso_audit_oa_id BIGINT;
BEGIN
    SELECT id INTO v_pc_id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1;
    SELECT id INTO v_admin_ua_id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' AND deleted_at IS NULL LIMIT 1;
    SELECT id INTO v_read_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'read' LIMIT 1;
    SELECT id INTO v_write_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'write' LIMIT 1;
    SELECT id INTO v_delete_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'delete' LIMIT 1;
    SELECT id INTO v_admin_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'admin' LIMIT 1;

    -- sso_admin OA（/api/admin/* 管理面资源）
    INSERT INTO isahl_auth.ngac_object_attribute
        (o_name, fk_policy_class, resource_type, fk_resource, resource_identifier, property, created_at)
    SELECT 'sso-admin', v_pc_id, 'sso_admin', 0, 'sso-admin',
           '{"description":"SSO 管理面（/api/admin/*）"}'::jsonb, NOW()
    WHERE NOT EXISTS (
        SELECT 1 FROM isahl_auth.ngac_object_attribute
        WHERE resource_type = 'sso_admin' AND fk_resource = 0 AND deleted_at IS NULL
    )
    RETURNING id INTO v_sso_admin_oa_id;

    IF v_sso_admin_oa_id IS NULL THEN
        SELECT id INTO v_sso_admin_oa_id FROM isahl_auth.ngac_object_attribute
        WHERE resource_type = 'sso_admin' AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1;
    END IF;

    -- sso_audit OA（/api/audit/* 审计面资源）
    INSERT INTO isahl_auth.ngac_object_attribute
        (o_name, fk_policy_class, resource_type, fk_resource, resource_identifier, property, created_at)
    SELECT 'sso-audit', v_pc_id, 'sso_audit', 0, 'sso-audit',
           '{"description":"SSO 审计面（/api/audit/*）"}'::jsonb, NOW()
    WHERE NOT EXISTS (
        SELECT 1 FROM isahl_auth.ngac_object_attribute
        WHERE resource_type = 'sso_audit' AND fk_resource = 0 AND deleted_at IS NULL
    )
    RETURNING id INTO v_sso_audit_oa_id;

    IF v_sso_audit_oa_id IS NULL THEN
        SELECT id INTO v_sso_audit_oa_id FROM isahl_auth.ngac_object_attribute
        WHERE resource_type = 'sso_audit' AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1;
    END IF;

    -- admin UA → sso_admin OA（全权）
    IF v_admin_ua_id IS NOT NULL AND v_sso_admin_oa_id IS NOT NULL THEN
        INSERT INTO isahl_auth.ngac_association
            (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at)
        SELECT v_admin_ua_id, v_sso_admin_oa_id,
               ARRAY[v_read_ar_id, v_write_ar_id, v_delete_ar_id, v_admin_ar_id], v_pc_id, NOW()
        WHERE NOT EXISTS (
            SELECT 1 FROM isahl_auth.ngac_association
            WHERE fk_user_attribute = v_admin_ua_id AND fk_object_attribute = v_sso_admin_oa_id AND deleted_at IS NULL
        );
    END IF;

    -- admin UA → sso_audit OA（全权）
    IF v_admin_ua_id IS NOT NULL AND v_sso_audit_oa_id IS NOT NULL THEN
        INSERT INTO isahl_auth.ngac_association
            (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at)
        SELECT v_admin_ua_id, v_sso_audit_oa_id,
               ARRAY[v_read_ar_id, v_write_ar_id, v_delete_ar_id, v_admin_ar_id], v_pc_id, NOW()
        WHERE NOT EXISTS (
            SELECT 1 FROM isahl_auth.ngac_association
            WHERE fk_user_attribute = v_admin_ua_id AND fk_object_attribute = v_sso_audit_oa_id AND deleted_at IS NULL
        );
    END IF;

    -- 触发 ngac_policy_version +1
    UPDATE isahl_auth.ngac_policy_version
    SET version = version + 1, updated_at = NOW()
    WHERE id = (SELECT id FROM isahl_auth.ngac_policy_version ORDER BY id LIMIT 1);
END $$;
