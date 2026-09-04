use actix_web::{web, HttpResponse};
use arc_swap::ArcSwap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;

use crate::ngac::pip::PostgresPip;

mod audit;
mod decide;
mod impact;
mod list;
mod matrix;
mod policy_graph;
mod review;

pub(crate) use audit::audit_decision;
pub(crate) use decide::decide_access;
pub use decide::{
    check_access, check_access_batch, ngac_decide, ngac_decide_explain, ngac_decide_explain_self,
    ExplainNode, ExplainResponse, ExplainStep, SelfExplainRequest,
};
pub use impact::{ImpactError, ImpactPreview};
pub use list::{list_column_access, list_resource_access};
pub use matrix::{MatrixError, PolicyMatrix};
pub use policy_graph::PolicyGraph;
pub use review::self_access_review;
pub use review::{ResourceAccessReview, ReviewError, UserAccessReview};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct NgacAssociation {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid")]
    pub fk_user_attribute: i64,
    #[serde(with = "common::serde_zuid")]
    pub fk_object_attribute: i64,
    #[serde(with = "common::serde_zuid::seq")]
    pub ak_access_rights: Vec<i64>,
    #[serde(with = "common::serde_zuid")]
    pub fk_policy_class: i64,
    pub conditions: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct NgacAccessRight {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    pub applicable_types: Vec<String>,
    pub is_system: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct NgacProhibition {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    #[serde(with = "common::serde_zuid")]
    pub fk_user_attribute: i64,
    #[serde(with = "common::serde_zuid")]
    pub fk_object_attribute: i64,
    #[serde(with = "common::serde_zuid::seq")]
    pub ak_access_rights: Vec<i64>,
    pub is_active: bool,
    pub conditions: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PdpCheckRequest {
    #[serde(with = "common::serde_zuid")]
    pub user_id: i64,
    pub resource: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PdpCheckResponse {
    pub permitted: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PdpCheckBatchRequest {
    #[serde(with = "common::serde_zuid")]
    pub user_id: i64,
    pub checks: Vec<CheckItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CheckItem {
    pub resource: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PdpCheckBatchResponse {
    pub results: Vec<PdpCheckResponse>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Permit,
    Deny,
    NotApplicable,
}

/// 单条策略的求值轨迹（`evaluate_pair` 返回，供 explain 端点渲染"为什么"）。
#[derive(Debug, Clone, Serialize)]
pub struct MatchedRule {
    /// "association"（允许） | "prohibition"（禁止）
    pub rule_type: String,
    /// "allow" | "deny"
    pub kind: String,
    pub access_rights: Vec<String>,
    pub conditions: Option<serde_json::Value>,
    pub conditions_met: bool,
    /// 本次操作是否命中（命中即决定该对结果）
    pub matched: bool,
}

impl std::fmt::Display for Decision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Decision::Permit => write!(f, "Permit"),
            Decision::Deny => write!(f, "Deny"),
            Decision::NotApplicable => write!(f, "NotApplicable"),
        }
    }
}

/// 条件授权上下文。PDP 决策时用于求值 association / prohibition 的 `conditions` JSONB。
#[derive(Debug, Clone)]
pub struct ConditionContext {
    /// 决策时刻（用于时间窗条件 `not_before` / `not_after`）。
    pub now: DateTime<Utc>,
    /// 决策用户的全部有效 UA 名（含认知派生/委托派生），供 `user_attr_in` 求值
    /// （add-ngac-condition-v2）。Default 为空集 → `user_attr_in` 永假（fail-closed）。
    pub user_ua_names: Vec<String>,
    /// 目标 OA 祖先闭包名（含自身），供 `object_attr_in` 求值。Default 为空集。
    pub oa_closure_names: Vec<String>,
}

impl Default for ConditionContext {
    fn default() -> Self {
        Self {
            now: Utc::now(),
            user_ua_names: Vec::new(),
            oa_closure_names: Vec::new(),
        }
    }
}

/// 求值一条策略的 `conditions` JSONB。
///
/// 语义（与 `docs/specs/NGAC_SPEC.md §2.4` 一致）：
/// - `NULL` / 空对象 `{}` → 无条件生效，返回 `true`；
/// - 任一约束不满足 → 返回 `false`（该策略不参与本次决策）；
/// - 字段非法 / 解析失败 → 返回 `false`（**失败封闭**，宁拒勿放）。
pub fn evaluate_conditions(conditions: &Option<Value>, ctx: &ConditionContext) -> bool {
    let conditions = match conditions {
        Some(v) if !v.is_null() => v,
        _ => return true,
    };
    let obj = match conditions.as_object() {
        Some(o) if !o.is_empty() => o,
        _ => return true,
    };

    // not_before：当前时间须 >= 该时间点
    if let Some(nb) = obj.get("not_before") {
        let nb = match parse_iso8601(nb) {
            Some(t) => t,
            None => return false,
        };
        if ctx.now < nb {
            return false;
        }
    }

    // not_after：当前时间须 <= 该时间点
    if let Some(na) = obj.get("not_after") {
        let na = match parse_iso8601(na) {
            Some(t) => t,
            None => return false,
        };
        if ctx.now > na {
            return false;
        }
    }

    // user_attr_in：用户有效 UA 集须含至少一个给定 UA 名（v2，add-ngac-condition-v2）。
    // 非法字段 / 空上下文 → fail-closed（永假）。
    if let Some(uai) = obj.get("user_attr_in") {
        let names: Vec<&str> = match uai.as_array() {
            Some(a) => a.iter().filter_map(|x| x.as_str()).collect(),
            None => return false,
        };
        if names.is_empty()
            || !names
                .iter()
                .any(|n| ctx.user_ua_names.iter().any(|u| u == n))
        {
            return false;
        }
    }

    // object_attr_in：目标 OA 祖先闭包须含至少一个给定 OA 名（v2）。
    if let Some(oai) = obj.get("object_attr_in") {
        let names: Vec<&str> = match oai.as_array() {
            Some(a) => a.iter().filter_map(|x| x.as_str()).collect(),
            None => return false,
        };
        if names.is_empty()
            || !names
                .iter()
                .any(|n| ctx.oa_closure_names.iter().any(|o| o == n))
        {
            return false;
        }
    }

    true
}

fn parse_iso8601(v: &Value) -> Option<DateTime<Utc>> {
    match v {
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc)),
        _ => None,
    }
}

/// 进程级 PDP 单例（NGAC_SPEC §8 策略版本契约）。
///
/// 决策路径共享同一 `PolicyGraph` 缓存：仅当 `ngac_policy_version` 变化时
/// 才全量 reload，消除每请求 `Pdp::new()` + 全表重扫（dev 库 1173 association）。
static GLOBAL_PDP: LazyLock<Pdp> = LazyLock::new(Pdp::new);

pub struct Pdp {
    /// 当前生效的策略图。`ArcSwap` 原子发布：reload 期间构建全新图，
    /// 填完一次性 store 换入，在途请求继续持有旧快照（杜绝原地 clear 的撕裂读）。
    policy_graph: ArcSwap<PolicyGraph>,
    /// 本地缓存的策略版本（`-1` = 从未加载，首次决策必 reload）。
    policy_version: AtomicI64,
    /// reload 串行化：并发决策只允许一个全量重建，其余等待后走 double-check。
    reload_lock: Arc<Mutex<()>>,
}

impl Clone for Pdp {
    fn clone(&self) -> Self {
        Self {
            policy_graph: ArcSwap::from(self.policy_graph.load_full()),
            policy_version: AtomicI64::new(self.policy_version.load(Ordering::Acquire)),
            reload_lock: self.reload_lock.clone(),
        }
    }
}

impl Pdp {
    pub fn new() -> Self {
        Self {
            policy_graph: ArcSwap::from_pointee(PolicyGraph::new()),
            policy_version: AtomicI64::new(-1),
            reload_lock: Arc::new(Mutex::new(())),
        }
    }

    /// 进程级共享实例（handler / PEP 决策路径统一入口）。
    pub fn global() -> &'static Pdp {
        &GLOBAL_PDP
    }

    /// 返回当前生效策略图的快照（`Arc` 引用计数，reload 不使其失效）。
    pub fn policy_graph(&self) -> Arc<PolicyGraph> {
        self.policy_graph.load_full()
    }

    /// 全量重建策略图并原子发布：本地构建全新 `PolicyGraph`，三个表全部
    /// 加载成功后才 `store` 换入 —— 失败时旧图保持原样（调用方按 fail-closed
    /// 语义处理），在途决策始终读到完整一致的快照，不会观察到半载状态。
    pub async fn load_policy_from_db(&self, pip: &PostgresPip) -> anyhow::Result<()> {
        let pool = pip.pool();
        let policy_graph = PolicyGraph::new();

        let associations = sqlx::query_as::<_, NgacAssociation>(
                r#"
                SELECT id, fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, conditions
                FROM isahl_auth.ngac_association
                WHERE deleted_at IS NULL
                "#,
            )
            .fetch_all(pool)
            .await?;

        for assoc in associations {
            policy_graph.add_association(assoc);
        }

        let access_rights = sqlx::query_as::<_, NgacAccessRight>(
            r#"
                SELECT id, o_name, applicable_types, is_system
                FROM isahl_auth.ngac_access_right
                "#,
        )
        .fetch_all(pool)
        .await?;

        for ar in access_rights {
            policy_graph.add_access_right(ar);
        }

        let prohibitions = sqlx::query_as::<_, NgacProhibition>(
                r#"
                SELECT id, o_name, fk_user_attribute, fk_object_attribute, ak_access_rights, is_active, conditions
                FROM isahl_auth.ngac_prohibition
                WHERE deleted_at IS NULL
                "#,
            )
            .fetch_all(pool)
            .await?;

        for proh in prohibitions {
            policy_graph.add_prohibition(proh);
        }

        // T6：bootstrap 阶段无 association 策略 → 告警提示部署 seed 缺失（不改 fail-open 语义）。
        if !policy_graph.has_associations() {
            log::warn!("NGAC bootstrap：无 association 策略，PDP 处于 fail-open 兜底（部署 seed 缺失？见 NGAC_SPEC §7.3）");
        }

        // 原子发布：一次性换入新图（Release 语义，先于 policy_version.store 的 Release，
        // 读取方先见新版本必见新图）。
        self.policy_graph.store(Arc::new(policy_graph));
        Ok(())
    }

    /// 决策前调用：对比 `ngac_policy_version` 与本地缓存版本，变化才全量 reload。
    ///
    /// - 版本一致 → 直接返回（零 DB 重扫，仅一次单行轻查询）；
    /// - 版本变化 / 从未加载 → 串行化 reload（`reload_lock`），拿到锁后 double-check，
    ///   避免并发请求重复全量重建；
    /// - 空表 → 幂等插入首行（version=1，`ON CONFLICT (id) DO NOTHING`，并发安全）；
    /// - 失败（表缺失 / DB 错误）→ `Err` 上抛，由调用方按既有 fail-closed 语义处理。
    pub async fn ensure_policy_loaded(&self, pip: &PostgresPip) -> anyhow::Result<()> {
        // 扩展表运行时幂等自愈（零 DDL 交付，add-ngac-runtime-ensure）——MUST 先于
        // 版本早退：DELEGATED_CTE 每决策引用 ngac_delegation，缺表即全决策 fail-closed。
        // per-process 一次（AtomicBool），实际开销为零。
        crate::ngac::ensure::ensure_ngac_extension_tables(pip.pool()).await;
        let db_version = Self::fetch_policy_version(pip).await?;
        if self.policy_version.load(Ordering::Acquire) == db_version {
            return Ok(());
        }

        // 串行化 reload；等待期间其它请求可能已完成加载，double-check 避免重复重建。
        let _guard = self.reload_lock.lock().await;
        let db_version = Self::fetch_policy_version(pip).await?;
        if self.policy_version.load(Ordering::Acquire) == db_version {
            return Ok(());
        }

        self.load_policy_from_db(pip).await?;
        self.policy_version.store(db_version, Ordering::Release);
        log::debug!("PDP policy graph reloaded (version {})", db_version);
        Ok(())
    }

    /// 读取策略版本（单行轻查询）。
    ///
    /// 热路径零写：先 SELECT 单行（决策热路径只读，不再每决策 INSERT）；
    /// 仅当表为空（返回无行）时才幂等插入首行 version=1 并重读
    /// （NGAC_SPEC §8 空表初始化契约；`ON CONFLICT DO NOTHING` 并发安全，
    /// 并发首插时同语句快照看不到彼此插入，故插入后必须 re-SELECT 兜底）。
    async fn fetch_policy_version(pip: &PostgresPip) -> anyhow::Result<i64> {
        let pool = pip.pool();
        if let Some(version) = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM isahl_auth.ngac_policy_version WHERE id = 1",
        )
        .fetch_optional(pool)
        .await?
        {
            return Ok(version);
        }

        // 空表：幂等插首行后重读（插入成功或并发方抢先插入，结果一致）。
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_policy_version (id, version) VALUES (1, 1) \
             ON CONFLICT (id) DO NOTHING",
        )
        .execute(pool)
        .await?;
        let version: i64 =
            sqlx::query_scalar("SELECT version FROM isahl_auth.ngac_policy_version WHERE id = 1")
                .fetch_one(pool)
                .await?;
        Ok(version)
    }

    /// 单点匹配实现：对 (UA, OA) 对求值，返回决策 + 命中的规则轨迹。
    ///
    /// **对级语义**：对内 prohibition 匹配即 Deny（早停）、否则 association 匹配即
    /// Permit（早停）、否则 NotApplicable。**跨对合并**由调用方统一为 deny-overrides
    /// （fix-ngac-decision-consistency）：任一 (UA,OA) 对 Deny → 终态 Deny；否则任一
    /// Permit → Permit；全不适用 → NotApplicable。`decide_access` / `explain_access` /
    /// `check_access_batch` / 矩阵 / 访问审查 / 影响预览 MUST 同此合并，禁止第二套语义。
    pub fn evaluate_pair(
        &self,
        fk_user_attribute: i64,
        fk_object_attribute: i64,
        operation: &str,
        ctx: &ConditionContext,
    ) -> (Decision, Vec<MatchedRule>) {
        // 单次决策持有一个图快照（ArcSwap load），整条求值路径看到的是
        // 同一份完整一致的策略图；reload 原子换图不影响在途决策。
        let pg = self.policy_graph.load();
        self.evaluate_pair_in(&pg, fk_user_attribute, fk_object_attribute, operation, ctx)
    }

    /// `evaluate_pair` 的图参数化变体（change `add-ngac-audit-trail-view` D2）：
    /// 在**给定** PolicyGraph 上求值，供影响预览用模拟图（删除目标边后的克隆）
    /// 做 before/after 比对。运行时决策路径仍走 `evaluate_pair`（ArcSwap 快照），
    /// 语义零变化——本方法即其唯一实现。
    pub fn evaluate_pair_in(
        &self,
        pg: &PolicyGraph,
        fk_user_attribute: i64,
        fk_object_attribute: i64,
        operation: &str,
        ctx: &ConditionContext,
    ) -> (Decision, Vec<MatchedRule>) {
        let ar_id = match pg.access_right_name_index.get(operation) {
            Some(id) => *id,
            None => return (Decision::NotApplicable, Vec::new()),
        };
        let ar_names = |ids: &[i64]| -> Vec<String> {
            ids.iter()
                .filter_map(|id| pg.access_rights.get(id).map(|r| r.o_name.clone()))
                .collect()
        };

        let mut rules: Vec<MatchedRule> = Vec::new();

        for prohibition in pg.prohibitions.iter() {
            if prohibition.is_active
                && prohibition.fk_user_attribute == fk_user_attribute
                && prohibition.fk_object_attribute == fk_object_attribute
            {
                let conditions_met = evaluate_conditions(&prohibition.conditions, ctx);
                let matched = prohibition.ak_access_rights.contains(&ar_id) && conditions_met;
                rules.push(MatchedRule {
                    rule_type: "prohibition".to_string(),
                    kind: "deny".to_string(),
                    access_rights: ar_names(&prohibition.ak_access_rights),
                    conditions: prohibition.conditions.clone(),
                    conditions_met,
                    matched,
                });
                if matched {
                    return (Decision::Deny, rules);
                }
            }
        }

        if let Some(assoc_ids) = pg.user_attr_index.get(&fk_user_attribute) {
            for assoc_id in assoc_ids.iter() {
                if let Some(assoc) = pg.associations.get(assoc_id) {
                    if assoc.fk_object_attribute == fk_object_attribute {
                        let conditions_met = evaluate_conditions(&assoc.conditions, ctx);
                        let matched = assoc.ak_access_rights.contains(&ar_id) && conditions_met;
                        rules.push(MatchedRule {
                            rule_type: "association".to_string(),
                            kind: "allow".to_string(),
                            access_rights: ar_names(&assoc.ak_access_rights),
                            conditions: assoc.conditions.clone(),
                            conditions_met,
                            matched,
                        });
                        if matched {
                            return (Decision::Permit, rules);
                        }
                    }
                }
            }
        }

        (Decision::NotApplicable, rules)
    }

    pub fn check_access(
        &self,
        fk_user_attribute: i64,
        fk_object_attribute: i64,
        operation: &str,
        ctx: &ConditionContext,
    ) -> Decision {
        self.evaluate_pair(fk_user_attribute, fk_object_attribute, operation, ctx)
            .0
    }
}

impl Default for Pdp {
    fn default() -> Self {
        Self::new()
    }
}

/// `GET /api/ngac/policy-version` — 策略版本探针（fix-ngac-decision-consistency D4）。
///
/// Gateway PEP 的 per-worker 决策/列缓存以此版本为失效信号：版本变化即清空本
/// worker 缓存。响应仅整数版本号（无敏感面）；挂 `/api/ngac` 前缀（PDP 决策类
/// 既有豁免，SECURITY_SPEC §3.1），与 decide 同一信任面。
pub async fn get_policy_version(pool: web::Data<PgPool>) -> HttpResponse {
    match sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(version), 0) FROM isahl_auth.ngac_policy_version",
    )
    .fetch_one(pool.get_ref())
    .await
    {
        Ok(version) => HttpResponse::Ok().json(serde_json::json!({ "version": version })),
        Err(e) => {
            log::error!("get_policy_version: query failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to fetch policy version"
            }))
        }
    }
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/pdp")
            .route("/check", web::post().to(check_access))
            .route("/check/batch", web::post().to(check_access_batch))
            .route("/list", web::post().to(list_resource_access))
            .route("/columns", web::post().to(list_column_access)),
    )
    .route("/decide", web::post().to(ngac_decide))
    .route("/policy-version", web::get().to(get_policy_version))
    .route("/decide/explain", web::post().to(ngac_decide_explain))
    // 本人作用域（add-ngac-self-access-review D1）：SSO handler 内强制 JWT，
    // 主体恒取 token sub；PEP 层 /api/ngac 前缀豁免仅免除 Gateway PEP 决策。
    .route(
        "/decide/explain/me",
        web::post().to(ngac_decide_explain_self),
    )
    .route("/review/me", web::get().to(self_access_review))
    // 权限申请本人端点（add-ngac-access-request D2）
    .configure(crate::ngac::access_request::configure_self_routes)
    // 通用委托本人端点（add-ngac-delegation D3）
    .configure(crate::ngac::delegation::configure_self_routes)
    // 绑定申请本人端点（add-ngac-binding-request D2）
    .configure(crate::ngac::binding_request::configure_self_routes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_at(ts: &str) -> ConditionContext {
        ConditionContext {
            now: DateTime::parse_from_rfc3339(ts)
                .unwrap()
                .with_timezone(&Utc),
            user_ua_names: Vec::new(),
            oa_closure_names: Vec::new(),
        }
    }

    #[test]
    fn conditions_null_is_unconditional() {
        assert!(evaluate_conditions(&None, &ctx_at("2026-06-01T00:00:00Z")));
    }

    #[test]
    fn conditions_empty_object_is_unconditional() {
        let c = Some(serde_json::json!({}));
        assert!(evaluate_conditions(&c, &ctx_at("2026-06-01T00:00:00Z")));
    }

    #[test]
    fn time_window_active() {
        let c = Some(serde_json::json!({
            "not_before": "2026-01-01T00:00:00Z",
            "not_after": "2026-12-31T23:59:59Z"
        }));
        assert!(evaluate_conditions(&c, &ctx_at("2026-06-01T00:00:00Z")));
    }

    #[test]
    fn time_window_before_not_before() {
        let c = Some(serde_json::json!({
            "not_before": "2026-01-01T00:00:00Z"
        }));
        assert!(!evaluate_conditions(&c, &ctx_at("2025-12-31T23:59:59Z")));
    }

    #[test]
    fn time_window_after_not_after() {
        let c = Some(serde_json::json!({
            "not_after": "2026-12-31T23:59:59Z"
        }));
        assert!(!evaluate_conditions(&c, &ctx_at("2027-01-01T00:00:00Z")));
    }

    #[test]
    fn malformed_timestamp_fails_closed() {
        let c = Some(serde_json::json!({
            "not_before": "not-a-date"
        }));
        assert!(!evaluate_conditions(&c, &ctx_at("2026-06-01T00:00:00Z")));
    }

    #[test]
    fn non_string_timestamp_fails_closed() {
        let c = Some(serde_json::json!({
            "not_before": 12345
        }));
        assert!(!evaluate_conditions(&c, &ctx_at("2026-06-01T00:00:00Z")));
    }

    // ========================================================================
    // 版本缓存（NGAC_SPEC §8）：version 变更触发 reload + 空表幂等初始化
    // ========================================================================

    async fn test_pool() -> sqlx::PgPool {
        let url = std::env::var("DATABASE_URL")
            .or_else(|_| std::env::var("SSO_TEST_DATABASE_URL"))
            .unwrap_or_else(|_| {
                let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_string());
                format!("postgres://{}@localhost:5432/aliothstudio_test", user)
            });
        sqlx::PgPool::connect(&url)
            .await
            .expect("无法连接测试库，请先运行 `bash scripts/db/reset-db.sh --test`")
    }

    async fn cleanup_version_cache_test_data(pool: &sqlx::PgPool) {
        sqlx::query(
            "DELETE FROM isahl_auth.ngac_association \
             WHERE fk_user_attribute IN (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='cachetest-ua')",
        )
        .execute(pool)
        .await
        .ok();
        sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE o_name='cachetest-oa'")
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name='cachetest-ua'")
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM isahl_auth.ngac_access_right WHERE o_name='cachetest-read'")
            .execute(pool)
            .await
            .ok();
    }

    /// 空表时幂等插入首行；版本变更后 `ensure_policy_loaded` 触发 reload；
    /// 版本未变时不重扫（决策路径无每请求全量 load）。
    #[tokio::test]
    async fn policy_version_cache_empty_init_and_reload_on_bump() {
        let pool = test_pool().await;
        let pip = PostgresPip::new(pool.clone());
        cleanup_version_cache_test_data(&pool).await;
        // 清空版本表 → 首次 ensure 必须幂等插首行（version=1）
        sqlx::query("DELETE FROM isahl_auth.ngac_policy_version")
            .execute(&pool)
            .await
            .expect("clear version table");

        let pdp = Pdp::new();
        pdp.ensure_policy_loaded(&pip)
            .await
            .expect("first ensure with empty version table");
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM isahl_auth.ngac_policy_version")
            .fetch_one(&pool)
            .await
            .expect("count version rows");
        assert_eq!(
            rows, 1,
            "empty table must be initialized with exactly one row"
        );

        // 幂等：再次清空 + 连续两次 ensure，仍只应有一行
        sqlx::query("DELETE FROM isahl_auth.ngac_policy_version")
            .execute(&pool)
            .await
            .expect("clear version table again");
        pdp.ensure_policy_loaded(&pip)
            .await
            .expect("re-init ensure");
        pdp.ensure_policy_loaded(&pip)
            .await
            .expect("re-init ensure (idempotent)");
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM isahl_auth.ngac_policy_version")
            .fetch_one(&pool)
            .await
            .expect("count version rows after re-init");
        assert_eq!(rows, 1, "repeated ensure must stay idempotent");

        // 播种唯一测试策略：UA + OA + AR + association
        let pc: i64 = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default' LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("default policy class");
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at) \
             VALUES ('cachetest-ua', $1, NOW(), NOW()) ON CONFLICT DO NOTHING",
        )
        .bind(pc)
        .execute(&pool)
        .await
        .ok();
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at) \
             VALUES ('cachetest-oa', $1, 'cachetest', 0, NOW(), NOW()) ON CONFLICT DO NOTHING",
        )
        .bind(pc)
        .execute(&pool)
        .await
        .ok();
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ('cachetest-read') \
             ON CONFLICT (o_name) DO NOTHING",
        )
        .execute(&pool)
        .await
        .ok();
        let ua: i64 = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='cachetest-ua' LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("cachetest UA");
        let oa: i64 = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_object_attribute WHERE o_name='cachetest-oa' LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("cachetest OA");
        let ar: i64 = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='cachetest-read' LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("cachetest AR");
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_association \
             (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at, updated_at) \
             VALUES ($1, $2, ARRAY[$3], $4, NOW(), NOW()) ON CONFLICT DO NOTHING",
        )
        .bind(ua)
        .bind(oa)
        .bind(ar)
        .bind(pc)
        .execute(&pool)
        .await
        .expect("seed cachetest association");

        // 版本未变：ensure 不 reload → 新播种的 association 不可见（无全量重扫）
        let ctx = ConditionContext::default();
        let decision_before = pdp.check_access(ua, oa, "cachetest-read", &ctx);
        assert_eq!(
            decision_before,
            Decision::NotApplicable,
            "no version bump → cached graph must NOT see the new association"
        );

        // 版本 +1（模拟迁移/写路径的 bump）→ ensure 触发 reload → 决策变为 Permit
        sqlx::query(
            "UPDATE isahl_auth.ngac_policy_version \
             SET version = version + 1, updated_at = NOW() \
             WHERE id = (SELECT id FROM isahl_auth.ngac_policy_version ORDER BY id LIMIT 1)",
        )
        .execute(&pool)
        .await
        .expect("bump policy version");
        pdp.ensure_policy_loaded(&pip)
            .await
            .expect("ensure after version bump");
        let decision_after = pdp.check_access(ua, oa, "cachetest-read", &ctx);
        assert_eq!(
            decision_after,
            Decision::Permit,
            "version bump must trigger reload making the association visible"
        );

        // 清理：删除测试数据（版本行保留，其它测试共享）
        cleanup_version_cache_test_data(&pool).await;
    }

    /// 030 触发器自动 bump：ngac_access_right / association / prohibition 的
    /// INSERT/UPDATE/DELETE 语句自动使 ngac_policy_version +1（review 修复：
    /// SSO 此前无任何写路径手动 bump，策略变更后 PDP 缓存永不失效）。
    ///
    /// 用事务 + 版本行 `FOR UPDATE` 锁定隔离并发测试的 bump，保证严格相等断言。
    #[tokio::test]
    async fn policy_version_trigger_auto_bumps_on_policy_write() {
        let pool = test_pool().await;
        let pip = PostgresPip::new(pool.clone());
        // 预清理（tx 外，其自身 bump 不影响断言起点）
        sqlx::query(
            "DELETE FROM isahl_auth.ngac_access_right WHERE o_name LIKE 'cachetest-trigger%'",
        )
        .execute(&pool)
        .await
        .ok();

        // 确保版本行存在（空表由 ensure 幂等插首行）
        let pdp = Pdp::new();
        pdp.ensure_policy_loaded(&pip)
            .await
            .expect("ensure version row exists");

        let mut tx = pool.begin().await.expect("begin tx");

        let v0: i64 = sqlx::query_scalar(
            "SELECT version FROM isahl_auth.ngac_policy_version WHERE id = 1 FOR UPDATE",
        )
        .fetch_one(&mut *tx)
        .await
        .expect("read version v0");
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ('cachetest-trigger-ar')",
        )
        .execute(&mut *tx)
        .await
        .expect("insert AR");
        let v1: i64 = sqlx::query_scalar(
            "SELECT version FROM isahl_auth.ngac_policy_version WHERE id = 1 FOR UPDATE",
        )
        .fetch_one(&mut *tx)
        .await
        .expect("read version v1");
        assert_eq!(v1, v0 + 1, "INSERT on ngac_access_right must bump version");

        sqlx::query(
            "UPDATE isahl_auth.ngac_access_right SET o_name = 'cachetest-trigger-ar2' \
             WHERE o_name = 'cachetest-trigger-ar'",
        )
        .execute(&mut *tx)
        .await
        .expect("update AR");
        let v2: i64 = sqlx::query_scalar(
            "SELECT version FROM isahl_auth.ngac_policy_version WHERE id = 1 FOR UPDATE",
        )
        .fetch_one(&mut *tx)
        .await
        .expect("read version v2");
        assert_eq!(v2, v1 + 1, "UPDATE on ngac_access_right must bump version");

        sqlx::query(
            "DELETE FROM isahl_auth.ngac_access_right WHERE o_name = 'cachetest-trigger-ar2'",
        )
        .execute(&mut *tx)
        .await
        .expect("delete AR");
        let v3: i64 = sqlx::query_scalar(
            "SELECT version FROM isahl_auth.ngac_policy_version WHERE id = 1 FOR UPDATE",
        )
        .fetch_one(&mut *tx)
        .await
        .expect("read version v3");
        assert_eq!(v3, v2 + 1, "DELETE on ngac_access_right must bump version");

        tx.rollback().await.expect("rollback");
    }

    /// 原子发布（review P1 撕裂读修复）：reload 构建全新图后一次性换入，
    /// 旧快照在换图后保持完整可用（不被原地 clear），在途决策不读半载状态。
    #[tokio::test]
    async fn policy_graph_atomic_swap_keeps_old_snapshot_usable() {
        let pool = test_pool().await;
        let pip = PostgresPip::new(pool.clone());
        // 清理 atomic-* 测试数据
        sqlx::query(
            "DELETE FROM isahl_auth.ngac_association \
             WHERE fk_user_attribute IN (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'atomic-ua%')",
        )
        .execute(&pool)
        .await
        .ok();
        sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE o_name='atomic-oa'")
            .execute(&pool)
            .await
            .ok();
        sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'atomic-ua%'")
            .execute(&pool)
            .await
            .ok();
        sqlx::query("DELETE FROM isahl_auth.ngac_access_right WHERE o_name LIKE 'atomic-read%'")
            .execute(&pool)
            .await
            .ok();

        let pdp = Pdp::new();
        pdp.ensure_policy_loaded(&pip)
            .await
            .expect("initial ensure (version row + empty graph)");

        let pc: i64 = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default' LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("default policy class");
        // 注意 resource_type 用 'atomic'：与 cachetest 测试的 uq_ngac_oa_resource
        // (resource_type, fk_resource) 空间隔离，避免并发测试互踩。
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at) \
             VALUES ('atomic-ua', $1, NOW(), NOW()), ('atomic-ua2', $1, NOW(), NOW()) ON CONFLICT DO NOTHING",
        )
        .bind(pc)
        .execute(&pool)
        .await
        .ok();
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, created_at, updated_at) \
             VALUES ('atomic-oa', $1, 'atomic', 0, NOW(), NOW()) ON CONFLICT DO NOTHING",
        )
        .bind(pc)
        .execute(&pool)
        .await
        .ok();
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ('atomic-read1') \
             ON CONFLICT (o_name) DO NOTHING",
        )
        .execute(&pool)
        .await
        .ok();
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ('atomic-read2') \
             ON CONFLICT (o_name) DO NOTHING",
        )
        .execute(&pool)
        .await
        .ok();
        let ua: i64 = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='atomic-ua' LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("atomic UA");
        let ua2: i64 = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name='atomic-ua2' LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("atomic UA2");
        let oa: i64 = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_object_attribute WHERE o_name='atomic-oa' LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("atomic OA");
        let ar1: i64 = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='atomic-read1' LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("atomic AR1");
        let ar2: i64 = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name='atomic-read2' LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("atomic AR2");
        let seed_assoc = |ua: i64, ar: i64| {
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "INSERT INTO isahl_auth.ngac_association \
                     (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at, updated_at) \
                     VALUES ($1, $2, ARRAY[$3], $4, NOW(), NOW()) ON CONFLICT DO NOTHING",
                )
                .bind(ua)
                .bind(oa)
                .bind(ar)
                .bind(pc)
                .execute(&pool)
                .await
                .expect("seed atomic association");
                sqlx::query_scalar::<_, i64>(
                    "SELECT id FROM isahl_auth.ngac_association \
                     WHERE fk_user_attribute = $1 AND fk_object_attribute = $2 AND $3 = ANY(ak_access_rights) \
                     LIMIT 1",
                )
                .bind(ua)
                .bind(oa)
                .bind(ar)
                .fetch_one(&pool)
                .await
                .expect("atomic association id")
            }
        };

        let assoc1_id = seed_assoc(ua, ar1).await;
        pdp.ensure_policy_loaded(&pip)
            .await
            .expect("reload with assoc1 (trigger bump)");

        // 快照 1：换图前持有
        let snap1 = pdp.policy_graph();
        assert!(
            snap1.associations.contains_key(&assoc1_id),
            "snapshot must see assoc1 loaded before swap"
        );
        let ctx = ConditionContext::default();
        let snap_pdp = Pdp {
            policy_graph: ArcSwap::from(snap1.clone()),
            policy_version: AtomicI64::new(0),
            reload_lock: Arc::new(Mutex::new(())),
        };
        assert_eq!(
            snap_pdp.check_access(ua, oa, "atomic-read1", &ctx),
            Decision::Permit,
            "snapshot evaluates the policy it captured"
        );
        assert_eq!(
            snap_pdp.check_access(ua, oa, "atomic-read2", &ctx),
            Decision::NotApplicable,
            "snapshot must not see policies added after it was taken"
        );

        // 第二次写入（UA2 关联 AR2）→ 触发器 bump → reload 换图
        let assoc2_id = seed_assoc(ua2, ar2).await;
        pdp.ensure_policy_loaded(&pip)
            .await
            .expect("reload with assoc2 (trigger bump)");

        let snap2 = pdp.policy_graph();
        assert!(
            snap2.associations.contains_key(&assoc2_id),
            "live graph must see assoc2 after swap"
        );
        assert_eq!(
            pdp.check_access(ua2, oa, "atomic-read2", &ctx),
            Decision::Permit,
            "live PDP sees the reloaded policy"
        );
        // 旧快照完好：完整保留换图前数据，未出现原地 clear 的撕裂
        assert!(
            snap1.associations.contains_key(&assoc1_id),
            "old snapshot must remain fully populated after swap (no in-place clear)"
        );
        assert!(
            !snap1.associations.contains_key(&assoc2_id),
            "old snapshot must not gain post-snapshot data"
        );
        assert_eq!(
            snap_pdp.check_access(ua2, oa, "atomic-read2", &ctx),
            Decision::NotApplicable,
            "in-flight decision on old snapshot keeps old semantics"
        );

        // 清理
        sqlx::query(
            "DELETE FROM isahl_auth.ngac_association \
             WHERE fk_user_attribute IN (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'atomic-ua%')",
        )
        .execute(&pool)
        .await
        .ok();
        sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE o_name='atomic-oa'")
            .execute(&pool)
            .await
            .ok();
        sqlx::query("DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'atomic-ua%'")
            .execute(&pool)
            .await
            .ok();
        sqlx::query("DELETE FROM isahl_auth.ngac_access_right WHERE o_name LIKE 'atomic-read%'")
            .execute(&pool)
            .await
            .ok();
    }
}
