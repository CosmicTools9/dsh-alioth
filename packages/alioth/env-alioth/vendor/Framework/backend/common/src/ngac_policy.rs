//! ngac_policy — org_policy 规范资产投影与 NGAC 派生**唯一实现**（Phase D-2 消费）
//!
//! 收编 D-1 规范资产线（org_policy_class/org_policy_rule）的派生侧：
//! - [`project_policy_class`]：D-1 spec §5 投影契约——读 `isahl_auth.org_policy_class`
//!   （仅 `state='active'`）+ `org_policy_rule`（仅 `state='active'`），产出
//!   UA/OA/action/label 投影，供派生器与 SSO 管理面 `/{id}/projection` 预览共用。
//! - [`derive_from_class`]：D-2 派生器——消费投影，幂等 ensure 类别 UA
//!   （`o_name` = 投影 ua_name，仿 [`crate::ngac_org::ensure_cognition_uas`]）+ 每
//!   resource_type 集合 OA（`fk_resource = 0`）+ association（UA→OA × actions），
//!   NOT EXISTS + ON CONFLICT 双保险，重复派生零新增。
//!
//! 消费方（SSO `ngac/org_policy.rs`、后续 Gateway 派生入口）MUST 调用本模块函数，
//! **禁止复制投影/派生 SQL**（NGAC_SPEC §2.2.3/§2.2.4 消费同源义务）——投影与派生
//! 表结构变更只改此处。SSO/Gateway 共用单一实现，派生语义漂移即违反本模块契约。
//!
//! 派生语义边界（D-2 设计 §2/§5）：
//! - 仅 `active` class 可派生（未激活/软删/不存在 → `sqlx::Error::RowNotFound`）；
//! - UA 挂 `default` 策略类（同 `ensure_cognition_uas` 物化约定），缺省以
//!   `derived_from='cognition'` 标记（系统派生节点，生命周期与认知派生 UA 同域，
//!   非用户手搓指派）；
//! - 集合 OA 名 = `{resource_type}-collection`（对齐 Gateway seed 集合 OA 命名）；
//!   已存在（含软删残留被全量唯一索引拦截）不重建——撤销语义归调用方；
//! - association 权限 = 该 resource_type 各 active rule 的 actions 并集（去重保序），
//!   映射到 `ngac_access_right` 存量行——未知动作名静默不授（fail-closed），全空则
//!   不建 association；不写策略审计日志（系统派生非用户策略编辑）。

use serde::Serialize;
use sqlx::PgPool;

/// D-2 派生器投影契约（D-1 spec §5）——派生器据此幂等 ensure UA/OA/association。
/// 字段与序列化形状 = SSO 管理面投影端点输出（薄转发，禁止第二份定义）。
#[derive(Debug, Clone, Serialize)]
pub struct PolicyProjection {
    /// 目标 UA o_name：`ua_template.name_rule` 中 `{code}` 令牌替换为 class.code；
    /// name_rule 缺失/不可解析时回退 class.code。
    pub ua_name: String,
    /// OA 引用：每个 active rule 一条 `(resource_type, rule.id)`（按 resource_type
    /// 去重保序）——派生器以 resource_type 为 OA 域、rule.id 为行级溯源引用。
    pub oa_refs: Vec<(String, i64)>,
    /// 动作白名单：class.scope.actions ∪ 各 active rule.actions（去重保序）。
    pub actions: Vec<String>,
    /// 分级：class.label_code 优先，缺省取首个带 label_code 的 active rule。
    pub label: Option<String>,
}

/// [`derive_from_class`] 执行统计——计数均为**本次新增**（幂等：重复派生归零）。
#[derive(Debug, Clone)]
pub struct DeriveStats {
    /// 类别 UA o_name（投影 ua_name）。
    pub ua_name: String,
    /// 类别 UA 本次新建数（0/1；缺省物化）。
    pub ua_created: i64,
    /// 集合 OA 本次新建数（按 resource_type 去重）。
    pub oa_created: i64,
    /// association 本次新建数（按 (UA, OA, 策略类) 幂等）。
    pub associations_created: i64,
    /// 参与派生的 active rule 行数。
    pub rules_processed: usize,
}

/// org_policy_class 投影所需行（最小列集；读侧唯一 SQL 见 [`load_active_class_rules`]）。
#[derive(sqlx::FromRow)]
struct OrgPolicyClassRow {
    code: String,
    scope: serde_json::Value,
    ua_template: serde_json::Value,
    label_code: Option<String>,
}

/// org_policy_rule 投影所需行。
#[derive(sqlx::FromRow)]
struct OrgPolicyRuleRow {
    id: i64,
    resource_type: String,
    actions: serde_json::Value,
    label_code: Option<String>,
}

/// D-2 派生器投影（D-1 spec §5）——读 active class + active rules。
/// class 不存在/未激活/软删 → `sqlx::Error::RowNotFound`（派生只认 active 规范）。
pub async fn project_policy_class(
    pool: &PgPool,
    class_id: i64,
) -> Result<PolicyProjection, sqlx::Error> {
    let mut conn = pool.acquire().await?;
    let (class, rules) = load_active_class_rules(&mut conn, class_id).await?;
    Ok(project(&class, &rules))
}

/// D-2 派生器：投影 → 幂等落地 类别 UA + 集合 OA + association。
///
/// 管线（单事务，全程同快照）：
/// 1. 投影 active class（缺失即 Err，未激活规范不派生）；
/// 2. ensure 类别 UA：`o_name` = 投影 ua_name，缺省创建并打
///    `derived_from='cognition'` 标记（仿 `ensure_cognition_uas`，按投影名而非
///    认知链命名——类别 UA 与认知派生 UA 同命名域，生命周期标记一致）；
/// 3. 逐条 rule 按 resource_type 聚合：ensure 集合 OA（`fk_resource=0`，缺省
///    `default` 策略类；名 `{resource_type}-collection`）→ 关联 UA→OA，权限 =
///    该 resource_type 各 rule 的 actions 并集映射 access_right 存量名
///    （NOT EXISTS + ON CONFLICT 幂等；重复调用零新增）。
///
/// 返回本次新增计数（UA/OA/association 各行独立计数）。
pub async fn derive_from_class(
    pool: &PgPool,
    class_id: i64,
) -> Result<DeriveStats, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let (class, rules) = load_active_class_rules(&mut tx, class_id).await?;
    let projection = project(&class, &rules);

    // 1. 类别 UA 物化（缺省创建；default 策略类幂等 upsert，同 ensure_cognition_uas）
    let ua_created = sqlx::query(
        r#"
        WITH pc_ins AS (
            INSERT INTO isahl_auth.ngac_policy_class (o_name, description)
            VALUES ('default', 'Default policy class')
            ON CONFLICT (o_name) DO NOTHING
            RETURNING id
        ),
        default_pc AS (
            SELECT id FROM pc_ins
            UNION ALL
            SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default'
            LIMIT 1
        )
        INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, property)
        SELECT $1, dp.id,
               jsonb_build_object(
                   'derived_from', 'cognition',
                   'source_kind', 'org_policy_class',
                   'source_code', $2
               )
        FROM default_pc dp
        ON CONFLICT (o_name, fk_policy_class) WHERE deleted_at IS NULL DO NOTHING
        "#,
    )
    .bind(&projection.ua_name)
    .bind(&class.code)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    // 2. 按 resource_type 聚合各 rule 的 actions（保序去重），并集授权
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    let mut seen_rt = std::collections::HashSet::new();
    for rule in &rules {
        let actions = class_from_value(&rule.actions);
        if seen_rt.insert(rule.resource_type.clone()) {
            groups.push((rule.resource_type.clone(), actions));
        } else if let Some(g) = groups
            .iter_mut()
            .find(|g| g.0 == rule.resource_type)
        {
            g.1.extend(actions);
        }
    }

    let mut stats = DeriveStats {
        ua_name: projection.ua_name.clone(),
        ua_created: ua_created as i64,
        oa_created: 0,
        associations_created: 0,
        rules_processed: rules.len(),
    };

    for (resource_type, actions) in groups {
        let actions = dedupe_keep_order(actions);
        // 集合 OA（fk_resource = 0；缺省 default 策略类；NOT EXISTS + ON CONFLICT
        // 双保险——部分唯一索引拦存活重复、全量索引拦软删残留）
        stats.oa_created += sqlx::query(
            r#"
            INSERT INTO isahl_auth.ngac_object_attribute
                (o_name, fk_policy_class, resource_type, fk_resource, property)
            SELECT $1,
                   (SELECT id FROM isahl_auth.ngac_policy_class
                    WHERE o_name = 'default' LIMIT 1),
                   $2, 0,
                   jsonb_build_object(
                       'description', 'org_policy 派生集合',
                       'derived_from', 'org_policy_class'
                   )
            WHERE NOT EXISTS (
                SELECT 1 FROM isahl_auth.ngac_object_attribute oa
                WHERE oa.resource_type = $2 AND oa.fk_resource = 0
                  AND oa.deleted_at IS NULL
            )
            ON CONFLICT (resource_type, fk_resource) DO NOTHING
            "#,
        )
        .bind(format!("{resource_type}-collection"))
        .bind(&resource_type)
        .execute(&mut *tx)
        .await?
        .rows_affected() as i64;

        // association（UA→OA × access_right 存量名；全空动作不建；幂等双保险）
        stats.associations_created += sqlx::query(
            r#"
            INSERT INTO isahl_auth.ngac_association
                (fk_user_attribute, fk_object_attribute, fk_policy_class,
                 ak_access_rights, created_at)
            SELECT ua.id, oa.id, ua.fk_policy_class,
                   ARRAY(SELECT ar.id FROM isahl_auth.ngac_access_right ar
                         WHERE ar.o_name = ANY($3::text[])),
                   NOW()
            FROM isahl_auth.ngac_user_attribute ua
            JOIN isahl_auth.ngac_object_attribute oa
              ON oa.resource_type = $2 AND oa.fk_resource = 0
             AND oa.deleted_at IS NULL
            WHERE ua.o_name = $1 AND ua.deleted_at IS NULL
              AND (SELECT count(*) FROM isahl_auth.ngac_access_right ar
                   WHERE ar.o_name = ANY($3::text[])) > 0
              AND NOT EXISTS (
                  SELECT 1 FROM isahl_auth.ngac_association a
                  WHERE a.fk_user_attribute = ua.id
                    AND a.fk_object_attribute = oa.id
                    AND a.fk_policy_class = ua.fk_policy_class
                    AND a.deleted_at IS NULL
              )
            ON CONFLICT (fk_user_attribute, fk_object_attribute, fk_policy_class)
                DO NOTHING
            "#,
        )
        .bind(&projection.ua_name)
        .bind(&resource_type)
        .bind(&actions)
        .execute(&mut *tx)
        .await?
        .rows_affected() as i64;
    }

    tx.commit().await?;
    Ok(stats)
}

/// active class + active rules 读侧唯一 SQL（投影与派生共用；缺 class → RowNotFound）。
async fn load_active_class_rules(
    conn: &mut sqlx::PgConnection,
    class_id: i64,
) -> Result<(OrgPolicyClassRow, Vec<OrgPolicyRuleRow>), sqlx::Error> {
    let class = sqlx::query_as::<_, OrgPolicyClassRow>(
        "SELECT code, scope, ua_template, label_code \
         FROM isahl_auth.org_policy_class \
         WHERE id = $1 AND state = 'active' AND deleted_at IS NULL",
    )
    .bind(class_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(sqlx::Error::RowNotFound)?;
    let rules = sqlx::query_as::<_, OrgPolicyRuleRow>(
        "SELECT id, resource_type, actions, label_code \
         FROM isahl_auth.org_policy_rule \
         WHERE policy_class_id = $1 AND state = 'active' AND deleted_at IS NULL \
         ORDER BY id",
    )
    .bind(class_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok((class, rules))
}

/// class × rules → 投影（纯函数；与派生共用，保证投影/落地不漂移）。
fn project(class: &OrgPolicyClassRow, rules: &[OrgPolicyRuleRow]) -> PolicyProjection {
    let ua_name = class
        .ua_template
        .get("name_rule")
        .and_then(|v| v.as_str())
        .filter(|r| r.contains("{code}"))
        .map(|r| r.replace("{code}", &class.code))
        .unwrap_or_else(|| class.code.clone());

    let mut oa_refs: Vec<(String, i64)> = Vec::new();
    let mut seen_oa = std::collections::HashSet::new();
    let mut actions = class_from_value(
        class
            .scope
            .get("actions")
            .unwrap_or(&serde_json::Value::Null),
    );
    let mut label = class.label_code.clone();
    for rule in rules {
        if seen_oa.insert(rule.resource_type.clone()) {
            oa_refs.push((rule.resource_type.clone(), rule.id));
        }
        actions.extend(class_from_value(&rule.actions));
        if label.is_none() {
            label = rule.label_code.clone();
        }
    }

    PolicyProjection {
        ua_name,
        oa_refs,
        actions: dedupe_keep_order(actions),
        label,
    }
}

/// JSONB 字符串数组 → Vec<String>（非数组/非串元素忽略）。
fn class_from_value(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// 保序去重。
fn dedupe_keep_order(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|x| seen.insert(x.clone()))
        .collect()
}
