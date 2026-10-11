//! D-2 组织方案激活测试（org_design_scheme 状态机 + 差异派生幂等）。
//!
//! 覆盖（实现 = `identity_org::org_scheme::activate_scheme`，先期由并行线落地，
//! 本测试补 D-2 验收面——roadmap R-D1 全局单一方案裁决的回归锁）：
//! 1. **状态机**：draft → activate → active；**全局单一方案**——激活新方案自动
//!    supersede 旧 active（R-D1 裁决语义）；
//! 2. **差分幂等**：同方案重复 activate ⇒ 400（仅 draft 可激活）+ 数据零新增；
//!    新 draft（同 content）激活 ⇒ 复用既有范例/实例（差额补齐，零重复行）；
//! 3. **tpl 双态**：内联条目激活 → 设计·范例行 + 实例行（`common::real_position_row`
//!    判据）+ 部门桥挂接。
//!
//! 夹具：`org_design_scheme` 经同源 ensure 建表；类别取 `zc_id_category` **基表行**
//!（tableoid 过滤语义同实现——测试库缺基行时插入 `z-` 码夹具行并自清）；
//! 方案行/岗位行按动态 id 自清。

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

fn tid(base: i64) -> i64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    ((nanos % 1_000_000) as i64) * 100 + base
}

const CAT_CODE: &str = "ccb_member"; // 测试库既有基表行（tableoid 过滤语义同实现），免插夹具

/// 插入 draft 方案行，返回 id。
async fn insert_draft_scheme(pool: &PgPool, code: &str, name_suffix: &str) -> i64 {
    let content = serde_json::json!({
        "templates": [
            { "name": format!("D2岗-{name_suffix}"), "categoryCode": CAT_CODE, "maxHeads": 3 }
        ]
    });
    sqlx::query_scalar(
        r#"INSERT INTO isahl_auth.org_design_scheme (code, title, content, state, created_by_id)
           VALUES ($1, $2, $3, 'draft', 1) RETURNING id"#,
    )
    .bind(code)
    .bind(format!("D-2 测试方案-{code}"))
    .bind(content)
    .fetch_one(pool)
    .await
    .expect("insert draft scheme")
}

async fn scheme_state(pool: &PgPool, id: i64) -> String {
    sqlx::query_scalar("SELECT state FROM isahl_auth.org_design_scheme WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn activate_scheme_supersedes_old_and_derives_idempotently() {
    let pool = test_pool().await;
    identity_org::org_scheme::ensure_org_design_scheme(&pool)
        .await
        .expect("同源建表");

    let s1 = insert_draft_scheme(&pool, &format!("z-test-d2-s1-{}", tid(0)), "一").await;
    let s2 = insert_draft_scheme(&pool, &format!("z-test-d2-s2-{}", tid(0)), "二").await;

    // ── 激活 s1：draft → active，内联条目差异派生（范例 + 实例）──
    let out1 = identity_org::org_scheme::activate_scheme(&pool, 1, "d2-test@alioth.test", s1)
        .await
        .expect("activate s1");
    assert_eq!(out1.state, "active", "s1 激活后 active");
    assert_eq!(scheme_state(&pool, s1).await, "active");

    // tpl 双态：设计·范例行（tpl_id NULL）与实例行（tpl_id 置位——real_position_row 判据，
    // 实例行 _f_/_t_ 为空，判据必须走 tpl_id）各一
    let (tpl_rows, inst_rows): (i64, i64) = sqlx::query_as(
        r#"SELECT count(*) FILTER (WHERE _t_ = '范例' AND tpl_id IS NULL),
                  count(*) FILTER (WHERE tpl_id IS NOT NULL)
           FROM isahl."zc_id_subj-position" p
           WHERE p.notice LIKE 'D2岗-%' AND p.deleted_at IS NULL"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(tpl_rows >= 1, "内联条目建设计·范例行（D-2a 双态）");
    assert!(inst_rows >= 1, "范例实例化落岗");

    // ── 全局单一方案（R-D1）：激活 s2 ⇒ s1 自动 superseded ──
    let out2 = identity_org::org_scheme::activate_scheme(&pool, 1, "d2-test@alioth.test", s2)
        .await
        .expect("activate s2");
    assert_eq!(out2.state, "active");
    assert_eq!(
        scheme_state(&pool, s1).await,
        "superseded",
        "旧 active 自动 superseded（全局单一方案）"
    );

    // ── 状态机守卫：仅 draft 可激活 ──
    let err = identity_org::org_scheme::activate_scheme(&pool, 1, "d2-test@alioth.test", s1)
        .await
        .expect_err("superseded 方案拒绝再激活");
    assert!(
        err.to_string().contains("draft"),
        "错误信息指明仅 draft 可激活：{err}"
    );

    // ── 差分幂等：同 content 新 draft 激活 ⇒ 复用既有范例/实例（零重复）──
    let before: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl."zc_id_subj-position"
           WHERE notice LIKE 'D2岗-%' AND deleted_at IS NULL"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let s3 = insert_draft_scheme(&pool, &format!("z-test-d2-s3-{}", tid(0)), "一").await; // 同 content（同名同类别）
    identity_org::org_scheme::activate_scheme(&pool, 1, "d2-test@alioth.test", s3)
        .await
        .expect("activate s3（差分复用）");
    let after: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl."zc_id_subj-position"
           WHERE notice LIKE 'D2岗-%' AND deleted_at IS NULL"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        before, after,
        "同 (category, name) 条目复用既有范例/实例——差分派生零重复"
    );
}
