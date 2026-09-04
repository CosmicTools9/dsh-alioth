//! 策略矩阵投影（`GET /api/admin/ngac/matrix` 的数据服务）。
//!
//! 语义对齐（change `refactor-ngac-policy-matrix` D3）：
//! - `direct`：精确 (UA, OA) 边上未删除 association 的原始 rights（PC 作用域内，
//!   含 conditions 未满足的也列出——前端仅做展示，编辑对象是这条边本身）。
//! - `effective` / `denied`：与 `/api/ngac/decide/explain` **同源**——对每个
//!   access right，按 (UA 祖先闭包 × OA 祖先闭包) deny-overrides 全扫描调用
//!   `Pdp::evaluate_pair`（fix-ngac-decision-consistency：任一对 Deny → `denied`，
//!   否则任一对 Permit → `effective`，遍历顺序不影响结果）；conditions 用
//!   单元格级 `ConditionContext`（用户 UA 闭包名 + OA 闭包名）求值。
//! - 祖先闭包由 `ancestor_ids` 直接父边迭代求可达集（`ancestor_closure`），与
//!   pip 的递归 CTE 同一可达集（集合内按 id 排序，与 `decide.rs` 的 `ORDER BY id` 一致）。
//! - 缓存：`(policy_class, ngac_policy_version)` 键 + 60s TTL；策略版本 bump
//!   （030 触发器自动 +1）即键变化，天然失效。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::*;
use crate::ngac::pip::PostgresPip;

/// 矩阵端点错误：区分 404（PC 不存在）与 500（加载/DB 失败）。
#[derive(Debug)]
pub enum MatrixError {
    PolicyClassNotFound(i64),
    Other(anyhow::Error),
}

impl std::fmt::Display for MatrixError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatrixError::PolicyClassNotFound(id) => {
                write!(f, "policy class {} not found", id)
            }
            MatrixError::Other(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for MatrixError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MatrixError::PolicyClassNotFound(_) => None,
            MatrixError::Other(e) => Some(e.as_ref()),
        }
    }
}

impl From<sqlx::Error> for MatrixError {
    fn from(e: sqlx::Error) -> Self {
        MatrixError::Other(e.into())
    }
}

impl From<anyhow::Error> for MatrixError {
    fn from(e: anyhow::Error) -> Self {
        MatrixError::Other(e)
    }
}

// ============================================================================
// 响应结构（端点契约：`openspec/changes/refactor-ngac-policy-matrix`）
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct MatrixPolicyClass {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct MatrixUserAttribute {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    /// 直接父属性 id（规范边；矩阵行头继承缩进由前端从本列表派生）。
    #[serde(with = "common::serde_zuid::seq")]
    pub ancestor_ids: Vec<i64>,
    /// `ngac_user_rr_attribute` 未删绑定行数（GROUP BY fk_user_attribute）。
    pub member_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatrixObjectAttribute {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_resource: Option<i64>,
    /// 业务可读标识（notice → code → 回退编号；NGAC_SPEC §2.2，add-ngac-oa-readable-identifier）。
    pub resource_identifier: Option<String>,
    /// 展示名（NGAC_SPEC §2.2 解析链；add-ngac-oa-display-name）。
    pub display_name: String,
    /// 直接父属性 id（规范边）。
    #[serde(with = "common::serde_zuid::seq")]
    pub ancestor_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatrixObjectGroup {
    pub resource_type: String,
    /// 资源域展示名（meta_collections.name → 内置映射 → 原 resource_type）。
    pub resource_type_display: String,
    /// 模块归属（add-ngac-oa-module-observability）：主要使用模块中文名；系统域 null。
    pub module_name: Option<String>,
    /// 模块归属：Gateway 模块路由前缀（前端跳转）；系统域 null。
    pub module_route: Option<String>,
    /// 模块归属：namespace；系统域 null。
    pub namespace: Option<String>,
    /// 页面预览（add-ngac-oa-preview，dev-only）：采集器截图 + 高亮 rect；未采集/未启用 null。
    /// 2026-08-21：矩阵列头模块徽章从跳转链接改为点击预览（不出戏），group 级携带。
    pub preview: Option<crate::ngac::display::OaPreviewInfo>,
    /// 集合级 OA（`fk_resource = 0`）为组首列；组内无集合 OA 时为 null。
    pub collection_oa: Option<MatrixObjectAttribute>,
    /// 实例级 OA（`fk_resource != 0`），折叠为计数徽章、可展开子列。
    pub instances: Vec<MatrixObjectAttribute>,
    /// 实例真实总数（`instances` 超 200 截断后仍以本字段汇报全量）。
    pub instance_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatrixCell {
    #[serde(with = "common::serde_zuid")]
    pub ua_id: i64,
    #[serde(with = "common::serde_zuid")]
    pub oa_id: i64,
    /// 精确边 association 的原始 rights（可编辑对象）。
    pub direct: Vec<String>,
    /// PDP 同源遍历得出的有效权限（含继承；只读虚显）。
    pub effective: Vec<String>,
    /// PDP 同源遍历命中的禁止集（红角标）。
    pub denied: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatrixAccessRight {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicyMatrix {
    pub policy_class: MatrixPolicyClass,
    pub user_attributes: Vec<MatrixUserAttribute>,
    pub object_groups: Vec<MatrixObjectGroup>,
    /// 稀疏：仅含 direct/effective/denied 任一非空的 (ua, oa) 对。
    pub cells: Vec<MatrixCell>,
    pub access_rights: Vec<MatrixAccessRight>,
    /// 当前 `ngac_policy_version`（缓存键的组成部分）。
    pub version: i64,
}

// ============================================================================
// 内存缓存：(policy_class, version) 键 + 60s TTL
// ============================================================================

const MATRIX_CACHE_TTL: Duration = Duration::from_secs(60);

struct CachedMatrix {
    at: Instant,
    data: Arc<PolicyMatrix>,
}

/// 进程级矩阵缓存（与 `GLOBAL_PDP` 同一生命周期）。
/// 策略版本 bump（写入路径经 030 触发器自动 +1）→ 键变化 → 缓存自动失效。
static MATRIX_CACHE: LazyLock<Mutex<HashMap<(i64, i64), CachedMatrix>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl Pdp {
    /// 计算 PC 作用域的矩阵投影。
    ///
    /// `ensure_policy_loaded` 先行（保证图快照与返回的 `version` 一致），
    /// 再以 `(policy_class, version)` 查缓存；未命中则全量计算并回填。
    pub async fn policy_matrix(
        &self,
        pip: &PostgresPip,
        pc_id: i64,
    ) -> Result<PolicyMatrix, MatrixError> {
        self.ensure_policy_loaded(pip).await?;
        let version = self.policy_version.load(Ordering::Acquire);

        {
            let cache = MATRIX_CACHE.lock().unwrap();
            if let Some(entry) = cache.get(&(pc_id, version)) {
                if entry.at.elapsed() < MATRIX_CACHE_TTL {
                    return Ok((*entry.data).clone());
                }
            }
        }

        let matrix = build_policy_matrix(self, pip, pc_id, version).await?;

        let mut cache = MATRIX_CACHE.lock().unwrap();
        // 惰性淘汰过期条目，防止长期运行下缓存无限增长。
        cache.retain(|_, e| e.at.elapsed() < MATRIX_CACHE_TTL);
        cache.insert(
            (pc_id, version),
            CachedMatrix {
                at: Instant::now(),
                data: Arc::new(matrix.clone()),
            },
        );
        Ok(matrix)
    }
}

/// 祖先闭包：从节点出发沿 `ancestor_ids`（直接父边）迭代求可达集，按 id 升序。
/// 与 pip 的 `WITH RECURSIVE ... ORDER BY id` 同一可达集与同一遍历顺序
/// （`decide.rs` 的 `decide_access`/`explain_access` 即按该序早停）。
pub(super) fn ancestor_closure(attr_id: i64, ancestors: &HashMap<i64, Vec<i64>>) -> Vec<i64> {
    let mut seen: HashSet<i64> = HashSet::new();
    let mut stack: Vec<i64> = vec![attr_id];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(parents) = ancestors.get(&id) {
            stack.extend(parents.iter().copied());
        }
    }
    let mut closure: Vec<i64> = seen.into_iter().collect();
    closure.sort_unstable();
    closure
}

async fn build_policy_matrix(
    pdp: &Pdp,
    pip: &PostgresPip,
    pc_id: i64,
    version: i64,
) -> Result<PolicyMatrix, MatrixError> {
    let pool = pip.pool();

    // 1. PC 元信息（不存在 → 404 语义）
    let pc: Option<(i64, String)> =
        sqlx::query_as("SELECT id, o_name FROM isahl_auth.ngac_policy_class WHERE id = $1")
            .bind(pc_id)
            .fetch_optional(pool)
            .await?;
    let (pc_id, pc_name) = match pc {
        Some(row) => row,
        None => return Err(MatrixError::PolicyClassNotFound(pc_id)),
    };

    // 2. UA（PC 作用域；NULL PC 行归入所选 PC 一并返回，JSON 保留 fk_policy_class: null）
    //    member_count：一条 GROUP BY 聚合（LEFT JOIN 子查询），未删绑定行数。
    let uas: Vec<MatrixUserAttribute> = sqlx::query_as::<_, MatrixUserAttribute>(
        r#"
        SELECT ua.id, ua.o_name, ua.fk_policy_class,
               COALESCE(ua.ancestor_ids, '{}'::bigint[]) AS ancestor_ids,
               COALESCE(mc.cnt, 0) AS member_count
        FROM isahl_auth.ngac_user_attribute ua
        LEFT JOIN (
            SELECT fk_user_attribute, COUNT(*)::BIGINT AS cnt
            FROM isahl_auth.ngac_user_rr_attribute
            WHERE deleted_at IS NULL
            GROUP BY fk_user_attribute
        ) mc ON mc.fk_user_attribute = ua.id
        WHERE ua.deleted_at IS NULL
          AND (ua.fk_policy_class = $1 OR ua.fk_policy_class IS NULL)
        ORDER BY ua.o_name, ua.id
        "#,
    )
    .bind(pc_id)
    .fetch_all(pool)
    .await?;

    // 3. OA（PC 作用域；resource_type 空值归 ""）
    let oa_rows: Vec<OaRow> = sqlx::query_as::<_, OaRow>(
        r#"
        SELECT id, o_name, fk_policy_class,
               COALESCE(ancestor_ids, '{}'::bigint[]) AS ancestor_ids,
               COALESCE(resource_type, '') AS resource_type,
               fk_resource,
               resource_identifier
        FROM isahl_auth.ngac_object_attribute
        WHERE deleted_at IS NULL
          AND (fk_policy_class = $1 OR fk_policy_class IS NULL)
        ORDER BY resource_type, id
        "#,
    )
    .bind(pc_id)
    .fetch_all(pool)
    .await?;

    // 3.5 展示名：一次批量 meta_collections 查询覆盖全部 resource_type
    //     （禁止 N+1；查询失败静默降级至内置映射/o_name——display.rs）
    let rt_set: std::collections::HashSet<String> =
        oa_rows.iter().map(|r| r.resource_type.clone()).collect();
    let meta_names = crate::ngac::display::meta_display_names(pool, &rt_set).await;
    let display_of = |row: &OaRow| {
        crate::ngac::display::resolve_display_name(
            row.fk_resource,
            row.resource_identifier.as_deref(),
            &row.resource_type,
            &meta_names,
            &row.o_name,
        )
    };

    // 4. 直接边：精确 (UA, OA) association（未删、PC 作用域）
    let direct_edges: Vec<(i64, i64, Vec<i64>)> = sqlx::query_as(
        r#"
        SELECT fk_user_attribute, fk_object_attribute, ak_access_rights
        FROM isahl_auth.ngac_association
        WHERE deleted_at IS NULL
          AND (fk_policy_class = $1 OR fk_policy_class IS NULL)
        "#,
    )
    .bind(pc_id)
    .fetch_all(pool)
    .await?;
    let mut direct_map: HashMap<(i64, i64), Vec<i64>> = HashMap::new();
    for (ua_id, oa_id, rights) in direct_edges {
        direct_map.entry((ua_id, oa_id)).or_default().extend(rights);
    }

    // 5. access right 词表（图快照内全量；按 id 稳定排序）
    let pg = pdp.policy_graph();
    let mut access_rights: Vec<MatrixAccessRight> = pg
        .access_rights
        .iter()
        .map(|e| MatrixAccessRight {
            id: *e.key(),
            o_name: e.value().o_name.clone(),
        })
        .collect();
    access_rights.sort_by_key(|a| a.id);
    let ar_names: HashMap<i64, String> = access_rights
        .iter()
        .map(|a| (a.id, a.o_name.clone()))
        .collect();
    let ua_names: HashMap<i64, String> = uas.iter().map(|u| (u.id, u.o_name.clone())).collect();

    // 6. 按 resource_type 分组：集合级（fk_resource=0）为组首列，实例级折叠
    let mut groups_map: BTreeMap<String, Vec<&OaRow>> = BTreeMap::new();
    for row in &oa_rows {
        groups_map
            .entry(row.resource_type.clone())
            .or_default()
            .push(row);
    }
    let mut object_groups: Vec<MatrixObjectGroup> = Vec::new();
    for (resource_type, rows_in_group) in groups_map {
        let resource_type_display =
            crate::ngac::display::resolve_resource_type_display(&resource_type, &meta_names);
        let mut collection_oa: Option<MatrixObjectAttribute> = None;
        let mut instances: Vec<MatrixObjectAttribute> = Vec::new();
        for row in rows_in_group {
            let oa = MatrixObjectAttribute {
                id: row.id,
                o_name: row.o_name.clone(),
                fk_policy_class: row.fk_policy_class,
                fk_resource: row.fk_resource,
                resource_identifier: row.resource_identifier.clone(),
                display_name: display_of(row),
                ancestor_ids: row.ancestor_ids.clone(),
            };
            if row.fk_resource == Some(0) {
                collection_oa = Some(oa);
            } else {
                instances.push(oa);
            }
        }
        instances.sort_by_key(|a| a.id);
        let instance_count = instances.len() as i64;
        instances.truncate(200);
        let (module_name, module_route, namespace) =
            crate::ngac::display::module_fields(&resource_type);
        object_groups.push(MatrixObjectGroup {
            resource_type,
            resource_type_display,
            module_name: module_name.map(String::from),
            module_route: module_route.map(String::from),
            namespace: namespace.map(String::from),
            preview: None, // handler 层填充（add-ngac-oa-preview，dev-only 文件数据不进缓存）
            collection_oa,
            instances,
            instance_count,
        });
    }

    // 7. 单元格：直接集 + 同源有效/禁止集（PDP evaluate_pair，禁止另写匹配逻辑）
    let oas: Vec<MatrixObjectAttribute> = oa_rows
        .into_iter()
        .map(|r| MatrixObjectAttribute {
            id: r.id,
            o_name: r.o_name.clone(),
            fk_policy_class: r.fk_policy_class,
            fk_resource: r.fk_resource,
            resource_identifier: r.resource_identifier.clone(),
            display_name: display_of(&r),
            ancestor_ids: r.ancestor_ids.clone(),
        })
        .collect();
    let oa_names: HashMap<i64, String> = oas.iter().map(|o| (o.id, o.o_name.clone())).collect();
    let ua_ancestors: HashMap<i64, Vec<i64>> =
        uas.iter().map(|u| (u.id, u.ancestor_ids.clone())).collect();
    let oa_ancestors: HashMap<i64, Vec<i64>> =
        oas.iter().map(|o| (o.id, o.ancestor_ids.clone())).collect();
    let oa_closures: HashMap<i64, Vec<i64>> = oas
        .iter()
        .map(|o| (o.id, ancestor_closure(o.id, &oa_ancestors)))
        .collect();

    let mut cells: Vec<MatrixCell> = Vec::new();
    for ua in &uas {
        let ua_closure = ancestor_closure(ua.id, &ua_ancestors);
        // v2 条件上下文（add-ngac-condition-v2）：cell 级用户 UA 闭包名 + OA 闭包名
        let user_ua_names: Vec<String> = ua_closure
            .iter()
            .filter_map(|id| ua_names.get(id).cloned())
            .collect();
        for oa in &oas {
            let oa_closure = ancestor_closure(oa.id, &oa_ancestors);
            let ctx = ConditionContext {
                now: Utc::now(),
                user_ua_names: user_ua_names.clone(),
                oa_closure_names: oa_closure
                    .iter()
                    .filter_map(|id| oa_names.get(id).cloned())
                    .collect(),
            };
            let mut direct: Vec<String> = Vec::new();
            if let Some(right_ids) = direct_map.get(&(ua.id, oa.id)) {
                let mut names: Vec<String> = right_ids
                    .iter()
                    .filter_map(|id| ar_names.get(id).cloned())
                    .collect();
                names.sort();
                names.dedup();
                direct = names;
            }

            let mut effective: Vec<String> = Vec::new();
            let mut denied: Vec<String> = Vec::new();
            for ar in &access_rights {
                // deny-overrides 全扫描（与 decide_access 同语义）：任一 (ua,oa) 对
                // Deny → denied；否则任一 Permit → effective；顺序不影响结果。
                let mut saw_deny = false;
                let mut saw_permit = false;
                for &ua_c in &ua_closure {
                    for &oa_c in oa_closures.get(&oa.id).expect("oa closure present") {
                        match pdp.evaluate_pair(ua_c, oa_c, &ar.o_name, &ctx).0 {
                            Decision::Deny => saw_deny = true,
                            Decision::Permit => saw_permit = true,
                            Decision::NotApplicable => {}
                        }
                    }
                }
                if saw_deny {
                    denied.push(ar.o_name.clone());
                } else if saw_permit {
                    effective.push(ar.o_name.clone());
                }
            }

            if !direct.is_empty() || !effective.is_empty() || !denied.is_empty() {
                cells.push(MatrixCell {
                    ua_id: ua.id,
                    oa_id: oa.id,
                    direct,
                    effective,
                    denied,
                });
            }
        }
    }

    Ok(PolicyMatrix {
        policy_class: MatrixPolicyClass {
            id: pc_id,
            o_name: pc_name,
        },
        user_attributes: uas,
        object_groups,
        cells,
        access_rights,
        version,
    })
}

/// OA 查询行（含分组所需的 resource_type；响应对象不携带该字段）。
#[derive(Debug, Clone, sqlx::FromRow)]
struct OaRow {
    id: i64,
    o_name: String,
    fk_policy_class: Option<i64>,
    ancestor_ids: Vec<i64>,
    resource_type: String,
    fk_resource: Option<i64>,
    resource_identifier: Option<String>,
}
