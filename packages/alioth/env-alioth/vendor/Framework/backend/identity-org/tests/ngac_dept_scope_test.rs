//! B-2 部门子集 OA 树 heal 测试（cover-org-resources-ngac task 4.2）。
//!
//! 判据（tasks 4.1/4.2，实现 = `common::ngac_org::heal_department_scope` 单源，
//! identity-org 侧薄转发挂接）：`zc_id_subj-org` 层级行（org_rr_subordinate 主链）→
//! heal 后部门行 OA 存在、ancestor 链 = 直接父 OA（根挂 `departments:0` 集合 OA），
//! 且**幂等**（重跑零新增、ancestor 不变）。
//!
//! 夹具自持：集合 OA 经 `common::ngac_grants::ensure_org_resource_coverage` 预铺
//!（`departments` 属平台默认注册集），不依赖 Gateway 启动自愈时序。
//! 动态 id（tid）不清理，对齐本套件惯例。

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

async fn ensure_org(pool: &PgPool, id: i64, notice: &str) {
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve org coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_orga-department" (id, notice, code, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING"#,
    )
    .bind(id)
    .bind(notice)
    .bind(format!("T-DEPT-SCOPE-{id}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(pool)
    .await
    .expect("ensure org");
}

/// 部门行 OA（id, ancestor_ids——bigint[] 可空元素）；不存在 → None。
async fn dept_oa(pool: &PgPool, dept_id: i64) -> Option<(i64, Option<Vec<Option<i64>>>)> {
    sqlx::query_as(
        "SELECT id, ancestor_ids FROM isahl_auth.ngac_object_attribute \
         WHERE resource_type = 'departments' AND fk_resource = $1 AND deleted_at IS NULL",
    )
    .bind(dept_id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn heal_department_scope_builds_subset_oa_tree_idempotently() {
    let pool = test_pool().await;
    let parent = tid(200);
    let child = tid(201);

    ensure_org(&pool, parent, "B-2 子集 OA 测试-父部门").await;
    ensure_org(&pool, child, "B-2 子集 OA 测试-子部门").await;
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-org_rr_subordinate" (ref_left, ref_right)
           VALUES ($1, $2) ON CONFLICT DO NOTHING"#,
    )
    .bind(parent)
    .bind(child)
    .execute(&pool)
    .await
    .expect("bridge parent→child");

    // 集合 OA 自持预铺（departments 属平台默认注册集）——覆盖 ensure 单入口
    let types = vec!["departments".to_string()];
    common::ngac_grants::ensure_org_resource_coverage(&pool, &types)
        .await
        .expect("coverage ensure（departments:0 集合 OA 预铺）");

    // ── heal 子部门：子集 OA 树 = 子行 OA（ancestor=[父 OA]）+ 父行 OA（ancestor=[departments:0]）──
    identity_org::ngac_org_ensure::heal_department_scope(&pool, child).await;

    let (child_oa, child_anc) = dept_oa(&pool, child).await.expect("子部门行 OA 存在");
    let (parent_oa, parent_anc) = dept_oa(&pool, parent).await.expect("父部门行 OA 存在");
    assert_eq!(
        child_anc,
        Some(vec![Some(parent_oa)]),
        "子 OA 直接父 = 父部门 OA（单链，逐节点回填）"
    );

    let collection: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute \
         WHERE resource_type = 'departments' AND fk_resource = 0 AND deleted_at IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        parent_anc,
        Some(vec![Some(collection)]),
        "根节点 ancestor = departments:0 集合 OA id"
    );

    // ── 幂等：重跑零新增、ancestor 不变 ──
    identity_org::ngac_org_ensure::heal_department_scope(&pool, child).await;
    let (child_oa2, child_anc2) = dept_oa(&pool, child).await.expect("重跑后子部门行 OA 仍在");
    assert_eq!(child_oa2, child_oa, "不重复建行");
    assert_eq!(child_anc2, child_anc, "ancestor 不漂移");

    let dept_row_oas: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.ngac_object_attribute \
         WHERE resource_type = 'departments' AND fk_resource = ANY($1) AND deleted_at IS NULL",
    )
    .bind(&[child, parent][..])
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(dept_row_oas, 2, "两节点各一行 OA");
}
