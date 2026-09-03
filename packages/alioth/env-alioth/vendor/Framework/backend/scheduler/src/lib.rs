//! # framework-scheduler — 通用 cron 调度器
//!
//! 消费 `isahl.zc_id_plan.cron` 计划定义，到点触发注册的业务 handler，
//! 执行实例写 `isahl.zc_id_oper-planing`（operation 族，业务可查询的质量活动记录）。
//!
//! ## 设计要点
//! - 计划定义持久化在 `zc_id_plan`（ALIOTH_ONTOLOGY_SPEC §8.4 cron 用户可写）；
//!   执行实例写 `zc_id_oper-planing`（同 oper-approve 写入范式）。
//! - 调度状态内存态（重启丢失可接受——计划定义持久，丢失触发由下次到点补偿或幂等兜底）。
//! - 执行以 SYSTEM_USER_ID 系统身份，写实例前经 NGAC 授权（对齐 sla_timeout 先例）。
//! - 幂等：同分钟不重复触发；业务副作用由各 handler 自行幂等（code 唯一/事务+行锁）。

mod cron;
pub mod mv_refresh;
pub mod task_deadline;

pub use cron::{CronError, CronSchedule};
pub use mv_refresh::{MvInventoryRefreshHandler, MV_INVENTORY_REFRESH_PLAN_CODE};
pub use task_deadline::{TaskDeadlineHandler, TASK_DEADLINE_PLAN_CODE};

use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 调度器错误
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("cron parse error: {0}")]
    Cron(#[from] CronError),
    #[error("plan execution record failed: {0}")]
    PlanExecution(#[from] common::plan_execution::PlanExecutionError),
    #[error("handler not registered: {0}")]
    HandlerNotRegistered(String),
    #[error("internal error: {0}")]
    Internal(String),
}

/// 调度上下文（pool + 计划信息）
#[derive(Clone)]
pub struct SchedulerContext {
    pub pool: PgPool,
    pub plan_id: i64,
    pub plan_code: String,
}

/// 调度执行结果
#[derive(Debug, Clone)]
pub struct SchedulerResult {
    /// 业务执行摘要（写入 oper-planing comments）
    pub summary: String,
    /// 本次处理数量（如检查 N 驳回 M）
    pub processed: u64,
}

/// 定时业务 handler trait
///
/// 业务模块实现并注册到 `SchedulerService`；`plan_code` 绑定 `zc_id_plan.code`。
#[async_trait]
pub trait ScheduledHandler: Send + Sync {
    /// 绑定的计划 code（zc_id_plan.code）
    fn plan_code(&self) -> &str;
    /// 执行一次计划（幂等；失败返回 Err 记日志，不 panic）
    async fn run(&self, ctx: &SchedulerContext) -> Result<SchedulerResult, SchedulerError>;
}

/// 计划行（zc_id_plan 扫描结果）
#[derive(Debug, Clone, sqlx::FromRow)]
struct PlanRow {
    id: i64,
    code: String,
    cron: String,
}

/// 通用调度服务
pub struct SchedulerService {
    pool: PgPool,
    handlers: RwLock<HashMap<String, Arc<dyn ScheduledHandler>>>,
    /// 上次触发分钟（幂等防重复：key=plan id，value=epoch 分钟）
    last_trigger_minute: RwLock<HashMap<i64, i64>>,
}

impl SchedulerService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            handlers: RwLock::new(HashMap::new()),
            last_trigger_minute: RwLock::new(HashMap::new()),
        }
    }

    /// 测试专用：无真实 DB 连接（仅注册/解析测试；不触碰 pool 的路径可安全使用）
    #[cfg(test)]
    fn new_with_unconnected() -> Self {
        // sqlx PgPoolOptions 可零连接构造（懒连接）；注册/解析测试不执行 SQL
        use sqlx::postgres::PgPoolOptions;
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("lazy pool construction");
        Self::new(pool)
    }

    /// 注册业务 handler（plan_code 唯一，重复注册覆盖）
    pub async fn register(&self, handler: Arc<dyn ScheduledHandler>) {
        let code = handler.plan_code().to_string();
        self.handlers.write().await.insert(code, handler);
    }

    /// 启动调度循环（后台 tokio task，不阻塞）
    ///
    /// 每 `scan_interval_secs` 扫描 `zc_id_plan` 中 cron 到点的计划 → 写 oper-planing 实例 → 调 handler。
    /// 调用方持 Arc<SchedulerService>，`start` 消费一份引用。
    pub fn start(self: &Arc<Self>, scan_interval_secs: u64) {
        let service = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(scan_interval_secs)).await;
                if let Err(e) = service.scan_once().await {
                    log::error!("[scheduler] scan failed: {e}");
                }
            }
        });
    }

    /// 单轮扫描：查 cron 到点计划并触发
    pub async fn scan_once(&self) -> Result<(), SchedulerError> {
        let plans: Vec<PlanRow> = sqlx::query_as(
            r#"
            SELECT id, code, cron::text AS cron
            FROM isahl."zc_id_plan"
            WHERE deleted_at IS NULL
              AND cron IS NOT NULL
              AND cron != ''
            ORDER BY id
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let now = Utc::now();
        let now_minute = now.timestamp() / 60;

        for plan in plans {
            let schedule = match CronSchedule::parse(&plan.cron) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("[scheduler] plan {} cron 解析失败: {}", plan.code, e);
                    continue;
                }
            };
            if !schedule.matches(now.timestamp()) {
                continue;
            }
            // 幂等：同分钟不重复触发
            {
                let mut last = self.last_trigger_minute.write().await;
                if last.get(&plan.id) == Some(&now_minute) {
                    continue;
                }
                last.insert(plan.id, now_minute);
            }

            // 找 handler
            let handler = {
                let handlers = self.handlers.read().await;
                handlers.get(&plan.code).cloned()
            };
            let ctx = SchedulerContext {
                pool: self.pool.clone(),
                plan_id: plan.id,
                plan_code: plan.code.clone(),
            };
            let Some(handler) = handler else {
                // 无注册 handler 的计划（如日程重复 cron）：仍写执行实例记录到点执行，
                // 供业务查询（日程重复历史/审计）；不再 warn（个人日程无 handler 属常态）。
                log::info!(
                    "[scheduler] plan {} cron 到点（无业务 handler，写执行实例）",
                    plan.code
                );
                let result = SchedulerResult {
                    summary: "cron 到点（无业务 handler，仅记录执行）".to_string(),
                    processed: 0,
                };
                if let Err(e) = self.record_execution(&ctx, &result).await {
                    log::error!("[scheduler] plan {} 执行实例写入失败: {}", plan.code, e);
                }
                continue;
            };

            match handler.run(&ctx).await {
                Ok(result) => {
                    if let Err(e) = self.record_execution(&ctx, &result).await {
                        log::error!("[scheduler] plan {} 执行实例写入失败: {}", plan.code, e);
                    }
                }
                Err(e) => {
                    log::error!("[scheduler] plan {} handler 执行失败: {}", plan.code, e);
                }
            }
        }
        Ok(())
    }

    /// 写执行实例（zc_id_oper-planing，operation 族）
    async fn record_execution(
        &self,
        ctx: &SchedulerContext,
        result: &SchedulerResult,
    ) -> Result<(), SchedulerError> {
        // 公共 helper（P5 泛化）：与 schedule/contract/airworthiness 计划执行实例同源
        common::plan_execution::record_plan_execution(
            &self.pool,
            ctx.plan_id,
            &format!("计划执行：{} — {}", ctx.plan_code, result.summary),
            None,
            common::SYSTEM_USER_ID,
        )
        .await
        .map_err(SchedulerError::PlanExecution)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct FakeHandler {
        code: String,
        calls: Arc<AtomicU64>,
    }

    #[async_trait]
    impl ScheduledHandler for FakeHandler {
        fn plan_code(&self) -> &str {
            &self.code
        }
        async fn run(&self, _ctx: &SchedulerContext) -> Result<SchedulerResult, SchedulerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(SchedulerResult {
                summary: "fake run".to_string(),
                processed: 1,
            })
        }
    }

    #[tokio::test]
    async fn cron_matches_now() {
        let s = CronSchedule::EveryMinutes(1);
        assert!(s.matches(Utc::now().timestamp()));
    }

    #[tokio::test]
    async fn register_keeps_handler() {
        let svc = Arc::new(SchedulerService::new_with_unconnected());
        let h = Arc::new(FakeHandler {
            code: "t".to_string(),
            calls: Arc::new(AtomicU64::new(0)),
        });
        svc.register(h).await;
        let handlers = svc.handlers.read().await;
        assert_eq!(handlers.len(), 1);
        assert!(handlers.contains_key("t"));
    }
}
