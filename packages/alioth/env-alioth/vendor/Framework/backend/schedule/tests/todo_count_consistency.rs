//! 回归测试：日程徽标计数 MUST 等于待办清单中未完成行数。
//!
//! 背景（AVIC-CAASEC/pre 实证）：`/schedule/overview` 的 `pending_todo_count`（顶栏
//! 「日程」徽标）与 `/schedule/todos`（面板待办清单）各自维护过滤条件——2026-09-09
//! 仅在清单侧排除系统翻转事实（`record_slice_flip` 产物，`code` 前缀 `flip-`），
//! 徽标仍把对账切片计为待办（徽标 8 / 清单可见行 0）。本测试锁定契约：
//! **徽标数 == 清单中 `done == false` 的行数**，且清单须保留已完成行（「显示已完成」展开依赖）。
//!
//! 夹具说明：已完成事件经生产写径 `ScheduleService::toggle_event_done` 播种——该路径
//! 自愈式补种「完成/end」状态行（此前硬取该行 id，测试库与 pre 库均无此种子 → 必失败）。

use framework_schedule::service::ScheduleService;
use framework_schedule::ScheduleRepository;
use sqlx::PgPool;

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

fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}", std::process::id(), nanos % 1_000_000_000)
}

async fn seed_event(pool: &PgPool, user_id: i64, code: &str, notice: Option<&str>) -> i64 {
    // 叶表坐标（§6.12）：值经 ontology_binding 解析 code→ZUID（禁硬编码 ZUID）
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("JE", "FBB", "↓_EE"))
        .await
        .expect("resolve even-alert coords");
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_even-alert" (created_by_id, code, notice, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"#,
    )
    .bind(user_id)
    .bind(code)
    .bind(notice)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("seed event")
}

/// 「完成/end」状态行是否已存在（`list_todos` 的 `done` 派生判据；生产写径自愈补种，
/// 见 `ScheduleRepository::toggle_event_done` Step 1）——用于清理时恢复库内原状。
async fn completed_status_seeded(pool: &PgPool) -> bool {
    sqlx::query_scalar::<_, bool>(
        r#"SELECT COUNT(*) > 0 FROM isahl."zc_id_stus-event"
           WHERE notice = '完成' AND flag = 'end' AND deleted_at IS NULL"#,
    )
    .fetch_one(pool)
    .await
    .expect("probe completed status row")
}

#[tokio::test]
async fn pending_todo_count_matches_visible_todo_rows() {
    let pool = test_pool().await;
    // 独立用户 id：与库内既有事件隔离（test 库 zc_id_even-alert 已有既有数据）
    let user_id: i64 = 9_000_000_000_000_000 + std::process::id() as i64;
    let suffix = unique_suffix();

    let normal = seed_event(
        &pool,
        user_id,
        &format!("t-todo-{suffix}-a"),
        Some("正常待办"),
    )
    .await;
    let flipped = seed_event(
        &pool,
        user_id,
        &format!("flip-t-todo-{suffix}-b"),
        Some("完成：实体状态推进"),
    )
    .await;
    let blank = seed_event(&pool, user_id, &format!("t-todo-{suffix}-c"), None).await;
    let done = seed_event(
        &pool,
        user_id,
        &format!("t-todo-{suffix}-d"),
        Some("已完成待办"),
    )
    .await;

    let repo = ScheduleRepository::new(pool.clone());
    let svc = ScheduleService::new(repo.clone());
    let scope = vec![normal, flipped, blank, done];

    // 已完成事件：走生产写径标记（`/schedule/toggle` 对 event id 即此路径；叶表
    // `zc_id_even-alert` 继承 `zc_id_event`，故 event 存在性检查命中本轮播种行）。
    // 该写径自愈式补种「完成/end」状态行——测试不硬编码其 id；读径判据仍由断言锁定：
    // 若补种行的 (notice, flag) 与 `list_todos` 派生不一致，下方「未完成行数」必为 2。
    let status_seeded_before = completed_status_seeded(&pool).await;
    let toggled = svc
        .toggle_event_done(done)
        .await
        .expect("toggle event done")
        .expect("seeded event must be visible via zc_id_event");
    assert_eq!(toggled.id, done, "toggle 返回本轮播种的 event");

    // 路径 1：RLS 可见集限定（PEP 注入场景）
    let count_scoped = repo
        .get_pending_todo_count(Some(user_id), Some(&scope))
        .await
        .expect("pending count (scoped)");
    let todos_scoped = svc
        .list_todos(user_id, 50, 0, Some(&scope))
        .await
        .expect("todo list (scoped)");

    assert_eq!(count_scoped, 1, "仅普通未完成待办计入徽标");
    assert_eq!(
        todos_scoped.len(),
        2,
        "清单保留未完成 + 已完成两行（flip- 与空 notice 行排除）"
    );
    let visible_scoped = todos_scoped.iter().filter(|t| !t.done).count();
    assert_eq!(visible_scoped, 1, "默认可见行 = 未完成行");
    assert_eq!(
        count_scoped as usize, visible_scoped,
        "徽标数 == 清单未完成行数"
    );
    assert!(
        todos_scoped.iter().any(|t| t.done && t.id == done),
        "已完成行必须保留在清单中（面板「显示已完成」展开依赖）"
    );
    assert!(
        todos_scoped
            .iter()
            .all(|t| t.id != flipped && t.id != blank),
        "flip- 对账切片与空 notice 行不得进入清单"
    );

    // 路径 2：不限定可见集（无 RLS 注入场景）
    let count_all = repo
        .get_pending_todo_count(Some(user_id), None)
        .await
        .expect("pending count (unscoped)");
    let todos_all = svc
        .list_todos(user_id, 50, 0, None)
        .await
        .expect("todo list (unscoped)");

    assert_eq!(
        count_all as usize,
        todos_all.iter().filter(|t| !t.done).count(),
        "徽标数 MUST 等于清单未完成行数（无 RLS 注入）"
    );

    // 未认证（None）不再统计全库待办
    assert_eq!(
        repo.get_pending_todo_count(None, None)
            .await
            .expect("pending count (anonymous)"),
        0
    );

    // 清理本轮播种
    sqlx::query(r#"DELETE FROM isahl."zc_id_lifecycle_r_primary-status" WHERE ref_left = ANY($1)"#)
        .bind(scope.clone())
        .execute(&pool)
        .await
        .expect("cleanup lifecycle rows");
    sqlx::query(r#"DELETE FROM isahl."zc_id_even-alert" WHERE id = ANY($1)"#)
        .bind(scope)
        .execute(&pool)
        .await
        .expect("cleanup events");
    // toggle 若触发自愈补种，恢复库内原状（仅在「本轮之前不存在」时删除，避免误删既有字典行）
    if !status_seeded_before {
        sqlx::query(
            r#"DELETE FROM isahl."zc_id_stus-event"
               WHERE notice = '完成' AND flag = 'end' AND deleted_at IS NULL"#,
        )
        .execute(&pool)
        .await
        .expect("cleanup lazily seeded completed status");
    }
}
