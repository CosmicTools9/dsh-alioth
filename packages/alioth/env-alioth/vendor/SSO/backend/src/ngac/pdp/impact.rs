//! 删除影响预览（`GET /api/admin/ngac/impact-preview` 的数据服务）。
//!
//! 语义对齐（change `add-ngac-audit-trail-view` D2，含审计 W-4 修正）：
//! - 克隆当前 PolicyGraph，按 entity_type 移除目标边/节点关联；受影响对
//!   **精确收敛于被删边集合**（`evaluate_pair_in` 仅匹配精确 (ua,oa) 行），
//!   UA/OA 后代仅用于受影响**主体与用户**的映射，不扩大求值对集。
//! - UA/OA **节点**删除会改变后代的祖先闭包（继承路径断开）：受影响主体的
//!   before/after 闭包分别在原祖先边集与"剔除目标节点"的边集上独立求。
//! - before/after 双图比对（同一 `evaluate_pair_in` + deny-overrides 合并，
//!   fix-ngac-decision-consistency），状态迁移记录 lost_allow（Permit→其他）/
//!   lost_deny（Deny→其他）。
//! - 上限：受影响对 × access rights 求值次数 > 5000 → `truncated: true`。
//! - 已知盲区（W-5）：按当前时刻求值（`ConditionContext::default()`）——
//!   删除尚未到 not_before 生效窗的边，其未来授权不体现在 lost_allow 中。

use std::collections::{HashMap, HashSet, VecDeque};

use serde::Serialize;

use super::matrix::ancestor_closure;
use super::*;
use crate::ngac::pip::PostgresPip;

/// 预览端点错误：400（entity_type 非法）/ 404（实体不存在或已删）/ 500。
#[derive(Debug)]
pub enum ImpactError {
    InvalidEntityType(String),
    NotFound(&'static str, i64),
    Other(anyhow::Error),
}

impl std::fmt::Display for ImpactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImpactError::InvalidEntityType(t) => write!(f, "invalid entity_type: {}", t),
            ImpactError::NotFound(t, id) => write!(f, "{} {} not found", t, id),
            ImpactError::Other(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for ImpactError {}

impl From<sqlx::Error> for ImpactError {
    fn from(e: sqlx::Error) -> Self {
        ImpactError::Other(e.into())
    }
}

impl From<anyhow::Error> for ImpactError {
    fn from(e: anyhow::Error) -> Self {
        ImpactError::Other(e)
    }
}

// ============================================================================
// 响应结构（端点契约：`openspec/changes/add-ngac-audit-trail-view`）
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct ImpactEntity {
    pub entity_type: String,
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImpactAffectedUser {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImpactAffected {
    #[serde(with = "common::serde_zuid")]
    pub ua_id: i64,
    pub o_name: String,
    pub resource_type: String,
    /// 资源域展示名（NGAC_SPEC §2.2 解析链；add-ngac-oa-display-name）。
    pub resource_type_display: String,
    pub lost_allow: Vec<String>,
    pub lost_deny: Vec<String>,
    pub users: Vec<ImpactAffectedUser>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImpactPreview {
    pub entity: ImpactEntity,
    /// 稀疏：仅含存在状态迁移的 (subject UA × resource_type) 条目。
    pub affected: Vec<ImpactAffected>,
    pub truncated: bool,
}

/// 求值次数上限（受影响对 × access rights），超出即截断。
const MAX_EVALUATIONS: usize = 5000;

/// 待移除的边（association 或 prohibition）。
#[derive(Debug, Clone, Copy)]
struct RemovedEdge {
    is_prohibition: bool,
    id: i64,
    ua_id: i64,
    oa_id: i64,
}

/// 后代闭包（含自身）：ancestor_ids 为直接父边 → 反转为子边做 BFS。
fn descendant_closure_inclusive(attr_id: i64, ancestors: &HashMap<i64, Vec<i64>>) -> Vec<i64> {
    let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
    for (child, parents) in ancestors {
        for &p in parents {
            children.entry(p).or_default().push(*child);
        }
    }
    let mut seen: HashSet<i64> = HashSet::new();
    let mut queue: VecDeque<i64> = VecDeque::from([attr_id]);
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(kids) = children.get(&id) {
            queue.extend(kids.iter().copied());
        }
    }
    let mut out: Vec<i64> = seen.into_iter().collect();
    out.sort_unstable();
    out
}

/// deny-overrides 合并决策（与 decide.rs 同语义，fix-ngac-decision-consistency）：
/// 任一对 Deny → Some(false)；否则任一对 Permit → Some(true)；全不适用 → None。
fn deny_overrides_in(
    pdp: &Pdp,
    pg: &PolicyGraph,
    ua_closure: &[i64],
    oa_closure: &[i64],
    access_right: &str,
    ctx: &ConditionContext,
) -> Option<bool> {
    let mut saw_permit = false;
    for &ua_c in ua_closure {
        for &oa_c in oa_closure {
            match pdp.evaluate_pair_in(pg, ua_c, oa_c, access_right, ctx).0 {
                Decision::Deny => return Some(false),
                Decision::Permit => saw_permit = true,
                Decision::NotApplicable => {}
            }
        }
    }
    if saw_permit {
        Some(true)
    } else {
        None
    }
}

impl Pdp {
    /// 删除影响预览：模拟删除目标实体，返回有效权限差异。
    pub async fn impact_preview(
        &self,
        pip: &PostgresPip,
        entity_type: &str,
        entity_id: i64,
    ) -> Result<ImpactPreview, ImpactError> {
        self.ensure_policy_loaded(pip).await?;
        let pg_before = self.policy_graph();
        let pool = pip.pool();

        // 1. 目标实体解析 + 待移除边枚举
        let (removed_edges, excised_ua, excised_oa, label) = match entity_type {
            "association" => {
                let row: Option<(i64, i64)> = sqlx::query_as(
                    "SELECT fk_user_attribute, fk_object_attribute \
                     FROM isahl_auth.ngac_association WHERE id = $1 AND deleted_at IS NULL",
                )
                .bind(entity_id)
                .fetch_optional(pool)
                .await?;
                let (ua, oa) = row.ok_or(ImpactError::NotFound("association", entity_id))?;
                let label = format!("association {} ({} → {})", entity_id, ua, oa);
                (
                    vec![RemovedEdge {
                        is_prohibition: false,
                        id: entity_id,
                        ua_id: ua,
                        oa_id: oa,
                    }],
                    None,
                    None,
                    label,
                )
            }
            "prohibition" => {
                let row: Option<(Option<i64>, Option<i64>)> = sqlx::query_as(
                    "SELECT fk_user_attribute, fk_object_attribute \
                     FROM isahl_auth.ngac_prohibition WHERE id = $1 AND deleted_at IS NULL",
                )
                .bind(entity_id)
                .fetch_optional(pool)
                .await?;
                let (ua, oa) = row.ok_or(ImpactError::NotFound("prohibition", entity_id))?;
                let edges = match (ua, oa) {
                    (Some(u), Some(o)) => vec![RemovedEdge {
                        is_prohibition: true,
                        id: entity_id,
                        ua_id: u,
                        oa_id: o,
                    }],
                    _ => vec![],
                };
                (edges, None, None, format!("prohibition {}", entity_id))
            }
            "user_attribute" => {
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM isahl_auth.ngac_user_attribute \
                     WHERE id = $1 AND deleted_at IS NULL)",
                )
                .bind(entity_id)
                .fetch_one(pool)
                .await?;
                if !exists {
                    return Err(ImpactError::NotFound("user_attribute", entity_id));
                }
                let name: String = sqlx::query_scalar(
                    "SELECT o_name FROM isahl_auth.ngac_user_attribute WHERE id = $1",
                )
                .bind(entity_id)
                .fetch_one(pool)
                .await?;
                let assocs: Vec<(i64, i64)> = sqlx::query_as(
                    "SELECT id, fk_object_attribute FROM isahl_auth.ngac_association \
                     WHERE fk_user_attribute = $1 AND deleted_at IS NULL",
                )
                .bind(entity_id)
                .fetch_all(pool)
                .await?;
                let prohs: Vec<(i64, i64)> = sqlx::query_as(
                    "SELECT id, fk_object_attribute FROM isahl_auth.ngac_prohibition \
                     WHERE fk_user_attribute = $1 AND deleted_at IS NULL AND is_active",
                )
                .bind(entity_id)
                .fetch_all(pool)
                .await?;
                let mut edges: Vec<RemovedEdge> = assocs
                    .into_iter()
                    .map(|(id, oa)| RemovedEdge {
                        is_prohibition: false,
                        id,
                        ua_id: entity_id,
                        oa_id: oa,
                    })
                    .collect();
                edges.extend(prohs.into_iter().map(|(id, oa)| RemovedEdge {
                    is_prohibition: true,
                    id,
                    ua_id: entity_id,
                    oa_id: oa,
                }));
                (
                    edges,
                    Some(entity_id),
                    None,
                    format!("user_attribute {}", name),
                )
            }
            "object_attribute" => {
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM isahl_auth.ngac_object_attribute \
                     WHERE id = $1 AND deleted_at IS NULL)",
                )
                .bind(entity_id)
                .fetch_one(pool)
                .await?;
                if !exists {
                    return Err(ImpactError::NotFound("object_attribute", entity_id));
                }
                let name: String = sqlx::query_scalar(
                    "SELECT o_name FROM isahl_auth.ngac_object_attribute WHERE id = $1",
                )
                .bind(entity_id)
                .fetch_one(pool)
                .await?;
                let assocs: Vec<(i64, i64)> = sqlx::query_as(
                    "SELECT id, fk_user_attribute FROM isahl_auth.ngac_association \
                     WHERE fk_object_attribute = $1 AND deleted_at IS NULL",
                )
                .bind(entity_id)
                .fetch_all(pool)
                .await?;
                let prohs: Vec<(i64, i64)> = sqlx::query_as(
                    "SELECT id, fk_user_attribute FROM isahl_auth.ngac_prohibition \
                     WHERE fk_object_attribute = $1 AND deleted_at IS NULL AND is_active",
                )
                .bind(entity_id)
                .fetch_all(pool)
                .await?;
                let mut edges: Vec<RemovedEdge> = assocs
                    .into_iter()
                    .map(|(id, ua)| RemovedEdge {
                        is_prohibition: false,
                        id,
                        ua_id: ua,
                        oa_id: entity_id,
                    })
                    .collect();
                edges.extend(prohs.into_iter().map(|(id, ua)| RemovedEdge {
                    is_prohibition: true,
                    id,
                    ua_id: ua,
                    oa_id: entity_id,
                }));
                (
                    edges,
                    None,
                    Some(entity_id),
                    format!("object_attribute {}", name),
                )
            }
            other => return Err(ImpactError::InvalidEntityType(other.to_string())),
        };

        // 2. 模拟图：克隆 + 移除目标边
        let pg_after = (*pg_before).clone();
        for edge in &removed_edges {
            if edge.is_prohibition {
                pg_after.remove_prohibition(edge.id);
            } else {
                pg_after.remove_association(edge.id);
            }
        }

        // 3. 祖先边集（before/after 双份；after 剔除被删节点）
        let ua_rows: Vec<(i64, String, Vec<i64>)> = sqlx::query_as(
            "SELECT id, o_name, COALESCE(ancestor_ids, '{}'::bigint[]) \
             FROM isahl_auth.ngac_user_attribute WHERE deleted_at IS NULL",
        )
        .fetch_all(pool)
        .await?;
        let oa_rows: Vec<(i64, String, Option<String>, Vec<i64>)> = sqlx::query_as(
            "SELECT id, o_name, resource_type, COALESCE(ancestor_ids, '{}'::bigint[]) \
             FROM isahl_auth.ngac_object_attribute WHERE deleted_at IS NULL",
        )
        .fetch_all(pool)
        .await?;

        let ua_names: HashMap<i64, String> =
            ua_rows.iter().map(|(id, n, _)| (*id, n.clone())).collect();
        let oa_meta: HashMap<i64, (String, String)> = oa_rows
            .iter()
            .map(|(id, n, rt, _)| (*id, (n.clone(), rt.clone().unwrap_or_default())))
            .collect();

        let ua_before: HashMap<i64, Vec<i64>> =
            ua_rows.iter().map(|(id, _, a)| (*id, a.clone())).collect();
        let oa_before: HashMap<i64, Vec<i64>> = oa_rows
            .iter()
            .map(|(id, _, _, a)| (*id, a.clone()))
            .collect();
        // 节点删除 → after 边集剔除目标节点（其后代继承路径断开）
        let ua_after: HashMap<i64, Vec<i64>> = match excised_ua {
            Some(target) => ua_rows
                .iter()
                .filter(|(id, _, _)| *id != target)
                .map(|(id, _, a)| (*id, a.iter().copied().filter(|p| *p != target).collect()))
                .collect(),
            None => ua_before.clone(),
        };
        let oa_after: HashMap<i64, Vec<i64>> = match excised_oa {
            Some(target) => oa_rows
                .iter()
                .filter(|(id, _, _, _)| *id != target)
                .map(|(id, _, _, a)| (*id, a.iter().copied().filter(|p| *p != target).collect()))
                .collect(),
            None => oa_before.clone(),
        };

        // 4. 受影响主体 × 被删边求值比对
        let ctx = ConditionContext::default();
        let access_rights: Vec<String> = pg_before
            .access_rights
            .iter()
            .map(|e| e.value().o_name.clone())
            .collect();

        let mut evaluations = 0usize;
        let mut truncated = false;
        // (subject_ua, resource_type) → (lost_allow, lost_deny)
        let mut affected_map: HashMap<(i64, String), (Vec<String>, Vec<String>)> = HashMap::new();

        'outer: for edge in &removed_edges {
            let resource_type = oa_meta
                .get(&edge.oa_id)
                .map(|(_, rt)| rt.clone())
                .unwrap_or_default();
            // 受影响主体 = 边的 UA 端及其后代（before 边集上的继承方向）
            for subject in descendant_closure_inclusive(edge.ua_id, &ua_before) {
                let ua_cl_before = ancestor_closure(subject, &ua_before);
                let ua_cl_after = ancestor_closure(subject, &ua_after);
                let oa_cl_before = ancestor_closure(edge.oa_id, &oa_before);
                let oa_cl_after = ancestor_closure(edge.oa_id, &oa_after);

                for ar in &access_rights {
                    evaluations += 1;
                    if evaluations > MAX_EVALUATIONS {
                        truncated = true;
                        break 'outer;
                    }
                    let before =
                        deny_overrides_in(self, &pg_before, &ua_cl_before, &oa_cl_before, ar, &ctx);
                    let after =
                        deny_overrides_in(self, &pg_after, &ua_cl_after, &oa_cl_after, ar, &ctx);
                    if before == after {
                        continue;
                    }
                    let entry = affected_map
                        .entry((subject, resource_type.clone()))
                        .or_default();
                    match (before, after) {
                        (Some(true), _) => entry.0.push(ar.clone()),
                        (Some(false), _) => entry.1.push(ar.clone()),
                        _ => {}
                    }
                }
            }
        }

        // 5. 组装 + 受影响用户解析（未删、未过期）
        // 展示名：受影响 resource_type 集一次批量 meta_collections 查询（禁止 N+1）
        let rt_set: std::collections::HashSet<String> =
            affected_map.keys().map(|(_, rt)| rt.clone()).collect();
        let meta_names = crate::ngac::display::meta_display_names(pool, &rt_set).await;
        let mut affected: Vec<ImpactAffected> = Vec::new();
        for ((subject, resource_type), (mut lost_allow, mut lost_deny)) in affected_map {
            lost_allow.sort();
            lost_allow.dedup();
            lost_deny.sort();
            lost_deny.dedup();
            let users: Vec<ImpactAffectedUser> = sqlx::query_as::<_, (i64, Option<String>)>(
                r#"
                SELECT u.id, u.username
                FROM isahl_auth.ngac_user_rr_attribute ur
                JOIN isahl_auth.auth_users u ON u.id = ur.fk_user
                WHERE ur.fk_user_attribute = $1 AND ur.deleted_at IS NULL
                  AND (ur.expires_at IS NULL OR ur.expires_at > NOW())
                ORDER BY u.id
                "#,
            )
            .bind(subject)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|(id, username)| ImpactAffectedUser { id, username })
            .collect();
            affected.push(ImpactAffected {
                ua_id: subject,
                o_name: ua_names.get(&subject).cloned().unwrap_or_default(),
                resource_type: resource_type.clone(),
                resource_type_display: crate::ngac::display::resolve_resource_type_display(
                    &resource_type,
                    &meta_names,
                ),
                lost_allow,
                lost_deny,
                users,
            });
        }
        affected.sort_by(|a, b| {
            a.ua_id
                .cmp(&b.ua_id)
                .then(a.resource_type.cmp(&b.resource_type))
        });

        Ok(ImpactPreview {
            entity: ImpactEntity {
                entity_type: entity_type.to_string(),
                id: entity_id,
                label,
            },
            affected,
            truncated,
        })
    }
}
