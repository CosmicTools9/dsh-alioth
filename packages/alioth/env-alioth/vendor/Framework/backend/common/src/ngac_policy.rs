//! ngac_policy — org_policy 权限规范投影与 NGAC 派生**唯一实现**（Phase D-2 消费）
//!
//! 收编 D-1 权限规范线（org_policy_class/org_policy_rule）的派生侧：
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
    /// 本次**刷新**既有派生集合 OA `property.label` 的行数（canonical 标签变更后 >0；
    /// 新增行计入 `oa_created`，二者互斥——保证「重复派生零新增」判据不被刷新污染）。
    pub oa_labels_refreshed: i64,
    /// 本次**刷新**既有派生 UA `property.clearance` 的行数（裁决 B-0；新增 UA 计入 `ua_created`）。
    pub ua_clearance_refreshed: i64,
    /// association 本次新建数（按 (UA, OA, 策略类) 幂等）。
    pub associations_created: i64,
    /// 规则义务派生：本次新建的 `ngac_obligation` 行数（幂等：重复派生归零）。
    pub obligations_created: i64,
    /// 规则义务派生：本次**刷新**（含系统墓碑复活）的行数（差异刷新语义，与 created 互斥）。
    pub obligations_refreshed: i64,
    /// 规则义务派生：本次**退役**（源清空 / rule 消失）而软删的行数。
    pub obligations_retired: i64,
    /// 规则条件接线：本次刷新 association `conditions.rhai` 的行数（含清空移除键；差异语义）。
    pub conditions_refreshed: i64,
    /// 负向模板派生：本次新建的 `ngac_prohibition` 行数。
    pub prohibitions_created: i64,
    /// 负向模板派生：本次刷新（含系统墓碑复活）的行数。
    pub prohibitions_refreshed: i64,
    /// 负向模板派生：本次退役（模板清空 / resource_type 消失）而软删的行数。
    pub prohibitions_retired: i64,
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
    /// 规范层负向模板（Rhai 源串；`null`/空 ⇒ 不派生 prohibition）。
    /// 本列自 2026-10-10 起接线（change `wire-org-policy-declaration-fields` 切片 B）。
    prohibition_template: Option<serde_json::Value>,
}

/// org_policy_rule 投影所需行。
#[derive(sqlx::FromRow)]
struct OrgPolicyRuleRow {
    id: i64,
    resource_type: String,
    actions: serde_json::Value,
    label_code: Option<String>,
    /// 规则边界条件（Rhai 源串；`null`/空 ⇒ 该 rule 无额外条件）。
    /// 本列自 2026-10-10 起接线（切片 B）。
    condition: Option<serde_json::Value>,
    /// D-1「义务引用」声明（Rhai 源串或 JSON 字符串数组；`null` ⇒ 不派生义务）。
    /// 本列自 2026-10-10 起接线（change `wire-org-policy-declaration-fields`）。
    obligation: Option<serde_json::Value>,
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
pub async fn derive_from_class(pool: &PgPool, class_id: i64) -> Result<DeriveStats, sqlx::Error> {
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
               ) || CASE WHEN $3::text IS NULL THEN '{}'::jsonb
                         ELSE jsonb_build_object('clearance', $3::text) END
        FROM default_pc dp
        ON CONFLICT (o_name, fk_policy_class) WHERE deleted_at IS NULL DO NOTHING
        "#,
    )
    .bind(&projection.ua_name)
    .bind(&class.code)
    .bind(projection.label.as_deref())
    .execute(&mut *tx)
    .await?
    .rows_affected();

    // 2. 按 resource_type 聚合各 rule 的 actions（保序去重）+ 规则条件源（切片 B 接线）
    let mut groups: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
    let mut seen_rt = std::collections::HashSet::new();
    for rule in &rules {
        let actions = class_from_value(&rule.actions);
        let cond = rhai_source(rule.condition.as_ref()).unwrap_or_default();
        if seen_rt.insert(rule.resource_type.clone()) {
            let mut conds = Vec::new();
            if !cond.is_empty() {
                conds.push(cond);
            }
            groups.push((rule.resource_type.clone(), actions, conds));
        } else if let Some(g) = groups.iter_mut().find(|g| g.0 == rule.resource_type) {
            g.1.extend(actions);
            if !cond.is_empty() && !g.2.contains(&cond) {
                g.2.push(cond);
            }
        }
    }

    let mut stats = DeriveStats {
        ua_name: projection.ua_name.clone(),
        ua_created: ua_created as i64,
        ua_clearance_refreshed: 0,
        oa_created: 0,
        oa_labels_refreshed: 0,
        associations_created: 0,
        obligations_created: 0,
        obligations_refreshed: 0,
        obligations_retired: 0,
        conditions_refreshed: 0,
        prohibitions_created: 0,
        prohibitions_refreshed: 0,
        prohibitions_retired: 0,
        rules_processed: rules.len(),
    };

    // 1b. 主体侧 clearance 刷新（**裁决 B-0**，2026-09-30）：canonical 声明变更后，重派生 MUST 使其落到
    //     派生 UA 的 `property.clearance`——该 UA 名 = `position:{class.code}`，与认知链物化行**同名同策略类**
    //     ⇒ 认知绑定的员工经 `effective_ua_ids` 继承该 clearance（端到端可用）。
    //     幂等：仅在新值**非空且与现存不同**时写入；`||` 合并保留认知标记（derived_from/source_*）。
    //     **限定 `default` 策略类**：派生器与认知链均只在 default 类物化该名 ⇒ 避免「他类同名 UA」被顺带写入
    //     clearance（名字碰撞不得成为授权路径）。
    stats.ua_clearance_refreshed += sqlx::query(
        r#"
        UPDATE isahl_auth.ngac_user_attribute ua
           SET property = COALESCE(ua.property, '{}'::jsonb)
                          || jsonb_build_object('clearance', $2::text),
               updated_at = NOW()
         WHERE ua.o_name = $1 AND ua.deleted_at IS NULL
           AND ua.fk_policy_class = (SELECT id FROM isahl_auth.ngac_policy_class
                                      WHERE o_name = 'default' LIMIT 1)
           AND $2::text IS NOT NULL
           AND COALESCE(ua.property ->> 'clearance', '') IS DISTINCT FROM $2::text
        "#,
    )
    .bind(&projection.ua_name)
    .bind(projection.label.as_deref())
    .execute(&mut *tx)
    .await?
    .rows_affected() as i64;

    let mut live_prohibitions: Vec<String> = Vec::new();
    for (resource_type, actions, rhai_conds) in groups {
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
                   ) || CASE WHEN $3::text IS NULL THEN '{}'::jsonb
                             ELSE jsonb_build_object('label', $3::text) END
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
        .bind(projection.label.as_deref())
        .execute(&mut *tx)
        .await?
        .rows_affected() as i64;

        // 分级标签刷新（NGAC_SPEC §2.7 资源侧 carrier 的唯一正本写入方）：canonical 声明
        // （`class.label_code` 优先，缺省首个带 label_code 的 active rule）变更后，重派生
        // MUST 使其落到集合 OA 的 `property.label`——否则「规范层改标签」不会影响判定。
        // 幂等：仅在新标签**非空且与现存不同**时写入（新标签为空 ⇒ 不触碰既有值，避免无差异写入被判为变更）。
        stats.oa_labels_refreshed += sqlx::query(
            r#"
            UPDATE isahl_auth.ngac_object_attribute oa
               SET property = COALESCE(oa.property, '{}'::jsonb)
                              || jsonb_build_object('label', $2::text),
                   updated_at = NOW()
             WHERE oa.resource_type = $1 AND oa.fk_resource = 0 AND oa.deleted_at IS NULL
               AND $2::text IS NOT NULL
               AND COALESCE(oa.property ->> 'label', '') IS DISTINCT FROM $2::text
            "#,
        )
        .bind(&resource_type)
        .bind(projection.label.as_deref())
        .execute(&mut *tx)
        .await?
        .rows_affected() as i64;

        // association（UA→OA × access_right 存量名；全空动作不建；幂等双保险）。
        // ON CONFLICT 分支**只复活系统撤销墓碑**（`updated_by_id = 0` =
        // revoke_expired_class_derivations 标记）——admin 软删行（updated_by_id ≠ 0）
        // 保持「不复活」语义；复活即重派生幂等重建（撤销→再入闭环）。
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
              AND ua.fk_policy_class = (SELECT id FROM isahl_auth.ngac_policy_class
                                        WHERE o_name = 'default')
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
                DO UPDATE SET deleted_at = NULL, updated_by_id = NULL,
                              updated_at = NOW(),
                              ak_access_rights = EXCLUDED.ak_access_rights
                WHERE ngac_association.deleted_at IS NOT NULL
                  AND ngac_association.updated_by_id = 0
            "#,
        )
        .bind(&projection.ua_name)
        .bind(&resource_type)
        .bind(&actions)
        .execute(&mut *tx)
        .await?
        .rows_affected() as i64;

        // 规则条件接线（切片 B）：该 resource_type 各 rule 的 `condition` 合成为
        // `conditions = {"rhai": "<expr>"}` 落到派生 association；无条件 ⇒ 移除 `rhai` 键。
        // 差异刷新（`IS DISTINCT FROM` 守卫）⇒ 重复派生零写入（幂等）。
        let composed = compose_rhai_conditions(&rhai_conds);
        stats.conditions_refreshed += sqlx::query(
            r#"
            UPDATE isahl_auth.ngac_association a
               SET conditions = CASE
                       WHEN $3::text IS NULL
                         THEN (COALESCE(a.conditions, '{}'::jsonb) - 'rhai')
                       ELSE jsonb_set(COALESCE(a.conditions, '{}'::jsonb),
                                      '{rhai}', to_jsonb($3::text))
                   END
             WHERE a.fk_user_attribute = (SELECT id FROM isahl_auth.ngac_user_attribute
                                           WHERE o_name = $1 AND deleted_at IS NULL
                                             AND fk_policy_class = (SELECT id
                                                                      FROM isahl_auth.ngac_policy_class
                                                                     WHERE o_name = 'default' LIMIT 1)
                                           LIMIT 1)
               AND a.fk_object_attribute = (SELECT id FROM isahl_auth.ngac_object_attribute
                                             WHERE resource_type = $2 AND fk_resource = 0
                                               AND deleted_at IS NULL LIMIT 1)
               AND a.deleted_at IS NULL
               AND COALESCE(a.conditions ->> 'rhai', '') IS DISTINCT FROM COALESCE($3::text, '')
            "#,
        )
        .bind(&projection.ua_name)
        .bind(&resource_type)
        .bind(composed.as_deref())
        .execute(&mut *tx)
        .await?
        .rows_affected() as i64;

        // 规范负向模板派生（切片 B）：`class.prohibition_template` 非空 ⇒ 该类 UA × 集合 OA 上
        // 一行 `ngac_prohibition`（`conditions = {"rhai": <模板>}`，`ak_access_rights` = 该类动作
        // 白名单映射）。模板空/NULL ⇒ 不派生（零行为变更），并由下方扫帚退役既往派生行。
        if let Some(tmpl) = rhai_source(class.prohibition_template.as_ref()) {
            let o_name = format!(
                "{}deny-{}",
                obligation_o_name_prefix(&class.code),
                resource_type
            );
            live_prohibitions.push(o_name.clone());
            stats.prohibitions_created += sqlx::query(
                r#"
                INSERT INTO isahl_auth.ngac_prohibition
                    (o_name, fk_user_attribute, fk_object_attribute, ak_access_rights,
                     is_active, conditions, created_by_id, created_at, updated_at)
                SELECT $1, ua.id, oa.id,
                       ARRAY(SELECT ar.id FROM isahl_auth.ngac_access_right ar
                              WHERE ar.o_name = ANY($4::text[])),
                       TRUE, jsonb_build_object('rhai', $3::text), 0, NOW(), NOW()
                  FROM isahl_auth.ngac_user_attribute ua
                  JOIN isahl_auth.ngac_object_attribute oa
                    ON oa.resource_type = $2 AND oa.fk_resource = 0 AND oa.deleted_at IS NULL
                 WHERE ua.o_name = $5 AND ua.deleted_at IS NULL
                   AND ua.fk_policy_class = (SELECT id FROM isahl_auth.ngac_policy_class
                                              WHERE o_name = 'default' LIMIT 1)
                   AND (SELECT count(*) FROM isahl_auth.ngac_access_right ar
                         WHERE ar.o_name = ANY($4::text[])) > 0
                   AND NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_prohibition p
                                    WHERE p.o_name = $1 AND p.deleted_at IS NULL)
                "#,
            )
            .bind(&o_name)
            .bind(&resource_type)
            .bind(&tmpl)
            .bind(&actions)
            .bind(&projection.ua_name)
            .execute(&mut *tx)
            .await?
            .rows_affected() as i64;

            stats.prohibitions_refreshed += sqlx::query(
                r#"
                UPDATE isahl_auth.ngac_prohibition p
                   SET is_active = TRUE, deleted_at = NULL,
                       ak_access_rights = ARRAY(SELECT ar.id FROM isahl_auth.ngac_access_right ar
                                                 WHERE ar.o_name = ANY($4::text[])),
                       conditions = jsonb_build_object('rhai', $3::text),
                       updated_by_id = CASE WHEN p.updated_by_id = 0 THEN NULL ELSE p.updated_by_id END,
                       updated_at = NOW()
                 WHERE p.o_name = $1
                   AND ((p.deleted_at IS NULL
                         AND COALESCE(p.conditions ->> 'rhai', '') IS DISTINCT FROM $3::text)
                        OR (p.deleted_at IS NOT NULL AND p.updated_by_id = 0))
                "#,
            )
            .bind(&o_name)
            .bind(&resource_type)
            .bind(&tmpl)
            .bind(&actions)
            .execute(&mut *tx)
            .await?
            .rows_affected() as i64;
        }
    }

    // 负向模板退役（切片 B）：模板清空 / resource_type 消失 ⇒ 软删存活派生 prohibition
    // （系统墓碑 `updated_by_id = 0`；重派生幂等复活、admin 软删行不复活）。
    stats.prohibitions_retired += sqlx::query(
        r#"
        UPDATE isahl_auth.ngac_prohibition
           SET deleted_at = NOW(), is_active = FALSE, updated_at = NOW(), updated_by_id = 0
         WHERE left(o_name, length($1)) = $1 AND deleted_at IS NULL
           AND o_name <> ALL($2::text[])
        "#,
    )
    .bind(obligation_o_name_prefix(&class.code))
    .bind(&live_prohibitions)
    .execute(&mut *tx)
    .await?
    .rows_affected() as i64;

    // 规则义务派生（`org_policy_rule.obligation` → `isahl_auth.ngac_obligation`；
    // 规约 = `openspec/specs/ngac-obligation-evaluation :: derived-rule-obligations`）。
    // 与 association 派生同事务同快照：源变更与派生物同批收敛。
    let (obl_created, obl_refreshed, obl_retired) =
        derive_rule_obligations(&mut tx, &class.code, &rules).await?;
    stats.obligations_created = obl_created;
    stats.obligations_refreshed = obl_refreshed;
    stats.obligations_retired = obl_retired;

    tx.commit().await?;
    Ok(stats)
}

// ── 规则义务派生（wire-org-policy-declaration-fields）────────────────────────

/// 规则义务派生行的 `o_name` 前缀：可溯源（class code）+ 幂等键 + 撤销扫帚的定位面。
fn obligation_o_name_prefix(class_code: &str) -> String {
    format!("org_policy:{class_code}:")
}

/// Rhai 源提取（`Value::String` 且 trim 非空 ⇒ `Some(src)`；其余 ⇒ `None`）。
///
/// `condition` / `prohibition_template` 的写径（`org_policy.rs`）只接受字符串或 `null`
/// （结构化占位被拒）⇒ 本函数与写径口径一致。
fn rhai_source(v: Option<&serde_json::Value>) -> Option<String> {
    match v? {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

/// 同 resource_type 的多条 rule 条件合成（任一成立即满足 ⇒ `(c1) || (c2)`）。
/// 空 ⇒ `None`（调用方据此**移除** association 的 `rhai` 键）。
fn compose_rhai_conditions(sources: &[String]) -> Option<String> {
    match sources {
        [] => None,
        [single] => Some(single.clone()),
        many => Some(
            many.iter()
                .map(|s| format!("({s})"))
                .collect::<Vec<_>>()
                .join(" || "),
        ),
    }
}

/// `org_policy_rule.obligation` 的有效源（非空 Rhai 源串 / 非空字符串数组；其余 ⇒ 不派生）。
fn obligation_source(v: &serde_json::Value) -> Option<&serde_json::Value> {
    match v {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(v),
        serde_json::Value::Array(a) if !a.is_empty() => Some(v),
        _ => None,
    }
}

/// 规则的事件匹配表达式（Rhai 布尔；上下文变量见 `ngac/obligation.rs::ctx_variables`）。
///
/// `resource` = PEP 单源 `format!("{resource_type}:{id}")`（`auth/middleware.rs`）⇒ 用
/// `starts_with("<resource_type>:")` 定位资源类型；`action` 为访问动作，规则声明动作集非空时追加
/// `contains` 判据。字符串字面量经 Rust `{:?}` 生成（资源类型/动作名均为注册表标识符，无需转义）。
fn rule_event_pattern(resource_type: &str, actions: &[String]) -> String {
    let mut p = format!("resource.starts_with({:?})", format!("{resource_type}:"));
    if !actions.is_empty() {
        let list = actions
            .iter()
            .map(|a| format!("{a:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        p.push_str(&format!(" && [{list}].contains(action)"));
    }
    p
}

/// 规则义务派生：幂等插入 + 差异刷新（含系统墓碑复活）+ 源清空即退役。
///
/// 返回 `(新建, 刷新/复活, 退役)`。语义边界：`phase = 'post'`（**不改写判决**，与既有义务语义一致）；
/// admin 软删行（`updated_by_id <> 0`）**不复活**（与 `revoke_expired_class_derivations` 同墓碑口径）。
async fn derive_rule_obligations(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    class_code: &str,
    rules: &[OrgPolicyRuleRow],
) -> Result<(i64, i64, i64), sqlx::Error> {
    let prefix = obligation_o_name_prefix(class_code);
    let mut live: Vec<String> = Vec::new();
    let mut created = 0i64;
    let mut refreshed = 0i64;

    for rule in rules {
        let Some(src) = rule.obligation.as_ref().and_then(obligation_source) else {
            continue;
        };
        let o_name = format!("{prefix}{}", rule.id);
        let actions = dedupe_keep_order(class_from_value(&rule.actions));
        let pattern = rule_event_pattern(&rule.resource_type, &actions);
        live.push(o_name.clone());

        // 1) 全新行（该 o_name 此前不存在任何行，含系统墓碑）⇒ 插入。
        created += sqlx::query(
            r#"
            INSERT INTO isahl_auth.ngac_obligation
                (o_name, event_pattern, response_ops, is_active, priority, phase,
                 created_at, updated_at)
            SELECT $1, $2, $3::jsonb, TRUE, 0, 'post', NOW(), NOW()
            WHERE NOT EXISTS (
                SELECT 1 FROM isahl_auth.ngac_obligation o WHERE o.o_name = $1
            )
            "#,
        )
        .bind(&o_name)
        .bind(&pattern)
        .bind(src)
        .execute(&mut **tx)
        .await?
        .rows_affected() as i64;

        // 2) 存活行差异刷新 + 系统墓碑复活（`updated_by_id = 0`）；admin 软删行不复活。
        refreshed += sqlx::query(
            r#"
            UPDATE isahl_auth.ngac_obligation
               SET event_pattern = $2, response_ops = $3::jsonb, is_active = TRUE,
                   phase = 'post', deleted_at = NULL,
                   updated_by_id = CASE WHEN updated_by_id = 0 THEN NULL ELSE updated_by_id END,
                   updated_at = NOW()
             WHERE o_name = $1
               AND (
                    (deleted_at IS NULL
                     AND (event_pattern IS DISTINCT FROM $2
                          OR response_ops IS DISTINCT FROM $3::jsonb))
                 OR (deleted_at IS NOT NULL AND updated_by_id = 0)
               )
            "#,
        )
        .bind(&o_name)
        .bind(&pattern)
        .bind(src)
        .execute(&mut **tx)
        .await?
        .rows_affected() as i64;
    }

    // 3) 退役：规则义务清空 / rule 消失 ⇒ 软删存活派生行（系统墓碑；admin 软删行不在面内）。
    let retired = sqlx::query(
        r#"
        UPDATE isahl_auth.ngac_obligation
           SET deleted_at = NOW(), is_active = FALSE, updated_at = NOW(), updated_by_id = 0
         WHERE left(o_name, length($1)) = $1 AND deleted_at IS NULL
           AND o_name <> ALL($2::text[])
        "#,
    )
    .bind(&prefix)
    .bind(&live)
    .execute(&mut **tx)
    .await?
    .rows_affected() as i64;

    Ok((created, refreshed, retired))
}

/// 生效窗口 SQL 片段（单源；`ngac_org::label_clearance_denies` 白名单经 `concat!` + 本宏引用）。
/// 语义：半开区间 `[from, until)`，NULL 端点 = 无界，时钟 = DB `NOW()`。
/// 形态 = `macro_rules!` ⇒ 消费方 `concat!` 可在编译期展开（零运行时拼接、零 `format!` 插值）。
#[macro_export]
macro_rules! org_policy_window_sql {
    () => {
        "(effective_from IS NULL OR effective_from <= NOW()) \
         AND (effective_until IS NULL OR effective_until > NOW())"
    };
}

/// active class + active rules 读侧唯一 SQL（投影与派生共用；缺 class → RowNotFound）。
async fn load_active_class_rules(
    conn: &mut sqlx::PgConnection,
    class_id: i64,
) -> Result<(OrgPolicyClassRow, Vec<OrgPolicyRuleRow>), sqlx::Error> {
    // SQL 由编译期宏展开（零运行时拼接、零 format! 插值——dynamic-table-name 门禁合规）
    let class = sqlx::query_as::<_, OrgPolicyClassRow>(concat!(
        "SELECT code, scope, ua_template, label_code, prohibition_template \
             FROM isahl_auth.org_policy_class \
             WHERE id = $1 AND state = 'active' AND deleted_at IS NULL AND ",
        ngac_host::org_policy_window_sql!()
    ))
    .bind(class_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(sqlx::Error::RowNotFound)?;
    let rules = sqlx::query_as::<_, OrgPolicyRuleRow>(
        "SELECT id, resource_type, actions, label_code, condition, obligation \
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

// ── 窗口失效类派生物撤销（revoke-expired-org-policy-derivations）──────────────

/// 撤销统计。
#[derive(Debug, Default, Clone, Copy)]
pub struct RevocationStats {
    /// 本次软删了派生 association 的失效类数（零行类不计）。
    pub classes_revoked: usize,
    /// 软删的派生 association 行数（重复调用为 0）。
    pub associations_revoked: i64,
    /// 软删的派生 `ngac_obligation` 行数（规则义务派生，重复调用为 0）。
    pub obligations_revoked: i64,
    /// 软删的派生 `ngac_prohibition` 行数（负向模板派生，重复调用为 0）。
    pub prohibitions_revoked: i64,
}

/// 窗口失效类的派生物撤销（幂等扫帚）。
///
/// 语义闭环：窗口外的 active class **视同未生效**（不派生、不构成白名单），但既往
/// 派生的 association 仍持权（「重派生不自动撤销」的差额）——本函数在进程启动时
/// 收敛该差额：对**失效类**（`state <> 'active'` 或窗口外）软删其派生 UA 名下、
/// org_policy 派生集合 OA 上的存活 association，并**仅对有撤销量的类**写策略审计行
/// （`action='revoke_expired'`，actor=0 系统通道；admin 面写者 = SSO `audit_writer`）。
///
/// - **UA 行不删**：`position:{code}` 与认知链物化行同名同策略类（共享名域，
///   `NGAC_SPEC §2.2.3`）；clearance 不动（主体属性归重派生按 canonical 差异刷新）；
/// - **幂等**：软删后重跑零行零审计；**撤销时点 = 扫帚运行时刻**（窗口语义声明性
///   `[from, until)` 半开，DB `NOW()`；启动期收敛，非到期即撤的实时语义）；
/// - **触发面**：Gateway host 启动（与 `ensure_org_resource_coverage` 同点
///   fire-and-forget）；类重新进入窗口 ⇒ 下次 `derive_from_class` 幂等重建。
pub async fn revoke_expired_class_derivations(
    pool: &PgPool,
) -> Result<RevocationStats, sqlx::Error> {
    let mut tx = pool.begin().await?;
    // 失效类 = 未删 && (state<>'active' OR 窗口外)——窗口谓词与读侧单源同宏
    let expired: Vec<(i64, String)> = sqlx::query_as(concat!(
        "SELECT id, code FROM isahl_auth.org_policy_class \
         WHERE deleted_at IS NULL AND (state <> 'active' OR NOT (",
        ngac_host::org_policy_window_sql!(),
        ")) ORDER BY id"
    ))
    .fetch_all(&mut *tx)
    .await?;

    let mut stats = RevocationStats::default();
    for (class_id, code) in &expired {
        let revoked = sqlx::query(
            r#"UPDATE isahl_auth.ngac_association a
               SET deleted_at = NOW(), updated_by_id = 0
               FROM isahl_auth.ngac_user_attribute ua,
                    isahl_auth.ngac_object_attribute oa
               WHERE ua.deleted_at IS NULL
                 AND ua.property ->> 'source_kind' = 'org_policy_class'
                 AND ua.property ->> 'source_code' = $1
                 AND a.fk_user_attribute = ua.id AND a.deleted_at IS NULL
                 AND oa.id = a.fk_object_attribute AND oa.deleted_at IS NULL
                 AND oa.property ->> 'derived_from' = 'org_policy_class'"#,
        )
        .bind(code)
        .execute(&mut *tx)
        .await?
        .rows_affected() as i64;

        // 规则义务派生的同批退役（`org_policy:{code}:{rule_id}` 前缀可精确界位；
        // `left(...)=` 而非 `LIKE`——避免前缀内 `_` 被当作通配符）。
        let ob_revoked = sqlx::query(
            r#"UPDATE isahl_auth.ngac_obligation
               SET deleted_at = NOW(), is_active = FALSE, updated_at = NOW(), updated_by_id = 0
               WHERE left(o_name, length($1)) = $1 AND deleted_at IS NULL"#,
        )
        .bind(obligation_o_name_prefix(code))
        .execute(&mut *tx)
        .await?
        .rows_affected() as i64;
        stats.obligations_revoked += ob_revoked;

        // 规则条件/负向模板派生的 prohibition（`org_policy:{code}:deny-*`）同批退役。
        let pro_revoked = sqlx::query(
            r#"UPDATE isahl_auth.ngac_prohibition
               SET deleted_at = NOW(), is_active = FALSE, updated_at = NOW(), updated_by_id = 0
               WHERE left(o_name, length($1)) = $1 AND deleted_at IS NULL"#,
        )
        .bind(obligation_o_name_prefix(code))
        .execute(&mut *tx)
        .await?
        .rows_affected() as i64;
        stats.prohibitions_revoked += pro_revoked;

        if revoked > 0 || ob_revoked > 0 || pro_revoked > 0 {
            sqlx::query(
                r#"INSERT INTO isahl_auth.ngac_policy_audit_log
                     (action, entity_type, entity_id, old_values, new_values, fk_user)
                   VALUES ('revoke_expired', 'org_policy_class', $1, NULL,
                           jsonb_build_object('associations_revoked', $2::bigint,
                                              'obligations_revoked', $4::bigint,
                                              'prohibitions_revoked', $5::bigint,
                                              'code', $3), 0)"#,
            )
            .bind(class_id)
            .bind(revoked)
            .bind(code)
            .bind(ob_revoked)
            .bind(pro_revoked)
            .execute(&mut *tx)
            .await?;
            stats.classes_revoked += 1;
            stats.associations_revoked += revoked;
        }
    }
    tx.commit().await?;
    Ok(stats)
}
