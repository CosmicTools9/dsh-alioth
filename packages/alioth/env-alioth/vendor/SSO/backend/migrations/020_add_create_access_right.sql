-- 020: 补 create AccessRight 并加入既有关联
--
-- 背景：HTTP POST 映射的 action 为 "create"（NgacPep 与 Gateway NgacEnforcer
-- 一致），但 005 仅 seed read/write/delete/approve/admin，关联中无 create
-- → 所有 POST 创建操作决策 NotApplicable → 403（此前被 bootstrap 兜底掩盖）。
-- 本迁移：seed create AR，并给 sso_admin/sso_audit/all-modules 关联补 create。

INSERT INTO isahl_auth.ngac_access_right (o_name)
VALUES ('create')
ON CONFLICT (o_name) DO NOTHING;

DO $$
DECLARE
    v_create_ar_id BIGINT;
    v_pc_id BIGINT;
    v_admin_ua_id BIGINT;
    v_operator_ua_id BIGINT;
    v_oa_id BIGINT;
BEGIN
    SELECT id INTO v_create_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'create' LIMIT 1;
    SELECT id INTO v_pc_id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1;
    SELECT id INTO v_admin_ua_id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' AND deleted_at IS NULL LIMIT 1;
    SELECT id INTO v_operator_ua_id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'operator' AND deleted_at IS NULL LIMIT 1;

    -- sso_admin / sso_audit 关联补 create（admin 全权）
    FOR v_oa_id IN
        SELECT id FROM isahl_auth.ngac_object_attribute
        WHERE resource_type IN ('sso_admin', 'sso_audit') AND fk_resource = 0 AND deleted_at IS NULL
    LOOP
        UPDATE isahl_auth.ngac_association
        SET ak_access_rights = array_append(ak_access_rights, v_create_ar_id)
        WHERE fk_object_attribute = v_oa_id
          AND deleted_at IS NULL
          AND NOT (v_create_ar_id = ANY(ak_access_rights));
    END LOOP;

    -- all-modules 关联补 create（admin/operator 读写含创建，符合 008 语义）
    UPDATE isahl_auth.ngac_association
    SET ak_access_rights = array_append(ak_access_rights, v_create_ar_id)
    WHERE fk_object_attribute = (
            SELECT id FROM isahl_auth.ngac_object_attribute
            WHERE resource_type = 'module' AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1
        )
      AND deleted_at IS NULL
      AND NOT (v_create_ar_id = ANY(ak_access_rights));

    -- 触发 ngac_policy_version +1
    UPDATE isahl_auth.ngac_policy_version
    SET version = version + 1, updated_at = NOW()
    WHERE id = (SELECT id FROM isahl_auth.ngac_policy_version ORDER BY id LIMIT 1);
END $$;
