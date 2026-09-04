-- 031: OpenAPI 产品 CRUD 资源 OA 种子（seed-openapi-product-oa）
--
-- 背景：/api/service/openapi/{configs,sales,purchases,mades}（产品 CRUD，
-- 管理面）经 Gateway PEP map_resource 解析为 {entity}:0 资源，但这 4 个
-- 资源 OA 此前从未 seed（只有 openapi_admin/openapi_analytics）。后果双向
-- 失效：bootstrap 阶段（无 association）任何 JWT 用户可改产品配置；生产
-- 阶段（有 association）连 admin 都 403。违反 OPENAPI_SPEC §1.2
-- 「configs/sales/purchases/mades 资源 OA 仅授予管理员 UA」。
-- 本迁移 seed 4 个 OA 并绑定 admin UA（行为不变式：admin 迁移前后全权）：
--   openapi-configs     → admin UA（read/write/create/update/delete/admin）
--   openapi-sales       → admin UA（read/write/create/update/delete/admin）
--   openapi-purchases   → admin UA（read/write/create/update/delete/admin）
--   openapi-mades       → admin UA（read/write/create/update/delete/admin）
-- rights 词表按 ngac_access_right 现值全名（019/020/021/024/029 先例，
-- PDP 按 action↔right o_name 精确匹配，缺一不可）。
-- 幂等可重放；判重 UNIQUE(resource_type, fk_resource)（uq_ngac_oa_resource，
-- 029 同款：WHERE NOT EXISTS ... AND deleted_at IS NULL + RETURNING 两段式）。

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
    v_oa_id BIGINT;
    v_entity TEXT;
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
        RAISE EXCEPTION '031 前置缺失：policy_class/admin UA/access rights 未就绪（先跑 019/020/021）';
    END IF;

    -- 4 个产品 CRUD 实体 OA（/api/service/openapi/{entity} 管理面；
    -- resource_type 与 PEP map_resource 推导逐字一致）
    FOREACH v_entity IN ARRAY ARRAY['configs', 'sales', 'purchases', 'mades'] LOOP
        -- OA seed（判重键 resource_type+fk_resource）
        INSERT INTO isahl_auth.ngac_object_attribute
            (o_name, fk_policy_class, resource_type, fk_resource, resource_identifier, property, created_at)
        SELECT 'openapi-' || v_entity, v_pc_id, v_entity, 0, 'openapi-' || v_entity,
               (('{"description":"OpenAPI 产品 CRUD 管理面（/api/service/openapi/' || v_entity || '）"}')::jsonb), NOW()
        WHERE NOT EXISTS (
            SELECT 1 FROM isahl_auth.ngac_object_attribute
            WHERE resource_type = v_entity AND fk_resource = 0 AND deleted_at IS NULL
        )
        RETURNING id INTO v_oa_id;
        IF v_oa_id IS NULL THEN
            SELECT id INTO v_oa_id FROM isahl_auth.ngac_object_attribute
            WHERE resource_type = v_entity AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1;
        END IF;

        -- admin UA → OA（全权：read/write/create/update/delete/admin）
        INSERT INTO isahl_auth.ngac_association
            (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at)
        SELECT v_admin_ua_id, v_oa_id,
               ARRAY[v_read_ar_id, v_write_ar_id, v_create_ar_id, v_update_ar_id, v_delete_ar_id, v_admin_ar_id],
               v_pc_id, NOW()
        WHERE NOT EXISTS (
            SELECT 1 FROM isahl_auth.ngac_association
            WHERE fk_user_attribute = v_admin_ua_id AND fk_object_attribute = v_oa_id AND deleted_at IS NULL
        );
    END LOOP;

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
