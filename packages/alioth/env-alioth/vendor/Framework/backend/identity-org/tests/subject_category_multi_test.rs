//! `GET /subjects` 社会分类**多标签并集**集成测试（change: add-subject-list-default-common-categories）。
//!
//! 直驱谓词单一实现 `push_subject_filters`（列表与计数同源；spec `subject-list-filter-semantics`）：
//! - `category=雇员,组织` → 两子树并集（自然人 + 非银行法人命中），子树外行（国家）排除；
//! - 多标签任一未知 → 整体 fail-closed 空集（不静默丢弃未知标签）；
//! - 单标签 `法人` 行为不变（含其子类）。
//!
//! fixture 全部落在 subjects 树**真叶表**（empl-natural / orga-non-banking-legal / subj-country），
//! 以纳秒后缀码隔离，结束硬删。

use identity_org::handlers::subjects::{push_subject_filters, SubjectFilters};
use sqlx::{PgPool, Postgres, QueryBuilder};

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

fn suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:x}", nanos % 0xF_FFFF_FFFF)
}

/// fixture 叶表（subjects 树真叶）——表名走编译期字面量（禁运行期 `format!` 拼表名）。
#[derive(Clone, Copy)]
enum Leaf {
    EmplNatural,
    OrgNonBankingLegal,
    Country,
}

impl Leaf {
    fn insert_sql(self) -> &'static str {
        match self {
            Self::EmplNatural => {
                "INSERT INTO isahl.\"zc_id_empl-natural\" \
                 (id, code, notice, dk_scene, dk_factor, dk_function, created_by_id) \
                 VALUES (isahl.gen_next_zuid(), $1, $2, \
                 (SELECT id FROM isahl.zc_id_scene LIMIT 1), \
                 (SELECT id FROM isahl.zc_id_factor LIMIT 1), \
                 (SELECT id FROM isahl.zc_id_function WHERE code LIKE '↓\\_%' LIMIT 1), 1) \
                 RETURNING id"
            }
            Self::OrgNonBankingLegal => {
                "INSERT INTO isahl.\"zc_id_orga-non-banking-legal\" \
                 (id, code, notice, dk_scene, dk_factor, dk_function, created_by_id) \
                 VALUES (isahl.gen_next_zuid(), $1, $2, \
                 (SELECT id FROM isahl.zc_id_scene LIMIT 1), \
                 (SELECT id FROM isahl.zc_id_factor LIMIT 1), \
                 (SELECT id FROM isahl.zc_id_function WHERE code LIKE '↓\\_%' LIMIT 1), 1) \
                 RETURNING id"
            }
            Self::Country => {
                "INSERT INTO isahl.\"zc_id_subj-country\" \
                 (id, code, notice, dk_scene, dk_factor, dk_function, created_by_id) \
                 VALUES (isahl.gen_next_zuid(), $1, $2, \
                 (SELECT id FROM isahl.zc_id_scene LIMIT 1), \
                 (SELECT id FROM isahl.zc_id_factor LIMIT 1), \
                 (SELECT id FROM isahl.zc_id_function WHERE code LIKE '↓\\_%' LIMIT 1), 1) \
                 RETURNING id"
            }
        }
    }
    fn delete_sql(self) -> &'static str {
        match self {
            Self::EmplNatural => "DELETE FROM isahl.\"zc_id_empl-natural\" WHERE code = $1",
            Self::OrgNonBankingLegal => {
                "DELETE FROM isahl.\"zc_id_orga-non-banking-legal\" WHERE code = $1"
            }
            Self::Country => "DELETE FROM isahl.\"zc_id_subj-country\" WHERE code = $1",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::EmplNatural => "zc_id_empl-natural",
            Self::OrgNonBankingLegal => "zc_id_orga-non-banking-legal",
            Self::Country => "zc_id_subj-country",
        }
    }
}

/// 叶表 fixture（真叶表 + 唯一后缀码）；返回行 id。
async fn insert_leaf(pool: &PgPool, leaf: Leaf, code: &str, notice: &str) -> i64 {
    sqlx::query_scalar(leaf.insert_sql())
        .bind(code)
        .bind(notice)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("fixture {}: {e}", leaf.name()))
}

async fn delete_leaf(pool: &PgPool, leaf: Leaf, code: &str) {
    let _ = sqlx::query(leaf.delete_sql())
        .bind(code)
        .execute(pool)
        .await;
}

/// 以**谓词唯一实现**构建查询并取 id 集（与 `SUBJECT_COUNT_FROM` 同一起点口径）。
async fn filter_ids(pool: &PgPool, category: &str) -> Vec<i64> {
    let mut b: QueryBuilder<Postgres> = QueryBuilder::new(
        "SELECT s.id FROM \"isahl\".\"zc_id_subjects\" s WHERE s.deleted_at IS NULL",
    );
    push_subject_filters(
        &mut b,
        &SubjectFilters {
            q: "",
            view: "",
            category,
            kind: "",
            exclude_system: true,
            exclude_disabled: false,
        },
    );
    b.push(" ORDER BY s.id");
    b.build_query_scalar()
        .fetch_all(pool)
        .await
        .expect("filter query")
}

#[tokio::test]
async fn multi_label_union_hits_both_subtrees_and_excludes_outside() {
    let pool = test_pool().await;
    let sfx = suffix();
    let empl_code = format!("T-MC-EMPL-{sfx}");
    let org_code = format!("T-MC-ORG-{sfx}");
    let country_code = format!("T-MC-CTY-{sfx}");

    let empl_id = insert_leaf(&pool, Leaf::EmplNatural, &empl_code, "多标签雇员子树行").await;
    let org_id = insert_leaf(
        &pool,
        Leaf::OrgNonBankingLegal,
        &org_code,
        "多标签组织子树行",
    )
    .await;
    let country_id = insert_leaf(&pool, Leaf::Country, &country_code, "多标签子树外行").await;

    let ids = filter_ids(&pool, "雇员,组织").await;
    assert!(ids.contains(&empl_id), "雇员子树（自然人）行未命中并集");
    assert!(ids.contains(&org_id), "组织子树（非银行法人）行未命中并集");
    assert!(
        !ids.contains(&country_id),
        "子树外行（国家）不得进入并集结果"
    );

    delete_leaf(&pool, Leaf::EmplNatural, &empl_code).await;
    delete_leaf(&pool, Leaf::OrgNonBankingLegal, &org_code).await;
    delete_leaf(&pool, Leaf::Country, &country_code).await;
    let _ = country_id;
}

#[tokio::test]
async fn multi_label_with_unknown_fails_closed() {
    let pool = test_pool().await;
    let sfx = suffix();
    let empl_code = format!("T-MC-UNK-{sfx}");
    let empl_id = insert_leaf(&pool, Leaf::EmplNatural, &empl_code, "fail-closed 雇员行").await;

    // 任一标签未知 → 整体空集；MUST NOT 等价于「只按已知标签过滤」
    let ids = filter_ids(&pool, &format!("雇员,不存在分类-{sfx}")).await;
    assert!(
        ids.is_empty(),
        "任一未知标签须整体 fail-closed，实际 {} 行",
        ids.len()
    );
    let _ = empl_id;
    delete_leaf(&pool, Leaf::EmplNatural, &empl_code).await;
}

#[tokio::test]
async fn single_label_behavior_unchanged() {
    let pool = test_pool().await;
    let sfx = suffix();
    let org_code = format!("T-MC-LEG-{sfx}");
    let org_id = insert_leaf(
        &pool,
        Leaf::OrgNonBankingLegal,
        &org_code,
        "单标签法人子类行",
    )
    .await;

    let ids = filter_ids(&pool, "法人").await;
    assert!(
        ids.contains(&org_id),
        "单标签=法人 须含其子类（非银行法人）"
    );

    delete_leaf(&pool, Leaf::OrgNonBankingLegal, &org_code).await;
}
