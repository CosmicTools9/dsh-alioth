//! 库存/物权物化视图自动刷新 handler（framework-scheduler 注册，plan_code=`mv-inventory-refresh`）
//!
//! 校准机制：mv_inventory / mv_title_ownership 是 REFRESH 时点快照——业务写路径
//! （守卫原语/签收/取消回补）已各自显式 REFRESH CONCURRENTLY，本 handler 作为
//! **周期自动兜底**（漏刷/外部直插/进程内其他路径写库存后未刷新），消除
//! "判定与展示分叉"。与 task-deadline-check 同模式（全局计划，任意 namespace 生效）。

use crate::{ScheduledHandler, SchedulerContext, SchedulerError, SchedulerResult};
use async_trait::async_trait;

/// 计划 code（zc_id_plan-perform 全局种子行）
pub const MV_INVENTORY_REFRESH_PLAN_CODE: &str = "mv-inventory-refresh";

/// 库存/物权物化视图刷新 handler
pub struct MvInventoryRefreshHandler {
    pool: sqlx::PgPool,
}

impl MvInventoryRefreshHandler {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// 单轮刷新（pub：集成测试直调）——顺序刷新 mv_inventory 与 mv_title_ownership；
    /// 前置 ensure_* 自愈（幂等）：同步/重建后的库可能尚未跑过任何业务写路径
    /// （视图由守卫原语 ensure 创建），周期兜底不得因视图缺失而恒错。
    pub async fn refresh_once(&self) -> Result<(), SchedulerError> {
        trigger_registry::stock_materialization::ensure_mv_inventory(&self.pool)
            .await
            .map_err(SchedulerError::Internal)?;
        trigger_registry::stock_materialization::ensure_mv_title_ownership(&self.pool)
            .await
            .map_err(SchedulerError::Internal)?;
        refresh_matview(&self.pool, "mv_inventory").await?;
        refresh_matview(&self.pool, "mv_title_ownership").await?;
        Ok(())
    }
}

/// 单视图刷新：CONCURRENTLY 需唯一索引（ensure_* 已建）；
/// 刷新失败仅由调用方 warn（业务路径已有显式刷新，周期兜底尽力而为）。
/// 批注 555ca3ab 复现链：mv 未填充（ispopulated=false，schema 重放仅建不填）时
/// CONCURRENTLY 永远报错（死锁）——先非并发初始 REFRESH 填充，再 CONCURRENTLY
async fn refresh_matview(pool: &sqlx::PgPool, name: &str) -> Result<(), SchedulerError> {
    let populated: bool = sqlx::query_scalar(
        "SELECT ispopulated FROM pg_matviews WHERE schemaname = 'isahl' AND matviewname = $1",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .map_err(SchedulerError::Database)?;
    if !populated {
        let refresh_sql = format!("REFRESH MATERIALIZED VIEW isahl.{name}");
        sqlx::query(sqlx::AssertSqlSafe(refresh_sql.as_str()))
            .execute(pool)
            .await
            .map_err(SchedulerError::Database)?;
        return Ok(()); // 非并发 REFRESH 已填充
    }
    let refresh_sql = format!("REFRESH MATERIALIZED VIEW CONCURRENTLY isahl.{name}");
    sqlx::query(sqlx::AssertSqlSafe(refresh_sql.as_str()))
        .execute(pool)
        .await
        .map_err(SchedulerError::Database)?;
    Ok(())
}

#[async_trait]
impl ScheduledHandler for MvInventoryRefreshHandler {
    fn plan_code(&self) -> &str {
        MV_INVENTORY_REFRESH_PLAN_CODE
    }

    async fn run(&self, _ctx: &SchedulerContext) -> Result<SchedulerResult, SchedulerError> {
        self.refresh_once().await?;
        Ok(SchedulerResult {
            summary: "mv_inventory/mv_title_ownership 周期自动刷新完成".to_string(),
            processed: 2,
        })
    }
}
