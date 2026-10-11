//! NGAC 资源授权写入的**唯一实现**（判定链之外的另一半：授权供给面）。
//!
//! 背景（2026-09-29 普查）：同一语义的「建资源 OA + 给创建者/admin 补 association」在
//! 平台 10 处 + ns 7 处以各自 SQL 各写一遍，`ak_access_rights` 写入面达 19 文件 ⇒
//! 同一资源在不同写入点可能拿到不同权限集（交叉错误），新资源类型上线需 N 处同步改（漏一处即静默缺权）。
//!
//! 收敛面 = 三个可组合函数 + 两套具名 right 集：
//! - [`ensure_resource_oa`]：行级 OA 幂等注册（`ON CONFLICT … DO NOTHING`），返回 OA id；
//! - [`grant_rights`]：给「创建者直派 UA」或「具名 UA（如 admin）」授予 right 集（幂等，`NOT EXISTS` 守卫）；
//!
//! 语义边界（消费方 MUST NOT 改动）：
//! - 创建者授权按其**直派** `ngac_user_rr_attribute` 行（不展开闭包/派生）——与历史行为一致；
//! - admin UA 名取自 [`ngac_host::ADMIN_UA_NAME`]（唯一名字集，MUST NOT 写字面量）；
//! - 所有写入幂等（`NOT EXISTS` + `ON CONFLICT DO NOTHING`），重复调用不产生重复行；
//! - 显式写 `id = isahl.gen_next_zuid()`（不依赖各环境 `id` DEFAULT——测试库曾实测缺省致 23502）；
//! - 失败语义由调用方决定（本模块返回 `Result`，调用方可 warn 降级或 fail-fast）。

use crate::AliothError;
use sqlx::{Executor, PgPool, Postgres};

/// 行级资源 OA 注册参数。
pub struct ResourceOaSpec<'a> {
    /// OA 名（`o_name`），惯用 `{resource_type}-{resource_id}`。
    pub o_name: String,
    pub resource_type: &'a str,
    pub resource_id: i64,
    /// 可读标识（`resource_identifier`），`None` ⇒ 不写该列。
    pub resource_identifier: Option<&'a str>,
    /// 创建者（`created_by_id`），`None` ⇒ NULL。
    pub created_by: Option<i64>,
    /// 策略类选择：`Some(name)` ⇒ 仅绑定该具名类（如 `"default"`；不存在则不写 OA）；
    /// `None` ⇒ 取任意首个类（平台侧历史行为）。
    pub policy_class_name: Option<&'a str>,
}

/// OA id 查询（内部：单查询）。
async fn oa_id<'c, E>(
    exec: E,
    resource_type: &str,
    resource_id: i64,
) -> Result<Option<i64>, AliothError>
where
    E: Executor<'c, Database = Postgres>,
{
    sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute \
         WHERE resource_type = $1 AND fk_resource = $2 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(resource_type)
    .bind(resource_id)
    .fetch_optional(exec)
    .await
    .map_err(AliothError::from_sqlx)
}

/// OA 幂等插入（内部：单条**静态** SQL——策略类选择与标识列均为 NULL 绑定，无动态拼接）。
///
/// 语义保真：`policy_class_name = Some(name)` ⇒ 仅绑定该具名类（不存在 ⇒ 0 行、不写 OA）；
/// `None` ⇒ 取首个类（按 id 升序，等价原 `LIMIT 1`）。`resource_identifier` 列恒在列清单内，
/// 未提供即写 NULL（该列可空、无默认值）。
async fn oa_insert<'c, E>(exec: E, spec: &ResourceOaSpec<'_>) -> Result<(), AliothError>
where
    E: Executor<'c, Database = Postgres>,
{
    sqlx::query(
        // property 列 = 集合标签继承（wire-instance-oa-labels / §2.7 实例级写径②）：
        // 实例行 OA 创建时从同 resource_type 集合 OA（fk_resource=0）继承 `label`
        //（集合自身创建时子查询空集 ⇒ NULL，无自指）；继承后 §2.7 判定谓词即对
        // 实例行生效（label_clearance_denies 读目标行 OA 的 property.label）。
        "INSERT INTO isahl_auth.ngac_object_attribute \
           (id, o_name, fk_policy_class, resource_type, fk_resource, resource_identifier, property, created_by_id) \
         SELECT isahl.gen_next_zuid(), $1, pc.id, $3, $4, $5, \
                (SELECT jsonb_build_object('label', c.property ->> 'label') \
                   FROM isahl_auth.ngac_object_attribute c \
                  WHERE c.resource_type = $3 AND c.fk_resource = 0 \
                    AND c.deleted_at IS NULL AND c.property ->> 'label' IS NOT NULL \
                  LIMIT 1), $6 \
           FROM isahl_auth.ngac_policy_class pc \
          WHERE ($2::text IS NULL \
                 AND pc.id = (SELECT id FROM isahl_auth.ngac_policy_class ORDER BY id LIMIT 1)) \
             OR pc.o_name = $2 \
          LIMIT 1 \
         ON CONFLICT(resource_type, fk_resource) DO NOTHING",
    )
    .bind(&spec.o_name)
    .bind(spec.policy_class_name)
    .bind(spec.resource_type)
    .bind(spec.resource_id)
    .bind(spec.resource_identifier)
    .bind(spec.created_by)
    .execute(exec)
    .await
    .map_err(AliothError::from_sqlx)?;
    Ok(())
}

/// 行级 OA 幂等注册（池入口）；返回 OA id（已存在同 `(resource_type, fk_resource)` 行 ⇒ 返回其 id）。
pub async fn ensure_resource_oa(
    pool: &PgPool,
    spec: &ResourceOaSpec<'_>,
) -> Result<Option<i64>, AliothError> {
    if oa_id(pool, spec.resource_type, spec.resource_id)
        .await?
        .is_none()
    {
        oa_insert(pool, spec).await?;
    }
    oa_id(pool, spec.resource_type, spec.resource_id).await
}

/// 行级 OA 幂等注册（事务入口：`&mut *conn` 重借）。
pub async fn ensure_resource_oa_tx(
    conn: &mut sqlx::PgConnection,
    spec: &ResourceOaSpec<'_>,
) -> Result<Option<i64>, AliothError> {
    if oa_id(&mut *conn, spec.resource_type, spec.resource_id)
        .await?
        .is_none()
    {
        oa_insert(&mut *conn, spec).await?;
    }
    oa_id(&mut *conn, spec.resource_type, spec.resource_id).await
}

/// 给授权主体授予指定 right 集（**加性** union；幂等；返回新增+扩展的行数）。
///
/// `rights` 为 `ngac_access_right.o_name` 列表（用 `ngac_host::{RIGHTS_CREATOR, RIGHTS_ADMIN}` 或调用方自有集）。
///
/// 扇出：按该 `o_name` 的**每个** UA 各写一行（`ngac_user_attribute.o_name` 无唯一约束）。
/// 创建者授权不在此物化（derive-creator-grants-from-lifecycle：判定时刻派生，见 common::ngac_org）。
///
/// 冲突（`(fk_user_attribute, fk_object_attribute, fk_policy_class)` 唯一）⇒ **并集**：
/// 已存在行取其 `ak_access_rights ∪ 本次集`，且**仅当本次集未被包含时**才写（已包含 ⇒ 零写入，
/// 避免策略版本 bump 触发器空转）。软删行占位（`deleted_at IS NOT NULL`）⇒ 不更新、不复活
/// （与原点位 `NOT EXISTS (deleted_at IS NULL)` 守卫同效）。故 `grant` 语义为单调加性：
/// 重复授予同集零新增、零改写；授予更大的集只增不减。
pub async fn grant_rights<'c, E>(
    exec: E,
    resource_type: &str,
    resource_id: i64,
    ua_name: &str,
    rights: &[&str],
) -> Result<u64, AliothError>
where
    E: Executor<'c, Database = Postgres>,
{
    grant_rights_until(exec, resource_type, resource_id, ua_name, rights, None).await
}

/// 同 [`grant_rights`]，但可带**到期时间**（`expires_at`，`None` = 永不过期）。
///
/// 加性 union 语义不变（`@>` 守卫 + 并集）；expires_at 冲突时取 `EXCLUDED` 非空值
/// （`COALESCE(EXCLUDED, 既有)`——既有已带到期时不被 None 抹除）。到期由决策侧装载面
/// 过滤（`ngac-host::load_policy_graph`：`expires_at IS NULL OR expires_at > NOW()`）+
/// 快照到期重载 ⇒ 决策时点即时生效。
pub async fn grant_rights_until<'c, E>(
    exec: E,
    resource_type: &str,
    resource_id: i64,
    ua_name: &str,
    rights: &[&str],
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<u64, AliothError>
where
    E: Executor<'c, Database = Postgres>,
{
    // 两个分支的 SQL 只有「主体表 + 主体列 + 主体条件」不同（均为**静态**字面组合，无用户输入拼接）
    let sql =
            "INSERT INTO isahl_auth.ngac_association \
               (id, fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, expires_at, created_at) \
             SELECT isahl.gen_next_zuid(), ua.id, oa.id, \
                    ARRAY(SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = ANY($4)), \
                    oa.fk_policy_class, $5, NOW() \
             FROM isahl_auth.ngac_user_attribute ua \
             JOIN isahl_auth.ngac_object_attribute oa \
               ON oa.resource_type = $2 AND oa.fk_resource = $3 AND oa.deleted_at IS NULL \
             WHERE ua.o_name = $1 AND ua.deleted_at IS NULL \
             ON CONFLICT (fk_user_attribute, fk_object_attribute, fk_policy_class) DO UPDATE \
               SET ak_access_rights = ARRAY(SELECT DISTINCT unnest( \
                     ngac_association.ak_access_rights || EXCLUDED.ak_access_rights)), \
                   expires_at = COALESCE(EXCLUDED.expires_at, ngac_association.expires_at) \
               WHERE ngac_association.deleted_at IS NULL \
                 AND NOT (ngac_association.ak_access_rights @> EXCLUDED.ak_access_rights)";

    let rights_vec: Vec<String> = rights.iter().map(|r| (*r).to_string()).collect();
    let mut q = sqlx::query(sql);
    q = q.bind(ua_name);
    let result = q
        .bind(resource_type)
        .bind(resource_id)
        .bind(&rights_vec)
        .bind(expires_at)
        .execute(exec)
        .await
        .map_err(AliothError::from_sqlx)?;
    Ok(result.rows_affected())
}

/// **撤销**指定 right 集（差异收缩，唯一实现）——[`grant_rights`] 的反向对偶：
/// 从匹配 association 的 `ak_access_rights` 中剔除给定 right；剩余为空 ⇒ 软删该行
/// （`deleted_at = NOW()`，不再供权）；剩余非空 ⇒ 原地收缩。right 集内不存在的码为 no-op。
///
/// 匹配面与 `grant_rights` 对称：`(ua_name → UA) × (resource_type, resource_id → 行 OA)`，
/// 且仅作用于 `deleted_at IS NULL` 的行。`o_name = ANY($4)` 的码查不到 ⇒ 剔除集为空 ⇒ 零行更新。
pub async fn revoke_rights<'c, E>(
    exec: E,
    resource_type: &str,
    resource_id: i64,
    ua_name: &str,
    rights: &[&str],
) -> Result<u64, AliothError>
where
    E: Executor<'c, Database = Postgres>,
{
    let sql = "WITH target AS ( \
                 SELECT a.id, \
                        ARRAY(SELECT x FROM unnest(a.ak_access_rights) x \
                              WHERE x <> ALL(ARRAY(SELECT id FROM isahl_auth.ngac_access_right \
                                                   WHERE o_name = ANY($4)))) AS remaining \
                   FROM isahl_auth.ngac_association a \
                   JOIN isahl_auth.ngac_user_attribute ua ON ua.id = a.fk_user_attribute \
                   JOIN isahl_auth.ngac_object_attribute oa ON oa.id = a.fk_object_attribute \
                  WHERE ua.o_name = $1 AND ua.deleted_at IS NULL \
                    AND oa.resource_type = $2 AND oa.fk_resource = $3 AND oa.deleted_at IS NULL \
                    AND a.deleted_at IS NULL \
               ) \
               UPDATE isahl_auth.ngac_association a \
                  SET ak_access_rights = t.remaining, \
                      deleted_at = CASE WHEN cardinality(t.remaining) = 0 THEN NOW() ELSE a.deleted_at END \
                 FROM target t \
                WHERE a.id = t.id AND a.ak_access_rights <> t.remaining";

    let rights_vec: Vec<String> = rights.iter().map(|r| (*r).to_string()).collect();
    let mut q = sqlx::query(sql);
    q = q.bind(ua_name);
    let result = q
        .bind(resource_type)
        .bind(resource_id)
        .bind(&rights_vec)
        .execute(exec)
        .await
        .map_err(AliothError::from_sqlx)?;
    Ok(result.rows_affected())
}

// ─────────────────────────────────────────────────────────────────────────────
// 组织资源覆盖（cover-org-resources-ngac / B-2）：注册面派生的幂等 ensure
// ─────────────────────────────────────────────────────────────────────────────

/// B-2 默认属权映射的 owner 槽 UA（建岗挂人写端角色；与 Gateway seed org 三集合先例同族）。
pub const OWNERSHIP_DEFAULT_OWNER_UA: &str = "operator";
/// B-2 默认属权映射的 benefit/permit/access 槽 UA（组织成员行级归属）。
pub const OWNERSHIP_DEFAULT_MEMBER_UA: &str = "employee";
/// operator 矩阵的 right 集（读面——运维可视全部资源；写径仍逐模块授权）。
pub const OPERATOR_READ_RIGHTS: &[&str] = &["read"];

/// 组织域资源类型（属权 `enabled` 显式启用的例外集；identity-org org_tree 写端 heal
/// 与 Gateway org 集合 OA 同族命名）。**唯一清单**：`Gateway/backend/src/seed/ngac_seed.rs`
/// 的 org 集合列表复用本常量，MUST NOT 复制第二份。
pub const ORG_RESOURCE_TYPES: &[&str] = &["positions", "departments", "organizations"];

/// 属权策略 `enabled` 默认判定（**单源**；align-ngac-seed-vocabulary D2 / NGAC_SPEC §2.2.5b）：
/// 组织域资源 [`ORG_RESOURCE_TYPES`] ⇒ `true`（B-2 属权行级归属启用——行级写仍须用户被该行
/// `ak_*_user` 具名）；其余资源类型 ⇒ `false` 安全缺省（覆盖 ensure 不放行任何行级写，
/// 管理员显式启用才生效）。
///
/// 两个写入方（本模块 [`ensure_org_resource_coverage`] 与 Gateway `ngac_seed::ensure_org_resources`）
/// MUST 消费本判定值，MUST NOT 各传字面值——两者均以 `NOT EXISTS(resource_type)` 守卫，
/// 相反字面值会使启动序决定行级写权限。
pub fn ownership_enabled_default(resource_type: &str) -> bool {
    ORG_RESOURCE_TYPES.contains(&resource_type)
}

/// 组织资源覆盖 ensure 统计（幂等重跑时各项应为 0）。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CoverageStats {
    /// 新建集合 OA 行数（已存在不计）。
    pub collection_oas_created: usize,
    /// 新插 ownership 默认行数（`enabled` = [`ownership_enabled_default`]，组织域启用）。
    pub ownership_defaults_created: usize,
    /// operator × 集合 OA 的 read association 新增/扩展行数（已含 read ⇒ 0）。
    pub operator_associations_written: u64,
}

/// 组织资源覆盖幂等 ensure（单入口，design D5；cover-org-resources-ngac）：
///
/// ① **集合 OA**——每个注册 `resource_type` 经 [`ensure_resource_oa_tx`] 幂等注册
///    `fk_resource = 0` 集合 OA（`o_name = "{type}-collection"`，绑定 `default` 策略类，
///    名制对齐 §7.3 种子先例）——列表端点 PDP 判定从此有锚点；
/// ② **ownership 默认四路**——`owner → operator`、`benefit/permit/access → employee`、
///    `enabled` = [`ownership_enabled_default`]（组织域资源显式启用；其余安全缺省 false——
///    覆盖 ensure 不放行任何行级写，管理员显式启用才生效）；
///    任一 UA 或基础 right 缺解析 ⇒ 该类型整行跳过（不写半行）；
/// ③ **operator 读矩阵**——operator UA × 全部集合 OA 的 `read` association
///    （经 [`grant_rights`] 单源；集合 OA 缺席 ⇒ 0 行）。
///
/// 串行化 = `pg_advisory_xact_lock`（跨进程）；进程级 once 由调用方自备
/// （Gateway host 启动一次性调用，天然 once）。失败返回 `Err`，调用方 warn 降级
///（与 `ensure.rs` 自愈先例同族：本模块只保证单次语义，不吞错）。
///
/// 载体注记：design 原文把入口挂 `SSO ensure.rs` 尾部——但 SSO 依赖图看不到 ns
/// 组合根扩展的 registry（ns 扩展注册在 ns 仓 gateway-host），全量覆盖只能在
/// **组合了 registry 的进程**（Gateway host）发生；本函数因此落在授权写入单源模块，
/// 由 Gateway host 启动调用（见 change design addendum）。
pub async fn ensure_org_resource_coverage(
    pool: &PgPool,
    resource_types: &[String],
) -> Result<CoverageStats, AliothError> {
    let mut tx = pool.begin().await.map_err(AliothError::from_sqlx)?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('ngac_org_resource_coverage'))")
        .execute(&mut *tx)
        .await
        .map_err(AliothError::from_sqlx)?;

    // ④ **推理目标自足**（2026-10-10 实测缺口）：`oa_insert` 的
    //   `ON CONFLICT (resource_type, fk_resource)` 要求本表上存在**无谓词**的全唯一索引/约束。
    //   参考库由 SSO 启动期 ensure（`ngac/ensure.rs`）补齐，但 **embedded 部署不跑 SSO 启动期
    //   ensure** ⇒ 该索引缺席时本函数每次调用都失败（prod 日志：
    //   `ngac org resource coverage ensure failed … there is no unique or exclusion constraint
    //   matching the ON CONFLICT specification`），并连带**集合 OA / 属权默认 / operator 授权
    //   全部写不进去**（C 端自注册用户因此拿不到任何授予）。
    //   ⇒ 本函数自带前置：幂等、加性、不改既有对象，使其不依赖其它进程的启动期 ensure。
    //   命名注意：只在「无合适索引」时**以独立名**新建 —— 同名关系若已被**部分索引**占用，
    //   `CREATE UNIQUE INDEX IF NOT EXISTS <同名>` 会静默 no-op（该缺口成因之一）。
    sqlx::query(
        r#"DO $oa_uniq$
        BEGIN
            IF NOT EXISTS (
                SELECT 1 FROM pg_index i
                JOIN pg_class c ON c.oid = i.indexrelid
                JOIN pg_namespace n ON n.oid = c.relnamespace
                WHERE n.nspname = 'isahl_auth'
                  AND i.indrelid = 'isahl_auth.ngac_object_attribute'::regclass
                  AND i.indisunique
                  AND i.indpred IS NULL
                  AND (SELECT array_agg(a.attname::text ORDER BY a.attname)
                         FROM unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord)
                         JOIN pg_attribute a
                           ON a.attrelid = i.indrelid AND a.attnum = k.attnum)
                      = ARRAY['fk_resource', 'resource_type']
            ) THEN
                CREATE UNIQUE INDEX IF NOT EXISTS uq_ngac_oa_rt_res_full
                    ON isahl_auth.ngac_object_attribute (resource_type, fk_resource);
            END IF;
        END
        $oa_uniq$;"#,
    )
    .execute(&mut *tx)
    .await
    .map_err(AliothError::from_sqlx)?;

    let mut stats = CoverageStats::default();
    for rt in resource_types {
        // ① 集合 OA（单源入口；已存在 ⇒ 幂等返回其 id，不重复建行）
        let existed = oa_id(&mut *tx, rt, 0).await?.is_some();
        ensure_resource_oa_tx(
            &mut tx,
            &ResourceOaSpec {
                o_name: format!("{rt}-collection"),
                resource_type: rt,
                resource_id: 0,
                resource_identifier: None,
                created_by: None,
                policy_class_name: Some("default"),
            },
        )
        .await?;
        if !existed {
            stats.collection_oas_created += 1;
        }

        // ② ownership 默认四路（NOT EXISTS 防重；UA/right 任一缺解析 ⇒ SELECT 空集跳过）
        // `enabled` 按类型取单源判定（ownership_enabled_default；align-ngac-seed-vocabulary
        // D2）——与 Gateway org ensure 同判定值，`NOT EXISTS` 首写者胜亦等价。
        let res = sqlx::query(
            r#"INSERT INTO isahl_auth.ngac_ownership_policy
                 (id, resource_type, owner_attr_id, benefit_attr_id, permit_attr_id, access_attr_id,
                  read_right_id, write_right_id, delete_right_id, enabled)
               SELECT isahl.gen_next_zuid(), $1, oua.id, mua.id, mua.id, mua.id,
                      (SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'read'),
                      (SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'write'),
                      (SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'delete'),
                      $4::bool
               FROM isahl_auth.ngac_policy_class pc
               JOIN isahl_auth.ngac_user_attribute oua
                 ON oua.fk_policy_class = pc.id AND oua.o_name = $2 AND oua.deleted_at IS NULL
               JOIN isahl_auth.ngac_user_attribute mua
                 ON mua.fk_policy_class = pc.id AND mua.o_name = $3 AND mua.deleted_at IS NULL
               WHERE pc.o_name = 'default'
                 AND NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_ownership_policy
                                  WHERE resource_type = $1)
                 AND (SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'read') IS NOT NULL
                 AND (SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'write') IS NOT NULL
                 AND (SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'delete') IS NOT NULL"#,
        )
        .bind(rt)
        .bind(OWNERSHIP_DEFAULT_OWNER_UA)
        .bind(OWNERSHIP_DEFAULT_MEMBER_UA)
        .bind(ownership_enabled_default(rt))
        .execute(&mut *tx)
        .await
        .map_err(AliothError::from_sqlx)?;
        stats.ownership_defaults_created += res.rows_affected() as usize;

        // ③ operator 读矩阵（grant_rights 单源加性；集合 OA 缺席或已含 read ⇒ 0 行）
        stats.operator_associations_written += grant_rights(
            &mut *tx,
            rt,
            0,
            OWNERSHIP_DEFAULT_OWNER_UA,
            OPERATOR_READ_RIGHTS,
        )
        .await?;
    }
    tx.commit().await.map_err(AliothError::from_sqlx)?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 属权 `enabled` 单源判定（align-ngac-seed-vocabulary D2 / NGAC_SPEC §2.2.5b）：
    /// 组织域三资源为真；其余资源类型（含通用资源与任意新类型）为假安全缺省。
    #[test]
    fn ownership_enabled_default_is_org_domain_only() {
        assert_eq!(
            ORG_RESOURCE_TYPES,
            &["positions", "departments", "organizations"],
            "组织域资源清单 MUST 为 positions/departments/organizations（两写入方共用的唯一清单）"
        );
        for rt in ORG_RESOURCE_TYPES {
            assert!(
                ownership_enabled_default(rt),
                "组织域资源 {rt} MUST 显式启用（enabled=true）"
            );
        }
        for rt in [
            "approvals",
            "verctrl",
            "consignments",
            "z-test-cov-alpha",
            "",
        ] {
            assert!(
                !ownership_enabled_default(rt),
                "非组织域资源 {rt:?} MUST 取安全缺省（enabled=false，覆盖 ensure 不放行行级写）"
            );
        }
    }
}
