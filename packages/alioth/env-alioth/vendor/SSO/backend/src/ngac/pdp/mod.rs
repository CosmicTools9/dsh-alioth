use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::ngac::pip::PostgresPip;

mod audit;
mod decide;
mod impact;
mod list;
mod matrix;
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
pub use review::self_access_review;
pub use review::{ResourceAccessReview, ReviewError, UserAccessReview};

// 决策核心唯一实现 = `ngac-decision` crate（openspec change
// `extract-ngac-decision-core`）：对求值 / deny-overrides 合并 / 决策序 /
// conditions 求值全量下放 crate，本模块只保留容器职责（快照 / 版本探针 /
// reload）与 HTTP 面；下列 re-export 是**单一定义**的模块门面（旧路径
// `crate::ngac::pdp::Decision` 等继续可用，非第二实现）。
/// 属性祖先闭包唯一实现（`pdp` 内共享；调用面见 `matrix`/`review`/`impact` 与
/// `/auth/me` 权限矩阵的逐行上下文构造——tasks 1.3）。
pub(crate) use matrix::ancestor_closure;
pub use ngac_decision::{
    evaluate_conditions, ConditionContext, Decision, MatchedRule, PairOutcome, PolicyGraph,
};
use ngac_decision::{evaluate_merge_in, evaluate_pair_in};

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

// PDP 请求/响应 DTO 单一定义 = `ngac-contract`（openspec extract-ngac-decision-host
// 的 C4 收口）：本模块不再自持镜像副本，直接复用契约类型（`Decision` 单一词汇 =
// `ngac_decision::Decision`）。`PdpCheckResponse` 携带 `obligations`（§2.6）。
pub use ngac_contract::{
    PdpCheckBatchRequest, PdpCheckBatchResponse, PdpCheckRequest, PdpCheckResponse,
};

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// 进程级 PDP 门面（NGAC_SPEC §8 策略版本契约）。
///
/// **无自有状态**：策略图快照 + 版本探针 + reload 串行化的唯一持有者 =
/// `ngac_host::Evaluator`（openspec extract-ngac-decision-host，容器合一）。
/// 本类型是 SSO 侧零状态门面，方法全部委托，用于保持既有调用面
/// （`Pdp::global()`）不变；MUST NOT 在本类型重新引入快照字段。
#[derive(Clone)]
pub struct Pdp;

static GLOBAL_PDP: Pdp = Pdp;

impl Pdp {
    pub fn new() -> Self {
        Self
    }

    /// 进程级共享实例（handler / PEP 决策路径统一入口）。
    pub fn global() -> &'static Pdp {
        &GLOBAL_PDP
    }

    /// 返回当前生效策略图的快照（唯一容器 = `ngac_host::Evaluator`）。
    pub fn policy_graph(&self) -> Arc<PolicyGraph> {
        ngac_host::Evaluator::global().policy_graph()
    }

    /// 全量重建策略图（委托唯一容器；保留方法名供既有调用点/测试）。
    pub async fn load_policy_from_db(&self, pip: &PostgresPip) -> anyhow::Result<()> {
        ngac_host::Evaluator::global()
            .ensure_policy_loaded(pip.pool())
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }

    /// 决策前调用：先扩展表幂等自愈（零 DDL 交付），再委托唯一容器做
    /// 「空表初始化（NGAC_SPEC §8）+ 版本探针 + 按需 reload」。
    pub async fn ensure_policy_loaded(&self, pip: &PostgresPip) -> anyhow::Result<()> {
        crate::ngac::ensure::ensure_ngac_extension_tables(pip.pool()).await;
        ngac_host::Evaluator::global()
            .ensure_policy_loaded(pip.pool())
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }

    /// 单点匹配：对 (UA, OA) 对求值（委托 `ngac_decision::evaluate_pair_in`——
    /// 唯一实现，openspec extract-ngac-decision-core）。快照语义不变：单次决策
    /// 持有一个 ArcSwap 图快照，reload 原子换图不影响在途决策。
    pub fn evaluate_pair(
        &self,
        fk_user_attribute: i64,
        fk_object_attribute: i64,
        operation: &str,
        ctx: &ConditionContext,
    ) -> (Decision, Vec<MatchedRule>) {
        let pg = self.policy_graph();
        evaluate_pair_in(&pg, fk_user_attribute, fk_object_attribute, operation, ctx)
    }

    /// `evaluate_pair` 的图参数化变体（change `add-ngac-audit-trail-view` D2）：
    /// 在**给定** PolicyGraph 上求值，供影响预览用模拟图（删除目标边后的克隆）
    /// 做 before/after 比较。运行时决策路径仍走 `evaluate_pair`（ArcSwap 快照）。
    pub fn evaluate_pair_in(
        &self,
        pg: &PolicyGraph,
        fk_user_attribute: i64,
        fk_object_attribute: i64,
        operation: &str,
        ctx: &ConditionContext,
    ) -> (Decision, Vec<MatchedRule>) {
        evaluate_pair_in(pg, fk_user_attribute, fk_object_attribute, operation, ctx)
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

    /// deny-overrides 全局合并 + admin 遍历后兜底（NGAC_SPEC §2.5）——委托
    /// `ngac_decision::evaluate_merge_in`（唯一实现）。消费方 MUST 调本方法或
    /// crate 同名函数，MUST NOT 自带第二份合并循环：`decide_access` /
    /// `explain_access` / 访问审查（review）/ 删除影响预览（impact）。策略矩阵
    /// 的 cell 级索引路径为规则投影面的性能特化，其语义由等价性测试守门。
    pub fn evaluate_merge_in(
        &self,
        pg: &PolicyGraph,
        ua_closure_ids: &[i64],
        oa_closure_ids: &[i64],
        operation: &str,
        ctx: &ConditionContext,
        is_admin: bool,
    ) -> (Decision, Vec<PairOutcome>) {
        evaluate_merge_in(pg, ua_closure_ids, oa_closure_ids, operation, ctx, is_admin)
    }
}

impl Default for Pdp {
    fn default() -> Self {
        Self::new()
    }
}

/// `GET /api/ngac/policy-version` 已随 VersionProbe 直查移除
/// （tighten-pdp-decision-surface：HTTP 探针端点零消费者后删除）。
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    // tighten-pdp-decision-surface B：服务决策端点统一收编 /pdp 子 scope
    // （check/batch/list/columns/decide——全部 Bearer-only + 主体一致性）；
    // /decide/explain（admin 面）与本人族留在 scope 根。
    cfg.service(
        web::scope("/pdp")
            .route("/check", web::post().to(check_access))
            .route("/check/batch", web::post().to(check_access_batch))
            .route("/list", web::post().to(list_resource_access))
            .route("/columns", web::post().to(list_column_access))
            .route("/decide", web::post().to(ngac_decide)),
    )
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

// ── 服务决策主体一致性（tighten-pdp-decision-surface A/B）──────────────────────

/// Bearer 通道检查（B：服务决策面拒绝纯 cookie 通道——JWT 有效性已由
/// RequireAuth 保证，此处只确认调用来源为 Authorization 头）。
pub(crate) fn require_bearer_channel(req: &HttpRequest) -> Result<(), HttpResponse> {
    let bearer_ok = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.len() > 7 && v.starts_with("Bearer "))
        .unwrap_or(false);
    if bearer_ok {
        Ok(())
    } else {
        Err(HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "BEARER_REQUIRED"
        })))
    }
}

/// 服务决策端点（decide/check/check_batch/list/columns）主体一致性校验。
/// 调用者 Claims 由 RequireAuth 在验证通过后注入 req extensions——
/// 自然人 token：`sub == user_id`；服务令牌（`sub=client:*`）：
/// `svc_user_id == user_id`（PEP 服务端代问语义）。不一致 → 403，不执行决策。
pub(crate) fn enforce_decision_subject(
    req: &HttpRequest,
    user_id: i64,
) -> Result<(), HttpResponse> {
    require_bearer_channel(req)?;
    let extensions = req.extensions();
    let claims = match extensions.get::<crate::auth::jwt::Claims>() {
        Some(c) => c,
        None => {
            log::error!("decision subject check: caller claims missing (middleware ordering?)");
            return Err(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "claims_missing"
            })));
        }
    };
    let ok = if claims.sub.starts_with("client:") {
        claims.svc_user_id > 0 && claims.svc_user_id == user_id
    } else {
        claims
            .sub
            .parse::<i64>()
            .map(|v| v == user_id)
            .unwrap_or(false)
    };
    if ok {
        Ok(())
    } else {
        log::warn!(
            "decision subject mismatch: caller sub='{}' svc_user_id={} vs body user_id={}",
            claims.sub,
            claims.svc_user_id,
            user_id
        );
        Err(HttpResponse::Forbidden().json(serde_json::json!({
            "error": "CROSS_SUBJECT_DECISION_DENIED"
        })))
    }
}

/// 响应义务求值（wire-obligations-into-pep-audit / §2.6）：decide 响应携带义务算子。
///
/// 零配置面短路（`ngac_obligation` 无 active 行 ⇒ 单次 EXISTS 即返回空 vec——
/// 多数部署零开销）；有配置才构建完整上下文：UA 名集经 PIP 同源解析
/// （`get_inherited_user_attributes`），OA 名集经闭包单源（`oa_closure_rows`）。
/// 求值失败 ⇒ 空 vec（失败封闭，不阻断判决响应——与 `pdp/audit.rs` 审计面同极性）。
pub(crate) async fn evaluate_response_obligations(
    pool: &sqlx::PgPool,
    user_id: i64,
    resource: &str,
    action: &str,
    decision: &ngac_decision::Decision,
) -> Vec<String> {
    let configured: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.ngac_obligation \
         WHERE is_active AND deleted_at IS NULL)",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(false);
    if !configured {
        return Vec::new();
    }
    // 资源标识 `{type}:{id}`（与 PEP 同构）
    let (rt, rid) = match resource.rsplit_once(':') {
        Some((rt, id)) => match id.parse::<i64>() {
            Ok(v) => (rt.to_string(), v),
            Err(_) => return Vec::new(),
        },
        None => return Vec::new(),
    };
    let pip = crate::ngac::pip::PostgresPip::new(pool.clone());
    let subject_names: Vec<String> = pip.inherited_ua_names(user_id).await.unwrap_or_default();
    let object_names: Vec<String> = ngac_host::oa_closure_rows(pool, &rt, rid)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(_, name)| name)
        .collect();
    crate::ngac::obligation::evaluate_obligations(
        pool,
        &crate::ngac::obligation::ObligationEvalContext {
            user_id,
            resource,
            action,
            decision: &decision.to_string(),
            subject_attributes: &subject_names,
            object_attributes: &object_names,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    // conditions v1/v2 求值的八例单测已随唯一实现迁入 `ngac-decision` crate
    // （extract-ngac-decision-core 任务 2.3）；本模块保留容器职责的版本缓存测试。

    fn ctx_at(ts: &str) -> ConditionContext {
        ConditionContext {
            now: DateTime::parse_from_rfc3339(ts)
                .unwrap()
                .with_timezone(&Utc),
            user_ua_names: Vec::new(),
            oa_closure_names: Vec::new(),
            rhai_eval: None,
        }
    }

    #[test]
    fn conditions_delegate_smoke() {
        // 门面委托冒烟：确认 re-export 与 crate 实现为同一定义（失败封闭语义见 crate 测试）。
        assert!(evaluate_conditions(&None, &ctx_at("2026-06-01T00:00:00Z")));
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

    /// 响应义务求值（wire-obligations-into-pep-audit）：零配置短路 + 有配置求值下发。
    /// 运行：`cargo test -p gateway-sso --lib response_obligations -- --ignored --test-threads=1`
    ///（需 aliothstudio_test）。
    #[tokio::test]
    #[ignore = "needs test DB"]
    async fn response_obligations_short_circuit_and_evaluate() {
        let pool = test_pool().await;
        let user_id: i64 =
            sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users ORDER BY id LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();

        // 夹具：一条 Permit 义务（event_pattern 恒真），测后自清
        sqlx::query(
            r#"INSERT INTO isahl_auth.ngac_obligation
                 (o_name, event_pattern, response_ops, priority, is_active)
               VALUES ('z-test-resp-obligation', 'true', '"[\"z:resp-op\"]"', 1, TRUE)
               ON CONFLICT DO NOTHING"#,
        )
        .execute(&pool)
        .await
        .unwrap();

        let got = super::evaluate_response_obligations(
            &pool,
            user_id,
            "z-test-resp-rt:0",
            "read",
            &ngac_decision::Decision::Permit,
        )
        .await;
        assert!(
            got.iter().any(|o| o.contains("z:resp-op")),
            "有配置面 ⇒ 求值下发：{got:?}"
        );

        sqlx::query(
            "DELETE FROM isahl_auth.ngac_obligation WHERE o_name = 'z-test-resp-obligation'",
        )
        .execute(&pool)
        .await
        .unwrap();

        // 零配置面短路（夹具清后若仍存他行则本断言面让位——以夹具自清为准）
        let empty_after: bool = sqlx::query_scalar(
            "SELECT NOT EXISTS (SELECT 1 FROM isahl_auth.ngac_obligation WHERE is_active AND deleted_at IS NULL)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        if empty_after {
            let got2 = super::evaluate_response_obligations(
                &pool,
                user_id,
                "z-test-resp-rt:0",
                "read",
                &ngac_decision::Decision::Permit,
            )
            .await;
            assert!(got2.is_empty(), "零配置 ⇒ 短路空 vec：{got2:?}");
        }
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
        // 快照隔离：直接用捕获的图求值（Pdp 已无状态；容器唯一 = ngac_host::Evaluator）
        assert_eq!(
            ngac_decision::evaluate_pair_in(&snap1, ua, oa, "atomic-read1", &ctx).0,
            Decision::Permit,
            "snapshot evaluates the policy it captured"
        );
        assert_eq!(
            ngac_decision::evaluate_pair_in(&snap1, ua, oa, "atomic-read2", &ctx).0,
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
            ngac_decision::evaluate_pair_in(&snap1, ua2, oa, "atomic-read2", &ctx).0,
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

    // ── evaluate_merge_in（deny-overrides + admin 遍历后兜底，唯一合并实现）────────

    fn mk_right(pg: &PolicyGraph, id: i64, name: &str) {
        pg.add_access_right(ngac_decision::AccessRight {
            id,
            o_name: name.to_string(),
            applicable_types: Vec::new(),
            is_system: false,
        });
    }

    fn mk_assoc(pg: &PolicyGraph, id: i64, ua: i64, oa: i64, rights: &[i64]) {
        pg.add_association(ngac_decision::Association {
            id,
            fk_user_attribute: ua,
            fk_object_attribute: oa,
            ak_access_rights: rights.to_vec(),
            conditions: None,
        });
    }

    fn mk_prohibition(pg: &PolicyGraph, id: i64, ua: i64, oa: i64, rights: &[i64], active: bool) {
        pg.add_prohibition(ngac_decision::Prohibition {
            id,
            o_name: format!("proh-{id}"),
            fk_user_attribute: ua,
            fk_object_attribute: oa,
            ak_access_rights: rights.to_vec(),
            is_active: active,
            conditions: None,
        });
    }

    fn ctx_now() -> ConditionContext {
        ConditionContext {
            now: Utc::now(),
            user_ua_names: Vec::new(),
            oa_closure_names: Vec::new(),
            rhai_eval: None,
        }
    }

    #[test]
    fn merge_deny_overrides_across_pairs() {
        let pdp = Pdp::new();
        let pg = PolicyGraph::new();
        mk_right(&pg, 1, "read");
        mk_assoc(&pg, 10, 100, 200, &[1]); // ua=100 × oa=200 → Permit
        mk_prohibition(&pg, 20, 101, 201, &[1], true); // ua=101 × oa=201 → Deny

        let (d, _) =
            pdp.evaluate_merge_in(&pg, &[100, 101], &[200, 201], "read", &ctx_now(), false);
        assert_eq!(d, Decision::Deny);
        // 遍历顺序不影响结果
        let (d2, _) =
            pdp.evaluate_merge_in(&pg, &[101, 100], &[201, 200], "read", &ctx_now(), false);
        assert_eq!(d2, Decision::Deny);
    }

    #[test]
    fn merge_permit_when_only_allow() {
        let pdp = Pdp::new();
        let pg = PolicyGraph::new();
        mk_right(&pg, 1, "read");
        mk_assoc(&pg, 10, 100, 200, &[1]);

        let (d, _) = pdp.evaluate_merge_in(&pg, &[100], &[200], "read", &ctx_now(), false);
        assert_eq!(d, Decision::Permit);
    }

    #[test]
    fn merge_not_applicable_without_matches() {
        let pdp = Pdp::new();
        let pg = PolicyGraph::new();
        mk_right(&pg, 1, "read");

        let (d, _) = pdp.evaluate_merge_in(&pg, &[100], &[200], "read", &ctx_now(), false);
        assert_eq!(d, Decision::NotApplicable);
    }

    #[test]
    fn merge_admin_fallback_permits_when_all_not_applicable() {
        let pdp = Pdp::new();
        let pg = PolicyGraph::new();
        mk_right(&pg, 1, "read");
        mk_assoc(&pg, 10, 100, 200, &[1]); // 存在 association，但不在本闭包对上

        let (d, _) = pdp.evaluate_merge_in(&pg, &[999], &[888], "read", &ctx_now(), true);
        assert_eq!(d, Decision::Permit, "admin 遍历后兜底：全不适用 → Permit");
    }

    #[test]
    fn merge_admin_still_denied_by_prohibition() {
        let pdp = Pdp::new();
        let pg = PolicyGraph::new();
        mk_right(&pg, 1, "read");
        mk_prohibition(&pg, 20, 999, 888, &[1], true);

        let (d, _) = pdp.evaluate_merge_in(&pg, &[999], &[888], "read", &ctx_now(), true);
        assert_eq!(
            d,
            Decision::Deny,
            "deny-overrides 无例外：prohibition 约束 admin"
        );
    }

    #[test]
    fn merge_conditions_fail_closed() {
        let pdp = Pdp::new();
        let pg = PolicyGraph::new();
        mk_right(&pg, 1, "read");
        // 时间窗未到的 prohibition → 不满足 → 不扣减
        pg.add_prohibition(ngac_decision::Prohibition {
            id: 20,
            o_name: "proh-future".to_string(),
            fk_user_attribute: 100,
            fk_object_attribute: 200,
            ak_access_rights: vec![1],
            is_active: true,
            conditions: Some(serde_json::json!({ "not_before": "2999-01-01T00:00:00Z" })),
        });
        mk_assoc(&pg, 10, 100, 200, &[1]);

        let (d, _) = pdp.evaluate_merge_in(&pg, &[100], &[200], "read", &ctx_now(), false);
        assert_eq!(d, Decision::Permit);
    }

    #[test]
    fn merge_inactive_prohibition_not_denying() {
        let pdp = Pdp::new();
        let pg = PolicyGraph::new();
        mk_right(&pg, 1, "read");
        mk_assoc(&pg, 10, 100, 200, &[1]);
        mk_prohibition(&pg, 20, 100, 200, &[1], false); // is_active=false

        let (d, _) = pdp.evaluate_merge_in(&pg, &[100], &[200], "read", &ctx_now(), false);
        assert_eq!(d, Decision::Permit);
    }
}
