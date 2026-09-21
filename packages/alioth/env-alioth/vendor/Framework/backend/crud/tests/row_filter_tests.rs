//! `ROW_FILTER` 行作用域谓词契约测试（真实测试库 `aliothstudio_test`）。
//!
//! 契约来源：capability `entity-coordinate-isolation` :: `entity-row-scope-filter`
//! （`docs/specs/BACKEND_FRAMEWORK.md §7.3.3`）。
//!
//! 背景：同表同坐标可同时承载业务行与**系统事实行**（框架 `record_slice_flip` 落在
//! even-alert 的 `code='flip-<plan_id>'` 行）。`COORDINATE_FILTER` 只判「本实体占表中哪一段」，
//! 行作用域须由 `ROW_FILTER` 表达，且必须在**全部读径**（列表 / 计数 / 单条 get / get_refs）
//! 同源生效——否则「列表过滤、单条可读」分裂。
//!
//! 覆盖：
//! ① 声明 ROW_FILTER → 列表排除命中行，且 `total` 与谓词口径一致（items/count 同源）
//! ② 未声明（默认空）→ 行为与历史一致（零回归）
//! ③ 单条 get / get_refs 与列表口径一致（命中行不可读）
//! ④ 谓词 NULL 安全（`code IS NULL` 行不被误杀）

use common::testing::connect_test_db;
use crud::entity::Identifiable;
use crud::query_builder::QueryBuilder;
use crud::{AliothDbEntity, HasReferenceJoins, ReferenceJoin};
use sqlx::PgPool;

const TEST_TABLE: &str = r#""isahl"."zc_id_stus-project""#;

macro_rules! status_row {
    ($name:ident) => {
        #[derive(sqlx::FromRow, serde::Serialize, Clone)]
        struct $name {
            id: i64,
            notice: Option<String>,
            code: Option<String>,
            #[sqlx(default)]
            #[serde(rename = "_refs")]
            _refs: Option<serde_json::Value>,
        }
        impl Identifiable for $name {
            fn id(&self) -> i64 {
                self.id
            }
        }
        impl HasReferenceJoins for $name {
            fn reference_joins() -> Vec<ReferenceJoin> {
                vec![]
            }
        }
    };
}

status_row!(ScopedEntity);
impl AliothDbEntity for ScopedEntity {
    fn table_name() -> &'static str {
        TEST_TABLE
    }
    const SELECT_FIELDS: &'static str = "id, notice, code";
    const ENTITY_NAME: &'static str = "test-row-filter-scoped";
    const SOFT_DELETE: bool = true;
    const ROW_FILTER: &'static str = "(code IS NULL OR code NOT LIKE 'flip-%')";
}

status_row!(UnscopedEntity);
impl AliothDbEntity for UnscopedEntity {
    fn table_name() -> &'static str {
        TEST_TABLE
    }
    const SELECT_FIELDS: &'static str = "id, notice, code";
    const ENTITY_NAME: &'static str = "test-row-filter-unscoped";
    const SOFT_DELETE: bool = true;
}

async fn insert_row(pool: &PgPool, notice: &str, code: Option<&str>) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_stus-project" (notice, code, flag) VALUES ($1, $2, 'doing') RETURNING id"#,
    )
    .bind(notice)
    .bind(code)
    .fetch_one(pool)
    .await
    .expect("insert test row")
}

async fn cleanup(pool: &PgPool, ids: &[i64]) {
    sqlx::query(r#"DELETE FROM isahl."zc_id_stus-project" WHERE id = ANY($1)"#)
        .bind(ids)
        .execute(pool)
        .await
        .expect("cleanup test rows");
}

/// ① 声明 ROW_FILTER：列表排除系统事实行，`total` 与谓词口径一致（count 与 items 同源）。
#[tokio::test]
async fn list_excludes_row_filter_rows_and_count_matches() {
    let pool = connect_test_db().await;
    let fact = insert_row(&pool, "完成：实体状态推进 → x", Some("flip-990001")).await;
    let business = insert_row(&pool, "发动机适航指令", Some("RF-TEST-001")).await;

    let resp = QueryBuilder::<ScopedEntity>::new(&pool)
        .fetch(1, 500)
        .await
        .expect("scoped fetch");

    assert!(
        !resp.items.iter().any(|e| e.id == fact),
        "ROW_FILTER 命中行 MUST 不出现在列表中"
    );
    assert!(
        resp.items.iter().any(|e| e.id == business),
        "业务行 MUST 保留"
    );

    // items 与 count 同源：total 必须等于按同一谓词直算的行数
    let expected: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_stus-project"
           WHERE deleted_at IS NULL AND (code IS NULL OR code NOT LIKE 'flip-%')"#,
    )
    .fetch_one(&pool)
    .await
    .expect("expected count");
    assert_eq!(resp.total, expected, "fetch_count 必须与 ROW_FILTER 同源");

    cleanup(&pool, &[fact, business]).await;
}

/// ② 默认空 ROW_FILTER：行为与历史一致（零回归）——同表同数据全量可见。
#[tokio::test]
async fn list_without_row_filter_unaffected() {
    let pool = connect_test_db().await;
    let fact = insert_row(&pool, "完成：实体状态推进 → x", Some("flip-990002")).await;

    let resp = QueryBuilder::<UnscopedEntity>::new(&pool)
        .fetch(1, 500)
        .await
        .expect("unscoped fetch");

    assert!(
        resp.items.iter().any(|e| e.id == fact),
        "未声明 ROW_FILTER 的实体 MUST 保持既有行集（零回归）"
    );

    cleanup(&pool, &[fact]).await;
}

/// ③ 单条读径与列表口径一致：命中行在 get / get_refs 上不可读。
#[tokio::test]
async fn single_row_reads_share_row_filter_scope() {
    let pool = connect_test_db().await;
    let fact = insert_row(&pool, "完成：NCR 流转 → closed", Some("flip-990003")).await;
    let business = insert_row(&pool, "结构检查指令", Some("RF-TEST-002")).await;

    assert!(
        QueryBuilder::<ScopedEntity>::get(&pool, fact, None, None)
            .await
            .expect("plain get")
            .is_none(),
        "plain get MUST 与列表同口径（命中行不可读）"
    );
    assert!(
        QueryBuilder::<ScopedEntity>::get_refs(&pool, fact, None)
            .await
            .expect("get_refs")
            .is_none(),
        "get_refs MUST 与列表同口径（命中行不可读）"
    );
    assert!(
        QueryBuilder::<ScopedEntity>::get_refs(&pool, business, None)
            .await
            .expect("get_refs business")
            .is_some(),
        "业务行 MUST 可读"
    );

    cleanup(&pool, &[fact, business]).await;
}

/// ④ 谓词 NULL 安全：`code IS NULL` 的业务行不得被 `NOT LIKE` 误杀。
#[tokio::test]
async fn null_code_rows_survive_row_filter() {
    let pool = connect_test_db().await;
    let null_code = insert_row(&pool, "无编号适航指令", None).await;

    let resp = QueryBuilder::<ScopedEntity>::new(&pool)
        .fetch(1, 500)
        .await
        .expect("scoped fetch");
    assert!(
        resp.items.iter().any(|e| e.id == null_code),
        "code 为 NULL 的行 MUST 保留（裸 NOT LIKE 会因三值逻辑误杀）"
    );

    cleanup(&pool, &[null_code]).await;
}
