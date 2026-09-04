//! 访问审查投影（`GET /api/admin/ngac/review/user/{id}` 与
//! `GET /api/admin/ngac/review/resource` 的数据服务）。
//!
//! 语义对齐（change `add-ngac-access-review`，合并语义 fix-ngac-decision-consistency）：
//! - `allowed` / `denied`：与 `/api/ngac/decide/explain` **同源**——对每个
//!   access right，按 (UA 闭包 × OA 闭包) deny-overrides 全扫描调用
//!   `Pdp::evaluate_pair`：任一对 Deny → `denied`，否则任一对 Permit → `allowed`，
//!   遍历顺序（UA 外层、OA 内层、集合内按 id 升序）不影响结果。
//!   按 action 分区（每 action 恰入一集），与 explain 逐 action 结论严格一致；
//!   conditions 用 `ConditionContext::default()` 求值。
//! - 用户视图：UA 闭包取自 `PostgresPip::get_all_user_attributes_with_inheritance`
//!   （与运行时决策同一指派源）；`assignments` 仅列直接指派（未删、未过期），
//!   `ancestor_chain` 自自身起沿祖先闭包展开。
//! - 资源视图：`holders` 稀疏（仅 allowed/denied 非空的 UA）；成员用户经
//!   `ngac_user_rr_attribute`（未删 + 未过期）解析。
//! - 缓存：`("user"|"resource", 主体, version)` 键 + 60s TTL（与 matrix.rs 同一
//!   失效模型：策略版本 bump 即键变化）。

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use actix_web::{web, HttpRequest, HttpResponse};
use serde::Serialize;
use sqlx::PgPool;

use super::matrix::ancestor_closure;
use super::*;
use crate::ngac::pip::{Pip, PostgresPip};

/// 审查端点错误：区分 404（用户不存在）与 500（加载/DB 失败）。
#[derive(Debug)]
pub enum ReviewError {
    UserNotFound(i64),
    Other(anyhow::Error),
}

impl std::fmt::Display for ReviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReviewError::UserNotFound(id) => write!(f, "user {} not found", id),
            ReviewError::Other(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for ReviewError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReviewError::UserNotFound(_) => None,
            ReviewError::Other(e) => Some(e.as_ref()),
        }
    }
}

impl From<sqlx::Error> for ReviewError {
    fn from(e: sqlx::Error) -> Self {
        ReviewError::Other(e.into())
    }
}

impl From<anyhow::Error> for ReviewError {
    fn from(e: anyhow::Error) -> Self {
        ReviewError::Other(e)
    }
}

// ============================================================================
// 响应结构（端点契约：`openspec/changes/add-ngac-access-review`）
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct ReviewUser {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub username: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewAssignment {
    #[serde(with = "common::serde_zuid")]
    pub ua_id: i64,
    pub o_name: String,
    /// 自身 o_name 起、沿祖先闭包（按 id 升序）的属性名链。
    pub ancestor_chain: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewPermission {
    pub resource_type: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    pub allowed: Vec<String>,
    pub denied: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserAccessReview {
    pub user: ReviewUser,
    pub assignments: Vec<ReviewAssignment>,
    /// 认知派生 UA 名清单（UA 闭包内 position:/view: 前缀，add-ngac-self-access-review D2）。
    /// 派生 UA 非直接指派，不出现于 assignments；本字段标注来源供「我」侧展示。
    pub derived_ua: Vec<String>,
    /// 稀疏：仅 allowed/denied 任一非空的 resource_type 行。
    pub permissions: Vec<ReviewPermission>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewResource {
    pub resource_type: String,
    #[serde(with = "common::serde_zuid")]
    pub fk_resource: i64,
    /// 业务可读标识（notice → code → 回退编号；NGAC_SPEC §2.2，add-ngac-oa-readable-identifier）。
    pub resource_identifier: Option<String>,
    /// 资源展示名（NGAC_SPEC §2.2 解析链；未建模 OA 时 fallback resource_type）。
    pub display_name: String,
    /// 该资源的 OA 节点（当前唯一约束下至多一个）。
    #[serde(with = "common::serde_zuid::seq")]
    pub oa_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewHolderUser {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub username: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewHolder {
    #[serde(with = "common::serde_zuid")]
    pub ua_id: i64,
    pub o_name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    pub allowed: Vec<String>,
    pub denied: Vec<String>,
    /// 当前有效（未删、未过期）绑定到该 UA 的用户。
    pub users: Vec<ReviewHolderUser>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceAccessReview {
    pub resource: ReviewResource,
    /// 稀疏：仅 allowed/denied 非空的 UA 行。
    pub holders: Vec<ReviewHolder>,
    pub version: i64,
}

// ============================================================================
// 内存缓存：(kind + 主体, version) 键 + 60s TTL（与 matrix.rs 同一模型）
// ============================================================================

const REVIEW_CACHE_TTL: Duration = Duration::from_secs(60);

enum CachedReview {
    User(Arc<UserAccessReview>),
    Resource(Arc<ResourceAccessReview>),
}

struct CachedReviewEntry {
    at: Instant,
    data: CachedReview,
}

static REVIEW_CACHE: LazyLock<Mutex<HashMap<String, CachedReviewEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// UA 闭包求值：对单个 access right 做 deny-overrides 全扫描（与 decide_access
/// 同语义）：任一对 Deny → Some(false)；否则任一对 Permit → Some(true)；
/// 全不适用 → None。
fn deny_overrides_decision(
    pdp: &Pdp,
    ua_closure: &[i64],
    oa_closure: &[i64],
    access_right: &str,
    ctx: &ConditionContext,
) -> Option<bool> {
    let mut saw_permit = false;
    for &ua_c in ua_closure {
        for &oa_c in oa_closure {
            match pdp.evaluate_pair(ua_c, oa_c, access_right, ctx).0 {
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
    /// 主体中心访问审查：某用户在各 resource_type（集合级 OA）上的有效权限。
    pub async fn user_access_review(
        &self,
        pip: &PostgresPip,
        user_id: i64,
    ) -> Result<UserAccessReview, ReviewError> {
        // 用户存在性先行（404 语义）；再 ensure_policy_loaded 取一致版本。
        let user: Option<(i64, Option<String>, Option<String>)> =
            sqlx::query_as("SELECT id, username, email FROM isahl_auth.auth_users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(pip.pool())
                .await?;
        let (uid, username, email) = match user {
            Some(row) => row,
            None => return Err(ReviewError::UserNotFound(user_id)),
        };

        self.ensure_policy_loaded(pip).await?;
        let version = self.policy_version.load(Ordering::Acquire);
        let cache_key = format!("user:{}:{}", user_id, version);
        {
            let cache = REVIEW_CACHE.lock().unwrap();
            if let Some(entry) = cache.get(&cache_key) {
                if entry.at.elapsed() < REVIEW_CACHE_TTL {
                    if let CachedReview::User(data) = &entry.data {
                        return Ok((**data).clone());
                    }
                }
            }
        }

        let pool = pip.pool();

        // 1. UA 闭包（运行时决策同一指派源）+ 祖先边
        let ua_attrs = pip
            .get_all_user_attributes_with_inheritance(user_id)
            .await?;
        let ua_name: HashMap<i64, String> =
            ua_attrs.iter().map(|u| (u.id, u.o_name.clone())).collect();
        let ua_ancestors: HashMap<i64, Vec<i64>> = ua_attrs
            .iter()
            .map(|u| (u.id, u.ancestor_ids.clone()))
            .collect();
        let ua_closure_ids: Vec<i64> = ua_attrs.iter().map(|u| u.id).collect();

        // 2. 直接指派（未删、未过期；assignments 展示对象）
        let direct_ua_ids: Vec<i64> = sqlx::query_scalar(
            r#"
            SELECT fk_user_attribute FROM isahl_auth.ngac_user_rr_attribute
            WHERE fk_user = $1 AND deleted_at IS NULL
              AND (expires_at IS NULL OR expires_at > NOW())
            ORDER BY fk_user_attribute
            "#,
        )
        .bind(user_id)
        .fetch_all(pool)
        .await?;
        let assignments: Vec<ReviewAssignment> = direct_ua_ids
            .iter()
            .filter_map(|&ua_id| {
                let closure = ancestor_closure(ua_id, &ua_ancestors);
                let chain: Vec<String> = closure
                    .iter()
                    .filter_map(|id| ua_name.get(id).cloned())
                    .collect();
                ua_name.get(&ua_id).map(|name| ReviewAssignment {
                    ua_id,
                    o_name: name.clone(),
                    ancestor_chain: chain,
                })
            })
            .collect();

        // 3. 集合级 OA（fk_resource = 0）逐个 resource_type 求有效权限
        let collection_oas: Vec<(i64, String, Option<i64>, Vec<i64>)> = sqlx::query_as(
            r#"
            SELECT id, COALESCE(resource_type, '') AS resource_type, fk_policy_class,
                   COALESCE(ancestor_ids, '{}'::bigint[]) AS ancestor_ids
            FROM isahl_auth.ngac_object_attribute
            WHERE deleted_at IS NULL AND fk_resource = 0
            ORDER BY resource_type, id
            "#,
        )
        .fetch_all(pool)
        .await?;

        let pg = self.policy_graph();
        // v2 条件上下文（add-ngac-condition-v2）：用户有效 UA 名全集（含派生），
        // OA 闭包名按当前行构造
        let user_ua_names: Vec<String> = ua_attrs
            .iter()
            .map(|u| u.o_name.clone())
            .collect::<Vec<_>>();
        let mut permissions: Vec<ReviewPermission> = Vec::new();
        for (oa_id, resource_type, fk_pc, oa_ancestors_direct) in collection_oas {
            let oa_ancestor_map: HashMap<i64, Vec<i64>> =
                [(oa_id, oa_ancestors_direct)].into_iter().collect();
            let oa_closure = ancestor_closure(oa_id, &oa_ancestor_map);
            let ctx = ConditionContext {
                now: Utc::now(),
                user_ua_names: user_ua_names.clone(),
                oa_closure_names: oa_closure
                    .iter()
                    .filter_map(|id| ua_name.get(id).cloned())
                    .collect(),
            };
            let mut allowed: Vec<String> = Vec::new();
            let mut denied: Vec<String> = Vec::new();
            for ar in pg.access_rights.iter() {
                match deny_overrides_decision(self, &ua_closure_ids, &oa_closure, &ar.o_name, &ctx)
                {
                    Some(true) => allowed.push(ar.o_name.clone()),
                    Some(false) => denied.push(ar.o_name.clone()),
                    None => {}
                }
            }
            if !allowed.is_empty() || !denied.is_empty() {
                permissions.push(ReviewPermission {
                    resource_type,
                    fk_policy_class: fk_pc,
                    allowed,
                    denied,
                });
            }
        }

        // 认知派生 UA 清单（UA 闭包内 position:/view: 前缀；add-ngac-self-access-review D2）
        let mut derived_ua: Vec<String> = ua_attrs
            .iter()
            .filter(|u| u.o_name.starts_with("position:") || u.o_name.starts_with("view:"))
            .map(|u| u.o_name.clone())
            .collect();
        derived_ua.sort();

        let review = UserAccessReview {
            user: ReviewUser {
                id: uid,
                username,
                email,
            },
            assignments,
            derived_ua,
            permissions,
            version,
        };

        let mut cache = REVIEW_CACHE.lock().unwrap();
        cache.retain(|_, e| e.at.elapsed() < REVIEW_CACHE_TTL);
        cache.insert(
            cache_key,
            CachedReviewEntry {
                at: Instant::now(),
                data: CachedReview::User(Arc::new(review.clone())),
            },
        );
        Ok(review)
    }

    /// 资源中心访问审查：某资源（resource_type + fk_resource）的持有者清单。
    /// 资源无对应 OA 节点时返回空 holders（非 404——资源本身可能尚未建模）。
    pub async fn resource_access_review(
        &self,
        pip: &PostgresPip,
        resource_type: &str,
        fk_resource: i64,
    ) -> Result<ResourceAccessReview, ReviewError> {
        self.ensure_policy_loaded(pip).await?;
        let version = self.policy_version.load(Ordering::Acquire);
        let cache_key = format!("resource:{}:{}:{}", resource_type, fk_resource, version);
        {
            let cache = REVIEW_CACHE.lock().unwrap();
            if let Some(entry) = cache.get(&cache_key) {
                if entry.at.elapsed() < REVIEW_CACHE_TTL {
                    if let CachedReview::Resource(data) = &entry.data {
                        return Ok((**data).clone());
                    }
                }
            }
        }

        let pool = pip.pool();

        // 1. 目标 OA（(resource_type, fk_resource) 唯一约束 → 至多一行）
        #[allow(clippy::type_complexity)] // sqlx 行类型
        let oa: Option<(i64, String, Option<i64>, Vec<i64>, Option<String>)> = sqlx::query_as(
            r#"
            SELECT id, o_name, fk_policy_class,
                   COALESCE(ancestor_ids, '{}'::bigint[]) AS ancestor_ids,
                   resource_identifier
            FROM isahl_auth.ngac_object_attribute
            WHERE deleted_at IS NULL AND resource_type = $1 AND fk_resource = $2
            "#,
        )
        .bind(resource_type)
        .bind(fk_resource)
        .fetch_optional(pool)
        .await?;

        let mut holders: Vec<ReviewHolder> = Vec::new();
        let mut oa_ids: Vec<i64> = Vec::new();
        let mut resource_identifier: Option<String> = None;
        // 展示名（NGAC_SPEC §2.2 解析链；单类型批量查询失败静默降级）
        let rt_set: std::collections::HashSet<String> =
            [resource_type.to_string()].into_iter().collect();
        let meta_names = crate::ngac::display::meta_display_names(pool, &rt_set).await;
        let mut display_name =
            crate::ngac::display::resolve_resource_type_display(resource_type, &meta_names);

        if let Some((oa_id, oa_name, _oa_pc, oa_ancestors_direct, oa_identifier)) = oa {
            resource_identifier = oa_identifier.clone();
            display_name = crate::ngac::display::resolve_display_name(
                Some(fk_resource),
                resource_identifier.as_deref(),
                resource_type,
                &meta_names,
                &oa_name,
            );
            oa_ids.push(oa_id);
            let oa_ancestor_map: HashMap<i64, Vec<i64>> =
                [(oa_id, oa_ancestors_direct)].into_iter().collect();
            let oa_closure = ancestor_closure(oa_id, &oa_ancestor_map);

            // 2. 全部活跃 UA（fk_policy_class 透传供前端按 PC 过滤）
            let uas: Vec<(i64, String, Option<i64>, Vec<i64>)> = sqlx::query_as(
                r#"
                SELECT id, o_name, fk_policy_class,
                       COALESCE(ancestor_ids, '{}'::bigint[]) AS ancestor_ids
                FROM isahl_auth.ngac_user_attribute
                WHERE deleted_at IS NULL
                ORDER BY id
                "#,
            )
            .fetch_all(pool)
            .await?;

            let ua_name_map: HashMap<i64, String> =
                uas.iter().map(|(id, n, _, _)| (*id, n.clone())).collect();
            let oa_name_map: HashMap<i64, String> =
                [(oa_id, oa_name.clone())].into_iter().collect();
            let pg = self.policy_graph();
            for (ua_id, ua_name, ua_pc, ua_ancestors_direct) in uas {
                let ua_ancestor_map: HashMap<i64, Vec<i64>> =
                    [(ua_id, ua_ancestors_direct)].into_iter().collect();
                let ua_closure = ancestor_closure(ua_id, &ua_ancestor_map);
                // v2 条件上下文（add-ngac-condition-v2）：holder 级 UA/OA 闭包名
                let ctx = ConditionContext {
                    now: Utc::now(),
                    user_ua_names: ua_closure
                        .iter()
                        .filter_map(|id| ua_name_map.get(id).cloned())
                        .collect(),
                    oa_closure_names: oa_closure
                        .iter()
                        .filter_map(|id| oa_name_map.get(id).cloned())
                        .collect(),
                };
                let mut allowed: Vec<String> = Vec::new();
                let mut denied: Vec<String> = Vec::new();
                for ar in pg.access_rights.iter() {
                    match deny_overrides_decision(self, &ua_closure, &oa_closure, &ar.o_name, &ctx)
                    {
                        Some(true) => allowed.push(ar.o_name.clone()),
                        Some(false) => denied.push(ar.o_name.clone()),
                        None => {}
                    }
                }
                if allowed.is_empty() && denied.is_empty() {
                    continue;
                }

                // 3. 成员用户（未删 + 未过期）；position:/view: 前缀 UA 追加认知持有者
                //    （add-ngac-cognition-derived-ua D3：反向解析 用户→雇员→岗位/标签 code，
                //    expires_at=None 表示认知派生持有，跟随任职生命周期）
                let mut users: Vec<ReviewHolderUser> = sqlx::query_as::<
                    _,
                    (i64, Option<String>, Option<chrono::DateTime<chrono::Utc>>),
                >(
                    r#"
                    SELECT u.id, u.username, ur.expires_at
                    FROM isahl_auth.ngac_user_rr_attribute ur
                    JOIN isahl_auth.auth_users u ON u.id = ur.fk_user
                    WHERE ur.fk_user_attribute = $1 AND ur.deleted_at IS NULL
                      AND (ur.expires_at IS NULL OR ur.expires_at > NOW())
                    ORDER BY u.id
                    "#,
                )
                .bind(ua_id)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(|(id, username, expires_at)| ReviewHolderUser {
                    id,
                    username,
                    expires_at,
                })
                .collect();

                if ua_name.starts_with("position:") || ua_name.starts_with("view:") {
                    // 认知持有者经 common::ngac_org 公共函数反向解析（唯一实现，禁止第二份）
                    let cognition_holders =
                        common::ngac_org::cognition_derived_user_holders(pool, &ua_name).await?;
                    for (id, username) in cognition_holders {
                        if !users.iter().any(|u| u.id == id) {
                            users.push(ReviewHolderUser {
                                id,
                                username,
                                expires_at: None,
                            });
                        }
                    }
                }

                holders.push(ReviewHolder {
                    ua_id,
                    o_name: ua_name,
                    fk_policy_class: ua_pc,
                    allowed,
                    denied,
                    users,
                });
            }
        }

        let review = ResourceAccessReview {
            resource: ReviewResource {
                resource_type: resource_type.to_string(),
                fk_resource,
                resource_identifier,
                display_name,
                oa_ids,
            },
            holders,
            version,
        };

        let mut cache = REVIEW_CACHE.lock().unwrap();
        cache.retain(|_, e| e.at.elapsed() < REVIEW_CACHE_TTL);
        cache.insert(
            cache_key,
            CachedReviewEntry {
                at: Instant::now(),
                data: CachedReview::Resource(Arc::new(review.clone())),
            },
        );
        Ok(review)
    }
}

/// `GET /api/ngac/review/me` — 本人作用域访问审查（add-ngac-self-access-review D1）。
///
/// 安全边界：SSO handler 内强制 JWT，主体恒取 token `sub`，拒绝任何 user_id 参数
/// （PEP 层 `/api/ngac` 前缀豁免仅免除 Gateway PEP 决策，非认证豁免）。
/// 投影复用 `Pdp::user_access_review`（与 admin review/user 同源同缓存模型）。
pub async fn self_access_review(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
) -> HttpResponse {
    let claims =
        match crate::auth::jwt::validate_access_token(&req, &state.verification_keys()).await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("self_access_review: token validation failed: {}", e);
                return HttpResponse::Unauthorized().json(serde_json::json!({
                    "error": "Invalid or missing authentication token"
                }));
            }
        };
    let user_id: i64 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::Unauthorized().json(serde_json::json!({
                "error": "Invalid token subject"
            }));
        }
    };

    let pip = PostgresPip::new(pool.get_ref().clone());
    match Pdp::global().user_access_review(&pip, user_id).await {
        Ok(review) => HttpResponse::Ok().json(review),
        Err(ReviewError::UserNotFound(id)) => {
            log::warn!("self_access_review: user {} not found", id);
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "User not found"
            }))
        }
        Err(e) => {
            log::error!("self_access_review: failed for user {}: {}", user_id, e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to load access review"
            }))
        }
    }
}
