//! NGAC AGE 图投影运行时自愈（change `add-age-graph-projection` A-1d；
//! rebuild-only 见 change `fix-age-ngac-cypher-trigger-crash`）
//!
//! 背景：reset-db 从 Backup 快照重建（零 DDL 交付，不重放迁移），且
//! `pg_dump --exclude-schema=ag_catalog` 使快照**不含图注册表**——restore 后
//! `isahl_auth` AGE 图注册必然丢失。本模块在 `build_server`（独立 SSO 与
//! Gateway 内嵌共用组合根）启动时幂等自愈：
//!
//! - 图已注册且对账零漂移 → 跳过（常态快速路径，一轮探测即返回）；
//! - 未注册 → 重放 037（图引导 + label ensure + rebuild）；
//! - 漂移非零（写路径不再同事务同步后属常态）→ 重放 038（停用 AGE cypher
//!   同步触发器 + 退役同步函数 + 全量重建）。
//!
//! **顺序契约**：任何路径重放 037 之后 MUST 重放 038——037 会重建 cypher
//! 同步触发器，而 AGE 1.8.0 + PG18 下该路径可致后端段错误（signal 11，全簇
//! reinit，且 PL/pgSQL 捕获不到）。038 是唯一的「写路径去 cypher」保证。
//!
//! 失败仅告警不阻断启动（AGE 属投影层，缺失 = 读路径降级，不影响决策——
//! NGAC_SPEC §11 / change design D1/D4）。

use sqlx::PgPool;

/// 037 迁移全文（幂等重放即自愈；与 namespace-db / init_db 通道共享同一文件）。
const PROJECTION_MIGRATION: &str =
    include_str!("../../migrations/037_age_ngac_graph_projection.sql");

/// 038 迁移全文：写路径去 cypher（rebuild-only）+ 全量重建（幂等）。
const PROJECTION_REBUILD_ONLY_MIGRATION: &str =
    include_str!("../../migrations/038_age_ngac_projection_rebuild_only.sql");

/// 启动自愈入口：fire-and-forget 语义由调用方决定，本函数同步完成检测与重放。
pub async fn ensure_ngac_age_projection(pool: &PgPool) {
    match graph_registered(pool).await {
        Ok(false) => {
            log::info!("[age-projection] 图未注册，重放 037+038 自愈...");
            if let Err(e) = apply(pool, PROJECTION_MIGRATION).await {
                log::warn!("[age-projection] 037 重放失败（读路径将保持 SQL 降级）: {e}");
                return;
            }
        }
        Ok(true) => {}
        Err(e) => {
            log::warn!("[age-projection] 注册探测异常（按需自愈处理）: {e}");
        }
    }
    match needs_heal(pool).await {
        Ok(false) => log::info!("[age-projection] 图就绪且对账零漂移，跳过自愈"),
        Ok(true) => {
            log::info!("[age-projection] 检测到漂移，重放 038（去 cypher + 重建）...");
            match apply(pool, PROJECTION_REBUILD_ONLY_MIGRATION).await {
                Ok(()) => log::info!("[age-projection] 自愈完成"),
                Err(e) => log::warn!("[age-projection] 自愈失败（读路径将保持 SQL 降级）: {e}"),
            }
        }
        Err(e) => log::warn!("[age-projection] 对账探测异常: {e}"),
    }
}

/// 图是否已注册（扩展 + ag_graph；表不存在/权限异常 → Err → 调用方按需重放）。
async fn graph_registered(pool: &PgPool) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_extension WHERE extname = 'age') \
         AND EXISTS(SELECT 1 FROM ag_catalog.ag_graph WHERE name = 'isahl_auth')",
    )
    .fetch_one(pool)
    .await
}

/// 是否需要自愈。判据（任一成立即需）：① 图未就绪；② 仍存在 AGE cypher 同步
/// 触发器（写路径崩溃面，MUST 恒为 0）；③ 对账漂移非零。
/// 写路径去 cypher（rebuild-only）后，漂移属常态而非缺口——重建即收敛。
async fn needs_heal(pool: &PgPool) -> Result<bool, sqlx::Error> {
    // 就绪判据单一源 = isahl_auth.age_ngac_graph_ready()（扩展可用 + 注册指向当前 schema +
    // 期望 label 关系齐备）。本模块 MUST NOT 自带第二份「按名判据」：整库重建后
    // 「注册行存活、label 关系随 schema 消失」的悬挂态会被按名判据误判为就绪，进而让
    // 触发器对不存在的关系执行 cypher()——AGE C 层段错误不可被 plpgsql EXCEPTION 捕获，
    // 会击穿整个 PG 实例（2026-09-20 事故）。函数不存在（首装库）→ Err → 走重放。
    let ready: bool = sqlx::query_scalar("SELECT isahl_auth.age_ngac_graph_ready()")
        .fetch_one(pool)
        .await?;
    if !ready {
        return Ok(true);
    }
    let cypher_triggers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger t JOIN pg_proc p ON p.oid = t.tgfoid \
          WHERE NOT t.tgisinternal AND p.proname LIKE 'age\\_sync\\_%'",
    )
    .fetch_one(pool)
    .await?;
    if cypher_triggers > 0 {
        return Ok(true);
    }
    // 对账：函数缺失（Err）视为需自愈；drift 非零视为需自愈
    let drift: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.age_ngac_projection_diff() WHERE drift <> 0",
    )
    .fetch_one(pool)
    .await?;
    Ok(drift != 0)
}

async fn apply(pool: &PgPool, sql: &'static str) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(sql).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::common::testing::connect_test_db;
    use std::sync::LazyLock;
    use tokio::sync::Mutex;

    /// 图引导类测试互斥：破坏性场景会临时丢注册/丢投影表，须串行以免互相观察中间态。
    static HEAL_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    /// 不变量（change `fix-age-ngac-cypher-trigger-crash`）：写路径 MUST NOT 存在
    /// AGE cypher 同步触发器——AGE 1.8.0 + PG18 下该路径可致后端进程段错误
    /// （signal 11 → 全簇 reinit → 并行会话连接被掐断，且 PL/pgSQL 捕获不到）。
    /// 自愈（037→038 顺序）后 MUST 零触发器、零漂移。
    #[tokio::test]
    async fn ensure_projection_leaves_no_cypher_triggers() {
        let _guard = HEAL_LOCK.lock().await;
        let pool = connect_test_db().await;
        ensure_ngac_age_projection(&pool).await;
        let triggers: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_trigger t JOIN pg_proc p ON p.oid = t.tgfoid \
              WHERE NOT t.tgisinternal AND p.proname LIKE 'age\\_sync\\_%'",
        )
        .fetch_one(&pool)
        .await
        .expect("触发器探测不应抛错");
        assert_eq!(
            triggers, 0,
            "自愈后 MUST NOT 残留 AGE cypher 同步触发器（写路径崩溃面）"
        );
        let drift: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM isahl_auth.age_ngac_projection_diff() WHERE drift <> 0",
        )
        .fetch_one(&pool)
        .await
        .expect("对账不应抛错");
        assert_eq!(drift, 0, "自愈后对账 MUST 零漂移");
    }

    /// 回归（2026-09-20 实证缺陷）：**restore 态**——注册丢失而图数据仍在——历史实现
    /// 在残留检测处 RAISE EXCEPTION（`count(*)` 经继承计入 `User`/`HAS_ATTRIBUTE` 等子表，
    /// 实测 1933 行）⇒ 整批迁移中止、注册不恢复、NGAC 读路径永降级。
    /// 自愈 MUST 重新注册并由 rebuild 回到零漂移。
    #[tokio::test]
    async fn ensure_projection_heals_after_registration_loss() {
        let _guard = HEAL_LOCK.lock().await;
        let pool = connect_test_db().await;
        let vertices_before: i64 = sqlx::query_scalar(r#"SELECT count(*) FROM isahl_auth."User""#)
            .fetch_one(&pool)
            .await
            .expect("前置：NGAC 投影应已物化");
        assert!(vertices_before > 0, "前置：test 库应有已物化顶点");

        sqlx::query(
            "DELETE FROM ag_catalog.ag_label \
              WHERE graph = (SELECT graphid FROM ag_catalog.ag_graph WHERE name = 'isahl_auth')",
        )
        .execute(&pool)
        .await
        .expect("模拟 restore：清 label 注册");
        sqlx::query("DELETE FROM ag_catalog.ag_graph WHERE name = 'isahl_auth'")
            .execute(&pool)
            .await
            .expect("模拟 restore：清图注册");
        assert!(
            needs_heal(&pool).await.expect("探测不应抛错"),
            "注册丢失后 MUST 判需自愈"
        );

        ensure_ngac_age_projection(&pool).await;

        let registered: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM ag_catalog.ag_graph WHERE name = 'isahl_auth')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(registered, "自愈后图 MUST 重新注册");
        let drift: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM isahl_auth.age_ngac_projection_diff() WHERE drift <> 0",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(drift, 0, "自愈后对账 MUST 零漂移");
        let triggers_after: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_trigger t JOIN pg_proc p ON p.oid = t.tgfoid \
              WHERE NOT t.tgisinternal AND p.proname LIKE 'age\\_sync\\_%'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            triggers_after, 0,
            "自愈后 MUST NOT 残留 AGE cypher 同步触发器（写路径崩溃面）"
        );
        let vertices_after: i64 = sqlx::query_scalar(r#"SELECT count(*) FROM isahl_auth."User""#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(vertices_after > 0, "自愈后顶点 MUST 由 rebuild 重建");
    }
}
