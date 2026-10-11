//! 宿主侧求值器：唯一策略图快照容器 + 资源作用域决策输入装配 + 决策编排
//! （openspec change `extract-ngac-decision-host`）。
//!
//! 语义与 `common::permissions::require_resource_access` / SSO `pdp/decide.rs` 收敛于此：
//! 两处宿主 MUST 经本 `Evaluator` 消费，MUST NOT 各自重装装配或重排决策序。

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, LazyLock};

use arc_swap::ArcSwap;
use chrono::Utc;
use sqlx::PgPool;
use tokio::sync::Mutex;

use crate::input::{
    attr_names_include_admin, creator_derived_permits, label_clearance_denies,
    load_policy_graph_with_expiry, oa_closure_rows,
};
use ngac_decision::{ConditionContext, Decision, PairOutcome, PolicyGraph};

/// 资源作用域决策输入（`prepare` 产出）。
#[derive(Debug, Clone)]
pub struct DecisionInputs {
    pub ua_ids: Vec<i64>,
    pub ua_names: Vec<String>,
    pub oa_ids: Vec<i64>,
    pub oa_names: Vec<String>,
    pub is_admin: bool,
    /// `Some(true)` = 密级上限命中（判据 SQL 可用）；`None`/`Some(false)` = 不介入。
    pub ceiling_denied: Option<bool>,
}

impl DecisionInputs {
    /// 空装配（bootstrap 等无策略上下文；义务上下文回退空名集，fail-closed）。
    pub(crate) fn empty() -> Self {
        Self {
            ua_ids: Vec::new(),
            ua_names: Vec::new(),
            oa_ids: Vec::new(),
            oa_names: Vec::new(),
            is_admin: false,
            ceiling_denied: None,
        }
    }
}

/// 决策 + 轨迹（`explain` 产出）。
#[derive(Debug, Clone)]
pub struct Verdict {
    pub decision: Decision,
    pub outcomes: Vec<PairOutcome>,
}

/// 唯一策略图快照容器（`ArcSwap` 原子发布 + 版本探针 + reload 串行化）。
///
/// 合并了原先 `common::permissions::POLICY_SNAPSHOT` 与 SSO `GLOBAL_PDP` 两处同构容器：
/// 嵌入模式（同进程同时链 `common` 与 `gateway-sso`）下只有一个策略图实例。
/// MUST NOT 缓存 (user, resource, action) → 决策结果（决策立即生效契约）。
pub struct Evaluator {
    graph: ArcSwap<PolicyGraph>,
    version: AtomicI64,
    /// 最早到期时刻（epoch 秒；`0` = 无到期行）——到期即重载快照（change
    /// `add-ngac-grant-revocation-and-expiry`）。
    next_expiry: AtomicI64,
    reload_lock: Mutex<()>,
}

static GLOBAL_EVALUATOR: LazyLock<Evaluator> = LazyLock::new(|| Evaluator {
    graph: ArcSwap::from_pointee(PolicyGraph::new()),
    version: AtomicI64::new(-1),
    next_expiry: AtomicI64::new(0),
    reload_lock: Mutex::new(()),
});

impl Default for Evaluator {
    fn default() -> Self {
        Self::new()
    }
}

impl Evaluator {
    pub fn new() -> Self {
        Self {
            graph: ArcSwap::from_pointee(PolicyGraph::new()),
            version: AtomicI64::new(-1),
            next_expiry: AtomicI64::new(0),
            reload_lock: Mutex::new(()),
        }
    }

    /// 进程级共享实例（决策路径统一入口）。
    pub fn global() -> &'static Evaluator {
        &GLOBAL_EVALUATOR
    }

    /// 当前生效策略图快照（`Arc` 引用计数，reload 不使其失效）。供投影面消费。
    pub fn policy_graph(&self) -> Arc<PolicyGraph> {
        self.graph.load_full()
    }

    /// 当前生效策略版本（缓存键用；`-1` = 从未加载）。矩阵/审查 TTL 缓存以
    /// `(键, 版本)` 失效，故消费方 MUST 在 `ensure_policy_loaded` 后取本值。
    pub fn version(&self) -> i64 {
        self.version.load(Ordering::Acquire)
    }

    /// 决策前调用：`isahl_auth.ngac_policy_version` 单行探针，版本变化才全量重建 + 原子换入。
    /// 空版本表 ⇒ 幂等插入首行（version=1；NGAC_SPEC §8 空表初始化契约）。失败
    /// （表缺失 / DB 错误）⇒ `Err` 上抛，由调用方 fail-closed。
    pub async fn ensure_policy_loaded(&self, pool: &PgPool) -> sqlx::Result<()> {
        let db_version = Self::fetch_policy_version(pool).await?;
        if self.version.load(Ordering::Acquire) == db_version
            && db_version != -1
            && !self.expiry_due()
        {
            return Ok(());
        }
        let _guard = self.reload_lock.lock().await;
        // double-check：等待期间其它请求可能已完成重建（版本一致且无到期行）。
        let db_version = Self::fetch_policy_version(pool).await?;
        if self.version.load(Ordering::Acquire) == db_version
            && db_version != -1
            && !self.expiry_due()
        {
            return Ok(());
        }
        let (pg, next_expiry) = load_policy_graph_with_expiry(pool).await?;
        self.graph.store(Arc::new(pg));
        self.next_expiry.store(
            next_expiry.map(|t| t.timestamp()).unwrap_or(0),
            Ordering::Release,
        );
        self.version.store(db_version, Ordering::Release);
        Ok(())
    }

    /// 快照内最早到期时刻是否已到（`0` = 无到期行 ⇒ 恒 false）——
    /// 到期即使无任何写入也 MUST 触发一次重载，使到期关联即时退出生效面。
    fn expiry_due(&self) -> bool {
        let exp = self.next_expiry.load(Ordering::Acquire);
        exp != 0 && Utc::now().timestamp() >= exp
    }

    /// 读取策略版本（单行轻查询）。空表 ⇒ 幂等插首行 `version=1` 并重读
    /// （`ON CONFLICT DO NOTHING` 并发安全；插入后 re-SELECT 兜底）。
    async fn fetch_policy_version(pool: &PgPool) -> sqlx::Result<i64> {
        if let Some(v) = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM isahl_auth.ngac_policy_version WHERE id = 1",
        )
        .fetch_optional(pool)
        .await?
        {
            return Ok(v);
        }
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_policy_version (id, version) VALUES (1, 1) \
             ON CONFLICT (id) DO NOTHING",
        )
        .execute(pool)
        .await?;
        sqlx::query_scalar("SELECT version FROM isahl_auth.ngac_policy_version WHERE id = 1")
            .fetch_one(pool)
            .await
    }

    /// 用户有效 UA 闭包行（`id + 名称`）：唯一实现 = [`crate::input::ua_closure_rows`]。
    async fn ua_closure_rows(
        &self,
        pool: &PgPool,
        user_id: i64,
        resource_type: &str,
        fk_resource: i64,
    ) -> sqlx::Result<Vec<(i64, String)>> {
        crate::input::ua_closure_rows(pool, user_id, resource_type, fk_resource).await
    }

    /// 资源作用域输入装配：密级上限 + UA 闭包（有效 ∪ 属权）+ OA 闭包 + admin 判据。
    /// 一次装配可供单条 decide 或投影面多行复用（避免 N+1）。
    pub async fn prepare(
        &self,
        pool: &PgPool,
        user_id: i64,
        resource_type: &str,
        fk_resource: i64,
    ) -> sqlx::Result<DecisionInputs> {
        // 基础设施缺失 / 查询失败 ⇒ 不介入（None）；命中 ⇒ Some(true)。
        let ceiling_denied = label_clearance_denies(pool, resource_type, fk_resource, user_id)
            .await
            .ok();
        let ua_rows = self
            .ua_closure_rows(pool, user_id, resource_type, fk_resource)
            .await?;
        let oa_rows = oa_closure_rows(pool, resource_type, fk_resource).await?;
        let is_admin = attr_names_include_admin(ua_rows.iter().map(|r| r.1.as_str()));
        Ok(DecisionInputs {
            ua_ids: ua_rows.iter().map(|r| r.0).collect(),
            ua_names: ua_rows.iter().map(|r| r.1.clone()).collect(),
            oa_ids: oa_rows.iter().map(|r| r.0).collect(),
            oa_names: oa_rows.iter().map(|r| r.1.clone()).collect(),
            is_admin,
            ceiling_denied,
        })
    }

    fn ctx(
        inputs: &DecisionInputs,
        user_id: i64,
        resource_type: &str,
        fk_resource: i64,
        action: &str,
    ) -> ConditionContext {
        ConditionContext {
            now: Utc::now(),
            user_ua_names: inputs.ua_names.clone(),
            oa_closure_names: inputs.oa_names.clone(),
            rhai_eval: Some(crate::input::rhai_condition_evaluator(
                user_id,
                format!("{resource_type}:{fk_resource}"),
                action.to_string(),
            )),
        }
    }

    /// 单资源决策（资源作用域）：ceiling → bootstrap → 合并 → creator 翻转（`decide_ordered`）。
    /// 仅 `Decision::Permit` 视为放行；`Deny`/`NotApplicable` 视为拒绝（调用方 fail-closed）。
    pub async fn decide(
        &self,
        pool: &PgPool,
        user_id: i64,
        resource_type: &str,
        fk_resource: i64,
        action: &str,
    ) -> sqlx::Result<Decision> {
        Ok(self
            .decide_detailed(pool, user_id, resource_type, fk_resource, action)
            .await?
            .0)
    }

    /// 与 [`Self::decide`] 同判，但**一并返回装配真值**（`DecisionInputs`）——供消费方
    /// （如 PDP 义务求值上下文）复用同一装配，MUST NOT 二次查询（单源）。
    pub async fn decide_detailed(
        &self,
        pool: &PgPool,
        user_id: i64,
        resource_type: &str,
        fk_resource: i64,
        action: &str,
    ) -> sqlx::Result<(Decision, DecisionInputs)> {
        self.ensure_policy_loaded(pool).await?;
        if self.policy_graph().bootstrap_empty() {
            return Ok((Decision::Permit, DecisionInputs::empty()));
        }
        let inputs = self
            .prepare(pool, user_id, resource_type, fk_resource)
            .await?;
        if inputs.ceiling_denied == Some(true) {
            return Ok((Decision::Deny, inputs));
        }
        let pg = self.policy_graph();
        let ctx = Self::ctx(&inputs, user_id, resource_type, fk_resource, action);
        let merged = ngac_decision::evaluate_merge_in(
            &pg,
            &inputs.ua_ids,
            &inputs.oa_ids,
            action,
            &ctx,
            inputs.is_admin,
        )
        .0;
        let derived_permits = merged == Decision::NotApplicable
            && creator_derived_permits(pool, user_id, fk_resource, action)
                .await
                .unwrap_or(false);
        Ok((
            ngac_decision::decide_ordered(None, false, merged, derived_permits),
            inputs,
        ))
    }

    /// 决策 + 完备轨迹（explain）：与 [`Self::decide`] 同源同遍历（`evaluate_merge_in` 全对不早停）。
    pub async fn explain(
        &self,
        pool: &PgPool,
        user_id: i64,
        resource_type: &str,
        fk_resource: i64,
        action: &str,
    ) -> sqlx::Result<Verdict> {
        self.ensure_policy_loaded(pool).await?;
        if self.policy_graph().bootstrap_empty() {
            return Ok(Verdict {
                decision: Decision::Permit,
                outcomes: Vec::new(),
            });
        }
        let inputs = self
            .prepare(pool, user_id, resource_type, fk_resource)
            .await?;
        if inputs.ceiling_denied == Some(true) {
            return Ok(Verdict {
                decision: Decision::Deny,
                outcomes: Vec::new(),
            });
        }
        let pg = self.policy_graph();
        let ctx = Self::ctx(&inputs, user_id, resource_type, fk_resource, action);
        let (merged, outcomes) = ngac_decision::evaluate_merge_in(
            &pg,
            &inputs.ua_ids,
            &inputs.oa_ids,
            action,
            &ctx,
            inputs.is_admin,
        );
        let derived_permits = merged == Decision::NotApplicable
            && creator_derived_permits(pool, user_id, fk_resource, action)
                .await
                .unwrap_or(false);
        Ok(Verdict {
            decision: ngac_decision::decide_ordered(None, false, merged, derived_permits),
            outcomes,
        })
    }
}
