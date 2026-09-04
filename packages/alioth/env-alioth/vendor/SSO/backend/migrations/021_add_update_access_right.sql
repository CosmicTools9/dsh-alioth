-- 021: 补 update AccessRight 并加入既有关联
--
-- 背景：HTTP PUT/PATCH 映射的 action 为 "update"（NgacPep 与 Gateway
-- NgacEnforcer 一致），但 AR 模型无 update → 更新操作决策 NotApplicable → 403。
-- 与 020 同模式：seed update AR，并给 sso_admin/sso_audit/all-modules 关联补 update。

INSERT INTO isahl_auth.ngac_access_right (o_name)
VALUES ('update')
ON CONFLICT (o_name) DO NOTHING;

DO $$
DECLARE
    v_update_ar_id BIGINT;
BEGIN
    SELECT id INTO v_update_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'update' LIMIT 1;

    -- sso_admin / sso_audit 关联补 update（admin 全权）
    UPDATE isahl_auth.ngac_association
    SET ak_access_rights = array_append(ak_access_rights, v_update_ar_id)
    WHERE fk_object_attribute IN (
            SELECT id FROM isahl_auth.ngac_object_attribute
            WHERE resource_type IN ('sso_admin', 'sso_audit') AND fk_resource = 0 AND deleted_at IS NULL
        )
      AND deleted_at IS NULL
      AND NOT (v_update_ar_id = ANY(ak_access_rights));

    -- all-modules 关联补 update（admin/operator 读写含更新）
    UPDATE isahl_auth.ngac_association
    SET ak_access_rights = array_append(ak_access_rights, v_update_ar_id)
    WHERE fk_object_attribute = (
            SELECT id FROM isahl_auth.ngac_object_attribute
            WHERE resource_type = 'module' AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1
        )
      AND deleted_at IS NULL
      AND NOT (v_update_ar_id = ANY(ak_access_rights));

    -- 触发 ngac_policy_version +1
    UPDATE isahl_auth.ngac_policy_version
    SET version = version + 1, updated_at = NOW()
    WHERE id = (SELECT id FROM isahl_auth.ngac_policy_version ORDER BY id LIMIT 1);
END $$;
