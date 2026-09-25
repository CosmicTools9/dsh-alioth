//! 写路径集成测试（真实测试库 `aliothstudio_test`）：生成仓储的 create/update 经 `crud::trigger` 通道。
//!
//! 契约（`openspec/changes/regenerate-services-through-trigger-channel` 期 3，Alioth ns 覆盖）：
//! ① `create` 经通道写入 `zc_id_stus-task`，系统管理列由通道注入；
//! ② 该实体 `table` 在产物中为**已带引号**形态（`"zc_id_stus-task"`）⇒ 通道 MUST 收到裸表名
//!    （限定名/带引号名会被 `crud::trigger` 标识符校验拒绝，运行时报 `Invalid SQL identifier`）；
//! ③ `update` 经同一通道且不存在的行返回 `None`。
//!
//! 注册表口径与 Gateway 生产一致（编译期继承图 + pg_catalog 刷新），见
//! `Framework/backend/crud/tests/trigger_write_path_test.rs`。

use alioth_service_todo_service::models::todo::{CreateTodoRequest, UpdateTodoRequest};
use alioth_service_todo_service::repositories::todo::TodoRepository;
use common::testing::connect_test_db;
use crud::repository::AliothRepository;
use sqlx::PgPool;

/// 断言用静态 SQL（遵循 dynamic-table-name 门禁）；表名与产物 `table_name()` 一致（裸名 + 引号）。
const SYSCOLS_SQL: &str =
    r#"SELECT created_by_id, updated_by_id FROM ONLY isahl."zc_id_stus-task" WHERE id = $1"#;
const HARD_DELETE_SQL: &str = r#"DELETE FROM ONLY isahl."zc_id_stus-task" WHERE id = $1"#;
const O_NUMBER_SQL: &str = r#"SELECT o_number FROM ONLY isahl."zc_id_stus-task" WHERE id = $1"#;

/// 规范形判据 `{YYYYMMDD}_{HHMMSSmmm}_{crc32hex}`（与 `crud/tests/trigger_write_path_test.rs` 同口径：
/// 8 位日期 + `_` + 9 位时分秒毫秒 + `_` + 8 位小写 hex）。
fn is_canonical_o_number(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 27
        && b[8] == b'_'
        && b[18] == b'_'
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[9..18].iter().all(u8::is_ascii_digit)
        && b[19..]
            .iter()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// 进程级 `OnceLock`：无条件初始化（并发下「已初始化」返回 Err 属正常，值必已就位）。
async fn ensure_registry(pool: &PgPool) {
    let _ = trigger_registry::init::init_smart_registry_global(
        pool,
        trigger_registry::AppContainer::Gateway,
    )
    .await;
    trigger_registry::init::refresh_smart_registry_from_pg_catalog(pool)
        .await
        .expect("refresh inheritance graph from pg_catalog");
}

#[tokio::test]
async fn create_and_update_go_through_trigger_channel() {
    let pool = connect_test_db().await;
    ensure_registry(&pool).await;
    let repo = TodoRepository::new(pool.clone());

    // ── ① create：经通道（裸表名），系统管理列注入 ────────────────────────────
    let created = repo
        .create(
            CreateTodoRequest {
                notice: Some("待办状态-通道".to_string()),
                code: None,
                flag: "start".to_string(),
                enable: Some(true),
                comments: None,
            },
            7,
        )
        .await
        .expect("create 经触发器通道（已带引号声明形态 MUST 归一为裸表名）");
    assert!(created.id > 0, "行 MUST 落库并回读 id");
    // 枚举列（`status_flag`，NOT NULL）：写径 MUST 给占位符补 `::"udt"` 强转，
    // 否则文本参数写枚举列直接违约（`column "flag" is of type status_flag but expression is of type text`）。
    assert_eq!(
        created.flag.as_deref(),
        Some("start"),
        "`flag` 枚举列 MUST 经强转写入并按 `flag::text` 回读"
    );
    assert_eq!(created.notice.as_deref(), Some("待办状态-通道"));
    assert_eq!(created.enable, Some(true));
    let (created_by, updated_by): (Option<i64>, Option<i64>) = sqlx::query_as(SYSCOLS_SQL)
        .bind(created.id)
        .fetch_one(&pool)
        .await
        .expect("system cols");
    assert_eq!(created_by, Some(7), "created_by_id MUST 由通道注入");
    assert_eq!(updated_by, Some(7), "updated_by_id MUST 由通道注入");
    // 派生列（迁移核心价值）：`zc_id_stus-task` 属 zc_id_object 后代 ⇒ BEFORE INSERT 模板
    // （`ObjectONumberTemplate`）经通道产出规范形 `o_number`——旧裸 SQL 路径不跑触发器，该列恒为空。
    let o_number: Option<String> = sqlx::query_scalar(O_NUMBER_SQL)
        .bind(created.id)
        .fetch_one(&pool)
        .await
        .expect("o_number");
    let o_number = o_number.expect("经通道 create MUST 产出 o_number（派生列）");
    assert!(
        is_canonical_o_number(&o_number),
        "o_number MUST 为规范形 {{YYYYMMDD}}_{{HHMMSSmmm}}_{{crc32hex}}，实际 {o_number}"
    );

    // ── ② update：经通道；不存在的行返回 None ────────────────────────────────
    let updated = repo
        .update(
            created.id,
            UpdateTodoRequest {
                notice: None,
                code: None,
                flag: Some("doing".to_string()),
                enable: None,
                comments: None,
            },
            8,
        )
        .await
        .expect("update 经触发器通道");
    let updated = updated.expect("已存在行 update MUST 返回 Some");
    assert_eq!(
        updated.flag.as_deref(),
        Some("doing"),
        "UPDATE 的枚举列 MUST 同样经 `::\"udt\"` 强转改写"
    );
    assert_eq!(
        updated.notice.as_deref(),
        Some("待办状态-通道"),
        "未提供的字段 MUST 保持原值（部分更新语义）"
    );
    let (_, updated_by_after): (Option<i64>, Option<i64>) = sqlx::query_as(SYSCOLS_SQL)
        .bind(created.id)
        .fetch_one(&pool)
        .await
        .expect("system cols after update");
    assert_eq!(
        updated_by_after,
        Some(8),
        "updated_by_id MUST 被本次调用者改写"
    );
    let missing = repo
        .update(
            i64::MAX,
            UpdateTodoRequest {
                notice: None,
                code: None,
                flag: None,
                enable: None,
                comments: None,
            },
            8,
        )
        .await
        .expect("update 缺失行不报错");
    assert!(missing.is_none(), "缺失行 update MUST 返回 None");

    // ── 清理（硬删夹具行，避免软删残留干扰后续批次）─────────────────────────
    sqlx::query(HARD_DELETE_SQL)
        .bind(created.id)
        .execute(&pool)
        .await
        .expect("cleanup");
}
