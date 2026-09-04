use crate::AliothError;
use crate::ngac_org::COGNITION_CTE;
use sqlx::{AssertSqlSafe, PgPool};

// 认知链推导 CTE 消费方（NGAC_SPEC §2.2.3 消费同源义务）：推导链唯一实现 =
// `crate::ngac_org::COGNITION_CTE`（B-0 consolidate-ngac-cognition-source 收编），
// 本模块引用常量拼装，禁止复制 SQL。`$1` = fk_user。

/// 认知派生 UA 行 id 并入 user_attrs（读侧派生——UA 行由 SSO PDP auto-ensure 物化，
/// 供管理面 association；本决策仅 JOIN 已物化行；未物化 = 无关联 = fail-closed）。
const COGNITION_UA_UNION: &str = r#"
            UNION
            SELECT ua.id as ua_id, 0 as depth
            FROM cognition_ua_names cn
            INNER JOIN isahl_auth.ngac_user_attribute ua
                ON ua.o_name = cn.o_name AND ua.deleted_at IS NULL"#;

pub async fn require_resource_access(
    pool: &PgPool,
    user_id: i64,
    resource_type: &str,
    resource_id: i64,
    action: &str,
) -> Result<(), AliothError> {
    // Skip all NGAC checks if the isahl_auth schema doesn't exist
    if !sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT FROM information_schema.schemata WHERE schema_name='isahl_auth')",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(false)
    {
        return Ok(());
    }
    // Admin 豁免（NGAC_SPEC §6.2）：持有 admin user_attribute（含继承）的用户
    // 全资源放行——与 PDP list/decide 的 admin 语义一致，避免 handler 二次校验
    // 对 admin 误拒（如 report 等未注册 OA 的报表资源 403）。
    // 注：认知派生名（position:/view: 前缀）不可能等于 'admin'，admin 判定无需并入认知。
    let is_admin: bool = sqlx::query_scalar(
        r#"
        WITH RECURSIVE user_attrs AS (
            SELECT fk_user_attribute AS ua_id, 0 AS depth
            FROM isahl_auth.ngac_user_rr_attribute
            WHERE fk_user = $1 AND deleted_at IS NULL
              AND (expires_at IS NULL OR expires_at > NOW())
            UNION ALL
            SELECT unnest(ua.ancestor_ids)::BIGINT AS ua_id, depth + 1
            FROM isahl_auth.ngac_user_attribute ua
            INNER JOIN user_attrs AS ua_cte ON ua.id = ua_cte.ua_id
            WHERE ua_cte.depth < 10 AND ua.deleted_at IS NULL
        )
        SELECT EXISTS(
            SELECT 1 FROM user_attrs ua
            INNER JOIN isahl_auth.ngac_user_attribute a ON a.id = ua.ua_id
            WHERE a.o_name = 'admin' AND a.deleted_at IS NULL
        )
        "#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AliothError::Internal(format!("Admin check: {}", e)))?;
    if is_admin {
        return Ok(());
    }
    // From here on, isahl_auth schema is guaranteed to exist
    let owner_cte = r#"
        UNION
        SELECT op.owner_attr_id as ua_id, 0 as depth
        FROM isahl_auth.ngac_ownership_policy op, isahl.zc_id_lifecycle r
        WHERE op.resource_type = $2 AND r.id = $3
          AND r.created_by_id = $1 AND op.enabled = TRUE
        UNION
        SELECT op.benefit_attr_id as ua_id, 0 as depth
        FROM isahl_auth.ngac_ownership_policy op, isahl.zc_id_lifecycle r
        WHERE op.resource_type = $2 AND r.id = $3
          AND $1 = ANY(r.ak_benefit_user) AND op.enabled = TRUE
        UNION
        SELECT op.permit_attr_id as ua_id, 0 as depth
        FROM isahl_auth.ngac_ownership_policy op, isahl.zc_id_lifecycle r
        WHERE op.resource_type = $2 AND r.id = $3
          AND $1 = ANY(r.ak_permit_user) AND op.enabled = TRUE
        UNION
        SELECT op.access_attr_id as ua_id, 0 as depth
        FROM isahl_auth.ngac_ownership_policy op, isahl.zc_id_lifecycle r
        WHERE op.resource_type = $2 AND r.id = $3
          AND $1 = ANY(r.ak_access_user) AND op.enabled = TRUE
    "#
    .to_string();

    let sql = format!(
        "WITH RECURSIVE {cog_cte},
        user_attrs AS (
            SELECT fk_user_attribute as ua_id, 0 as depth
            FROM isahl_auth.ngac_user_rr_attribute
            WHERE fk_user = $1 AND deleted_at IS NULL AND (expires_at IS NULL OR expires_at > NOW())
            {owner}
            {cog_union}
            UNION ALL
            SELECT unnest(ua.ancestor_ids)::BIGINT as ua_id, depth + 1
            FROM isahl_auth.ngac_user_attribute ua
            INNER JOIN user_attrs AS ua_cte ON ua.id = ua_cte.ua_id
            WHERE ua_cte.depth < 10 AND ua.deleted_at IS NULL
        ),
        resource_attrs AS (
            SELECT id as oa_id, 0 as depth
            FROM isahl_auth.ngac_object_attribute
            WHERE resource_type = $2 AND fk_resource = $3 AND deleted_at IS NULL
            UNION ALL
            -- 通配资源（resource_type='*'）：覆盖所有实体类型
            SELECT id as oa_id, 0 as depth
            FROM isahl_auth.ngac_object_attribute
            WHERE resource_type = '*' AND deleted_at IS NULL
            UNION ALL
            -- 全局对象属性回落：具体资源无属性时使用 fk_resource=0 的全局属性
            SELECT id as oa_id, 0 as depth
            FROM isahl_auth.ngac_object_attribute
            WHERE resource_type = $2 AND fk_resource = 0 AND deleted_at IS NULL
              AND NOT EXISTS (
                  SELECT 1 FROM isahl_auth.ngac_object_attribute
                  WHERE resource_type = $2 AND fk_resource = $3 AND deleted_at IS NULL
              )
            UNION ALL
            SELECT unnest(oa.ancestor_ids)::BIGINT as oa_id, depth + 1
            FROM isahl_auth.ngac_object_attribute oa
            INNER JOIN resource_attrs AS ra_cte ON oa.id = ra_cte.oa_id
            WHERE ra_cte.depth < 10 AND oa.deleted_at IS NULL
        )
        SELECT EXISTS(
            SELECT 1 FROM isahl_auth.ngac_association a
            INNER JOIN user_attrs AS ua ON a.fk_user_attribute = ua.ua_id
            INNER JOIN resource_attrs AS ra ON a.fk_object_attribute = ra.oa_id
            WHERE a.deleted_at IS NULL
              AND EXISTS(SELECT 1 FROM isahl_auth.ngac_access_right ar
                         WHERE ar.id = ANY(a.ak_access_rights) AND ar.o_name = $4)
        ) as permitted",
        cog_cte = COGNITION_CTE,
        owner = owner_cte,
        cog_union = COGNITION_UA_UNION,
    );

    let permitted: bool = sqlx::query_scalar(AssertSqlSafe(sql.as_str()))
        .bind(user_id)
        .bind(resource_type)
        .bind(resource_id)
        .bind(action)
        .fetch_one(pool)
        .await
        .map_err(|e| AliothError::Internal(format!("Permission check: {}", e)))?;

    if !permitted {
        let bootstrap: (bool,) = sqlx::query_as(
            "SELECT COUNT(*)=0 FROM isahl_auth.ngac_association WHERE deleted_at IS NULL",
        )
        .fetch_one(pool)
        .await
        .map_err(|e| AliothError::Internal(format!("Bootstrap: {}", e)))?;

        if !bootstrap.0 {
            return Err(AliothError::Forbidden(format!(
                "Access denied: user {} lacks '{}' on {}:{}",
                user_id, action, resource_type, resource_id
            )));
        }
    }
    Ok(())
}
