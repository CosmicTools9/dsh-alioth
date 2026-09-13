//! SSO 周期后台维护（fix-sso-auth-gaps G4）
//!
//! 独立进程（`build_server`）与 Gateway 内嵌（`#[cfg(feature="sso")]`）两种形态
//! 均调用 [`spawn`] 启动：周期执行 ① 过期会话标记 ② 超保留期会话物理删除
//! （子表 FK 级联） ③ 超保留期吊销/过期 refresh token 物理删除。
//!
//! 全部 SQL 幂等、条件删除——多实例并发执行安全；无进程内存态
//! （SECURITY_SPEC §11.1 内存态清单无需登记）。单轮失败仅记日志，循环不退出。

use std::time::Duration;

use sqlx::PgPool;

use crate::auth::login::purge_expired_tokens;
use crate::auth::session::SessionManager;

/// 清理周期：1 小时
const CLEANUP_INTERVAL: Duration = Duration::from_secs(3600);
/// 会话保留期：30 天（expired/revoked 行超过保留期才物理删除）
const SESSION_RETENTION_DAYS: i64 = 30;
/// 刷新令牌保留期：30 天（吊销/过期行保留审计窗口后删除）
const TOKEN_RETENTION_DAYS: i64 = 30;

/// 启动周期清理后台任务。必须在 tokio runtime 上下文内调用
/// （actix main / build_server async fn / Gateway main）。
/// 首轮立即执行一次，随后按 [`CLEANUP_INTERVAL`] 周期执行。
pub fn spawn(pool: PgPool) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(CLEANUP_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(e) = run_once(&pool).await {
                log::error!("SSO housekeeping round failed: {}", e);
            }
        }
    });
}

/// 执行一轮清理（对测试与一次性运维暴露；幂等可重入）。
pub async fn run_once(pool: &PgPool) -> anyhow::Result<()> {
    let manager = SessionManager::new(pool.clone());
    let marked = manager.cleanup_expired_sessions().await?;
    let purged_sessions = manager
        .purge_expired_sessions(SESSION_RETENTION_DAYS)
        .await?;
    let purged_tokens = purge_expired_tokens(pool, TOKEN_RETENTION_DAYS).await?;
    // DB-backed 限流窗口行（fix-sso-residual-gaps G3）：超 1 小时窗口无审计价值
    let purged_rate_limits = sqlx::query(
        "DELETE FROM isahl_auth.auth_rate_limits WHERE window_start < NOW() - INTERVAL '1 hour'",
    )
    .execute(pool)
    .await?;
    log::info!(
        "SSO housekeeping: {} sessions marked expired, {} sessions purged (retention {}d), {} refresh tokens purged (retention {}d), {} rate-limit windows purged",
        marked,
        purged_sessions,
        SESSION_RETENTION_DAYS,
        purged_tokens,
        TOKEN_RETENTION_DAYS,
        purged_rate_limits.rows_affected()
    );
    Ok(())
}
