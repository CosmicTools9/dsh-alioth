use crate::ngac_org::COGNITION_CTE;
use crate::AliothError;
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
        -- M218 framework-prohibition：deny-overrides 分支复用同一闭包 CTE（单 SQL 双列）。
        -- Framework 仅评估无条件 prohibition（conditions 为空）；条件式 prohibition
        -- 由 SSO PDP decide 全量评估（documented boundary）。
        SELECT EXISTS(
            SELECT 1 FROM isahl_auth.ngac_association a
            INNER JOIN user_attrs AS ua ON a.fk_user_attribute = ua.ua_id
            INNER JOIN resource_attrs AS ra ON a.fk_object_attribute = ra.oa_id
            WHERE a.deleted_at IS NULL
              AND EXISTS(SELECT 1 FROM isahl_auth.ngac_access_right ar
                         WHERE ar.id = ANY(a.ak_access_rights) AND ar.o_name = $4)
        ) AS permitted,
        EXISTS(
            SELECT 1 FROM isahl_auth.ngac_prohibition p
            JOIN user_attrs ua ON p.fk_user_attribute = ua.ua_id
            JOIN resource_attrs ra ON p.fk_object_attribute = ra.oa_id
            WHERE p.deleted_at IS NULL AND p.is_active
              AND (p.conditions IS NULL OR p.conditions = '{{}}'::jsonb)
              AND EXISTS(SELECT 1 FROM isahl_auth.ngac_access_right ar
                         WHERE ar.id = ANY(p.ak_access_rights) AND ar.o_name = $4)
        ) AS prohibited",
        cog_cte = COGNITION_CTE,
        owner = owner_cte,
        cog_union = COGNITION_UA_UNION,
    );

    let (permitted, prohibited): (bool, bool) =
        sqlx::query_as::<_, (bool, bool)>(AssertSqlSafe(sql.as_str()))
            .bind(user_id)
            .bind(resource_type)
            .bind(resource_id)
            .bind(action)
            .fetch_one(pool)
            .await
            .map_err(|e| AliothError::Internal(format!("Permission check: {}", e)))?;

    // M218 语义顺序（deny-overrides，对齐 SSO PDP decide）：prohibition 无条件优先，
    // 先于 admin 豁免判定——prohibition 对 admin 同样生效。prohibited/permitted 单 SQL
    // 同算，判定不依赖提前返回。
    if prohibited {
        return Err(AliothError::Forbidden(format!(
            "Access denied: user {} is prohibited '{}' on {}:{}",
            user_id, action, resource_type, resource_id
        )));
    }
    if permitted {
        return Ok(());
    }
    // Admin 治理豁免降级为**兜底**（NGAC_SPEC §6.2，与 decide.rs「遍历后兜底」一致）：
    // 仅在无 prohibition 且无 association 命中（NotApplicable 态）时放行——admin 绕过
    // 的是「无策略」而非显式 prohibition。注：认知派生名（position:/view: 前缀）不可能
    // 等于 'admin'，admin 判定无需并入认知。此查询仅两者皆否时执行。
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

    // Bootstrap 判定（fail-open 条件收窄，Phase C hardening）：到达此处意味着
    // !prohibited && !permitted && !admin。仅当整库从未种入任何 NGAC 策略（无 default
    // policy class **且** association 表空）时才按 bootstrap 期放行；任一条件不满足即
    // deny——防「policy class 已种（种子事务成功）但 association 种子部分失败 → 表空」
    // 窗口被误判为从未初始化而 fail-open。
    let bootstrap: (bool,) = sqlx::query_as(
        "SELECT ((SELECT COUNT(*) FROM isahl_auth.ngac_policy_class WHERE o_name='default') = 0)
                AND ((SELECT COUNT(*) FROM isahl_auth.ngac_association WHERE deleted_at IS NULL) = 0)",
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
    Ok(())
}
