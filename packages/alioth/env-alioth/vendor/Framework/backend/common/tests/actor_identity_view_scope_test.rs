//! `common::actor_identity` 视角聚合的**主体级**回归（change `fix-view-tag-host-read-paths`）。
//!
//! 两条不变量（视角标签宿主迁移后的读径）：
//! 1. 宿主 = 岗位↔主体**关联行**（`zc_id_relation-post_view_r_tags.ref_left` = `zc_id_subj-post_rr_view.id`）；
//!    宿主为**岗位 id** 的过时行 MUST NOT 参与解析。
//! 2. 聚合以**被看待主体**为界（`zc_id_subj-post_rr_view.ref_right` = 主体）：同一岗位名下其他主体的
//!    关联行标签 MUST NOT 串入（修复前为岗位级并集——非雇员主体会拿到邻位主体的视角，如实测 `VIEW-EMPLOYEE`）。
//!
//! 运行（连共享测试库，`*_test` 由 `common::testing` 强制）：
//!   CARGO_TARGET_DIR=/tmp/alioth-check \
//!     cargo test -p common --test actor_identity_view_scope_test -- --nocapture
//!
//! 数据卫生：自建行以 `AIVS` 前缀标记，测试尾部硬删除（仅本测试创建的行）。
//! `resolve_actor_identity` 与 `actor_identity_of_subject` 的后两跳为同一实现（前者只在前面多一步
//! 账号→主体绑定），故此处直接覆盖被复用的那个函数。

use common::actor_identity::actor_identity_of_subject;
use common::testing::connect_test_db;
use sqlx::PgPool;
use std::time::{SystemTime, UNIX_EPOCH};

const MARK: &str = "AIVS";

fn mk_suffix() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis()
        .to_string()
}

/// 前置清理：清掉本测试家族的历史残留（上一轮中断的运行），保证重跑干净。
async fn purge_stale_fixtures(pool: &PgPool) {
    for sql in [
        r#"DELETE FROM isahl."zc_id_relation-post_view_r_tags" WHERE code LIKE 'AIVS-TAGROW-%'"#,
        r#"DELETE FROM isahl."zc_id_subj-post_rr_view" WHERE code LIKE 'AIVS-PAIR-%'"#,
        r#"DELETE FROM isahl."zc_id_orga-non-banking-legal" WHERE code LIKE 'AIVS-%'"#,
        r#"DELETE FROM isahl."zc_id_subj-org" WHERE code LIKE 'AIVS-%'"#,
        r#"DELETE FROM isahl."zc_id_subj-position" WHERE code LIKE 'AIVS-POS-%'"#,
        r#"DELETE FROM isahl."zc_id_tags-post_view" WHERE code LIKE 'AIVS-TAG-%'"#,
    ] {
        let _ = sqlx::query(sql).execute(pool).await;
    }
}

/// 视角字典行自愈（列集随模型演进，只填消费处用到的列），返回 id。
async fn ensure_view_dict(pool: &PgPool, code: &str) -> i64 {
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_tags-post_view" (code, notice, created_by_id)
           SELECT $1, $2, 1
           WHERE NOT EXISTS (
               SELECT 1 FROM isahl."zc_id_tags-post_view" WHERE code = $1 AND deleted_at IS NULL
           )"#,
    )
    .bind(code)
    .bind(format!("{MARK} 测试视角字典"))
    .execute(pool)
    .await
    .expect("ensure view dict");
    sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_tags-post_view" WHERE code = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1"#,
    )
    .bind(code)
    .fetch_one(pool)
    .await
    .expect("fetch view dict")
}

/// 岗位行（坐标三元组经 ontology_binding 解析，禁硬编码 ZUID），返回 id。
async fn ensure_position(pool: &PgPool, code: &str) -> i64 {
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve position coords");
    let inserted: Option<i64> = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_subj-position"
               (code, notice, created_by_id, updated_by_id, dk_scene, dk_factor, dk_function)
           SELECT $1, $2, 1, 1, $3, $4, $5
           WHERE NOT EXISTS (
               SELECT 1 FROM isahl."zc_id_subj-position" WHERE code = $1 AND deleted_at IS NULL
           )
           RETURNING id"#,
    )
    .bind(code)
    .bind(format!("{MARK} 测试岗位"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_optional(pool)
    .await
    .expect("ensure position");
    match inserted {
        Some(id) => id,
        None => sqlx::query_scalar(
            r#"SELECT id FROM isahl."zc_id_subj-position" WHERE code = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1"#,
        )
        .bind(code)
        .fetch_one(pool)
        .await
        .expect("fetch position"),
    }
}

/// 组织主体行（`zc_id_orga-non-banking-legal` 叶），返回 id。
async fn insert_subject(pool: &PgPool, tag: &str, suffix: &str) -> i64 {
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve subject coords");
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_orga-non-banking-legal"
               (notice, code, created_by_id, updated_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, 1, 1, $3, $4, $5) RETURNING id"#,
    )
    .bind(format!("{MARK} 测试主体-{tag}-{suffix}"))
    .bind(format!("{MARK}-{tag}-{suffix}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("insert subject")
}

/// 岗位↔主体视角关联行（ref_left=岗位 / ref_right=被看待主体），返回关联行 id。
async fn insert_view_pair(pool: &PgPool, position_id: i64, subject_id: i64) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_subj-post_rr_view"
               (code, notice, ref_left, ref_right, created_by_id)
           VALUES ($1, $2, $3, $4, 1) RETURNING id"#,
    )
    .bind(format!("{MARK}-PAIR-{subject_id}"))
    .bind(format!("{MARK} 视角关联行"))
    .bind(position_id)
    .bind(subject_id)
    .fetch_one(pool)
    .await
    .expect("insert view pair")
}

/// 标签行（宿主 = 给定左端 id）。
async fn host_view_tag(pool: &PgPool, ref_left: i64, tag_id: i64, notice: &str) {
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_relation-post_view_r_tags"
               (code, notice, ref_left, ref_right, created_by_id)
           VALUES ($1, $2, $3, $4, 1)"#,
    )
    .bind(format!("{MARK}-TAGROW-{ref_left}"))
    .bind(notice.to_string())
    .bind(ref_left)
    .bind(tag_id)
    .execute(pool)
    .await
    .expect("host view tag");
}

/// 主体级视角聚合：同岗位两名主体各持独立视角 + 1 条过时宿主行，断言互不串味。
#[tokio::test]
async fn view_tags_are_subject_scoped_across_a_shared_position() {
    let pool = connect_test_db().await;
    purge_stale_fixtures(&pool).await;
    let suffix = mk_suffix();
    let tag_mine = ensure_view_dict(&pool, &format!("{MARK}-TAG-MINE-{suffix}")).await;
    let tag_other = ensure_view_dict(&pool, &format!("{MARK}-TAG-OTHER-{suffix}")).await;
    let position = ensure_position(&pool, &format!("{MARK}-POS-{suffix}")).await;
    let me = insert_subject(&pool, "me", &suffix).await;
    let other = insert_subject(&pool, "other", &suffix).await;
    let pair_me = insert_view_pair(&pool, position, me).await;
    let pair_other = insert_view_pair(&pool, position, other).await;
    // 正确宿主：各自的关联行；过时宿主：岗位 id（迁移前形态）
    host_view_tag(&pool, pair_me, tag_mine, "AIVS host=relation-row").await;
    host_view_tag(&pool, pair_other, tag_other, "AIVS host=relation-row").await;
    host_view_tag(&pool, position, tag_mine, "AIVS host=position（过时）").await;

    let mine = {
        let mut conn = pool.acquire().await.expect("conn");
        actor_identity_of_subject(&mut conn, me)
            .await
            .expect("actor identity of me")
    };
    let theirs = {
        let mut conn = pool.acquire().await.expect("conn");
        actor_identity_of_subject(&mut conn, other)
            .await
            .expect("actor identity of other")
    };

    let mine_code = format!("{MARK}-TAG-MINE-{suffix}");
    let other_code = format!("{MARK}-TAG-OTHER-{suffix}");
    assert!(
        mine.position_ids.contains(&position),
        "岗位集语义不变（关联行 ref_right = 本主体 ⇒ 岗位归属）：{:?}",
        mine.position_ids
    );
    assert_eq!(
        mine.view_tags,
        vec![mine_code.clone()],
        "视角集 MUST 只含本主体关联行宿主标签（过时岗位键行不解析；同岗位其他主体标签不串入）"
    );
    assert_eq!(
        theirs.view_tags,
        vec![other_code.clone()],
        "同岗位另一主体 MUST 只见自己的关联行标签"
    );

    // 硬清理（仅本测试创建的行）
    let _ = sqlx::query(
        r#"DELETE FROM isahl."zc_id_relation-post_view_r_tags"
           WHERE code LIKE 'AIVS-TAGROW-%'"#,
    )
    .execute(&pool)
    .await;
    let _ =
        sqlx::query(r#"DELETE FROM isahl."zc_id_subj-post_rr_view" WHERE code LIKE 'AIVS-PAIR-%'"#)
            .execute(&pool)
            .await;
    let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_subj-org" WHERE id = ANY($1)"#)
        .bind(vec![me, other])
        .execute(&pool)
        .await;
    let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_orga-non-banking-legal" WHERE id = ANY($1)"#)
        .bind(vec![me, other])
        .execute(&pool)
        .await;
    let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_subj-position" WHERE code = $1"#)
        .bind(format!("{MARK}-POS-{suffix}"))
        .execute(&pool)
        .await;
    for code in [mine_code, other_code] {
        let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_tags-post_view" WHERE code = $1"#)
            .bind(code)
            .execute(&pool)
            .await;
    }
}
