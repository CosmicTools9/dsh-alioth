//! 逐项应用协议（deep module，change `deepen-chat-action-seam` C3）。
//!
//! chat 动作族（`system_config_apply` / `ngac_org_policy_apply` /
//! `ngac_strategy_apply`）此前各自复制一份应用骨架：admin 门控 → `operations[]`
//! → `EMPTY_OPERATIONS` → `resolve_user_email` → 逐项循环 → `{count, applied}`。
//! 骨架逐字相同，而真正需要一致的部分已经分叉（审计口径三样、测试覆盖
//! 4 / 4 / 0）。
//!
//! 本 module 把骨架收进实现，把两个真正的差异点留作注入缝：
//! - [`ItemExecutor`]：单项的领域执行（解析 + 服务层调用 + 该动作自己的审计口径）；
//! - [`gate_admin`]：admin 门控（判据唯一实现 = `common::permissions::is_admin_user`，
//!   仅错误文本按动作定制）。
//!
//! **不变量（MUST 保持，收拢不得改变）**：门控先于任何取参/写库；`operations`
//! 空集拒绝文本 `EMPTY_OPERATIONS`；逐项执行、失败即停且错误文本
//! `STEP i/N 失败：<原因>（已完成 k 项，不自动回滚）`；回执 `{count, applied}`；
//! 主体邮箱单次解析（MUST NOT 逐项查询）。
//!
//! **不在本模块**：单项形状解析（各动作领域语义）、跨项事务（逐项语义即
//! 「失败即停、不自动回滚」，MUST NOT 借收拢引入整体事务）。

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::PgPool;

use super::tool_bridge::resolve_user_email;

/// admin 门控结果。fail-closed：非 `Ok` 时 [`apply_operations`] MUST NOT
/// 触碰 `params`、MUST NOT 解析邮箱、MUST NOT 写库。
pub type Gate = Result<(), String>;

/// 单动作应用上下文：本次 apply 解析一次、逐项共用。
pub struct ApplyCtx<'a> {
    pub pool: &'a PgPool,
    pub user_id: i64,
    /// 主体主邮箱（缺省回落 `user:{id}`）——回执与审计用。
    pub email: String,
}

impl ApplyCtx<'_> {
    /// 审计主体标识：`common::audit::resolve_subject`（username 优先，回落
    /// `user:{id}`）——唯一实现。按需解析（不预取），供需要它的执行体调用。
    pub async fn subject(&self) -> String {
        let username: Option<String> =
            sqlx::query_scalar("SELECT username FROM isahl_auth.auth_users WHERE id = $1")
                .bind(self.user_id)
                .fetch_optional(self.pool)
                .await
                .ok()
                .flatten();
        common::audit::resolve_subject(username.as_deref().unwrap_or(""), self.user_id)
    }
}

/// 单项执行体：一个动作的全部领域差异所在。
#[async_trait]
pub trait ItemExecutor: Send + Sync {
    /// 执行单项。`Ok(result)` 进入 `applied` 账本；`Err` 触发失败即停，
    /// 文本进入 `STEP i/N 失败：…` 的错误信封。
    async fn apply_one(&self, item: &Value, ctx: &ApplyCtx<'_>) -> Result<Value, String>;
}

/// admin 门控（判据唯一实现 = `common::permissions::is_admin_user`）。
///
/// `what` = 动作对象短语（如「系统设置变更」/「权限规范写入」/「策略操作」），
/// 仅用于复刻各动作既有的 `ADMIN_REQUIRED` 错误文本。
pub async fn gate_admin(pool: &PgPool, user_id: i64, what: &str) -> Gate {
    match common::permissions::is_admin_user(pool, user_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!("ADMIN_REQUIRED: {what}仅限管理员")),
        Err(e) => Err(format!("ADMIN_CHECK_FAILED: {e}")),
    }
}

/// 应用协议：门控 → `operations[]` → 空集拒绝 → 逐项执行 → `{count, applied}`。
///
/// `params` MUST 为动作请求的 params 对象（取 `operations` 数组；缺失/非数组
/// 视为空 ⇒ `EMPTY_OPERATIONS`，与既有语义一致）。
pub async fn apply_operations<E: ItemExecutor>(
    pool: &PgPool,
    user_id: i64,
    gate: Gate,
    params: &Value,
    executor: E,
) -> Result<Value, String> {
    gate?;
    let ops = params
        .get("operations")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if ops.is_empty() {
        return Err("EMPTY_OPERATIONS: operations 不能为空".to_string());
    }
    let ctx = ApplyCtx {
        pool,
        user_id,
        email: resolve_user_email(pool, user_id).await,
    };
    let mut applied: Vec<Value> = Vec::with_capacity(ops.len());
    for (i, op) in ops.iter().enumerate() {
        match executor.apply_one(op, &ctx).await {
            Ok(v) => applied.push(v),
            Err(e) => {
                return Err(format!(
                    "STEP {}/{} 失败：{}（已完成 {} 项，不自动回滚）",
                    i + 1,
                    ops.len(),
                    e,
                    applied.len()
                ));
            }
        }
    }
    Ok(json!({ "count": applied.len(), "applied": applied }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// 记录调用次数的假执行器。惰性池（`connect_lazy`，不建连接）使
    /// `resolve_user_email` 走 `user:{id}` 回落，测试无需 DB。
    ///
    /// 计数器经 `Arc` 外持：`apply_operations` 按值取执行体（`E: ItemExecutor`），
    /// 故调用后不能再借 `rec` 读数。
    struct Recorder {
        calls: Arc<AtomicUsize>,
        /// 0-based：第几个 item 失败（None = 全成功）。
        fail_at: Option<usize>,
    }

    impl Recorder {
        fn new(fail_at: Option<usize>) -> (Self, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    calls: Arc::clone(&calls),
                    fail_at,
                },
                calls,
            )
        }
    }

    #[async_trait]
    impl ItemExecutor for Recorder {
        async fn apply_one(&self, item: &Value, ctx: &ApplyCtx<'_>) -> Result<Value, String> {
            let idx = self.calls.fetch_add(1, Ordering::SeqCst);
            let name = item
                .get("op")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            if self.fail_at == Some(idx) {
                return Err(format!("boom:{name}"));
            }
            Ok(json!({ "op": name, "email": ctx.email }))
        }
    }

    fn lazy_pool() -> PgPool {
        PgPool::connect_lazy("postgres://unused:unused@127.0.0.1:1/unused").expect("lazy pool")
    }

    /// 门控失败：MUST NOT 取参、MUST NOT 调用执行器（fail-closed）。
    #[tokio::test]
    async fn gate_failure_short_circuits_without_touching_params() {
        let pool = lazy_pool();
        let (rec, calls) = Recorder::new(None);
        let params = json!({ "operations": [{ "op": "x" }] });
        let err = apply_operations(
            &pool,
            7,
            Err("ADMIN_REQUIRED: 策略操作仅限管理员".to_string()),
            &params,
            rec,
        )
        .await
        .unwrap_err();
        assert_eq!(err, "ADMIN_REQUIRED: 策略操作仅限管理员");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "门控失败 MUST NOT 触达执行器"
        );
    }

    /// 空集拒绝：文本固定，且不触达执行器（缺失/非数组同判 —— 语义等价于空）。
    #[tokio::test]
    async fn empty_operations_rejected_without_touching_executor() {
        let pool = lazy_pool();
        for params in [json!({ "operations": [] }), json!({})] {
            let (rec, calls) = Recorder::new(None);
            let err = apply_operations(&pool, 7, Ok(()), &params, rec)
                .await
                .unwrap_err();
            assert_eq!(err, "EMPTY_OPERATIONS: operations 不能为空");
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
    }

    /// 逐项执行：全部成功 → `{count, applied}`；邮箱单次解析（回落形）。
    #[tokio::test]
    async fn applies_every_item_and_returns_ledger() {
        let pool = lazy_pool();
        let (rec, calls) = Recorder::new(None);
        let out = apply_operations(
            &pool,
            7,
            Ok(()),
            &json!({ "operations": [{ "op": "a" }, { "op": "b" }] }),
            rec,
        )
        .await
        .unwrap();
        assert_eq!(out["count"], 2);
        assert_eq!(out["applied"][0]["op"], "a");
        assert_eq!(out["applied"][1]["op"], "b");
        assert_eq!(out["applied"][0]["email"], "user:7");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// 失败即停：错误信封文本 + 已完成项计数 + 后续项 MUST NOT 执行。
    #[tokio::test]
    async fn failure_stops_immediately_with_completed_count() {
        let pool = lazy_pool();
        let (rec, calls) = Recorder::new(Some(1));
        let err = apply_operations(
            &pool,
            7,
            Ok(()),
            &json!({ "operations": [{ "op": "a" }, { "op": "b" }, { "op": "c" }] }),
            rec,
        )
        .await
        .unwrap_err();
        assert_eq!(err, "STEP 2/3 失败：boom:b（已完成 1 项，不自动回滚）");
        assert_eq!(calls.load(Ordering::SeqCst), 2, "第三项 MUST NOT 执行");
    }

    /// 门控错误文本模板：`ADMIN_REQUIRED: <what>仅限管理员`。
    #[test]
    fn gate_admin_error_template_is_stable() {
        for what in ["系统设置变更", "权限规范写入", "策略操作"] {
            let rendered = format!("ADMIN_REQUIRED: {what}仅限管理员");
            assert!(rendered.starts_with("ADMIN_REQUIRED: "));
            assert!(rendered.ends_with("仅限管理员"));
        }
    }
}
