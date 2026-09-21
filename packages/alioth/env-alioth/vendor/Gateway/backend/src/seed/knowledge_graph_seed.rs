//! 知识图谱 AGE 投影运行时自愈
//!
//! 纯重建制投影（零 isahl 触发器）+ 对账：知识种子重放 / AVIC 服务 CRUD 后图滞后，
//! 本钩子在 Gateway 启动（`ensure_gateway_seed_self_check` 链尾）检测三类缺口 → 幂等
//! 重放 022 迁移（图引导 + label ensure + 全量 rebuild + 多跳读投影函数）。三态探测：
//! ① 图未注册（含 restore 后 `ag_graph` 注册丢失——ag_catalog 被 pg_dump 排除）；
//! ② 对齐漂移（`age_knowledge_projection_diff()` 非零）；③ **投影函数缺失**
//! （`isahl_knowledge.knowledge_multi_hop`——022 新增读面，已就绪库须靠本项补建）。
//!
//! 失败仅告警（知识图属投影层，缺失 = 多跳读降级 SQL，检索端点永不因 AGE 故障失败）。

use sqlx::PgPool;

/// 022 迁移全文（幂等重放即自愈）。
const KNOWLEDGE_MIGRATION: &str =
    include_str!("../../migrations/022_knowledge_graph_projection.sql");

/// 自愈入口：drift 探测 → 重放迁移。
pub async fn ensure_knowledge_graph_self_check(pool: &PgPool) {
    match needs_heal(pool).await {
        Ok(false) => {
            log::info!("[knowledge-graph] 图就绪且对账零漂移，跳过自愈");
        }
        Ok(true) => {
            log::info!("[knowledge-graph] 检测到未注册/漂移/投影函数缺失，重放 022 迁移自愈...");
            if let Err(e) = heal(pool).await {
                log::warn!("[knowledge-graph] 自愈失败（多跳读将保持 SQL 降级）: {e}");
            }
        }
        Err(e) => {
            log::warn!("[knowledge-graph] 探测异常（按需自愈处理）: {e}");
            if let Err(e) = heal(pool).await {
                log::warn!("[knowledge-graph] 自愈失败（多跳读将保持 SQL 降级）: {e}");
            }
        }
    }
}

async fn needs_heal(pool: &PgPool) -> Result<bool, sqlx::Error> {
    let ready: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_extension WHERE extname = 'age') \
         AND EXISTS(SELECT 1 FROM ag_catalog.ag_graph WHERE name = 'isahl_knowledge')",
    )
    .fetch_one(pool)
    .await?;
    if !ready {
        return Ok(true);
    }
    // 投影函数缺失（022 新增读面先行落地于既有已就绪库）：ready + 零漂移即跳过会漏建函数。
    let projection_fn: bool = sqlx::query_scalar(
        "SELECT to_regprocedure('isahl_knowledge.knowledge_multi_hop(text,bigint,integer)') IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;
    if !projection_fn {
        return Ok(true);
    }
    let drift: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_knowledge.age_knowledge_projection_diff() WHERE drift <> 0",
    )
    .fetch_one(pool)
    .await?;
    Ok(drift != 0)
}

async fn heal(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(KNOWLEDGE_MIGRATION).execute(pool).await?;
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

    /// 集成：**收敛后**图就绪零漂移 → needs_heal 判 false（022 已应用）。
    ///
    /// 知识图为**纯重建制**（零触发器）：关系表变动而尚未 rebuild 即产生「陈旧漂移」，
    /// 属正常态（自愈钩子会 rebuild 收敛），并非触发器缺口。故本用例先执行权威重建
    /// （`age_rebuild_knowledge_graph()`）再断言跳过语义，避免受共享 test 库中其他用例
    /// 遗留行的干扰（2026-09-20 实测该干扰会令裸断言偶发失败）。
    #[tokio::test]
    async fn ensure_knowledge_graph_is_noop_when_reconciled() {
        let _guard = HEAL_LOCK.lock().await;
        let pool = connect_test_db().await;
        sqlx::query("SELECT isahl_knowledge.age_rebuild_knowledge_graph()")
            .execute(&pool)
            .await
            .expect("前置：投影重建失败");
        assert!(!needs_heal(&pool).await.expect("探测不应抛错"));
    }

    /// 回归（2026-09-20 实证缺陷）：**restore 态**——`ag_catalog` 被 `pg_dump` 排除 ⇒
    /// 注册丢失，而投影表与图数据仍在（含继承子表行）——历史实现在此 RAISE EXCEPTION
    /// （`count(*)` 经继承计入子表）⇒ 整批迁移中止、注册不恢复、AGE 读路径永降级。
    /// 自愈 MUST 恢复注册并回到零漂移（投影为派生数据，重建权威由关系表提供）。
    #[tokio::test]
    async fn ensure_knowledge_graph_heals_after_registration_loss() {
        let _guard = HEAL_LOCK.lock().await;
        let pool = connect_test_db().await;
        let nodes_before: i64 =
            sqlx::query_scalar(r#"SELECT count(*) FROM isahl_knowledge."Node""#)
                .fetch_one(&pool)
                .await
                .expect("前置：知识投影应已物化");
        assert!(nodes_before > 0, "前置：test 库应有已物化节点");

        sqlx::query(
            "DELETE FROM ag_catalog.ag_label \
              WHERE graph = (SELECT graphid FROM ag_catalog.ag_graph WHERE name = 'isahl_knowledge')",
        )
        .execute(&pool)
        .await
        .expect("模拟 restore：清 label 注册");
        sqlx::query("DELETE FROM ag_catalog.ag_graph WHERE name = 'isahl_knowledge'")
            .execute(&pool)
            .await
            .expect("模拟 restore：清图注册");
        assert!(
            needs_heal(&pool).await.expect("探测不应抛错"),
            "注册丢失后 MUST 判需自愈"
        );

        ensure_knowledge_graph_self_check(&pool).await;

        let registered: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM ag_catalog.ag_graph WHERE name = 'isahl_knowledge')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(registered, "自愈后图 MUST 重新注册");
        let drift: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM isahl_knowledge.age_knowledge_projection_diff() WHERE drift <> 0",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(drift, 0, "自愈后对账 MUST 零漂移");
        let nodes_after: i64 = sqlx::query_scalar(r#"SELECT count(*) FROM isahl_knowledge."Node""#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(nodes_after > 0, "自愈后节点 MUST 由 rebuild 重建");
    }
}
