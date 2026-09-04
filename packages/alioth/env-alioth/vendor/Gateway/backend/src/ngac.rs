//! NGAC 权限解析模块
//!
//! 从 NGAC 数据库中查询用户的策略上下文，供 chat_sessions 等模块使用。

use serde_json::Value;
use sqlx::PgPool;

/// 解析用户在 NGAC 系统中的完整权限上下文。
///
/// 返回 JSON 包含用户的策略类、实体引用和已分配的 NGAC 属性列表。
/// 供 AI session 的 `permissions` 列存储，LLM agent 据此约束数据 API 调用范围。
pub async fn resolve_user_permissions(pool: &PgPool, user_id: i64) -> Result<Value, String> {
    // 1. 查询用户基础信息 + NGAC 策略类
    let user_row = sqlx::query_as::<_, (Option<i64>, Option<String>, Option<i64>)>(
        r#"
        SELECT fk_policy_class, entity_table, entity_id
        FROM isahl_auth.auth_users
        WHERE id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("DB error querying auth_users: {}", e))?
    .ok_or_else(|| format!("User {} not found", user_id))?;

    let (fk_policy_class, entity_table, entity_id) = user_row;

    // 2. 查询策略类名称
    let policy_class_name: Option<String> = match fk_policy_class {
        Some(pc_id) => sqlx::query_scalar::<_, String>(
            r#"SELECT o_name FROM isahl_auth.ngac_policy_class WHERE id = $1"#,
        )
        .bind(pc_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("DB error querying ngac_policy_class: {}", e))?,
        None => None,
    };

    // 3. 查询用户已分配的 NGAC 属性名称（含认知派生 UA——position:/view: 前缀；
    //    B-0 consolidate-ngac-cognition-source：推导链唯一实现 =
    //    `common::ngac_org::COGNITION_CTE`（NGAC_SPEC §2.2.3 消费同源义务），
    //    本函数引用常量拼装、禁止复制 SQL；仅并入已物化 UA。本函数为 AI session
    //    上下文提示，裁决以 PEP/PDP 为准）
    let user_attributes: Vec<String> = {
        let sql = format!(
            r#"
            WITH {COGNITION_CTE}
            SELECT attr.o_name
            FROM isahl_auth.ngac_user_rr_attribute rel
            JOIN isahl_auth.ngac_user_attribute attr ON attr.id = rel.fk_user_attribute
            WHERE rel.fk_user = $1
              AND (attr.deleted_at IS NULL)
            UNION
            SELECT attr.o_name
            FROM isahl_auth.ngac_user_attribute attr
            JOIN cognition_ua_names cn ON cn.o_name = attr.o_name
            WHERE attr.deleted_at IS NULL
            "#,
            COGNITION_CTE = common::ngac_org::COGNITION_CTE
        );
        sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(user_id)
            .fetch_all(pool)
            .await
            .map_err(|e| format!("DB error querying user attributes: {}", e))?
    };

    Ok(serde_json::json!({
        "userId": user_id,
        "policyClassId": fk_policy_class,
        "policyClass": policy_class_name,
        "entityId": entity_id,
        "entityTable": entity_table,
        "userAttributes": user_attributes,
    }))
}
