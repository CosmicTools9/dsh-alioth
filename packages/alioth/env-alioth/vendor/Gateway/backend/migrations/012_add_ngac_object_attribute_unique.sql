-- 012_add_ngac_object_attribute_unique.sql
-- 为 ngac_object_attribute 增加 (resource_type, fk_resource) 唯一约束
--
-- 背景：crud_create 的 NGAC 对象属性创建使用
--   INSERT ... ON CONFLICT(resource_type, fk_resource) DO NOTHING
-- 但表无该唯一约束 → ON CONFLICT 报错被 `let _ =` 吞掉 → 对象属性从未创建
-- → 所有 API 创建的实体后续 update/delete 全部 403（NGAC 判定无资源匹配）。
--
-- 兼容：历史库可能存在重复行（如 oa_engineers fk_resource=0 多条），
-- 先清理重复再建约束。

DO $$
DECLARE
    v_dup INT;
BEGIN
    -- 清理重复（保留每组中 id 最大的行）
    DELETE FROM isahl_auth.ngac_object_attribute a
    USING isahl_auth.ngac_object_attribute b
    WHERE a.id < b.id
      AND a.resource_type = b.resource_type
      AND a.fk_resource = b.fk_resource
      AND a.deleted_at IS NULL AND b.deleted_at IS NULL;

    -- 建唯一约束（幂等）
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'uq_ngac_oa_resource'
          AND conrelid = 'isahl_auth.ngac_object_attribute'::regclass
    ) THEN
        ALTER TABLE isahl_auth.ngac_object_attribute
            ADD CONSTRAINT uq_ngac_oa_resource UNIQUE (resource_type, fk_resource);
    END IF;
END $$;
