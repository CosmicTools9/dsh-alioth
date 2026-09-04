//! identity-org NGAC org 资源行级 OA ensure（cover-org-resources-ngac / B-2）
//!
//! org 写端（org_tree handlers）在业务事务提交后调用本组件的幂等 heal 入口；
//! 失败仅 warn、绝不阻断主写（对齐 Gateway ensure_cognition_uas / ngac_seed
//! 的运行期自愈容错语义）。全部函数可安全重放：先查后插 / ON CONFLICT DO NOTHING /
//! `ancestor_ids IS DISTINCT FROM` 免写。
//!
//! 维护两族行级对象属性（OA）：
//! - `positions`：岗位行 OA（resource_type='positions'，fk_resource=岗位行 id）；
//! - `departments`：部门/组织树节点行 OA（resource_type='departments'，fk_resource=org 行 id，
//!   双叶表 zc_id_orga-department ∪ zc_id_orga-non-banking-legal 统一按 org 行解析）。
//!
//! 层级规则（B-2 设计 §3 b/c；NGAC_SPEC §2.2 OA 闭包）：
//! - 部门子集 OA 树：节点 OA.ancestor_ids = 直接上级节点 OA id（org_rr_subordinate
//!   ref_left=上级；多父取最小桥 id 主链，与 DEPARTMENT_SELECT 读模型一致），
//!   根节点挂 `departments:0` 集合 OA id；
//! - 岗位行 OA：ancestor_ids = 全部在任分配部门 OA id（org_rr_position ref_left=部门；
//!   多部门并存则各域闭包独立放行），未分配 → 挂 `positions:0` 集合 OA id（兜底）。
//!   集合 OA（fk_resource=0）由 Gateway 通用层种子 ngac_seed 预置（cover-org-resources-ngac）；
//!   缺失时本组件降级写空祖先并 warn 说明——网关 seed 自愈后随下次 heal 收敛。
//!
//! 边界：域 association（operator/类别 UA → 部门 OA）维护与 delete/update/org-tree
//! 写端的全量收束属 Phase C「写端收束统一 service」；本组件当前挂接点 =
//! create_department / add_org_tree_child（heal_department_scope）/
//! create_position / assign_position_to_department / add_position_employee /
//! remove_position_from_department（heal_position_scope）——行 OA + 层级 ensure，事务外。

use sqlx::PgPool;

/// 部门树主链爬升深度上限（防脏数据环死循环；PDP 递归闭包自身限深 10 层，
/// 本上限仅防桥表脏数据导致的无限爬升）
const DEPTH_CAP: usize = 32;

/// isahl_auth schema 不存在（未启用 NGAC 的部署）→ heal 跳过且不告警。
async fn ngac_enabled(pool: &PgPool) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT FROM information_schema.schemata WHERE schema_name = 'isahl_auth')",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(false)
}

/// org 行业务标识 best-effort（notice → code；两 org 叶表合并解析；
/// 行不存在/已软删 → None；均空 → Some("") 由调用方按 '{rt}-{id}' 兜底）。
async fn org_identifier(pool: &PgPool, org_id: i64) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT COALESCE(NULLIF(notice::text, ''), NULLIF(code::text, ''), '')
           FROM isahl."zc_id_orga-department" WHERE id = $1 AND deleted_at IS NULL
           UNION ALL
           SELECT COALESCE(NULLIF(notice::text, ''), NULLIF(code::text, ''), '')
           FROM isahl."zc_id_orga-non-banking-legal" WHERE id = $1 AND deleted_at IS NULL
           LIMIT 1"#,
    )
    .bind(org_id)
    .fetch_optional(pool)
    .await
}

/// 岗位行业务标识 best-effort（notice → code；不存在/已软删 → None）。
async fn position_identifier(pool: &PgPool, position_id: i64) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT COALESCE(NULLIF(notice::text, ''), NULLIF(code::text, ''), '')
           FROM isahl."zc_id_subj-position" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(position_id)
    .fetch_optional(pool)
    .await
}

/// 集合 OA id（fk_resource=0；Gateway ngac_seed 预置）。
async fn collection_oa_id(pool: &PgPool, resource_type: &str) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute \
         WHERE resource_type = $1 AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(resource_type)
    .fetch_optional(pool)
    .await
}

/// 直接上级 org id（org_rr_subordinate 主链：多父取最小桥 id，与 DEPARTMENT_SELECT
/// 一致；上级须存活于两 org 叶表之一，否则视作根）。ref_left=上级 / ref_right=下属。
async fn parent_org_id(pool: &PgPool, org_id: i64) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT r.ref_left FROM isahl."zc_id_subj-org_rr_subordinate" r
           WHERE r.ref_right = $1 AND r.deleted_at IS NULL
             AND EXISTS (
                 SELECT 1 FROM isahl."zc_id_orga-department" d
                 WHERE d.id = r.ref_left AND d.deleted_at IS NULL
                 UNION ALL
                 SELECT 1 FROM isahl."zc_id_orga-non-banking-legal" d
                 WHERE d.id = r.ref_left AND d.deleted_at IS NULL
             )
           ORDER BY r.id LIMIT 1"#,
    )
    .bind(org_id)
    .fetch_optional(pool)
    .await
}

/// 行级 OA upsert（仿 crud::handler::register_created_resource_ngac：o_name `{rt}-{id}`、
/// 首个 policy class、resource_identifier；created_by_id=NULL=系统 ensure）。返回 OA id；
/// 唯一键被软删幽灵行占据等退化场景 → None（放弃本次，留待 Phase C 清理）。
async fn ensure_row_oa(
    pool: &PgPool,
    resource_type: &str,
    row_id: i64,
    identifier: &str,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_object_attribute
           (o_name, fk_policy_class, resource_type, fk_resource, resource_identifier, created_by_id)
           VALUES ($1, (SELECT id FROM isahl_auth.ngac_policy_class LIMIT 1), $2, $3, $4, NULL)
           ON CONFLICT (resource_type, fk_resource) DO NOTHING"#,
    )
    .bind(format!("{resource_type}-{row_id}"))
    .bind(resource_type)
    .bind(row_id)
    .bind(identifier)
    .execute(pool)
    .await?;
    sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute \
         WHERE resource_type = $1 AND fk_resource = $2 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(resource_type)
    .bind(row_id)
    .fetch_optional(pool)
    .await
}

/// ancestor_ids 差量回填（`IS DISTINCT FROM` 免写；软删行不触碰）。
async fn set_ancestors(pool: &PgPool, oa_id: i64, ancestors: &[i64]) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"UPDATE isahl_auth.ngac_object_attribute
           SET ancestor_ids = $2::bigint[], updated_at = NOW()
           WHERE id = $1 AND deleted_at IS NULL
             AND ancestor_ids IS DISTINCT FROM $2::bigint[]"#,
    )
    .bind(oa_id)
    .bind(ancestors)
    .execute(pool)
    .await?;
    Ok(())
}

/// 部门子集 OA 树 ensure：自底向上沿 org_rr_subordinate 主链收集节点（含自身），
/// 自上而下逐节点保证行 OA 存在并回填 ancestor_ids（根挂 departments:0 集合 OA id；
/// 集合缺失 → 空祖先，warn 说明）。返回本节点 OA id（节点不可达 → None）。
async fn ensure_department_chain(pool: &PgPool, org_id: i64) -> Result<Option<i64>, sqlx::Error> {
    let mut chain: Vec<i64> = vec![org_id];
    let mut cur = org_id;
    for _ in 0..DEPTH_CAP {
        match parent_org_id(pool, cur).await? {
            Some(parent) => {
                chain.push(parent);
                cur = parent;
            }
            None => break,
        }
    }
    let collection_oa = collection_oa_id(pool, "departments").await?;
    if collection_oa.is_none() {
        // Gateway 通用层种子未就绪（启动竞态/降级部署）——行 OA 树暂孤立，
        // 根节点空祖先（fail-closed），seed 自愈后由后续 heal 收敛。
        common::telemetry::warn!(
            "ngac_org_ensure: departments:0 集合 OA 缺失（Gateway ngac_seed 未自愈）——部门子集树暂挂空祖先"
        );
    }
    // chain 逆序 = 顶层（根）→ 本节点；parent_anchor 跟踪上一级（更近根）OA id
    let mut parent_anchor: Option<i64> = collection_oa;
    let mut own_oa: Option<i64> = None;
    for &node in chain.iter().rev() {
        let node_oa = match org_identifier(pool, node).await? {
            Some(ident) => ensure_row_oa(pool, "departments", node, &ident).await?,
            None => None, // 节点已删/不存在：跳过，子级视作根续挂
        };
        if let Some(oa) = node_oa {
            let ancestors: Vec<i64> = parent_anchor.into_iter().collect();
            set_ancestors(pool, oa, &ancestors).await?;
            parent_anchor = node_oa;
            own_oa = node_oa;
        }
    }
    Ok(own_oa)
}

/// 岗位行 OA + 层级 ensure：行 OA 存在 → ancestor_ids = 在任分配部门 OA 列表
/// （多部门并存 = 多父，各域闭包独立放行）；未分配 → positions:0 集合 OA id。
async fn ensure_position_scope(pool: &PgPool, position_id: i64) -> Result<(), sqlx::Error> {
    let Some(identifier) = position_identifier(pool, position_id).await? else {
        return Ok(()); // 岗位不存在/已软删：heal 无对象（主写已在别处处理）
    };
    let Some(oa_id) = ensure_row_oa(pool, "positions", position_id, &identifier).await? else {
        return Ok(()); // 退化场景（唯一键被幽灵行占据）：放弃本次，Phase C 清理
    };
    let dept_ids: Vec<i64> = sqlx::query_scalar(
        r#"SELECT ref_left FROM isahl."zc_id_subj-org_rr_position"
           WHERE ref_right = $1 AND deleted_at IS NULL ORDER BY id"#,
    )
    .bind(position_id)
    .fetch_all(pool)
    .await?;
    let mut ancestors: Vec<i64> = Vec::new();
    for dept_id in dept_ids {
        if let Some(dept_oa) = ensure_department_chain(pool, dept_id).await? {
            ancestors.push(dept_oa);
        }
    }
    if ancestors.is_empty() {
        // 未分配部门（或分配部门均不可达）→ 挂 positions:0 集合（B-2 兜底）；
        // 集合缺失时保持空祖先（fail-closed），由 seed 自愈后的 heal 收敛。
        match collection_oa_id(pool, "positions").await? {
            Some(coll) => ancestors.push(coll),
            None => common::telemetry::warn!(
                "ngac_org_ensure: positions:0 集合 OA 缺失——岗位 {position_id} 暂挂空祖先"
            ),
        }
    }
    set_ancestors(pool, oa_id, &ancestors).await?;
    Ok(())
}

/// 岗位 heal 入口（事务外幂等）：NGAC 未启用/任何失败 → 仅 warn，绝不阻断 org 主写。
pub async fn heal_position_scope(pool: &PgPool, position_id: i64) {
    if !ngac_enabled(pool).await {
        return;
    }
    if let Err(e) = ensure_position_scope(pool, position_id).await {
        common::telemetry::warn!("ngac_org_ensure: 岗位 {position_id} 行 OA ensure 失败: {e}");
    }
}

/// 部门 heal 入口（事务外幂等）：部门/org 树节点行 OA + 部门子集 OA 树链。
pub async fn heal_department_scope(pool: &PgPool, org_id: i64) {
    if !ngac_enabled(pool).await {
        return;
    }
    if let Err(e) = ensure_department_chain(pool, org_id).await {
        common::telemetry::warn!("ngac_org_ensure: 部门 {org_id} 子集 OA ensure 失败: {e}");
    }
}
