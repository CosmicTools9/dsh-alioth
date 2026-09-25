//! 真实岗位类谓词回归（fix-position-class-predicate）
//!
//! 覆盖 `common::real_position_row!`（`_t_ IS DISTINCT FROM '范例'`）的**可观测数据行为**：
//! - 类派生岗位行（`_f_='实现' / _t_='实例'`，ns 种子链类列归一器 `seed-zz-class-columns-normalize.sql`
//!   对 `dk_function='↓_GG'` 的产物）MUST 经岗位读径可见、且可被写径守卫处置；
//! - 类列 NULL 的存量行 MUST 保留可见（`IS DISTINCT FROM` 语义，不得因 NULL 比较被丢弃）；
//! - 编制范例行（`_f_='设计' / _t_='范例'`）MUST 被读径与写径守卫**双双**排除。
//!
//! 修复前判据为 `_f_ IS NULL` ⇒ 前两类岗位全部不可见（2026-09-23 事故：AVIC 审批节点
//! 「审批岗位」下拉恒空、组织管理岗位列表 3/13、WZ 1/22）。本文件即该事故的回归防线。
//!
//! 读径断言用 handler 同源常量 `POSITION_SELECT` + 生产谓词宏（`PositionRow` 为元组别名、
//! handler 读 SQL 内联，无公开 handler 入口——`org_management_test.rs` 同约定）；
//! 写径直接调 `org_write` 的真实公开入口。

use sqlx::{AssertSqlSafe, PgPool};

async fn test_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://isahl@localhost:5432/aliothstudio_test".to_string());
    let pool = PgPool::connect(&url).await.expect("connect test db");
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("current_database");
    assert!(db.contains("_test"), "REFUSED: non-test db {db}");
    pool
}

/// 动态测试 id 段（进程+纳秒派生，跨运行不冲突；测试不清理数据）
fn tid(base: i64) -> i64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    ((nanos % 1_000_000) as i64) * 100 + base
}

/// 落岗位行：`class = Some((_f_, _t_))` 显式类列（模拟归一器产物 / 编制范例），`None` = 类列 NULL。
/// dk_function 经 ontology_binding 解析（§6.12 声明即必须，禁硬编码 ZUID）。
async fn insert_position(pool: &PgPool, id: i64, notice: &str, class: Option<(&str, &str)>) -> i64 {
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve position coords");
    let (f_, t_) = match class {
        Some((f_, t_)) => (Some(f_), Some(t_)),
        None => (None, None),
    };
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-position"
               (id, notice, code, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(id)
    .bind(notice)
    .bind(format!("T-POS-{id}"))
    .bind(f_)
    .bind(t_)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(pool)
    .await
    .expect("insert position");
    id
}

/// 岗位是否存活（未软删）
async fn is_alive(pool: &PgPool, id: i64) -> bool {
    sqlx::query_scalar::<_, bool>(
        r#"SELECT COUNT(*) > 0 FROM isahl."zc_id_subj-position" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("is_alive")
}

async fn notice_of(pool: &PgPool, id: i64) -> String {
    sqlx::query_scalar::<_, String>(
        r#"SELECT notice::text FROM isahl."zc_id_subj-position" WHERE id = $1"#,
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("notice_of")
}

#[tokio::test]
async fn real_position_predicate_visibility_and_guards() {
    use identity_org::handlers::org_tree::{PositionRow, POSITION_SELECT};
    use identity_org::service::org_write;

    const USER: i64 = 1;
    let pool = test_pool().await;

    let base = tid(7100);
    let instance = insert_position(&pool, base, "T-类派生岗位", Some(("实现", "实例"))).await;
    let template = insert_position(&pool, base + 1, "T-编制范例", Some(("设计", "范例"))).await;
    let legacy = insert_position(&pool, base + 2, "T-存量岗位", None).await;
    let ids: Vec<i64> = vec![instance, template, legacy];

    // ── 列表读径（handler 同源 SQL + 生产谓词）──
    let list_sql = format!(
        "{} WHERE p.deleted_at IS NULL AND {} AND p.id = ANY($1::BIGINT[]) ORDER BY p.id",
        POSITION_SELECT,
        common::real_position_row!(p)
    );
    let rows: Vec<PositionRow> = sqlx::query_as(AssertSqlSafe(list_sql.as_str()))
        .bind(&ids)
        .fetch_all(&pool)
        .await
        .expect("list positions");
    let visible: Vec<i64> = rows.iter().map(|r| r.0).collect();
    assert_eq!(
        visible,
        vec![instance, legacy],
        "类派生岗位行与类列 NULL 的存量行 MUST 可见；编制范例行 MUST 被排除（修复前可见集为空）"
    );

    // ── 详情读径 ──
    let detail_sql = format!(
        "{} WHERE p.id = $1 AND p.deleted_at IS NULL AND {}",
        POSITION_SELECT,
        common::real_position_row!(p)
    );
    let detail_template: Option<PositionRow> = sqlx::query_as(AssertSqlSafe(detail_sql.as_str()))
        .bind(template)
        .fetch_optional(&pool)
        .await
        .expect("read template detail");
    assert!(
        detail_template.is_none(),
        "编制范例行 MUST 不可经 /positions/{{id}} 读"
    );
    let detail_instance: Option<PositionRow> = sqlx::query_as(AssertSqlSafe(detail_sql.as_str()))
        .bind(instance)
        .fetch_optional(&pool)
        .await
        .expect("read instance detail");
    assert!(
        detail_instance.is_some(),
        "类派生岗位行 MUST 可经 /positions/{{id}} 读"
    );

    // ── 存在性/写径守卫（真实公开入口）──
    let updated = org_write::update_approver_position(&pool, template, "T-被改", None, None, USER)
        .await
        .expect("update template via approver entry");
    assert!(
        updated.is_none(),
        "编制范例行 MUST NOT 可经 Approver 更新入口修改"
    );
    assert_eq!(
        notice_of(&pool, template).await,
        "T-编制范例",
        "编制范例行名称 MUST 保持不变"
    );

    let delete_err = org_write::delete_position(&pool, template, USER)
        .await
        .expect_err("编制范例行 MUST NOT 可经岗位软删入口删除（守卫应返回 NotFound）");
    assert!(
        matches!(delete_err, common::AliothError::NotFound(_)),
        "编制范例行删除守卫 MUST 以 404 语义拒绝，实测: {delete_err:?}"
    );
    assert!(
        is_alive(&pool, template).await,
        "编制范例行 MUST 保持存活（仍是模板）"
    );

    org_write::delete_position(&pool, instance, USER)
        .await
        .expect("delete instance via position entry");
    assert!(
        !is_alive(&pool, instance).await,
        "类派生真实岗位行 MUST 可被岗位软删入口删除（修复前该守卫会静默漏掉它）"
    );
}
