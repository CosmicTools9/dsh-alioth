//! 决策输入 SQL 单一定义面（openspec change `extract-ngac-decision-host`）。
//!
//! 自 `common::{ngac_org, permissions, ngac_grants, ngac_policy}` 迁入——资源作用域
//! 决策所需的 SQL / 谓词 / 常量。`common` 及其下游 MUST 引用 `ngac_host::…`，
//! MUST NOT 复制（门禁 `check-ngac-decision-chain-single-source.ts`）。
//!
//! 边界：**不含**用户级、与资源无关的 `effective_ua_ids` 谓词函数（其 DDL 与其
//! 认知/委托 CTE 常量仍归 `common::permissions` / `common::ngac_org`，本 crate 以
//! 函数名直接调用）；亦不含组织面（heal / holders / member 解析——留 `common::ngac_org`）。

use sqlx::{AssertSqlSafe, PgPool};

// ── admin 判据（自 common::permissions 迁入）──────────────────────────────────

/// admin UA 名 —— **唯一名字集**（不保留 `superadmin`）。
///
/// 增删别名 MUST 先改规约（`openspec/specs/ngac-admin-predicate`），MUST NOT 由单个
/// 消费方或前端自行扩展；门禁 `scripts/check/check-admin-predicate-single-source.ts`
/// 拒绝白名单外的 `'admin'` / `"admin"` / `superadmin` 字面量。
pub const ADMIN_UA_NAME: &str = "admin";

/// 内存版 admin 判定（**唯一实现**）：输入 = 已解析的有效 UA 名集合。
pub fn attr_names_include_admin<'a, I>(names: I) -> bool
where
    I: IntoIterator<Item = &'a str>,
{
    names.into_iter().any(|n| n == ADMIN_UA_NAME)
}

// ── 资源级授权 right 集（自 common::ngac_grants 迁入）─────────────────────────

/// 创建者对资源行持有的 right 集（`derive-creator-grants-from-lifecycle`；不含 approve）。
pub const RIGHTS_CREATOR: &[&str] = &["read", "write", "delete", "update", "create"];
/// 管理员全量 right 集（前者 + approve）。
pub const RIGHTS_ADMIN: &[&str] = &["read", "write", "delete", "update", "create", "approve"];

// ── 生效窗口 SQL 片段（自 common::ngac_policy 迁入）───────────────────────────

/// 生效窗口 SQL 片段（单源）。语义：半开区间 `[from, until)`，NULL 端点 = 无界，
/// 时钟 = DB `NOW()`。形态 = `macro_rules!` ⇒ 消费方 `concat!` 可在编译期展开。
#[macro_export]
macro_rules! org_policy_window_sql {
    () => {
        "(effective_from IS NULL OR effective_from <= NOW()) \
         AND (effective_until IS NULL OR effective_until > NOW())"
    };
}

// ── 属权派生 CTE（自 common::ngac_org 迁入）─────────────────────────────────

/// 资源行单次取值（`isahl.zc_id_lifecycle` 按 id 取行；**视图规划成本收敛的唯一实现**）。
///
/// 该视图为 ~1895 分支 UNION，每次引用都要枚举规划整个视图（单次 ≈ 0.45–1.9s，
/// 执行仅 ms 级）。凡需按 `$3`（资源行 id）读该行的语句 MUST 以本 CTE 单次取值再复用
/// （`AS MATERIALIZED` 防内联重放），MUST NOT 直接多引用 `isahl.zc_id_lifecycle`。
pub const OWN_RES_CTE: &str = r#"
own_res AS MATERIALIZED (
    SELECT created_by_id, deleted_at, ak_benefit_user, ak_permit_user, ak_access_user
    FROM isahl.zc_id_lifecycle
    WHERE id = $3
)"#;

/// 属权派生 CTE 的分支主体（与 [`ownership_cte`] 装配使用；直接消费请调 [`ownership_cte`]）。
const OWNERSHIP_BRANCHES: &str = r#"
,
ownership_ua AS (
    SELECT op.owner_attr_id AS ua_id
    FROM isahl_auth.ngac_ownership_policy op, own_res r
    WHERE op.resource_type = $2
      AND r.created_by_id = $1 AND op.enabled = TRUE
    UNION
    SELECT op.benefit_attr_id
    FROM isahl_auth.ngac_ownership_policy op, own_res r
    WHERE op.resource_type = $2
      AND $1 = ANY(r.ak_benefit_user) AND op.enabled = TRUE
    UNION
    SELECT op.permit_attr_id
    FROM isahl_auth.ngac_ownership_policy op, own_res r
    WHERE op.resource_type = $2
      AND $1 = ANY(r.ak_permit_user) AND op.enabled = TRUE
    UNION
    SELECT op.access_attr_id
    FROM isahl_auth.ngac_ownership_policy op, own_res r
    WHERE op.resource_type = $2
      AND $1 = ANY(r.ak_access_user) AND op.enabled = TRUE
)
"#;

/// 属权派生 CTE（M16 判定链同源）——**资源受限**的 UA 派生。
///
/// 资源行为 `isahl.zc_id_lifecycle` 实体（`$3` = 行 id），属权字段（NGAC_SPEC §2.2 属权四路）
/// 命中 `$1`（用户）时，并入 `ngac_ownership_policy` 为该 `resource_type`（`$2`）声明的属性 id。
/// `op.enabled = TRUE` 才生效；四路 UNION（去重）。
///
/// **为什么是装配函数而非消费方内联**：属权派生不属于通用有效 UA 集
/// （`isahl_auth.effective_ua_ids` 是用户级、与资源无关），必须由**有资源上下文**的判定点叠加；
/// Framework 与 SSO 两条判定链 MUST 引用同一装配，**禁止复制 SQL**。
///
/// 参数约定：`$1` = fk_user，`$2` = resource_type，`$3` = 资源行 id。
///
/// 用法一（单一资源，decide）：`WITH RECURSIVE {ownership_cte()}, user_attrs AS (… )`。
/// 用法二（逐行相关子查询，pdp/list 候选超集）：把 `$2`/`$3` 替换为外层行表达式后嵌入 LATERAL。
pub fn ownership_cte() -> String {
    format!("{OWN_RES_CTE}{OWNERSHIP_BRANCHES}")
}

// ── 创建者授权派生（自 common::ngac_org 迁入）───────────────────────────────

/// 创建者授权派生标记（`derive-creator-grants-from-lifecycle`）：创建者对资源行持有的
/// [`RIGHTS_CREATOR`] 这一事实**不物化**，由判定时刻计算。
pub const CREATOR_GRANT_DOC: &str = "creator-derived";

/// 创建者派生授权的**存在性判定形**（候选清单注入用；纯布尔表达式）。
///
/// **前置条件**：消费方 MUST 已拼装 [`ownership_cte`]（提供 `own_res`，`$3` 已替换为资源行
/// 表达式）且本片段置于 `own_res` 可见的同层作用域（`LEFT JOIN LATERAL (…)` 的 SELECT 列表位）。
/// MUST NOT 置于外层 WHERE（外层不可对 CTE 相关绑定）。
pub const CREATOR_DERIVED_EXISTS_OA: &str = r#"
              EXISTS (
                  SELECT 1
                  FROM own_res lr
                  JOIN isahl_auth.ngac_user_rr_attribute crr
                    ON crr.fk_user = lr.created_by_id
                  WHERE lr.deleted_at IS NULL
                    AND crr.deleted_at IS NULL
                    AND (crr.expires_at IS NULL OR crr.expires_at > NOW())
                    AND crr.fk_user_attribute IN (SELECT c.ua_id FROM user_attrs c)
              )"#;

/// 创建者派生授权的**实例锚点 × 创建者活 UA 集**（matrix 单元格 / 图快照的内联消费用）。
///
/// 返回 `fk_resource <> 0` 锚点 OA id → 创建者当前有效 rr 指向的 UA id 集；
/// 与 [`creator_derived_permits`] 同源（同一时效口径）。
pub async fn creator_ua_ids_by_instance_oa(
    pool: &PgPool,
) -> sqlx::Result<std::collections::HashMap<i64, Vec<i64>>> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        r#"SELECT oa.id, rr.fk_user_attribute
           FROM isahl_auth.ngac_object_attribute oa
           JOIN isahl.zc_id_lifecycle lr ON lr.id = oa.fk_resource
           JOIN isahl_auth.ngac_user_rr_attribute rr ON rr.fk_user = lr.created_by_id
           WHERE oa.fk_resource <> 0
             AND oa.deleted_at IS NULL
             AND lr.deleted_at IS NULL
             AND rr.deleted_at IS NULL
             AND (rr.expires_at IS NULL OR rr.expires_at > NOW())"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .fold(std::collections::HashMap::new(), |mut m, (oa, ua)| {
            m.entry(oa).or_default().push(ua);
            m
        }))
}

/// 创建者派生授权的**反向资源 id 集**（清单路径用：pip/list 的候选并入）。
///
/// 与 [`creator_derived_permits`] 同源（同一时效口径）；返回「创建者与判定用户共享有效 UA」的
/// 全部 lifecycle 行 id。拆两段——① 先取创建者候选（`effective_ua_ids` 单源闭包），空集短路；
/// ② 按 `created_by_id = ANY($1)` 定向取行。
pub async fn creator_derived_resource_ids(pool: &PgPool, user_id: i64) -> sqlx::Result<Vec<i64>> {
    let creators: Vec<i64> = sqlx::query_scalar(
        r#"SELECT DISTINCT crr.fk_user
           FROM isahl_auth.ngac_user_rr_attribute crr
          WHERE crr.deleted_at IS NULL
            AND (crr.expires_at IS NULL OR crr.expires_at > NOW())
            AND crr.fk_user_attribute IN (SELECT ua_id FROM isahl_auth.effective_ua_ids($1))"#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    if creators.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(i64,)> = sqlx::query_as(
        r#"SELECT DISTINCT lr.id
           FROM isahl.zc_id_lifecycle lr
          WHERE lr.deleted_at IS NULL
            AND lr.created_by_id = ANY($1)"#,
    )
    .bind(&creators)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// 创建者派生授权的**独立标量谓词**（无候选管道的判定位用：matrix cell / review 行 / explain 辅证）。
///
/// 语义与 [`CREATOR_DERIVED_EXISTS_OA`] 同源（同一时效口径、同一 rights 字面）；`operation`
/// 不在 [`RIGHTS_CREATOR`] 内 ⇒ false。UA 闭包经 `isahl_auth.effective_ua_ids($1)`（PIP 单源）。
pub async fn creator_derived_permits(
    pool: &PgPool,
    user_id: i64,
    resource_id: i64,
    operation: &str,
) -> sqlx::Result<bool> {
    let rights = RIGHTS_CREATOR
        .iter()
        .map(|r| format!("'{}'", r))
        .collect::<Vec<_>>()
        .join(", ");
    let own_res = OWN_RES_CTE.replace("$3", "$2");
    let sql = format!(
        r#"WITH {own_res}
        SELECT EXISTS(
            SELECT 1
            FROM own_res lr
            JOIN isahl_auth.ngac_user_rr_attribute crr
              ON crr.fk_user = lr.created_by_id
            WHERE lr.deleted_at IS NULL
              AND crr.deleted_at IS NULL
              AND (crr.expires_at IS NULL OR crr.expires_at > NOW())
              AND crr.fk_user_attribute IN (SELECT ua_id FROM isahl_auth.effective_ua_ids($1))
        ) AND $3 = ANY(ARRAY[{rights}]::text[])"#,
        own_res = own_res,
        rights = rights
    );
    sqlx::query_scalar::<_, bool>(AssertSqlSafe(sql.as_str()))
        .bind(user_id)
        .bind(resource_id)
        .bind(operation)
        .fetch_one(pool)
        .await
}

// ── 分级上限谓词（自 common::ngac_org 迁入）─────────────────────────────────
/// 分级（密级）上限判定（单资源路径）：白名单资源类型上主体 clearance 低于资源标签 ⇒ 拒绝。
///
/// SQL 由编译期宏展开（零运行时拼接、零 `format!` 插值——dynamic-table-name 门禁合规）。
pub async fn label_clearance_denies(
    pool: &PgPool,
    resource_type: &str,
    fk_resource: i64,
    fk_user: i64,
) -> sqlx::Result<bool> {
    sqlx::query_scalar::<_, bool>(concat!(
        r#"
            WITH whitelisted AS (
                SELECT 1 FROM isahl_auth.org_policy_class c
                WHERE c.deleted_at IS NULL AND c.state = 'active' AND "#,
        crate::org_policy_window_sql!(),
        r#"
                  AND c.scope IS NOT NULL
                  AND c.scope -> 'resource_types' @> to_jsonb($1::text)
                LIMIT 1
            ),
            res_label AS (
                SELECT l.rank
                FROM isahl_auth.ngac_object_attribute oa
                JOIN isahl_auth.org_policy_label l
                  ON l.code = (oa.property ->> 'label') AND l.deleted_at IS NULL AND l.is_active
                WHERE oa.resource_type = $1 AND oa.fk_resource = $2 AND oa.deleted_at IS NULL
                LIMIT 1
            ),
            subj_clear AS (
                SELECT MAX(l.rank) AS rank
                FROM isahl_auth.ngac_user_attribute ua
                JOIN isahl_auth.org_policy_label l
                  ON l.code = (ua.property ->> 'clearance') AND l.deleted_at IS NULL AND l.is_active
                WHERE ua.deleted_at IS NULL
                  AND ua.id IN (SELECT ua_id FROM isahl_auth.effective_ua_ids($3))
            )
            SELECT (
                EXISTS(SELECT 1 FROM whitelisted)
                AND EXISTS(SELECT 1 FROM res_label)
                AND COALESCE((SELECT rank FROM subj_clear), 0) < (SELECT rank FROM res_label)
            )
            "#
    ))
    .bind(resource_type)
    .bind(fk_resource)
    .bind(fk_user)
    .fetch_one(pool)
    .await
}

/// 分级谓词比较规则的**纯 Rust 唯一实现**（供批量路径预取数据后逐行复用；单资源路径用
/// [`label_clearance_denies`] 的 SQL 等价式）。规则：不在白名单或资源无标签 ⇒ **不介入**
/// （false）；否则 `subj < 资源标签` ⇒ 拒绝（缺 clearance 视作 0 ⇒ 拒绝）。
pub fn label_clearance_denies_rule(
    whitelisted: bool,
    res_label_rank: Option<i32>,
    subj_rank: Option<i32>,
) -> bool {
    match (whitelisted, res_label_rank) {
        (true, Some(label)) => subj_rank.unwrap_or(0) < label,
        _ => false,
    }
}

// ── 策略图装载 + OA 闭包（自 common::ngac_org 迁入）─────────────────────────

/// association 装载行（[`load_policy_graph`] 内部 FromRow 面）：末位 = `expires_at`（NULL = 永不过期）。
type AssocRow = (
    i64,
    i64,
    i64,
    Vec<i64>,
    Option<serde_json::Value>,
    Option<chrono::DateTime<chrono::Utc>>,
);
/// prohibition 装载行（同上）。DB 真值（`describe-table isahl_auth.ngac_prohibition`）：
/// `fk_user_attribute`/`fk_object_attribute`/`is_active` **可空** ⇒ 按 `Option` 解码。
type ProhRow = (
    i64,
    String,
    Option<i64>,
    Option<i64>,
    Vec<i64>,
    Option<bool>,
    Option<serde_json::Value>,
);

/// 装载策略图（唯一实现）——内容 = association / access_right / prohibition（软删过滤）+
/// 策略类名登记（`PolicyGraph::bootstrap_empty` 判据输入）。失败返回 `Err`，由调用方按
/// fail-closed 语义处理。
pub async fn load_policy_graph(pool: &PgPool) -> sqlx::Result<ngac_decision::PolicyGraph> {
    Ok(load_policy_graph_with_expiry(pool).await?.0)
}

/// 策略图装载 + **最早到期时刻**（快照到期重载用）。
///
/// 到期语义（change `add-ngac-grant-revocation-and-expiry`）：仅
/// `expires_at IS NULL OR expires_at > NOW()` 的 association 入图；返回值为入图行中最早的
/// `expires_at`（`None` = 无到期行）——宿主据此在到期时点重载快照，使到期即时生效。
pub async fn load_policy_graph_with_expiry(
    pool: &PgPool,
) -> sqlx::Result<(
    ngac_decision::PolicyGraph,
    Option<chrono::DateTime<chrono::Utc>>,
)> {
    let pg = ngac_decision::PolicyGraph::new();

    let associations: Vec<AssocRow> = sqlx::query_as(
        "SELECT id, fk_user_attribute, fk_object_attribute, ak_access_rights, conditions, expires_at \
             FROM isahl_auth.ngac_association \
            WHERE deleted_at IS NULL AND (expires_at IS NULL OR expires_at > NOW())",
    )
    .fetch_all(pool)
    .await?;
    let mut next_expiry: Option<chrono::DateTime<chrono::Utc>> = None;
    for (id, ua, oa, rights, conditions, expires_at) in associations {
        if let Some(exp) = expires_at {
            next_expiry = Some(match next_expiry {
                Some(cur) if cur <= exp => cur,
                _ => exp,
            });
        }
        pg.add_association(ngac_decision::Association {
            id,
            fk_user_attribute: ua,
            fk_object_attribute: oa,
            ak_access_rights: rights,
            conditions,
        });
    }

    let rights: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, o_name FROM isahl_auth.ngac_access_right")
            .fetch_all(pool)
            .await?;
    for (id, o_name) in rights {
        pg.add_access_right(ngac_decision::AccessRight {
            id,
            o_name,
            applicable_types: Vec::new(),
            is_system: false,
        });
    }

    let prohibitions: Vec<ProhRow> = sqlx::query_as(
        "SELECT id, o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active, conditions \
         FROM isahl_auth.ngac_prohibition WHERE deleted_at IS NULL",
    )
    .fetch_all(pool)
    .await?;
    for (id, o_name, ua, oa, rights, is_active, conditions) in prohibitions {
        // DB 允许 UA/OA 侧为 NULL（§3.1）：NULL 侧无法等于任何具体 id ⇒ 永不匹配任何
        // (UA, OA) 对（跳过并告警，MUST NOT 因单行 NULL 而 fail-closed 整图加载失败）。
        let (Some(ua), Some(oa)) = (ua, oa) else {
            log::warn!(
                "load_policy_graph: prohibition id={id} has NULL UA/OA side — skipped (never matches any pair)"
            );
            continue;
        };
        pg.add_prohibition(ngac_decision::Prohibition {
            id,
            o_name,
            fk_user_attribute: ua,
            fk_object_attribute: oa,
            ak_access_rights: rights,
            // `is_active` NULL ⇒ 未启用（决策侧仅 `is_active = true` 参与）。
            is_active: is_active.unwrap_or(false),
            conditions,
        });
    }

    let classes: Vec<(String, i64)> =
        sqlx::query_as("SELECT o_name, id FROM isahl_auth.ngac_policy_class")
            .fetch_all(pool)
            .await?;
    for (name, id) in classes {
        pg.add_policy_class(&name, id);
    }

    // 策略类分区登记（NGAC）：UA/OA → 策略类，供求值对级短路。
    let ua_pcs: Vec<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, fk_policy_class FROM isahl_auth.ngac_user_attribute WHERE deleted_at IS NULL",
    )
    .fetch_all(pool)
    .await?;
    for (id, pc) in ua_pcs {
        if let Some(pc) = pc {
            pg.add_user_attribute_pc(id, pc);
        }
    }
    let oa_pcs: Vec<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, fk_policy_class FROM isahl_auth.ngac_object_attribute WHERE deleted_at IS NULL",
    )
    .fetch_all(pool)
    .await?;
    for (id, pc) in oa_pcs {
        if let Some(pc) = pc {
            pg.add_object_attribute_pc(id, pc);
        }
    }

    Ok((pg, next_expiry))
}

/// 目标资源的 OA 闭包（**装配唯一实现**）：实例行 OA（`resource_type` + `fk_resource`，
/// 集合级传 `0`）∪ 通配 `*` OA ∪ ancestor 展开（深度 <10，`deleted_at` 过滤），返回
/// `(id, o_name)` 行（名称供 conditions v2 `object_attr_in` 上下文）。
pub async fn oa_closure_rows(
    pool: &PgPool,
    resource_type: &str,
    fk_resource: i64,
) -> sqlx::Result<Vec<(i64, String)>> {
    sqlx::query_as(
        "WITH RECURSIVE oa AS ( \
             SELECT id, 0 AS depth FROM isahl_auth.ngac_object_attribute \
             WHERE resource_type = $1 AND fk_resource = $2 AND deleted_at IS NULL \
             UNION \
             SELECT id, 0 FROM isahl_auth.ngac_object_attribute \
             WHERE resource_type = '*' AND deleted_at IS NULL \
             UNION ALL \
             SELECT unnest(o.ancestor_ids)::BIGINT, c.depth + 1 \
             FROM isahl_auth.ngac_object_attribute o \
             JOIN oa c ON o.id = c.id \
             WHERE c.depth < 10 AND o.deleted_at IS NULL \
         ) \
         SELECT DISTINCT a.id, a.o_name \
         FROM oa JOIN isahl_auth.ngac_object_attribute a ON a.id = oa.id AND a.deleted_at IS NULL",
    )
    .bind(resource_type)
    .bind(fk_resource)
    .fetch_all(pool)
    .await
}

/// 用户资源作用域有效 UA 闭包行（`id + 名称`）。**装配唯一实现**（门禁
/// `check-sql-literal-single-source` 单源）：基座 = PIP 单源 `isahl_auth.effective_ua_ids($1)`
/// （直派 ∪ 认知派生 ∪ 委托派生 ∪ 祖先闭包 + 时效；其 DDL 内嵌 `common::ngac_org` 的
/// cognition/delegation CTE）∪ **属权派生**（[`ownership_cte`]，资源受限）∪ 再闭包。
/// SQL MUST NOT 复制闭包/时效/属权语义。
pub async fn ua_closure_rows(
    pool: &PgPool,
    user_id: i64,
    resource_type: &str,
    fk_resource: i64,
) -> sqlx::Result<Vec<(i64, String)>> {
    let sql = format!(
        "WITH RECURSIVE
        {ownership},
        user_attrs AS (
            SELECT ua_id, 0 as depth FROM isahl_auth.effective_ua_ids($1)
            UNION
            SELECT ua_id, 0 FROM ownership_ua
            UNION ALL
            SELECT unnest(ua.ancestor_ids)::BIGINT as ua_id, depth + 1
            FROM isahl_auth.ngac_user_attribute ua
            INNER JOIN user_attrs AS ua_cte ON ua.id = ua_cte.ua_id
            WHERE ua_cte.depth < 10 AND ua.deleted_at IS NULL
        )
        SELECT DISTINCT u.ua_id, x.o_name
        FROM user_attrs u
        JOIN isahl_auth.ngac_user_attribute x ON x.id = u.ua_id AND x.deleted_at IS NULL",
        ownership = ownership_cte(),
    );
    sqlx::query_as::<_, (i64, String)>(AssertSqlSafe(sql.as_str()))
        .bind(user_id)
        .bind(resource_type)
        .bind(fk_resource)
        .fetch_all(pool)
        .await
}

// ── Rhai 条件求值器（`{"rhai": …}` 第三形态的宿主侧唯一实现）────────────────────

/// 构建 `conditions = {"rhai": "<expr>"}` 的求值器（**宿主侧唯一实现**；全部消费面 MUST 同源注入，
/// 否则 introspection 面与真决策分叉——§2.7 同族教训）。
///
/// 变量（与授权写径的白名单键集一致）= `user_id` / `resource` / `action` /
/// `subject_attributes`（主体有效 UA 名）/ `object_attributes`（目标 OA 闭包名）。
/// **不含** `decision` / `permitted`——条件在判决终态前求值，引用即求值失败 ⇒ 不满足（fail-closed）；
/// 授权写径对 `condition` / `prohibition_template` 使用**收窄键集**
/// （`SSO/backend/src/ngac/org_policy.rs::ORG_POLICY_CONDITION_EXPR_KEYS`，与本文变量集逐名一致），
/// 故正常数据不会引用这两个变量；`obligation` 列（义务求值上下文含 `decision`）不受该收窄影响。
///
/// 求值失败（语法/运行期/未注册函数）⇒ `false` + warn（**不满足**）。
/// 零 `rhai` 条件时求值器**从不被调用**（核心仅在 `conditions` 含 `rhai` 键时回调）⇒ 热路径零开销。
pub fn rhai_condition_evaluator(
    user_id: i64,
    resource: String,
    action: String,
) -> ngac_decision::RhaiCondEval {
    // rhai `Engine` 含非 `Sync` 内部表（`Box<CustomSyntax>` 族）⇒ MUST NOT 入 `static`；
    // 用**线程局部**实例（每线程一次构建、跨调用复用），与「按次构建」的
    // `ngac/obligation.rs` 口径等价而更省。求值器本身仍 `Send + Sync`（只捕获可发送标量）。
    thread_local! {
        static RHAI_ENGINE: runtime_engine::RhaiExpressionEngine =
            runtime_engine::RhaiExpressionEngine::new();
    }
    std::sync::Arc::new(
        move |src: &str, ctx: &ngac_decision::ConditionContext| -> bool {
            let mut vars: std::collections::HashMap<String, serde_json::Value> =
                std::collections::HashMap::new();
            vars.insert("user_id".to_string(), serde_json::Value::from(user_id));
            vars.insert(
                "resource".to_string(),
                serde_json::Value::from(resource.clone()),
            );
            vars.insert(
                "action".to_string(),
                serde_json::Value::from(action.clone()),
            );
            vars.insert(
                "subject_attributes".to_string(),
                serde_json::Value::from(ctx.user_ua_names.clone()),
            );
            vars.insert(
                "object_attributes".to_string(),
                serde_json::Value::from(ctx.oa_closure_names.clone()),
            );
            let evaluated = RHAI_ENGINE.with(|engine| engine.evaluate_bool(src, &vars));
            match evaluated {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("ngac rhai condition 求值失败（按不满足）：{e}");
                    false
                }
            }
        },
    )
}

// ── `conditions` 写径白名单校验（tasks 1.5：静默放行面收口）────────────────────

/// 校验 `conditions` JSONB 的键集（**写径唯一实现**；键集真源 = `ngac_decision::CONDITION_KEYS`）。
///
/// 语义：`null` / 空对象 ⇒ 允许（等价无条件）；对象含白名单外键 ⇒ `Err(键名)`。
/// 非对象（数组/标量）⇒ `Err`（核心按失败封闭处理，写径 MUST 更早拒绝以便调用方回 400）。
///
/// **为何写径也要拒**：核心对未知键 fail-closed（该策略不满足）⇒ 若写径接受，则「写入成功但
/// 永不生效」= 静默面。写径拒绝把失败前移到请求方可见处（两侧同源键集，MUST NOT 各自硬编码）。
pub fn validate_condition_keys(conditions: &serde_json::Value) -> Result<(), String> {
    if conditions.is_null() {
        return Ok(());
    }
    let obj = match conditions.as_object() {
        Some(o) => o,
        None => return Err("conditions 必须是 JSON 对象或 null".to_string()),
    };
    if let Some(k) = obj
        .keys()
        .find(|k| !ngac_decision::CONDITION_KEYS.contains(&k.as_str()))
    {
        return Err(format!(
            "conditions 含未知键 `{k}`（合法键：{}）",
            ngac_decision::CONDITION_KEYS.join(" / ")
        ));
    }
    Ok(())
}

// ── 决策审计抑制判据（wire-org-policy-declaration-fields）───────────────────

/// `org_policy_class.audit_required` 接线：该类资源类型的决策是否**抑制审计事件**。
///
/// 语义（规约 `ngac-admin :: org-policy-declaration-field-wiring`）：存在 **active、未删、在生效窗口内**
/// 的 `org_policy_class`，其 `scope->'resource_types'` 覆盖目标资源类型且 `audit_required = FALSE`
/// ⇒ 抑制该类资源类型的决策审计（判决不变）。唯一调用点 = SSO `ngac/pdp/audit.rs::audit_decision`。
///
/// **fail-safe**：查询失败（表/列缺失、连接错误）⇒ 返回 `false`（照常审计）+ warn——
/// 附加约束 MUST NOT 造成审计静默丢失。
pub async fn audit_suppressed_for_resource_type(pool: &PgPool, resource_type: &str) -> bool {
    // SQL 由编译期宏展开（零运行时拼接、零 `format!` 插值——dynamic-table-name 门禁合规）。
    match sqlx::query_scalar::<_, bool>(concat!(
        "SELECT EXISTS(SELECT 1 FROM isahl_auth.org_policy_class c \
          WHERE c.deleted_at IS NULL AND c.state = 'active' AND c.audit_required = FALSE \
            AND c.scope IS NOT NULL AND c.scope -> 'resource_types' @> to_jsonb($1::text) AND ",
        crate::org_policy_window_sql!(),
        ")"
    ))
    .bind(resource_type)
    .fetch_one(pool)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            log::warn!("audit_required 判据查询失败（照常审计）：{e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ngac_decision::ConditionContext;

    /// 求值器实现口径（切片 B）：变量取自参数 + 条件上下文；失败 ⇒ false（fail-closed）。
    #[test]
    fn rhai_condition_evaluator_uses_documented_variables() {
        let ctx = ConditionContext {
            now: chrono::Utc::now(),
            user_ua_names: vec!["view:VIEW-SUPP".to_string()],
            oa_closure_names: vec!["invoice-collection".to_string()],
            rhai_eval: None,
        };
        let ev = rhai_condition_evaluator(7, "invoice:1".to_string(), "read".to_string());
        // action / resource / user_id / 闭包名四个文档化变量逐个命中
        assert!(ev("action == \"read\"", &ctx));
        assert!(ev("resource == \"invoice:1\"", &ctx));
        assert!(ev("user_id == 7", &ctx));
        assert!(
            ev("subject_attributes.contains(\"view:VIEW-SUPP\")", &ctx),
            "subject_attributes = 主体有效 UA 名"
        );
        assert!(
            ev("object_attributes.contains(\"invoice-collection\")", &ctx),
            "object_attributes = 目标 OA 闭包名"
        );
        // 未命中 ⇒ false（不是错误）
        assert!(!ev("action == \"delete\"", &ctx));
        // 引判决期不可知变量 ⇒ Rhai 未注册标识符 ⇒ 求值失败 ⇒ false（fail-closed）
        assert!(
            !ev("decision == \"permit\"", &ctx),
            "decision 不在求值上下文（判决终态前求值）⇒ MUST 不满足"
        );
        // 语法错误 ⇒ false（fail-closed，不 panic）
        assert!(!ev("this is not rhai", &ctx));
    }

    /// 写径键集校验：合法五键放行；未知键 / 非对象拒绝；键集与核心常量同源。
    #[test]
    fn condition_key_validation_rejects_unknown_keys() {
        // 合法：null / 空对象 / 五个白名单键（含组合）
        assert!(validate_condition_keys(&serde_json::Value::Null).is_ok());
        assert!(validate_condition_keys(&serde_json::json!({})).is_ok());
        for k in ngac_decision::CONDITION_KEYS {
            let v = match k {
                "not_before" | "not_after" => serde_json::json!({ k: "2026-01-01T00:00:00Z" }),
                "rhai" => serde_json::json!({ k: "action == \"read\"" }),
                _ => serde_json::json!({ k: ["ua-1"] }),
            };
            assert!(validate_condition_keys(&v).is_ok(), "{k} 应放行");
        }
        assert!(validate_condition_keys(&serde_json::json!({
            "not_before": "2026-01-01T00:00:00Z", "rhai": "true"
        }))
        .is_ok());
        // 未知键（含拼写错误）⇒ Err，且报错点出该键名
        let err = validate_condition_keys(&serde_json::json!({ "bogus": 1 })).unwrap_err();
        assert!(err.contains("bogus"), "报错 MUST 点出未知键：{err}");
        assert!(validate_condition_keys(&serde_json::json!({ "not_befor": "x" })).is_err());
        assert!(validate_condition_keys(&serde_json::json!({
            "not_before": "2026-01-01T00:00:00Z", "nope": true
        }))
        .is_err());
        // 非对象 ⇒ Err（数组 / 标量）
        assert!(validate_condition_keys(&serde_json::json!(["a"])).is_err());
        assert!(validate_condition_keys(&serde_json::json!("x")).is_err());
    }
}
