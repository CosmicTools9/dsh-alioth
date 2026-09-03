//! `ScalarService` 标量日期/时段引用 COW（写时复制）判定逻辑单元测试。
//!
//! 覆盖 `parse_date_time` 三种输入、`update_date_ref` 三分支、
//! `update_segm_date_ref` 三分支，直接在受控 `ref_count` 下调用方法本体，
//! 不经过任何业务 repository，隔离验证判定逻辑本身。
//!
//! `ref_count` 由 DB 触发器维护；测试库未装触发器时其值为 NULL（按独占处理），
//! 共享分支通过手动 `UPDATE ref_count = N` 模拟。
//!
//! Run with:
//!   DATABASE_URL=postgres://isahl@localhost:5432/aliothstudio_test \
//!   cargo test -p common --test scalar_ref_count_test -- --test-threads=1

use common::scalar::{ScalarDateValue, ScalarService};
use common::testing::{connect_test_db, setup_test_schema_light};
use sqlx::PgPool;

fn unique_notice(tag: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}-{}", tag, std::process::id(), nanos % 1_000_000_000)
}

async fn fresh_pool() -> PgPool {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    // 清空标量表，避免跨测试/跨运行残留数据干扰复用判定（--test-threads=1 串行安全）
    sqlx::query(r#"DELETE FROM isahl."zc_id_scal-date""#)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(r#"DELETE FROM isahl."zc_id_segm-date""#)
        .execute(&pool)
        .await
        .unwrap();
    pool
}

/// 插入一个 `zc_id_scal-date` 行，返回其 id（ref_count 由调用方设定）。
async fn insert_scal_date(pool: &PgPool, date_text: &str, ref_count: i64) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_scal-date" (notice, date, ref_count, created_by_id)
           VALUES ($1, $2::timestamptz, $3, 1) RETURNING id"#,
    )
    .bind(unique_notice("ut-scal-date"))
    .bind(date_text)
    .bind(ref_count)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 插入一个 `zc_id_segm-date` 行，返回其 id。
async fn insert_segm_date(pool: &PgPool, date_text: &str, ref_count: i64) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_segm-date" (notice, date_st, date_ed, ref_count, created_by_id)
           VALUES ($1, $2::timestamptz, $3::timestamptz, $4, 1) RETURNING id"#,
    )
    .bind(unique_notice("ut-segm-date"))
    .bind(date_text)
    .bind(date_text)
    .bind(ref_count)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn read_scal_date(pool: &PgPool, id: i64) -> (Option<String>, Option<i64>) {
    let (date, rc): (Option<chrono::DateTime<chrono::Utc>>, Option<i64>) =
        sqlx::query_as(r#"SELECT date, ref_count FROM isahl."zc_id_scal-date" WHERE id = $1"#)
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
    (date.map(|d| d.format("%Y-%m-%d").to_string()), rc)
}

async fn read_segm_date(pool: &PgPool, id: i64) -> ((Option<String>, Option<String>), Option<i64>) {
    let (ds, de, rc): (
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<i64>,
    ) = sqlx::query_as(
        r#"SELECT date_st, date_ed, ref_count FROM isahl."zc_id_segm-date" WHERE id = $1"#,
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap();
    (
        (
            ds.map(|d| d.format("%Y-%m-%d").to_string()),
            de.map(|d| d.format("%Y-%m-%d").to_string()),
        ),
        rc,
    )
}

// ── parse_date_time：三种输入形态 ────────────────────────────────────

#[tokio::test]
async fn parse_date_time_accepts_date_only() {
    // 经 update_date_ref 无旧引用路径验证："YYYY-MM-DD" 解析成功且复用已有行
    let pool = fresh_pool().await;
    let existing = insert_scal_date(&pool, "2026-05-05", 0).await;
    let svc = ScalarService::new(pool.clone());

    let id = svc.update_date_ref(None, "2026-05-05").await.unwrap();
    // 无旧引用 + 相同值已存在 → 复用，不新建
    assert_eq!(id, existing);
}

#[tokio::test]
async fn parse_date_time_rejects_bad_format() {
    let pool = fresh_pool().await;
    let svc = ScalarService::new(pool.clone());
    let err = svc.update_date_ref(None, "not-a-date").await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Invalid date format"), "got: {}", msg);
}

#[tokio::test]
async fn parse_date_time_accepts_datetime_with_time() {
    // 时间部分进入 "time" 列：独占行原位修订后 "time" 应为 09:30:00
    let pool = fresh_pool().await;
    let row = insert_scal_date(&pool, "2026-01-15", 1).await;
    let svc = ScalarService::new(pool.clone());

    let id = svc
        .update_date_ref(Some(row), "2026-02-20 09:30:00")
        .await
        .unwrap();
    assert_eq!(id, row, "独占 → 原位修订，ID 不变");

    let t: Option<chrono::NaiveTime> =
        sqlx::query_scalar(r#"SELECT "time" FROM isahl."zc_id_scal-date" WHERE id = $1"#)
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        t.map(|v| v.format("%H:%M:%S").to_string()),
        Some("09:30:00".into())
    );
}

// ── update_date_ref：三分支 ──────────────────────────────────────────

#[tokio::test]
async fn update_date_ref_exclusive_revises_in_place() {
    let pool = fresh_pool().await;
    let row = insert_scal_date(&pool, "2026-01-15", 1).await; // 独占
    let svc = ScalarService::new(pool.clone());

    let id = svc.update_date_ref(Some(row), "2026-02-20").await.unwrap();
    assert_eq!(id, row, "ref_count<2 → 原位修订，外键 ID 不变");
    let (date, _) = read_scal_date(&pool, row).await;
    assert_eq!(date, Some("2026-02-20".into()));
}

#[tokio::test]
async fn update_date_ref_shared_inserts_new_row() {
    let pool = fresh_pool().await;
    let row = insert_scal_date(&pool, "2026-01-15", 2).await; // 共享
    let svc = ScalarService::new(pool.clone());

    let id = svc.update_date_ref(Some(row), "2026-03-10").await.unwrap();
    assert_ne!(id, row, "ref_count>=2 → 重新 insert，换绑新行");

    let (new_date, _) = read_scal_date(&pool, id).await;
    assert_eq!(new_date, Some("2026-03-10".into()));
    let (old_date, old_rc) = read_scal_date(&pool, row).await;
    assert_eq!(old_date, Some("2026-01-15".into()), "旧共享行不被污染");
    // 换绑自维护：旧行 ref_count 2 → 1；新行未启用维护（NULL）→ +1 不生效保持 NULL
    assert_eq!(old_rc, Some(1), "旧引用 -1");
    let (_, new_rc) = read_scal_date(&pool, id).await;
    assert_eq!(new_rc, None, "未维护环境新行 ref_count 保持 NULL");
}

#[tokio::test]
async fn update_date_ref_no_current_creates_row() {
    let pool = fresh_pool().await;
    let svc = ScalarService::new(pool.clone());

    let id = svc.update_date_ref(None, "2026-04-01").await.unwrap();
    let (date, _) = read_scal_date(&pool, id).await;
    assert_eq!(date, Some("2026-04-01".into()));
}

#[tokio::test]
async fn update_date_ref_invalid_current_id_falls_back_to_create() {
    // 当前 ID 指向不存在的行（如已软删）→ ref_count 读不到（NULL→0 <2），
    // 原位 UPDATE 不命中任何行，但仍返回原 ID——不 panic、不破坏。
    let pool = fresh_pool().await;
    let svc = ScalarService::new(pool.clone());
    let ghost = i64::MAX - 42;
    let id = svc
        .update_date_ref(Some(ghost), "2026-06-06")
        .await
        .unwrap();
    assert_eq!(id, ghost);
}

#[tokio::test]
async fn update_date_ref_shared_with_existing_value_reuses() {
    // 共享 + 新值已存在（另一行）→ 复用该行而非再插入
    let pool = fresh_pool().await;
    let shared = insert_scal_date(&pool, "2026-01-15", 2).await;
    let existing = insert_scal_date(&pool, "2026-03-10", 1).await;
    let svc = ScalarService::new(pool.clone());

    let id = svc
        .update_date_ref(Some(shared), "2026-03-10")
        .await
        .unwrap();
    assert_eq!(id, existing, "共享分支复用相同值行");
    // 换绑自维护：旧行 2 → 1，目标行 1 → 2
    let (_, shared_rc) = read_scal_date(&pool, shared).await;
    assert_eq!(shared_rc, Some(1), "旧引用 -1");
    let (_, existing_rc) = read_scal_date(&pool, existing).await;
    assert_eq!(existing_rc, Some(2), "新引用 +1");
}

#[tokio::test]
async fn update_date_ref_exclusive_with_existing_value_reuses() {
    // 客观唯一：独占（ref_count<2）但新值已有行 → 也必须复用该行，
    // 绝不产生同值双行（日期是客观唯一计量尺，同 date 必须同 id）
    let pool = fresh_pool().await;
    let mine = insert_scal_date(&pool, "2026-01-15", 1).await;
    let existing = insert_scal_date(&pool, "2026-03-10", 1).await;
    let svc = ScalarService::new(pool.clone());

    let id = svc.update_date_ref(Some(mine), "2026-03-10").await.unwrap();
    assert_eq!(id, existing, "独占也须先复用同值行，保持客观唯一");
    let (mine_date, _) = read_scal_date(&pool, mine).await;
    assert_eq!(mine_date, Some("2026-01-15".into()), "原行不被改写");
    // 换绑自维护：独占行 1 → 0（孤儿），目标行 1 → 2
    let (_, mine_rc) = read_scal_date(&pool, mine).await;
    assert_eq!(mine_rc, Some(0), "旧引用 -1（独占归零）");
    let (_, existing_rc) = read_scal_date(&pool, existing).await;
    assert_eq!(existing_rc, Some(2), "新引用 +1");
}

#[tokio::test]
async fn update_date_ref_same_value_is_noop() {
    // 新值 == 旧值 → 复用当前行自身，ID 不变（no-op）
    let pool = fresh_pool().await;
    let row = insert_scal_date(&pool, "2026-01-15", 2).await;
    let svc = ScalarService::new(pool.clone());

    let id = svc.update_date_ref(Some(row), "2026-01-15").await.unwrap();
    assert_eq!(id, row, "同值更新不换绑、不新建");
}

// ── update_segm_date_ref：三分支 ─────────────────────────────────────

#[tokio::test]
async fn update_segm_date_ref_exclusive_revises_in_place() {
    let pool = fresh_pool().await;
    let row = insert_segm_date(&pool, "2026-01-15", 1).await;
    let svc = ScalarService::new(pool.clone());

    let id = svc
        .update_segm_date_ref(Some(row), "2026-02-20", "2026-02-28")
        .await
        .unwrap();
    assert_eq!(id, row, "ref_count<2 → 原位修订，外键 ID 不变");
    let ((ds, de), _) = read_segm_date(&pool, row).await;
    assert_eq!(ds, Some("2026-02-20".into()));
    assert_eq!(de, Some("2026-02-28".into()));
}

#[tokio::test]
async fn update_segm_date_ref_shared_inserts_new_row() {
    let pool = fresh_pool().await;
    let row = insert_segm_date(&pool, "2026-01-15", 2).await;
    let svc = ScalarService::new(pool.clone());

    let id = svc
        .update_segm_date_ref(Some(row), "2026-03-10", "2026-03-15")
        .await
        .unwrap();
    assert_ne!(id, row, "ref_count>=2 → 重新 insert，换绑新行");

    let ((ds, de), _) = read_segm_date(&pool, id).await;
    assert_eq!(ds, Some("2026-03-10".into()));
    assert_eq!(de, Some("2026-03-15".into()));
    let ((ods, ode), old_rc) = read_segm_date(&pool, row).await;
    assert_eq!(ods, Some("2026-01-15".into()), "旧共享行不被污染");
    assert_eq!(ode, Some("2026-01-15".into()));
    assert_eq!(old_rc, Some(1), "旧引用 -1");
    let (_, new_rc) = read_segm_date(&pool, id).await;
    assert_eq!(new_rc, None, "未维护环境新行 ref_count 保持 NULL");
}

#[tokio::test]
async fn update_segm_date_ref_exclusive_with_existing_value_reuses() {
    // 客观唯一：独占但新值已有行 → 复用，不产生同值双行
    let pool = fresh_pool().await;
    let mine = insert_segm_date(&pool, "2026-01-15", 1).await;
    let existing = insert_segm_date(&pool, "2026-03-10", 1).await;
    let svc = ScalarService::new(pool.clone());

    let id = svc
        .update_segm_date_ref(Some(mine), "2026-03-10", "2026-03-10")
        .await
        .unwrap();
    assert_eq!(id, existing, "独占也须先复用同值行");
    let ((ods, _), _) = read_segm_date(&pool, mine).await;
    assert_eq!(ods, Some("2026-01-15".into()), "原行不被改写");
    // 换绑自维护：独占行 1 → 0，目标行 1 → 2
    let (_, mine_rc) = read_segm_date(&pool, mine).await;
    assert_eq!(mine_rc, Some(0), "旧引用 -1（独占归零）");
    let (_, existing_rc) = read_segm_date(&pool, existing).await;
    assert_eq!(existing_rc, Some(2), "新引用 +1");
}

#[tokio::test]
async fn update_segm_date_ref_same_value_is_noop() {
    let pool = fresh_pool().await;
    let row = insert_segm_date(&pool, "2026-01-15", 2).await;
    let svc = ScalarService::new(pool.clone());

    let id = svc
        .update_segm_date_ref(Some(row), "2026-01-15", "2026-01-15")
        .await
        .unwrap();
    assert_eq!(id, row, "同值更新不换绑、不新建");
}

#[tokio::test]
async fn update_segm_date_ref_no_current_creates_row() {
    let pool = fresh_pool().await;
    let svc = ScalarService::new(pool.clone());

    let id = svc
        .update_segm_date_ref(None, "2026-04-01", "2026-04-10")
        .await
        .unwrap();
    let ((ds, de), _) = read_segm_date(&pool, id).await;
    assert_eq!(ds, Some("2026-04-01".into()));
    assert_eq!(de, Some("2026-04-10".into()));
}

// ── find_or_create_segm_date：相同值复用 ─────────────────────────────

#[tokio::test]
async fn find_or_create_segm_date_reuses_existing() {
    let pool = fresh_pool().await;
    let existing = insert_segm_date(&pool, "2026-05-05", 1).await;
    let svc = ScalarService::new(pool.clone());

    let id = svc
        .find_or_create_segm_date("2026-05-05", "2026-05-05")
        .await
        .unwrap();
    assert_eq!(id, existing, "相同 (date_st, date_ed) 复用已有行");

    // DTO 值对象仍可正常构造（编译期验证 import 有效）
    let _v = ScalarDateValue {
        value: "2026-05-05".into(),
    };
}
