//! 事务辅助模块
//!
//! 提供数据库事务的封装辅助，简化多操作原子性管理。
//!
//! # 使用示例
//!
//! ```rust,ignore
//! use crud::transaction::with_transaction;
//! use sqlx::PgPool;
//!
//! async fn transfer(pool: &PgPool) -> Result<(), sqlx::Error> {
//!     with_transaction(pool, |tx| Box::pin(async move {
//!         // 所有操作使用同一个事务 tx
//!         sqlx::query("UPDATE accounts SET balance = balance - 100 WHERE id = 1")
//!             .execute(&mut **tx)
//!             .await?;
//!         sqlx::query("UPDATE accounts SET balance = balance + 100 WHERE id = 2")
//!             .execute(&mut **tx)
//!             .await?;
//!         Ok(())
//!     })).await
//! }
//! ```

use sqlx::{PgPool, Postgres};

/// 在事务上下文中执行闭包。
///
/// - 成功时自动 `COMMIT`
/// - 失败时自动 `ROLLBACK`
///
/// 闭包接收 `&mut Transaction<'_, Postgres>`，返回值必须是 `Result<T, sqlx::Error>`。
/// 由于 async closure 的 lifetime 限制，闭包需包装为 `Box::pin(async move { ... })`。
pub async fn with_transaction<F, T>(pool: &PgPool, f: F) -> Result<T, sqlx::Error>
where
    F: for<'a> FnOnce(
        &'a mut sqlx::Transaction<'_, Postgres>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<T, sqlx::Error>> + 'a>,
    >,
{
    let mut tx = pool.begin().await?;
    match f(&mut tx).await {
        Ok(result) => {
            tx.commit().await?;
            Ok(result)
        }
        Err(e) => {
            tx.rollback().await?;
            Err(e)
        }
    }
}

/// 为 `AliothRepository` 实现者提供的便捷事务入口。
///
/// 与 `with_transaction` 等价，但从 `&PgPool` 解耦，便于在 Repository 方法中直接调用。
pub async fn with_tx_from_pool<F, T>(pool: &PgPool, f: F) -> Result<T, sqlx::Error>
where
    F: for<'a> FnOnce(
        &'a mut sqlx::Transaction<'_, Postgres>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<T, sqlx::Error>> + 'a>,
    >,
{
    with_transaction(pool, f).await
}
