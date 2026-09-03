//! 级联软删除集成测试（真实测试库，isahl schema 已 seed）。
//!
//! 覆盖：关系表级联 / 子实体递归级联 / 业务引用默认不级联+显式开启 /
//! 批量级联 / 单事务原子性（级联失败整体回滚）。
//! 每用例插入的行与临时触发器在用例内清理（触发器名带 nanos 后缀防并行冲突）。

use common::testing::connect_test_db;
use crud::entity::Identifiable;
use crud::query_builder::QueryBuilder;
use crud::AliothDbEntity;
use sqlx::{AssertSqlSafe, PgPool, Row};

// ─────────────────────────────────────────────────────────────────────────────
// 测试实体（复用真实表结构的最小 AliothDbEntity）
// ─────────────────────────────────────────────────────────────────────────────

#[derive(sqlx::FromRow, serde::Serialize, Clone)]
struct StatusEntity {
    id: i64,
}
impl Identifiable for StatusEntity {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for StatusEntity {
    fn table_name() -> &'static str {
        r#""isahl"."zc_id_status""#
    }
    const SELECT_FIELDS: &'static str = "id";
    const ENTITY_NAME: &'static str = "test-status";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

#[derive(sqlx::FromRow, serde::Serialize, Clone)]
struct LifecycleEntity {
    id: i64,
}
impl Identifiable for LifecycleEntity {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for LifecycleEntity {
    fn table_name() -> &'static str {
        r#""isahl"."zc_id_lifecycle""#
    }
    const SELECT_FIELDS: &'static str = "id";
    const ENTITY_NAME: &'static str = "test-lifecycle";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

#[derive(sqlx::FromRow, serde::Serialize, Clone)]
struct SubjectsEntity {
    id: i64,
}
impl Identifiable for SubjectsEntity {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for SubjectsEntity {
    fn table_name() -> &'static str {
        r#""isahl"."zc_id_subjects""#
    }
    const SELECT_FIELDS: &'static str = "id";
    const ENTITY_NAME: &'static str = "test-subjects";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

#[derive(sqlx::FromRow, serde::Serialize, Clone)]
struct VersionEntity {
    id: i64,
}
impl Identifiable for VersionEntity {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for VersionEntity {
    fn table_name() -> &'static str {
        r#""isahl"."zc_id_version""#
    }
    const SELECT_FIELDS: &'static str = "id";
    const ENTITY_NAME: &'static str = "test-version";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

// ─────────────────────────────────────────────────────────────────────────────
// 数据助手
// ─────────────────────────────────────────────────────────────────────────────

fn tag() -> String {
    format!(
        "cascade-test-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    )
}

async fn insert_row(pool: &PgPool, table: &str, notice: &str) -> i64 {
    let sql = format!(
        "INSERT INTO isahl.\"{}\" (notice) VALUES ($1) RETURNING id",
        table
    );
    sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(notice)
        .fetch_one(pool)
        .await
        .expect("insert row")
        .get::<i64, _>("id")
}

/// 插入 r_status 桥接行（ref_left → lifecycle，ref_right → status）
async fn insert_relation_row(pool: &PgPool, notice: &str, ref_left: i64, ref_right: i64) -> i64 {
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_lifecycle_r_status" (notice, ref_left, ref_right)
           VALUES ($1, $2, $3) RETURNING id"#,
    )
    .bind(notice)
    .bind(ref_left)
    .bind(ref_right)
    .fetch_one(pool)
    .await
    .expect("insert relation row")
    .get::<i64, _>("id")
}

/// 插入 deta-trade-order 业务引用行（fk_list → lifecycle）
async fn insert_biz_ref_row(pool: &PgPool, notice: &str, fk_item: i64) -> i64 {
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_deta-tsp" (notice, fk_item)
           VALUES ($1, $2) RETURNING id"#,
    )
    .bind(notice)
    .bind(fk_item)
    .fetch_one(pool)
    .await
    .expect("insert biz ref row")
    .get::<i64, _>("id")
}

/// 插入 version 层级行（可选 fk_previous 指向父行）
async fn insert_manu_row(pool: &PgPool, notice: &str, fk_previous: Option<i64>) -> i64 {
    match fk_previous {
        Some(parent) => sqlx::query(
            r#"INSERT INTO isahl."zc_id_version" (notice, fk_previous)
               VALUES ($1, $2) RETURNING id"#,
        )
        .bind(notice)
        .bind(parent)
        .fetch_one(pool)
        .await
        .expect("insert manu row")
        .get::<i64, _>("id"),
        None => insert_row(pool, "zc_id_version", notice).await,
    }
}

async fn deleted_at_of(
    pool: &PgPool,
    table: &str,
    id: i64,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let sql = format!("SELECT deleted_at FROM isahl.\"{}\" WHERE id = $1", table);
    sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(id)
        .fetch_optional(pool)
        .await
        .expect("query deleted_at")
        .and_then(|r| {
            r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("deleted_at")
                .ok()
                .flatten()
        })
}

async fn hard_delete(pool: &PgPool, table: &str, id: i64) {
    let sql = format!("DELETE FROM isahl.\"{}\" WHERE id = $1", table);
    sqlx::query(AssertSqlSafe(sql.as_str()))
        .bind(id)
        .execute(pool)
        .await
        .expect("hard delete cleanup");
}

/// status + r_status 桥接行（ref_right → status）
async fn seed_status_relation(pool: &PgPool, notice: &str) -> (i64, i64, i64) {
    let status_id = insert_row(pool, "zc_id_status", notice).await;
    let lc_id = insert_row(pool, "zc_id_lifecycle", notice).await;
    let rel_id = insert_relation_row(pool, notice, lc_id, status_id).await;
    (status_id, lc_id, rel_id)
}

/// lifecycle + deta-trade-order 业务引用行（fk_list → lifecycle）
async fn seed_biz_ref(pool: &PgPool, notice: &str) -> (i64, i64) {
    let lc_id = insert_row(pool, "zc_id_lifecycle", notice).await;
    // 现存真实标量业务引用（fk_index 生成数据）：zc_id_deta-tsp.fk_item → zc_id_lifecycle
    // （历史测试用 deta-trade_order.fk_list 指向 lifecycle，模型已演进 fk_list →
    // zc_id_stat-trade_order，种子随之更新）
    let order_id = insert_biz_ref_row(pool, notice, lc_id).await;
    (lc_id, order_id)
}

/// 清理上次异常中断可能残留的同名前缀测试触发器（自愈，防跨 run 污染）。
async fn cleanup_leaked_fail_triggers(pool: &PgPool) {
    let leftover: Vec<String> = sqlx::query(
        "SELECT tgname FROM pg_trigger WHERE NOT tgisinternal AND tgname LIKE 'tg_cascade_fail_%'",
    )
    .fetch_all(pool)
    .await
    .expect("list leftover triggers")
    .into_iter()
    .map(|r| r.get::<String, _>("tgname"))
    .collect();
    for t in leftover {
        sqlx::query(AssertSqlSafe(format!(
            "DROP TRIGGER IF EXISTS \"{}\" ON isahl.\"zc_id_lifecycle_r_status\"",
            t
        )))
        .execute(pool)
        .await
        .expect("drop leftover trigger");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 3.1 关系表级联（REQ-NFR-005 验收(1)：JOIN 可见关联数据 deleted_at 均置位）
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn relation_cascade_on_soft_delete() {
    let pool = connect_test_db().await;
    cleanup_leaked_fail_triggers(&pool).await;
    let notice = tag();
    let (status_id, lc_id, rel_id) = seed_status_relation(&pool, &notice).await;

    let rows = QueryBuilder::<StatusEntity>::soft_delete(&pool, status_id, 1)
        .await
        .expect("soft delete status");
    assert_eq!(rows, 1);

    assert!(
        deleted_at_of(&pool, "zc_id_lifecycle_r_status", rel_id)
            .await
            .is_some(),
        "关系表行应随主删除同事务置位 deleted_at"
    );
    // 非级联目标（生命周期行本身）不受影响
    assert!(
        deleted_at_of(&pool, "zc_id_lifecycle", lc_id)
            .await
            .is_none(),
        "无关行 deleted_at 不得置位"
    );

    hard_delete(&pool, "zc_id_lifecycle_r_status", rel_id).await;
    hard_delete(&pool, "zc_id_lifecycle", lc_id).await;
    hard_delete(&pool, "zc_id_status", status_id).await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 3.3 业务引用级联：非明细默认不级联；明细（detail 族）默认级联（用户裁决）
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn detail_business_ref_cascades_by_default() {
    let pool = connect_test_db().await;
    let notice = tag();
    let (lc_id, order_id) = seed_biz_ref(&pool, &notice).await;

    // 默认配置（detail_business_refs=true）：明细随主实体软删级联
    QueryBuilder::<LifecycleEntity>::soft_delete(&pool, lc_id, 1)
        .await
        .expect("soft delete lifecycle");

    assert!(
        deleted_at_of(&pool, "zc_id_deta-tsp", order_id)
            .await
            .is_some(),
        "明细（zc_id_deta-tsp）引用默认应级联（2026-08-27 用户裁决）"
    );

    hard_delete(&pool, "zc_id_deta-tsp", order_id).await;
    hard_delete(&pool, "zc_id_lifecycle", lc_id).await;
}

#[tokio::test]
async fn non_detail_business_ref_not_cascaded_by_default() {
    let pool = connect_test_db().await;
    let notice = tag();
    // 非明细业务引用：zc_id_appr-payment.fk_subject → zc_id_subjects
    let subject_id = insert_row(&pool, "zc_id_subjects", &notice).await;
    let payment_id: i64 = sqlx::query(
        r#"INSERT INTO isahl."zc_id_appr-payment" (notice, fk_subject)
           VALUES ($1, $2) RETURNING id"#,
    )
    .bind(notice)
    .bind(subject_id)
    .fetch_one(&pool)
    .await
    .expect("insert appr-payment row")
    .get::<i64, _>("id");

    QueryBuilder::<SubjectsEntity>::soft_delete(&pool, subject_id, 1)
        .await
        .expect("soft delete subjects");

    assert!(
        deleted_at_of(&pool, "zc_id_appr-payment", payment_id)
            .await
            .is_none(),
        "非明细业务引用默认不得级联"
    );

    hard_delete(&pool, "zc_id_appr-payment", payment_id).await;
    hard_delete(&pool, "zc_id_subjects", subject_id).await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 3.2 子实体递归级联（父 → 子 → 孙，fk_previous 链）
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn child_entity_cascade_recursive() {
    let pool = connect_test_db().await;
    let notice = tag();

    let parent = insert_manu_row(&pool, &notice, None).await;
    let child = insert_manu_row(&pool, &notice, Some(parent)).await;
    let grandchild = insert_manu_row(&pool, &notice, Some(child)).await;

    QueryBuilder::<VersionEntity>::soft_delete(&pool, parent, 1)
        .await
        .expect("soft delete parent");

    assert!(
        deleted_at_of(&pool, "zc_id_version", child).await.is_some(),
        "子行应随父删除级联"
    );
    assert!(
        deleted_at_of(&pool, "zc_id_version", grandchild)
            .await
            .is_some(),
        "孙行应递归级联"
    );

    hard_delete(&pool, "zc_id_version", parent).await;
    hard_delete(&pool, "zc_id_version", child).await;
    hard_delete(&pool, "zc_id_version", grandchild).await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 3.4 批量删除级联
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn batch_soft_delete_cascades_relation_rows() {
    let pool = connect_test_db().await;
    cleanup_leaked_fail_triggers(&pool).await;
    let notice = tag();

    let (s1, lc1, r1) = seed_status_relation(&pool, &format!("{}-a", notice)).await;
    let (s2, lc2, r2) = seed_status_relation(&pool, &format!("{}-b", notice)).await;

    let rows = QueryBuilder::<StatusEntity>::batch_soft_delete(&pool, &[s1, s2], 1)
        .await
        .expect("batch soft delete");
    assert_eq!(rows, 2);

    assert!(
        deleted_at_of(&pool, "zc_id_lifecycle_r_status", r1)
            .await
            .is_some(),
        "batch 级联：第一条 status 的关联行应置位"
    );
    assert!(
        deleted_at_of(&pool, "zc_id_lifecycle_r_status", r2)
            .await
            .is_some(),
        "batch 级联：第二条 status 的关联行应置位"
    );

    hard_delete(&pool, "zc_id_lifecycle_r_status", r1).await;
    hard_delete(&pool, "zc_id_lifecycle", lc1).await;
    hard_delete(&pool, "zc_id_status", s1).await;
    hard_delete(&pool, "zc_id_lifecycle_r_status", r2).await;
    hard_delete(&pool, "zc_id_lifecycle", lc2).await;
    hard_delete(&pool, "zc_id_status", s2).await;
}

// ─────────────────────────────────────────────────────────────────────────────
// 3.4 单事务原子性：级联目标 UPDATE 失败 → 主实体一并回滚
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cascade_rolls_back_on_target_failure() {
    let pool = connect_test_db().await;
    let notice = tag();
    let (status_id, lc_id, rel_id) = seed_status_relation(&pool, &notice).await;

    // 自愈：清理上次异常中断可能残留的同名前缀触发器
    cleanup_leaked_fail_triggers(&pool).await;

    // 在关系表上安装 BEFORE UPDATE 触发器，令级联 UPDATE 必然失败
    let fn_name = format!(
        "tf_cascade_fail_{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let tg_name = format!("tg_cascade_fail_{}", fn_name);
    sqlx::query(AssertSqlSafe(format!(
        "CREATE OR REPLACE FUNCTION isahl.{}() RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN RAISE EXCEPTION 'cascade rollback test'; END $$",
        fn_name
    )))
    .execute(&pool)
    .await
    .expect("create fail function");
    sqlx::query(AssertSqlSafe(format!(
        "CREATE TRIGGER {} BEFORE UPDATE ON isahl.\"zc_id_lifecycle_r_status\" FOR EACH ROW EXECUTE FUNCTION isahl.{}()",
        tg_name, fn_name
    )))
    .execute(&pool)
    .await
    .expect("create fail trigger");

    // 级联目标 UPDATE 失败 → 整体回滚（主实体 deleted_at 不得置位）
    let result = QueryBuilder::<StatusEntity>::soft_delete(&pool, status_id, 1).await;
    assert!(result.is_err(), "级联失败应返回错误");

    assert!(
        deleted_at_of(&pool, "zc_id_status", status_id)
            .await
            .is_none(),
        "级联失败时主实体必须回滚（无部分级联残留）"
    );
    assert!(
        deleted_at_of(&pool, "zc_id_lifecycle_r_status", rel_id)
            .await
            .is_none(),
        "级联失败时关联行不得置位"
    );

    // 清理触发器 + 数据
    sqlx::query(AssertSqlSafe(format!(
        "DROP TRIGGER {} ON isahl.\"zc_id_lifecycle_r_status\"",
        tg_name
    )))
    .execute(&pool)
    .await
    .expect("drop fail trigger");
    sqlx::query(AssertSqlSafe(format!("DROP FUNCTION isahl.{}()", fn_name)))
        .execute(&pool)
        .await
        .expect("drop fail function");
    hard_delete(&pool, "zc_id_lifecycle_r_status", rel_id).await;
    hard_delete(&pool, "zc_id_lifecycle", lc_id).await;
    hard_delete(&pool, "zc_id_status", status_id).await;
}
