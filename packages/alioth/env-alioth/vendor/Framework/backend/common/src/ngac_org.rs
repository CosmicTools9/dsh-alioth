//! ngac_org — 组织任职体系 ↔ NGAC 认知派生链**唯一实现**（B-0 收编）
//!
//! 认知/委托派生系（add-ngac-cognition-derived-ua / add-ngac-delegation）单一定义
//! 落位本模块：推导 CTE [`COGNITION_CTE`] / [`DELEGATED_CTE`]、幂等物化
//! [`ensure_cognition_uas`]、存量实例码 UA association 迁移
//! [`migrate_legacy_position_associations`]（B-1 配套，调用方 D-2/Phase C）、
//! 持有者反解 [`cognition_derived_user_holders`] /
//! [`cognition_derived_holders_batch`]。
//!
//! 消费方（SSO `ngac/pip.rs`、`/auth/me` 矩阵、`permissions.rs` 决策、Gateway
//! `resolve_user_permissions`、本模块成员解析）MUST 引用常量 / 调用函数，
//! **禁止复制 SQL**（NGAC_SPEC §2.2.3/§2.2.4 消费同源义务）。
//! 推导链：user → empl-agent/empl-natural(fk_user) →
//! `zc_id_subj-post_rr_employee`(ref_left=岗位, ref_right=雇员) → position；
//! 全部边 `deleted_at IS NULL`。
//!
//! 语义收敛（integrate-framework-cognition-ua）：审批/通知成员解析目标可为
//! ① 认知派生名（`position:{类别code}` / `view:{code}`，任职桥持有者；
//!    类别 = 岗位 `ck_category` 指向的 `zc_id_category` **基表行** code，子族字典不派生）、
//! ② 岗位标识（id / code / notice，直管 fk_user ∪ 任职桥持有者）、
//! ③ 指派型 UA 名（`ngac_user_rr_attribute` 物化成员，兼容既有角色配置）。
//! 三者并集去重——岗位任职（读侧派生）与指派 UA 不再双轨分叉。

use sqlx::{AssertSqlSafe, PgConnection, PgPool};

/// 委托派生 CTE（add-ngac-delegation D2，B-0 收编）——与认知派生同构：被委托人
/// 的有效 UA 集并入 active 且时间窗内的委托 UA。委托源在 SSO 侧（PEP 侧上下文
/// 提示不含委托派生）。消费方 MUST 引用本常量，禁止复制。参数约定：`$1` = 被委托人。
pub const DELEGATED_CTE: &str = r#"
delegated_ua AS (
    SELECT ua.id
    FROM isahl_auth.ngac_delegation d
    JOIN isahl_auth.ngac_user_attribute ua ON ua.id = d.fk_user_attribute AND ua.deleted_at IS NULL
    WHERE d.fk_delegatee = $1 AND d.status = 'active' AND d.deleted_at IS NULL
      AND d.date_st <= NOW() AND d.date_ed > NOW()
)
"#;

/// 认知链推导 CTE（add-ngac-cognition-derived-ua D3，B-0 收编）——推导链唯一实现。
/// 消费方（SSO `ngac/pip.rs`、`/auth/me` 矩阵、`permissions.rs` 决策、Gateway
/// `resolve_user_permissions`、`ensure_cognition_uas`）MUST 引用本常量拼装，禁止复制
/// （NGAC_SPEC §2.2.3/§2.2.4 消费同源义务）。
/// 推导链与 `/auth/me` 主体认知同构：
/// user → empl-agent/empl-natural(fk_user) → post_rr_employee(ref_left=岗位, ref_right=雇员)
///      → position；岗位 → relation-post_view_r_tags → tags-post_view。全部边 deleted_at IS NULL。
/// `position:` 派生名取岗位 `ck_category` → `zc_id_category` 的类别 code；类别行须为基表行
/// （`tableoid = 'isahl.zc_id_category'::regclass`）——`zc_id_cate-position` 等子族字典
/// （组织架构岗分类）与空 `ck_category` 均不派生（B-1 align-cognition-ua-category）。
/// 岗位行 `code` 仅作业务标识，不参与派生命名。参数约定：CTE 内引用 `$1` = fk_user。
pub const COGNITION_CTE: &str = r#"
my_positions AS (
    SELECT sp.id, sp.code, c.code AS category_code
    FROM isahl."zc_id_empl-agent" ea
    JOIN isahl."zc_id_subj-post_rr_employee" spre
        ON spre.ref_right = ea.id AND spre.deleted_at IS NULL
    JOIN isahl."zc_id_subj-position" sp
        ON sp.id = spre.ref_left AND sp.deleted_at IS NULL
    LEFT JOIN isahl.zc_id_category c
        ON c.id = sp.ck_category AND c.tableoid = 'isahl.zc_id_category'::regclass
        AND c.deleted_at IS NULL
    WHERE ea.fk_user = $1 AND ea.deleted_at IS NULL
    UNION
    SELECT sp.id, sp.code, c.code AS category_code
    FROM isahl."zc_id_empl-natural" en
    JOIN isahl."zc_id_subj-post_rr_employee" spre
        ON spre.ref_right = en.id AND spre.deleted_at IS NULL
    JOIN isahl."zc_id_subj-position" sp
        ON sp.id = spre.ref_left AND sp.deleted_at IS NULL
    LEFT JOIN isahl.zc_id_category c
        ON c.id = sp.ck_category AND c.tableoid = 'isahl.zc_id_category'::regclass
        AND c.deleted_at IS NULL
    WHERE en.fk_user = $1 AND en.deleted_at IS NULL
),
cognition_ua_names AS (
    SELECT 'position:' || mp.category_code AS o_name
    FROM my_positions mp
    WHERE mp.category_code IS NOT NULL AND mp.category_code <> ''
    UNION
    SELECT 'view:' || vt.code
    FROM my_positions mp
    JOIN isahl."zc_id_relation-post_view_r_tags" r
        ON r.ref_left = mp.id AND r.deleted_at IS NULL
    JOIN isahl."zc_id_tags-post_view" vt
        ON vt.id = r.ref_right AND vt.deleted_at IS NULL
    WHERE vt.code IS NOT NULL AND vt.code <> ''
)
"#;

/// 认知派生 UA 自动确保（add-ngac-cognition-derived-ua D2，B-0 收编）：
/// 首次遇到未物化的 `position:{类别code}` / `view:{code}` 名时幂等创建 UA 行，
/// 供管理面对其建立 association。先幂等 upsert `default` 策略类
/// （`fk_policy_class` NOT NULL + FK，实测 2026-08-26）。
///
/// - 冲突目标 = 部分唯一索引 `(o_name, fk_policy_class) WHERE deleted_at IS NULL`
/// - 失败仅告警不阻断：UA 缺席 = 无关联授予，语义天然 fail-closed
/// - 不触发策略版本 bump（ngac_user_attribute 无版本触发器，UA 不入 PolicyGraph）、
///   不写策略审计日志（系统派生节点，非用户策略编辑）
/// - isahl_auth 扩展表自愈 ensure 语义同源：与 SSO `ngac/ensure.rs` 运行时幂等
///   ensure 先例一致——缺对象幂等创建，绝不因派生物化失败阻断决策主路径
pub async fn ensure_cognition_uas(pool: &PgPool, fk_user: i64) {
    let sql = format!(
        r#"
        WITH {COGNITION_CTE},
        pc_ins AS (
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
        SELECT cn.o_name, dp.id,
               jsonb_build_object(
                   'derived_from', 'cognition',
                   'source_kind', split_part(cn.o_name, ':', 1),
                   'source_code', split_part(cn.o_name, ':', 2)
               )
        FROM cognition_ua_names cn
        CROSS JOIN default_pc dp
        ON CONFLICT (o_name, fk_policy_class) WHERE deleted_at IS NULL DO NOTHING
        "#,
        COGNITION_CTE = COGNITION_CTE
    );
    // SQL 由编译期常量 COGNITION_CTE 拼装（无用户输入），AssertSqlSafe 显式审计
    if let Err(e) = sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(fk_user)
        .execute(pool)
        .await
    {
        log::warn!(
            "ensure_cognition_uas: user {} ensure failed: {}",
            fk_user,
            e
        );
    }
}

/// 存量实例码 `position:{岗位code}` UA → 类别码 `position:{类别code}` UA 的
/// association 幂等迁移（B-1 align-cognition-ua-category 配套）。
///
/// B-1 前认知派生命名取岗位**实例 code**（`position:{实例code}`），B-1 起取岗位
/// `ck_category` 指向的 `zc_id_category` **基表行** code（与 [`COGNITION_CTE`]
/// 同源，子族字典与空 `ck_category` 均不派生）。存量实例码 UA 上由管理面配置的
/// association 若不迁移将随 D-2 遗留清理而丢失——本函数在删除前把**仍存活**
/// （`deleted_at IS NULL`）的 association 复制到对应类别 UA。
///
/// - 存量 UA 识别：`position:` 前缀且（`property.derived_from='cognition'`
///   ——B-1 前 ensure 物化标记；或名字不在类别派生名集合内——任何存活岗位的
///   基表类别 code 派生名以外的 `position:` 名）
/// - 对应类别 UA：UA 名后缀 = 岗位实例 code → 该岗位 `ck_category` 基表类别行
///   code；仅当目标类别 UA 存在且该 (UA, OA, policy class) 三元组在目标上不存在
///   才插入 —— `NOT EXISTS`（语义判活）+ `ON CONFLICT … DO NOTHING`
///   （软删残留三元组防撞）双保险，**重复调用恒幂等**
/// - 纯读 + `INSERT … SELECT`：不删除存量 UA/association（遗留清理归调用方），
///   不写策略审计日志（系统迁移非用户策略编辑）；association 版本触发器正常
///   bump（复制即真实策略变更）
///
/// 调用方 = D-2/Phase C 收束任务（本模块不接调用点）。失败经 `sqlx::Result`
/// 上抛、返回本次实际复制行数——迁移中断不得静默，调用方须确认复制成功后方可
/// 处置存量 UA。
pub async fn migrate_legacy_position_associations(pool: &PgPool) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        r#"
        WITH
        cat_names AS (
            -- 类别派生名集合（B-1 语义，同 COGNITION_CTE 基表行约束）：任一存活
            -- 岗位 ck_category 指向的基表类别 code 派生名，非实例码名
            SELECT DISTINCT 'position:' || c.code AS o_name
            FROM isahl."zc_id_subj-position" sp
            JOIN isahl.zc_id_category c
                ON c.id = sp.ck_category
               AND c.tableoid = 'isahl.zc_id_category'::regclass
               AND c.deleted_at IS NULL
            WHERE sp.deleted_at IS NULL AND sp._f_ IS NULL
              AND c.code IS NOT NULL AND c.code <> ''
        ),
        legacy_ua AS (
            -- 存量实例码 UA：position: 前缀且（B-1 前 ensure 物化标记 或 非类别派生名）
            SELECT ua.id AS src_ua_id, split_part(ua.o_name, ':', 2) AS inst_code
            FROM isahl_auth.ngac_user_attribute ua
            WHERE ua.deleted_at IS NULL
              AND ua.o_name LIKE 'position:%'
              AND (ua.property ->> 'derived_from' = 'cognition'
                   OR NOT EXISTS (SELECT 1 FROM cat_names cn WHERE cn.o_name = ua.o_name))
        ),
        mapped AS (
            -- 实例 code → 类别 code 映射：命中存活岗位且类别为基表行
            SELECT DISTINCT lug.src_ua_id,
                   'position:' || c.code AS dst_o_name
            FROM legacy_ua lug
            JOIN isahl."zc_id_subj-position" sp
                ON sp.code = lug.inst_code AND sp.deleted_at IS NULL AND sp._f_ IS NULL
            JOIN isahl.zc_id_category c
                ON c.id = sp.ck_category
               AND c.tableoid = 'isahl.zc_id_category'::regclass
               AND c.deleted_at IS NULL
            WHERE c.code IS NOT NULL AND c.code <> ''
        ),
        dst AS (
            -- 目标类别 UA 必须已物化（deleted_at IS NULL）
            SELECT m.src_ua_id, ua.id AS dst_ua_id
            FROM mapped m
            JOIN isahl_auth.ngac_user_attribute ua
                ON ua.o_name = m.dst_o_name AND ua.deleted_at IS NULL
        ),
        ins AS (
            INSERT INTO isahl_auth.ngac_association
                (fk_user_attribute, fk_object_attribute, ak_access_rights,
                 fk_policy_class, conditions, condition_expr)
            SELECT d.dst_ua_id, a.fk_object_attribute, a.ak_access_rights,
                   a.fk_policy_class, a.conditions, a.condition_expr
            FROM dst d
            JOIN isahl_auth.ngac_association a
                ON a.fk_user_attribute = d.src_ua_id AND a.deleted_at IS NULL
            WHERE NOT EXISTS (
                SELECT 1 FROM isahl_auth.ngac_association t
                WHERE t.fk_user_attribute = d.dst_ua_id
                  AND t.fk_object_attribute = a.fk_object_attribute
                  AND t.fk_policy_class = a.fk_policy_class
                  AND t.deleted_at IS NULL
            )
            ON CONFLICT (fk_user_attribute, fk_object_attribute, fk_policy_class)
                DO NOTHING
            RETURNING id
        )
        SELECT COUNT(*) FROM ins
        "#,
    )
    .fetch_one(pool)
    .await
}

/// 认知派生 UA 的持有者反向解析（add-ngac-cognition-derived-ua D3，评审采纳选项 b；
/// B-0 收编）——`position:{类别code}` / `view:{code}` UA 的持有者（用户 → 雇员 →
/// 岗位类别/标签 code）。供 review/resource 成员清单与后续「岗位成员查询」端点共用
/// ——**唯一实现，禁止第二份**。非认知前缀的 o_name 返回空集。返回 (user_id, username)。
pub async fn cognition_derived_user_holders(
    pool: &PgPool,
    o_name: &str,
) -> sqlx::Result<Vec<(i64, Option<String>)>> {
    if !o_name.starts_with("position:") && !o_name.starts_with("view:") {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, (i64, Option<String>)>(
        r#"
        WITH pos AS (
            SELECT ea.fk_user, sp.id AS position_id, c.code AS category_code
            FROM isahl."zc_id_empl-agent" ea
            JOIN isahl."zc_id_subj-post_rr_employee" spre
                ON spre.ref_right = ea.id AND spre.deleted_at IS NULL
            JOIN isahl."zc_id_subj-position" sp
                ON sp.id = spre.ref_left AND sp.deleted_at IS NULL
            LEFT JOIN isahl.zc_id_category c
                ON c.id = sp.ck_category AND c.tableoid = 'isahl.zc_id_category'::regclass
                AND c.deleted_at IS NULL
            WHERE ea.deleted_at IS NULL
            UNION ALL
            SELECT en.fk_user, sp.id, c.code
            FROM isahl."zc_id_empl-natural" en
            JOIN isahl."zc_id_subj-post_rr_employee" spre
                ON spre.ref_right = en.id AND spre.deleted_at IS NULL
            JOIN isahl."zc_id_subj-position" sp
                ON sp.id = spre.ref_left AND sp.deleted_at IS NULL
            LEFT JOIN isahl.zc_id_category c
                ON c.id = sp.ck_category AND c.tableoid = 'isahl.zc_id_category'::regclass
                AND c.deleted_at IS NULL
            WHERE en.deleted_at IS NULL
        ),
        holders AS (
            SELECT fk_user FROM pos
            WHERE category_code IS NOT NULL AND 'position:' || category_code = $1
            UNION
            SELECT p.fk_user FROM pos p
            JOIN isahl."zc_id_relation-post_view_r_tags" r
                ON r.ref_left = p.position_id AND r.deleted_at IS NULL
            JOIN isahl."zc_id_tags-post_view" vt
                ON vt.id = r.ref_right AND vt.deleted_at IS NULL
            WHERE vt.code IS NOT NULL AND 'view:' || vt.code = $1
        )
        SELECT DISTINCT u.id, u.username
        FROM holders h
        JOIN isahl_auth.auth_users u ON u.id = h.fk_user
        ORDER BY u.id
        "#,
    )
    .bind(o_name)
    .fetch_all(pool)
    .await
}

/// 认知派生 UA 持有者批量反向解析（refactor-ngac-admin-nl-graph D7；B-0 收编）：
/// 全部 `position:`/`view:` UA 名 → 持有者集合，供图快照（`ngac/graph.rs`）一次取全。
/// 推导链与 [`cognition_derived_user_holders`] 同构（本模块唯一实现，仅去单个
/// o_name 过滤、按名分组）。返回 (o_name, user_id, username)。
pub async fn cognition_derived_holders_batch(
    pool: &PgPool,
) -> sqlx::Result<Vec<(String, i64, Option<String>)>> {
    sqlx::query_as::<_, (String, i64, Option<String>)>(
        r#"
        WITH pos AS (
            SELECT ea.fk_user, sp.id AS position_id, c.code AS category_code
            FROM isahl."zc_id_empl-agent" ea
            JOIN isahl."zc_id_subj-post_rr_employee" spre
                ON spre.ref_right = ea.id AND spre.deleted_at IS NULL
            JOIN isahl."zc_id_subj-position" sp
                ON sp.id = spre.ref_left AND sp.deleted_at IS NULL
            LEFT JOIN isahl.zc_id_category c
                ON c.id = sp.ck_category AND c.tableoid = 'isahl.zc_id_category'::regclass
                AND c.deleted_at IS NULL
            WHERE ea.deleted_at IS NULL
            UNION ALL
            SELECT en.fk_user, sp.id, c.code
            FROM isahl."zc_id_empl-natural" en
            JOIN isahl."zc_id_subj-post_rr_employee" spre
                ON spre.ref_right = en.id AND spre.deleted_at IS NULL
            JOIN isahl."zc_id_subj-position" sp
                ON sp.id = spre.ref_left AND sp.deleted_at IS NULL
            LEFT JOIN isahl.zc_id_category c
                ON c.id = sp.ck_category AND c.tableoid = 'isahl.zc_id_category'::regclass
                AND c.deleted_at IS NULL
            WHERE en.deleted_at IS NULL
        ),
        holders AS (
            SELECT 'position:' || pos.category_code AS o_name, pos.fk_user
            FROM pos
            WHERE pos.category_code IS NOT NULL AND pos.category_code <> ''
            UNION
            SELECT 'view:' || vt.code, p.fk_user
            FROM pos p
            JOIN isahl."zc_id_relation-post_view_r_tags" r
                ON r.ref_left = p.position_id AND r.deleted_at IS NULL
            JOIN isahl."zc_id_tags-post_view" vt
                ON vt.id = r.ref_right AND vt.deleted_at IS NULL
            WHERE vt.code IS NOT NULL AND vt.code <> ''
        )
        SELECT DISTINCT h.o_name, u.id, u.username
        FROM holders h
        JOIN isahl_auth.auth_users u ON u.id = h.fk_user
        ORDER BY h.o_name, u.id
        "#,
    )
    .fetch_all(pool)
    .await
}

/// 认知派生名（`position:` / `view:` 前缀）的**活跃用户**持有者反解——本模块内
/// 通知/审批语义变体（`is_active = TRUE` 过滤、仅返回 user id），推导链与
/// [`cognition_derived_user_holders`] 同构同源；SSO 侧 review 清单用全量版。
/// 限前缀名；非前缀返回空集。
async fn cognition_holders(conn: &mut PgConnection, o_name: &str) -> Result<Vec<i64>, sqlx::Error> {
    if !o_name.starts_with("position:") && !o_name.starts_with("view:") {
        return Ok(Vec::new());
    }
    sqlx::query_scalar(
        r#"
        WITH pos AS (
            SELECT ea.fk_user, sp.id AS position_id, c.code AS category_code
            FROM isahl."zc_id_empl-agent" ea
            JOIN isahl."zc_id_subj-post_rr_employee" spre
                ON spre.ref_right = ea.id AND spre.deleted_at IS NULL
            JOIN isahl."zc_id_subj-position" sp
                ON sp.id = spre.ref_left AND sp.deleted_at IS NULL
            LEFT JOIN isahl.zc_id_category c
                ON c.id = sp.ck_category AND c.tableoid = 'isahl.zc_id_category'::regclass
                AND c.deleted_at IS NULL
            WHERE ea.deleted_at IS NULL
            UNION ALL
            SELECT en.fk_user, sp.id, c.code
            FROM isahl."zc_id_empl-natural" en
            JOIN isahl."zc_id_subj-post_rr_employee" spre
                ON spre.ref_right = en.id AND spre.deleted_at IS NULL
            JOIN isahl."zc_id_subj-position" sp
                ON sp.id = spre.ref_left AND sp.deleted_at IS NULL
            LEFT JOIN isahl.zc_id_category c
                ON c.id = sp.ck_category AND c.tableoid = 'isahl.zc_id_category'::regclass
                AND c.deleted_at IS NULL
            WHERE en.deleted_at IS NULL
        ),
        holders AS (
            SELECT fk_user FROM pos
            WHERE category_code IS NOT NULL AND 'position:' || category_code = $1
            UNION
            SELECT p.fk_user FROM pos p
            JOIN isahl."zc_id_relation-post_view_r_tags" r
                ON r.ref_left = p.position_id AND r.deleted_at IS NULL
            JOIN isahl."zc_id_tags-post_view" vt
                ON vt.id = r.ref_right AND vt.deleted_at IS NULL
            WHERE vt.code IS NOT NULL AND 'view:' || vt.code = $1
        )
        SELECT DISTINCT u.id
        FROM holders h
        JOIN isahl_auth.auth_users u ON u.id = h.fk_user
        WHERE u.is_active = TRUE
        ORDER BY u.id
        "#,
    )
    .bind(o_name)
    .fetch_all(conn)
    .await
}

/// 岗位标识（id / code / notice）的成员：直管 fk_user ∪ 任职桥持有者。
async fn position_members(
    conn: &mut PgConnection,
    val: &str,
    limit: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        WITH target AS (
            SELECT id, fk_user FROM isahl."zc_id_subj-position"
            WHERE deleted_at IS NULL
              AND (id::text = $1 OR code = $1 OR notice = $1)
            LIMIT 1
        ),
        employed AS (
            SELECT ea.fk_user AS uid
            FROM target t
            JOIN isahl."zc_id_subj-post_rr_employee" spre
                ON spre.ref_left = t.id AND spre.deleted_at IS NULL
            JOIN isahl."zc_id_empl-agent" ea
                ON ea.id = spre.ref_right AND ea.deleted_at IS NULL
            UNION
            SELECT en.fk_user AS uid
            FROM target t
            JOIN isahl."zc_id_subj-post_rr_employee" spre
                ON spre.ref_left = t.id AND spre.deleted_at IS NULL
            JOIN isahl."zc_id_empl-natural" en
                ON en.id = spre.ref_right AND en.deleted_at IS NULL
        ),
        members AS (
            SELECT fk_user AS uid FROM target WHERE fk_user IS NOT NULL
            UNION
            SELECT uid FROM employed WHERE uid IS NOT NULL
        )
        SELECT DISTINCT u.id
        FROM members m
        JOIN isahl_auth.auth_users u ON u.id = m.uid
        WHERE u.is_active = TRUE
        ORDER BY u.id
        LIMIT $2
        "#,
    )
    .bind(val)
    .bind(limit)
    .fetch_all(conn)
    .await
}

/// 指派型 UA 名（`ngac_user_rr_attribute` 物化成员）——legacy 角色路径。
async fn assigned_ua_members(
    conn: &mut PgConnection,
    ua_name: &str,
    limit: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT u.id FROM isahl_auth.auth_users u
           JOIN isahl_auth.ngac_user_rr_attribute rel
             ON rel.fk_user = u.id AND rel.deleted_at IS NULL
             AND (rel.expires_at IS NULL OR rel.expires_at > NOW())
           JOIN isahl_auth.ngac_user_attribute ua
             ON ua.id = rel.fk_user_attribute AND ua.deleted_at IS NULL
           WHERE ua.o_name = $1 AND u.is_active = TRUE
           ORDER BY u.id
           LIMIT $2"#,
    )
    .bind(ua_name)
    .bind(limit)
    .fetch_all(conn)
    .await
}

/// 收敛成员解析（详见模块头三路并集语义）。
///
/// - `val` 以 `position:`/`view:` 前缀 → 仅认知持有者
/// - 裸值 → 认知派生名不适用：指派 UA 成员 ∪ 岗位标识成员（id/code/notice）
///
/// 任何单路失败降级其他路（warn 由调用方语义兜底：空集 → 不投递/不建单）。
pub async fn resolve_member_user_ids(
    conn: &mut PgConnection,
    val: &str,
    limit: i64,
) -> Vec<i64> {
    if val.is_empty() {
        return Vec::new();
    }
    if val.starts_with("position:") || val.starts_with("view:") {
        return cognition_holders(conn, val).await.unwrap_or_default();
    }
    let mut acc: Vec<i64> = Vec::new();
    if let Ok(ua) = assigned_ua_members(conn, val, limit).await {
        acc.extend(ua);
    }
    if let Ok(pos) = position_members(conn, val, limit).await {
        acc.extend(pos);
    }
    acc.sort_unstable();
    acc.dedup();
    acc.truncate(limit as usize);
    acc
}
