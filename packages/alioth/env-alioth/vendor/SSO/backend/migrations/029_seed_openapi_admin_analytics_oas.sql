-- 029: OpenAPI 管理面/统计面 OA 种子（refactor-openapi-admin-ngac-pdp）
--
-- 背景：OpenAPI 管理面（SSO /api/admin/api-{clients,plans,subscriptions,reconcile}）
-- 与统计面（Gateway /api/openapi/usage{,/outbound}、scope-catalog）授权此前为
-- handler 内硬编码 admin SQL + PEP 资源错配（sso_admin / 未注册 openapi_usage）。
-- 本迁移 seed 两个 OA 并绑定 admin UA（行为不变式：admin 迁移前后全权）：
--   openapi_admin     → admin UA（read/write/create/update/delete/admin）
--   openapi_analytics → admin UA（read；统计端点全 GET）
-- rights 词表按 ngac_access_right 现值全名（019/020/021/024 先例，
-- PDP 按 action↔right o_name 精确匹配，缺一不可）。
-- 幂等可重放；判重 UNIQUE(resource_type, fk_resource)（o_name 无唯一约束）。

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
    v_admin_oa_id BIGINT;
    v_analytics_oa_id BIGINT;
BEGIN
    SELECT id INTO v_pc_id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1;
    SELECT id INTO v_admin_ua_id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' AND deleted_at IS NULL LIMIT 1;
    SELECT id INTO v_read_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'read' LIMIT 1;
    SELECT id INTO v_write_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'write' LIMIT 1;
    SELECT id INTO v_create_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'create' LIMIT 1;
    SELECT id INTO v_update_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'update' LIMIT 1;
    SELECT id INTO v_delete_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'delete' LIMIT 1;
    SELECT id INTO v_admin_ar_id FROM isahl_auth.ngac_access_right WHERE o_name = 'admin' LIMIT 1;

    -- 硬依赖缺失即报错（不静默）：admin UA 与 rights 必须已 seed
    IF v_pc_id IS NULL OR v_admin_ua_id IS NULL OR v_read_ar_id IS NULL
       OR v_write_ar_id IS NULL OR v_create_ar_id IS NULL OR v_update_ar_id IS NULL
       OR v_delete_ar_id IS NULL OR v_admin_ar_id IS NULL THEN
        RAISE EXCEPTION '029 前置缺失：policy_class/admin UA/access rights 未就绪（先跑 019/020/021）';
    END IF;

    -- openapi_admin OA（管理面：SSO /api/admin/api-* + Gateway scope-catalog）
    INSERT INTO isahl_auth.ngac_object_attribute
        (o_name, fk_policy_class, resource_type, fk_resource, resource_identifier, property, created_at)
    SELECT 'openapi-admin', v_pc_id, 'openapi_admin', 0, 'openapi-admin',
           '{"description":"OpenAPI 管理面（/api/admin/api-* 与 scope-catalog）"}'::jsonb, NOW()
    WHERE NOT EXISTS (
        SELECT 1 FROM isahl_auth.ngac_object_attribute
        WHERE resource_type = 'openapi_admin' AND fk_resource = 0 AND deleted_at IS NULL
    )
    RETURNING id INTO v_admin_oa_id;
    IF v_admin_oa_id IS NULL THEN
        SELECT id INTO v_admin_oa_id FROM isahl_auth.ngac_object_attribute
        WHERE resource_type = 'openapi_admin' AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1;
    END IF;

    -- openapi_analytics OA（统计面：/api/openapi/usage{,/outbound}）
    INSERT INTO isahl_auth.ngac_object_attribute
        (o_name, fk_policy_class, resource_type, fk_resource, resource_identifier, property, created_at)
    SELECT 'openapi-analytics', v_pc_id, 'openapi_analytics', 0, 'openapi-analytics',
           '{"description":"OpenAPI 统计面（/api/openapi/usage 系列）"}'::jsonb, NOW()
    WHERE NOT EXISTS (
        SELECT 1 FROM isahl_auth.ngac_object_attribute
        WHERE resource_type = 'openapi_analytics' AND fk_resource = 0 AND deleted_at IS NULL
    )
    RETURNING id INTO v_analytics_oa_id;
    IF v_analytics_oa_id IS NULL THEN
        SELECT id INTO v_analytics_oa_id FROM isahl_auth.ngac_object_attribute
        WHERE resource_type = 'openapi_analytics' AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1;
    END IF;

    -- admin UA → openapi_admin（全权：read/write/create/update/delete/admin）
    INSERT INTO isahl_auth.ngac_association
        (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at)
    SELECT v_admin_ua_id, v_admin_oa_id,
           ARRAY[v_read_ar_id, v_write_ar_id, v_create_ar_id, v_update_ar_id, v_delete_ar_id, v_admin_ar_id],
           v_pc_id, NOW()
    WHERE NOT EXISTS (
        SELECT 1 FROM isahl_auth.ngac_association
        WHERE fk_user_attribute = v_admin_ua_id AND fk_object_attribute = v_admin_oa_id AND deleted_at IS NULL
    );

    -- admin UA → openapi_analytics（只读：read）
    INSERT INTO isahl_auth.ngac_association
        (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at)
    SELECT v_admin_ua_id, v_analytics_oa_id,
           ARRAY[v_read_ar_id],
           v_pc_id, NOW()
    WHERE NOT EXISTS (
        SELECT 1 FROM isahl_auth.ngac_association
        WHERE fk_user_attribute = v_admin_ua_id AND fk_object_attribute = v_analytics_oa_id AND deleted_at IS NULL
    );

    -- 策略版本 bump：空表插首行，否则 +1（PolicyGraph 缓存版本标记，NGAC_SPEC §8）
    IF EXISTS (SELECT 1 FROM isahl_auth.ngac_policy_version) THEN
        UPDATE isahl_auth.ngac_policy_version
        SET version = version + 1, updated_at = NOW()
        WHERE id = (SELECT id FROM isahl_auth.ngac_policy_version ORDER BY id LIMIT 1);
    ELSE
        INSERT INTO isahl_auth.ngac_policy_version (version, updated_at)
        VALUES (1, NOW());
    END IF;
END $$;
