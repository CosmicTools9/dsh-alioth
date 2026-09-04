//! NGAC 通用策略种子：policy_class / UA / access_right / approval_* OA + admin 关联
//!
//! 边界（NGAC_SPEC §7.3 三层种子边界）：本模块只持有**跨 namespace 通用结构**——
//! policy_class 'default'、UA（admin/operator/auditor/user/employee）、基础 access_right、
//! approval_flows/approval_instances/approval_actions 三 collection OA 与 admin 全权关联。
//! 禁止出现任何 namespace 业务资源名；禁止 UPDATE/DELETE——仅补缺失行。
//!
//! 例外（add-ct-git-vc-interop）：verctrl 集合 OA 亦在本模块预置——恒定资源
//! string_id 模式（对齐 approvals/delegation_rules 先例），PEP 资源解析由 namespace
//! 注册表门控，OA 仅 Cosmic-Tools 命名空间可达，不跨 namespace 生效。
//!
//! 与 SSO migrations 005/008/019 幂等互补：迁移是部署期初始化，本模块是运行期自愈。

use sqlx::PgPool;

use super::SeedStats;

/// 通用 UA：o_name → description
const USER_ATTRIBUTES: &[(&str, &str)] = &[
    ("admin", "平台管理员"),
    ("operator", "业务操作员"),
    ("auditor", "审计员 (只读)"),
    ("user", "普通用户 (注册默认)"),
    ("employee", "员工（注册审批通过授予）"),
];

/// 基础 access_right
const ACCESS_RIGHTS: &[&str] = &[
    "read", "write", "delete", "approve", "admin", "create", "update", "list",
    // fix-approval-endpoint-gates M2：审批转办/加签动作（transfer_cc handler 消费）
    "transfer", "cc",
    // fix-approval-engine-semantics：申请人撤回动作（withdraw handler 消费）
    "withdraw",
];
/// approval 因子 collection OA（Gateway 通用层资源，非 namespace 业务）
const APPROVAL_COLLECTIONS: &[(&str, &str)] = &[
    ("approval_flows", "approval_flows-collection"),
    ("approval_instances", "approval_instances-collection"),
    ("approval_actions", "approval_actions-collection"),
];
/// org 资源集合 OA（cover-org-resources-ngac / B-2）：identity-org org_tree 写端
/// heal（ngac_org_ensure）将岗位/部门行 OA 挂到这些集合之下；operator UA 在集合上
/// 持 read/create/update，行级 delete 走属权策略（见 ensure_org_resources）。
const ORG_COLLECTIONS: &[&str] = &["positions", "departments", "organizations"];

/// NGAC 通用策略自检入口。
pub async fn ensure(pool: &PgPool) -> SeedStats {
    let mut stats = SeedStats::default();

    // 1. policy_class 'default'
    match ensure_policy_class(pool).await {
        Ok(preexisted) => {
            if preexisted {
                stats.existing += 1;
            } else {
                stats.created += 1;
            }
        }
        Err(e) => common::telemetry::warn!("seed[ngac]: policy_class 自愈失败: {e}"),
    }

    // 2. UA
    for (name, desc) in USER_ATTRIBUTES {
        match ensure_user_attribute(pool, name, desc).await {
            Ok(preexisted) => {
                if preexisted {
                    stats.existing += 1;
                } else {
                    stats.created += 1;
                }
            }
            Err(e) => common::telemetry::warn!("seed[ngac]: UA {name} 自愈失败: {e}"),
        }
    }

    // 3. access_right
    for right in ACCESS_RIGHTS {
        match ensure_access_right(pool, right).await {
            Ok(preexisted) => {
                if preexisted {
                    stats.existing += 1;
                } else {
                    stats.created += 1;
                }
            }
            Err(e) => common::telemetry::warn!("seed[ngac]: access_right {right} 自愈失败: {e}"),
        }
    }

    // 4. approval_* collection OA + admin 全权关联
    for (resource_type, o_name) in APPROVAL_COLLECTIONS {
        match ensure_collection_oa(pool, resource_type, o_name).await {
            Ok(created) => {
                if created {
                    stats.created += 1;
                } else {
                    stats.existing += 1;
                }
            }
            Err(e) => {
                common::telemetry::warn!("seed[ngac]: OA {o_name} 自愈失败: {e}");
            }
        }
    }
    match ensure_admin_associations(pool).await {
        Ok(created) => {
            if created > 0 {
                stats.healed += created as usize;
            }
        }
        Err(e) => common::telemetry::warn!("seed[ngac]: admin 关联自愈失败: {e}"),
    }
    // 4b. 审批操作端点 OA + 关联（fix-approval-endpoint-gates）：
    //     approvals:0 恒定资源——admin 全权；user UA create（rejected 用户
    //     自助重新申请 /api/approvals/apply 的 PDP 放行路径，零业务资源扩大）。
    if let Err(e) = ensure_collection_oa(pool, "approvals", "approvals-collection").await {
        common::telemetry::warn!("seed[ngac]: approvals OA 自愈失败: {e}");
    }
    match ensure_ua_collection_rights(pool, "admin", "approvals", None).await {
        Ok(created) => {
            if created > 0 {
                stats.healed += created as usize;
            }
        }
        Err(e) => common::telemetry::warn!("seed[ngac]: admin→approvals 关联自愈失败: {e}"),
    }
    match ensure_ua_collection_rights(pool, "user", "approvals", Some("create")).await {
        Ok(created) => {
            if created > 0 {
                stats.healed += created as usize;
            }
        }
        Err(e) => common::telemetry::warn!("seed[ngac]: user→approvals 关联自愈失败: {e}"),
    }
    // 4b2. 撤回权限（fix-approval-engine-semantics）：user UA → approval_instances:withdraw
    //     ——申请人自助撤回自己单据的 PDP 放行路径（handler 另做创建者所有权守卫；
    //     admin 经步骤 4 的 approval_* 全权 ARRAY(SELECT 全量) 自动覆盖 withdraw right）。
    match ensure_ua_collection_rights(pool, "user", "approval_instances", Some("withdraw")).await {
        Ok(created) => {
            if created > 0 {
                stats.healed += created as usize;
            }
        }
        Err(e) => {
            common::telemetry::warn!("seed[ngac]: user→approval_instances:withdraw 自愈失败: {e}")
        }
    }
    // 4c. 委托规则端点 OA + admin 关联（复查补缺：delegation_rules:0 集合 OA 缺失
    //     → 委托管理页 GET/POST 恒 403）
    if let Err(e) =
        ensure_collection_oa(pool, "delegation_rules", "delegation_rules-collection").await
    {
        common::telemetry::warn!("seed[ngac]: delegation_rules OA 自愈失败: {e}");
    }
    match ensure_ua_collection_rights(pool, "admin", "delegation_rules", None).await {
        Ok(created) => {
            if created > 0 {
                stats.healed += created as usize;
            }
        }
        Err(e) => common::telemetry::warn!("seed[ngac]: admin→delegation_rules 关联自愈失败: {e}"),
    }
    // 4d. ct-git 版控集合 OA + 关联（add-ct-git-vc-interop）：verctrl:0 恒定资源
    //     ——admin 全动作（create/read/update/delete），普通 user 只读（read）。
    //     OA 按 (resource_type, fk_resource=0) EXISTS 幂等；关联 NOT EXISTS 防重复。
    //     PEP 资源解析由 namespace 注册表门控（仅 Cosmic-Tools resolve 得 verctrl），
    //     OA 不会跨 namespace 生效。ver_branch:0 当前无独立端点，不预置 OA。
    if let Err(e) = ensure_collection_oa(pool, "verctrl", "verctrl-collection").await {
        common::telemetry::warn!("seed[ngac]: verctrl OA 自愈失败: {e}");
    }
    match ensure_ua_collection_rights_multi(
        pool,
        "admin",
        "verctrl",
        &["create", "read", "update", "delete"],
    )
    .await
    {
        Ok(created) => {
            if created > 0 {
                stats.healed += created as usize;
            }
        }
        Err(e) => common::telemetry::warn!("seed[ngac]: admin→verctrl 关联自愈失败: {e}"),
    }
    match ensure_ua_collection_rights_multi(pool, "user", "verctrl", &["read"]).await {
        Ok(created) => {
            if created > 0 {
                stats.healed += created as usize;
            }
        }
        Err(e) => common::telemetry::warn!("seed[ngac]: user→verctrl 关联自愈失败: {e}"),
    }
    // 4e. ct-git Issue/托管集合 OA（add-ct-git-hosting-issue）：issues:0 / tasks:0 / git:0
    //     恒定资源——admin 全动作、普通 user 只读（对齐 4d verctrl 模式）。
    for res in ["issues", "tasks", "git"] {
        if let Err(e) = ensure_collection_oa(pool, res, &format!("{res}-collection")).await {
            common::telemetry::warn!("seed[ngac]: {res} OA 自愈失败: {e}");
            continue;
        }
        match ensure_ua_collection_rights_multi(
            pool,
            "admin",
            res,
            &["create", "read", "update", "delete"],
        )
        .await
        {
            Ok(created) => {
                if created > 0 {
                    stats.healed += created as usize;
                }
            }
            Err(e) => common::telemetry::warn!("seed[ngac]: admin→{res} 关联自愈失败: {e}"),
        }
        match ensure_ua_collection_rights_multi(pool, "user", res, &["read"]).await {
            Ok(created) => {
                if created > 0 {
                    stats.healed += created as usize;
                }
            }
            Err(e) => common::telemetry::warn!("seed[ngac]: user→{res} 关联自愈失败: {e}"),
        }
    }
    // 4f. org 三集合 OA + ownership + operator 关联（cover-org-resources-ngac / B-2）：
    //     positions/departments/organizations:0 恒定集合 OA 是 identity-org
    //     ngac_org_ensure 行级 OA 的挂载点（缺失时其降级空祖先并 warn，网关自愈后收敛）。
    //     ownership 四件套：owner=operator（建岗挂人写端角色）、benefit/permit/access=employee
    //     （组织成员行级归属；四槽语义 = owner_cte 四 ak_* 生命周期字段）；
    //     operator → 三集合 read/create/update = B-2「operator 建岗挂人放行」授权面。
    //     例外（对齐 4d/4e 先例）：org 三资源族为 identity-org 域共享结构而非 namespace
    //     业务资源；本模块仅补缺失行（无 UPDATE/DELETE）。
    match ensure_org_resources(pool).await {
        Ok(created) => {
            if created > 0 {
                stats.healed += created as usize;
            }
        }
        Err(e) => common::telemetry::warn!("seed[ngac]: org 集合资源自愈失败: {e}"),
    }
    // 5. employee 基础门户关联（审批通过用户的最小授权：module/dashboard read）
    //    ——add-register-approval-closure 缺口 1：消除「审批通过即零资源死路」。
    match ensure_employee_base_associations(pool).await {
        Ok(created) => {
            if created > 0 {
                stats.healed += created as usize;
            }
        }
        Err(e) => common::telemetry::warn!("seed[ngac]: employee 基础关联自愈失败: {e}"),
    }

    stats
}

/// policy_class 'default'（仅补缺失）。
async fn ensure_policy_class(pool: &PgPool) -> Result<bool, sqlx::Error> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.ngac_policy_class WHERE o_name = 'default')",
    )
    .fetch_one(pool)
    .await?;
    if exists {
        return Ok(true);
    }

    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_policy_class (o_name, description, is_active)
           VALUES ('default', '默认策略类，所有现有用户归属', TRUE)"#,
    )
    .execute(pool)
    .await?;

    Ok(false)
}

/// UA 幂等确保（按 o_name；空层级，不触碰层级写一致性校验义务）。
async fn ensure_user_attribute(pool: &PgPool, name: &str, desc: &str) -> Result<bool, sqlx::Error> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL)",
    )
    .bind(name)
    .fetch_one(pool)
    .await?;
    if exists {
        return Ok(true);
    }

    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_attribute
           (id, o_name, fk_policy_class, ancestor_ids, children_ids, property)
           SELECT isahl.gen_next_zuid(), $1,
                  (SELECT id FROM isahl_auth.ngac_policy_class
                   WHERE o_name = 'default' LIMIT 1),
                  '{}'::bigint[], '{}'::bigint[], jsonb_build_object('description', $2)
           WHERE EXISTS (SELECT 1 FROM isahl_auth.ngac_policy_class WHERE o_name = 'default')"#,
    )
    .bind(name)
    .bind(desc)
    .execute(pool)
    .await?;

    Ok(false)
}

/// access_right 幂等确保。
async fn ensure_access_right(pool: &PgPool, name: &str) -> Result<bool, sqlx::Error> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.ngac_access_right WHERE o_name = $1)",
    )
    .bind(name)
    .fetch_one(pool)
    .await?;
    if exists {
        return Ok(true);
    }

    sqlx::query("INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ($1) ON CONFLICT (o_name) DO NOTHING")
        .bind(name)
        .execute(pool)
        .await?;

    Ok(false)
}

/// approval 因子 collection OA（fk_resource=0）。返回是否本次新建。
async fn ensure_collection_oa(
    pool: &PgPool,
    resource_type: &str,
    o_name: &str,
) -> Result<bool, sqlx::Error> {
    let exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (SELECT 1 FROM isahl_auth.ngac_object_attribute
           WHERE resource_type = $1 AND fk_resource = 0 AND deleted_at IS NULL)"#,
    )
    .bind(resource_type)
    .fetch_one(pool)
    .await?;
    if exists {
        return Ok(false);
    }

    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_object_attribute
           (id, o_name, fk_policy_class, resource_type, fk_resource, property)
           SELECT isahl.gen_next_zuid(), $1,
                  (SELECT id FROM isahl_auth.ngac_policy_class
                   WHERE o_name = 'default' LIMIT 1),
                  $2, 0, jsonb_build_object('description', 'approval 因子集合')
           WHERE EXISTS (SELECT 1 FROM isahl_auth.ngac_policy_class WHERE o_name = 'default')"#,
    )
    .bind(o_name)
    .bind(resource_type)
    .execute(pool)
    .await?;

    Ok(true)
}

/// admin UA → approval_* collection OA 全权关联（幂等，NOT EXISTS 防重复）。
/// 返回本次新建关联数。
async fn ensure_admin_associations(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let created = sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_association
            (fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights, created_at)
        SELECT ua.id, oa.id,
               (SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1),
               ARRAY(SELECT id FROM isahl_auth.ngac_access_right),
               NOW()
        FROM isahl_auth.ngac_user_attribute ua
        JOIN isahl_auth.ngac_object_attribute oa
          ON oa.resource_type IN ('approval_flows', 'approval_instances', 'approval_actions')
         AND oa.fk_resource = 0 AND oa.deleted_at IS NULL
        WHERE ua.o_name = 'admin' AND ua.deleted_at IS NULL
          AND NOT EXISTS (
              SELECT 1 FROM isahl_auth.ngac_association a
              WHERE a.fk_user_attribute = ua.id
                AND a.fk_object_attribute = oa.id
                AND a.deleted_at IS NULL
          )
        "#,
    )
    .execute(pool)
    .await?
    .rows_affected();

    Ok(created as i64)
}

/// employee UA → module/dashboard collection read 关联（幂等，add-register-approval-closure）。
/// 审批通过用户的最小授权：能看已部署模块列表与工作台骨架；
/// 不含任何业务资源（零 consignments/waybills 等）——业务权限由管理员另行授予。
/// 返回本次新建关联数。
async fn ensure_employee_base_associations(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let created = sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_association
            (fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights, created_at)
        SELECT ua.id, oa.id,
               (SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1),
               ARRAY(SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'read'),
               NOW()
        FROM isahl_auth.ngac_user_attribute ua
        JOIN isahl_auth.ngac_object_attribute oa
          ON oa.resource_type IN ('module', 'dashboard')
         AND oa.fk_resource = 0 AND oa.deleted_at IS NULL
        WHERE ua.o_name = 'employee' AND ua.deleted_at IS NULL
          AND NOT EXISTS (
              SELECT 1 FROM isahl_auth.ngac_association a
              WHERE a.fk_user_attribute = ua.id
                AND a.fk_object_attribute = oa.id
                AND a.deleted_at IS NULL
          )
        "#,
    )
    .execute(pool)
    .await?
    .rows_affected();

    Ok(created as i64)
}

/// 指定 UA → 指定 resource_type 的 collection:0 关联（幂等）。
/// rights=None → 全部 access_right；Some(name) → 仅该 right。
/// fix-approval-endpoint-gates：admin→approvals 全权 / user→approvals create。
/// 返回本次新建关联数。
async fn ensure_ua_collection_rights(
    pool: &PgPool,
    ua_name: &str,
    resource_type: &str,
    right: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let created = sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_association
            (fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights, created_at)
        SELECT ua.id, oa.id,
               (SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1),
               CASE WHEN $3::text IS NULL
                    THEN ARRAY(SELECT id FROM isahl_auth.ngac_access_right)
                    ELSE ARRAY(SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = $3)
               END,
               NOW()
        FROM isahl_auth.ngac_user_attribute ua
        JOIN isahl_auth.ngac_object_attribute oa
          ON oa.resource_type = $2 AND oa.fk_resource = 0 AND oa.deleted_at IS NULL
        WHERE ua.o_name = $1 AND ua.deleted_at IS NULL
          AND NOT EXISTS (
              SELECT 1 FROM isahl_auth.ngac_association a
              WHERE a.fk_user_attribute = ua.id
                AND a.fk_object_attribute = oa.id
                AND a.deleted_at IS NULL
          )
        "#,
    )
    .bind(ua_name)
    .bind(resource_type)
    .bind(right)
    .execute(pool)
    .await?
    .rows_affected();

    Ok(created as i64)
}

/// 指定 UA → 指定 resource_type 的 collection:0 多权限关联（幂等）。
/// rights 按 o_name 查 ngac_access_right id（`o_name = ANY($3)`），支持任选动作子集；
/// 与 ensure_ua_collection_rights 同构（NOT EXISTS 防重复）。
/// add-ct-git-vc-interop：verctrl:0 集合 OA 的 admin 全动作 / user 只读关联。
/// 返回本次新建关联数。
async fn ensure_ua_collection_rights_multi(
    pool: &PgPool,
    ua_name: &str,
    resource_type: &str,
    rights: &[&str],
) -> Result<i64, sqlx::Error> {
    let created = sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_association
            (fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights, created_at)
        SELECT ua.id, oa.id,
               (SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1),
               ARRAY(SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = ANY($3::text[])),
               NOW()
        FROM isahl_auth.ngac_user_attribute ua
        JOIN isahl_auth.ngac_object_attribute oa
          ON oa.resource_type = $2 AND oa.fk_resource = 0 AND oa.deleted_at IS NULL
        WHERE ua.o_name = $1 AND ua.deleted_at IS NULL
          AND NOT EXISTS (
              SELECT 1 FROM isahl_auth.ngac_association a
              WHERE a.fk_user_attribute = ua.id
                AND a.fk_object_attribute = oa.id
                AND a.deleted_at IS NULL
          )
        "#,
    )
    .bind(ua_name)
    .bind(resource_type)
    .bind(rights)
    .execute(pool)
    .await?
    .rows_affected();

    Ok(created as i64)
}

/// org 三集合资源 ensure（幂等，cover-org-resources-ngac / B-2）——与 4d/4e 同构的
/// 单一收口入口：集合 OA（fk_resource=0）+ ownership 属权策略 + operator UA 集合关联。
/// - OA：按 (resource_type, fk_resource=0) EXISTS 幂等，缺失补行（identity-org
///   ngac_org_ensure 行级 OA heal 的挂载点，模块 doc 约定由本模块预置）；
/// - ownership 四件套（同 Deploy/AVIC-CAASEC/seed/ngac.sql 结构）：owner=operator
///   （建岗挂人写端角色，created_by_id 归属）、benefit/permit/access=employee
///   （组织成员行级归属，对应 ak_benefit_user/ak_permit_user/ak_access_user 三列表
///   的 PDP owner_cte 推导）；NOT EXISTS（resource_type）防重复；
/// - 关联：operator → 三集合 read/create/update（INSERT..SELECT NOT EXISTS，
///   仿 ensure_ua_collection_rights_multi）——B-2「operator 建岗挂人放行」授权面。
/// UA 均来自 USER_ATTRIBUTES（ensure 步骤 2 先确保；此处兜底 ensure_user_attribute）。
/// 返回本次新建行数（OA + 属权策略 + 关联合计）。
async fn ensure_org_resources(pool: &PgPool) -> Result<i64, sqlx::Error> {
    for ua in ["operator", "employee"] {
        let desc = match ua {
            "operator" => "业务操作员",
            _ => "员工（注册审批通过授予）",
        };
        ensure_user_attribute(pool, ua, desc).await?;
    }

    let mut created: i64 = 0;

    // 1. 集合 OA（fk_resource=0；o_name `{rt}-collection`，对齐 4d/4e 命名）
    for res in ORG_COLLECTIONS {
        let rows = sqlx::query(
            r#"
            INSERT INTO isahl_auth.ngac_object_attribute
                (id, o_name, fk_policy_class, resource_type, fk_resource, property)
            SELECT isahl.gen_next_zuid(), $1,
                   (SELECT id FROM isahl_auth.ngac_policy_class
                    WHERE o_name = 'default' LIMIT 1),
                   $2, 0, jsonb_build_object('description', 'org 集合（identity-org 域共享）')
            WHERE NOT EXISTS (
                SELECT 1 FROM isahl_auth.ngac_object_attribute
                WHERE resource_type = $2 AND fk_resource = 0 AND deleted_at IS NULL
            )
            "#,
        )
        .bind(format!("{res}-collection"))
        .bind(res)
        .execute(pool)
        .await?
        .rows_affected();
        created += rows as i64;
    }

    // 2. ownership 属权策略（每 resource_type 一行）
    let rows = sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_ownership_policy
            (resource_type, owner_attr_id, benefit_attr_id, permit_attr_id, access_attr_id,
             read_right_id, write_right_id, delete_right_id, enabled)
        SELECT rt.resource_type,
               (SELECT id FROM isahl_auth.ngac_user_attribute
                WHERE o_name = 'operator' AND deleted_at IS NULL),
               (SELECT id FROM isahl_auth.ngac_user_attribute
                WHERE o_name = 'employee' AND deleted_at IS NULL),
               (SELECT id FROM isahl_auth.ngac_user_attribute
                WHERE o_name = 'employee' AND deleted_at IS NULL),
               (SELECT id FROM isahl_auth.ngac_user_attribute
                WHERE o_name = 'employee' AND deleted_at IS NULL),
               (SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'read'),
               (SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'write'),
               (SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = 'delete'),
               TRUE
        FROM unnest($1::text[]) AS rt(resource_type)
        WHERE NOT EXISTS (
            SELECT 1 FROM isahl_auth.ngac_ownership_policy op
            WHERE op.resource_type = rt.resource_type
        )
        "#,
    )
    .bind(ORG_COLLECTIONS)
    .execute(pool)
    .await?
    .rows_affected();
    created += rows as i64;

    // 3. operator → 三集合 read/create/update 关联（幂等复用多权限 helper）
    for res in ORG_COLLECTIONS {
        created += ensure_ua_collection_rights_multi(pool, "operator", res, &["read", "create", "update"])
            .await?;
    }

    Ok(created)
}
