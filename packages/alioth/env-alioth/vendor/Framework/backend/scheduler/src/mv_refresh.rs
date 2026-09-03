//! 库存物化视图自动刷新 handler（framework-scheduler 注册，plan_code=`mv-inventory-refresh`）
//!
//! 校准机制：mv_inventory 是 REFRESH 时点快照——业务写路径（守卫原语/签收/取消回补）
//! 已各自显式 REFRESH CONCURRENTLY，本 handler 作为**周期自动兜底**（漏刷/外部直插/
//! 进程内其他路径写库存后未刷新），消除"判定与展示分叉"。与 task-deadline-check 同
//! 模式（全局计划，任意 namespace 生效）。

use crate::{ScheduledHandler, SchedulerContext, SchedulerError, SchedulerResult};
use async_trait::async_trait;

/// 计划 code（zc_id_plan-perform 全局种子行）
pub const MV_INVENTORY_REFRESH_PLAN_CODE: &str = "mv-inventory-refresh";

/// 库存物化视图刷新 handler
pub struct MvInventoryRefreshHandler {
    pool: sqlx::PgPool,
}

impl MvInventoryRefreshHandler {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// 单轮刷新（pub：集成测试直调）
    pub async fn refresh_once(&self) -> Result<(), SchedulerError> {
        // CONCURRENTLY 需唯一索引（ensure_mv_inventory 已建 idx_mv_inventory_id）；
        // 刷新失败仅 warn（业务路径已有显式刷新，周期兜底尽力而为）
        // 批注 555ca3ab 复现链：mv 未填充（ispopulated=false，schema 重放仅建不填）时
        // CONCURRENTLY 永远报错（死锁）——先非并发初始 REFRESH 填充，再 CONCURRENTLY
        let populated: bool = sqlx::query_scalar(
            "SELECT ispopulated FROM pg_matviews WHERE schemaname = 'isahl' AND matviewname = 'mv_inventory'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(SchedulerError::Database)?;
        if !populated {
            sqlx::query("REFRESH MATERIALIZED VIEW isahl.mv_inventory")
                .execute(&self.pool)
                .await
                .map_err(SchedulerError::Database)?;
            return Ok(()); // 非并发 REFRESH 已填充
        }
        sqlx::query("REFRESH MATERIALIZED VIEW CONCURRENTLY isahl.mv_inventory")
            .execute(&self.pool)
            .await
            .map_err(SchedulerError::Database)?;
        Ok(())
    }
}

#[async_trait]
impl ScheduledHandler for MvInventoryRefreshHandler {
    fn plan_code(&self) -> &str {
        MV_INVENTORY_REFRESH_PLAN_CODE
    }

    async fn run(&self, _ctx: &SchedulerContext) -> Result<SchedulerResult, SchedulerError> {
        self.refresh_once().await?;
        Ok(SchedulerResult {
            summary: "mv_inventory 周期自动刷新完成".to_string(),
            processed: 1,
        })
    }
}
