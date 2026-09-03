//! identity factor — E2E 集成测试
//!
//! 验证 Engineer / SkillTag / ApprovalRole / CCBMember CRUD 操作的
//! 数据库写入与读取正确性：
//! - CREATE（INSERT）：插入一行，验证返回
//! - READ（SELECT）：按 ID 查询，验证 name 匹配
//! - UPDATE（UPDATE）：更新 name，验证变更
//! - DELETE（soft）：设置 deleted_at，验证被过滤
//! - LIST：查询全部，验证数量
//!
//! 数据依赖：
//! - isahl."zc_id_subj-employee"（工程师）
//! - isahl."zc_id_tags-skill"（技能标签）
//! - isahl."zc_id_cate-approve_role"（审批岗位）
//! - isahl."zc_id_subj-position"（CCB 岗位成员）

mod common;
use ::common::testing::connect_test_db;
use common::{setup_test_schema, test_code};

use sqlx::PgPool;

// ═══════════════════════════════════════════════════════════════════════════════
// 辅助函数 — Engineer
// ═══════════════════════════════════════════════════════════════════════════════

async fn insert_engineer(pool: &PgPool, notice: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_subj-employee" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(notice)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn get_engineer_name(pool: &PgPool, id: i64) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        r#"SELECT notice FROM isahl."zc_id_subj-employee" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

async fn update_engineer_name(pool: &PgPool, id: i64, new_name: &str) {
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-employee" SET notice = $1, updated_by_id = 1
           WHERE id = $2 AND deleted_at IS NULL"#,
    )
    .bind(new_name)
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

async fn soft_delete_engineer(pool: &PgPool, id: i64) {
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-employee" SET deleted_at = NOW(), deleted_by_id = 1
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

async fn count_engineers(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*) FROM isahl."zc_id_subj-employee" WHERE deleted_at IS NULL"#,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

// ═══════════════════════════════════════════════════════════════════════════════
// 辅助函数 — SkillTag
// ═══════════════════════════════════════════════════════════════════════════════

async fn insert_skill_tag(pool: &PgPool, notice: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_tags-skill" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(notice)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn get_skill_tag_name(pool: &PgPool, id: i64) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        r#"SELECT notice FROM isahl."zc_id_tags-skill" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

async fn update_skill_tag_name(pool: &PgPool, id: i64, new_name: &str) {
    sqlx::query(
        r#"UPDATE isahl."zc_id_tags-skill"
           SET notice = $1, updated_by_id = 1
           WHERE id = $2 AND deleted_at IS NULL"#,
    )
    .bind(new_name)
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

async fn soft_delete_skill_tag(pool: &PgPool, id: i64) {
    sqlx::query(
        r#"UPDATE isahl."zc_id_tags-skill"
           SET deleted_at = NOW(), deleted_by_id = 1
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

async fn count_skill_tags(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*) FROM isahl."zc_id_tags-skill" WHERE deleted_at IS NULL"#,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

// ═══════════════════════════════════════════════════════════════════════════════
// 辅助函数 — ApprovalRole
// ═══════════════════════════════════════════════════════════════════════════════

async fn insert_approval_role(pool: &PgPool, notice: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_cate-approve_role" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(notice)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn get_approval_role_name(pool: &PgPool, id: i64) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        r#"SELECT notice FROM isahl."zc_id_cate-approve_role" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

async fn update_approval_role_name(pool: &PgPool, id: i64, new_name: &str) {
    sqlx::query(
        r#"UPDATE isahl."zc_id_cate-approve_role" SET notice = $1, updated_by_id = 1
           WHERE id = $2 AND deleted_at IS NULL"#,
    )
    .bind(new_name)
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

async fn soft_delete_approval_role(pool: &PgPool, id: i64) {
    sqlx::query(
        r#"UPDATE isahl."zc_id_cate-approve_role" SET deleted_at = NOW(), deleted_by_id = 1
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

async fn count_approval_roles(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*) FROM isahl."zc_id_cate-approve_role" WHERE deleted_at IS NULL"#,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

// ═══════════════════════════════════════════════════════════════════════════════
// 辅助函数 — CCBMember
// ═══════════════════════════════════════════════════════════════════════════════

async fn insert_ccb_member(pool: &PgPool, notice: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_subj-position" (notice, created_by_id)
           VALUES ($1, 1) RETURNING id"#,
    )
    .bind(notice)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn get_ccb_member_name(pool: &PgPool, id: i64) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        r#"SELECT notice FROM isahl."zc_id_subj-position" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

async fn update_ccb_member_name(pool: &PgPool, id: i64, new_name: &str) {
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-position" SET notice = $1, updated_by_id = 1
           WHERE id = $2 AND deleted_at IS NULL"#,
    )
    .bind(new_name)
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

async fn soft_delete_ccb_member(pool: &PgPool, id: i64) {
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-position" SET deleted_at = NOW(), deleted_by_id = 1
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

async fn count_ccb_members(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*) FROM isahl."zc_id_subj-position" WHERE deleted_at IS NULL"#,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

// ═══════════════════════════════════════════════════════════════════════════════
// 测试用例 — Engineer CRUD
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn engineer_create_and_read() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let name = test_code("engineer");
    let id = insert_engineer(&pool, &name).await;

    assert!(id > 0, "Engineer 插入后应返回正数 ID");

    let fetched = get_engineer_name(&pool, id).await;
    assert_eq!(fetched, Some(name), "READ 应返回刚插入的名称");
}

#[tokio::test]
async fn engineer_update_name() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let original = test_code("engineer-orig");
    let id = insert_engineer(&pool, &original).await;

    let updated_name = test_code("engineer-upd");
    update_engineer_name(&pool, id, &updated_name).await;

    let fetched = get_engineer_name(&pool, id).await;
    assert_eq!(fetched, Some(updated_name), "UPDATE 后名称应变更");
}

#[tokio::test]
async fn engineer_soft_delete_filters() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let name = test_code("engineer-del");
    let id = insert_engineer(&pool, &name).await;

    // 删除前应能查到
    assert_eq!(
        get_engineer_name(&pool, id).await,
        Some(name.clone()),
        "删除前应查到名称"
    );

    soft_delete_engineer(&pool, id).await;

    // 删除后应被过滤（deleted_at IS NULL）
    assert_eq!(get_engineer_name(&pool, id).await, None, "软删除后不应查到");
}

#[tokio::test]
async fn engineer_list_counts() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let initial = count_engineers(&pool).await;

    insert_engineer(&pool, &test_code("engineer-a")).await;
    insert_engineer(&pool, &test_code("engineer-b")).await;
    insert_engineer(&pool, &test_code("engineer-c")).await;

    assert_eq!(
        count_engineers(&pool).await,
        initial + 3,
        "插入 3 条后 LIST 计数应增加 3"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// 测试用例 — SkillTag CRUD
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn skill_tag_create_and_read() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let name = test_code("skill-tag");
    let id = insert_skill_tag(&pool, &name).await;

    assert!(id > 0, "SkillTag 插入后应返回正数 ID");

    let fetched = get_skill_tag_name(&pool, id).await;
    assert_eq!(fetched, Some(name), "READ 应返回刚插入的名称");
}

#[tokio::test]
async fn skill_tag_update_name() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let original = test_code("skill-tag-orig");
    let id = insert_skill_tag(&pool, &original).await;

    let updated = test_code("skill-tag-upd");
    update_skill_tag_name(&pool, id, &updated).await;

    let fetched = get_skill_tag_name(&pool, id).await;
    assert_eq!(fetched, Some(updated), "UPDATE 后名称应变更");
}

#[tokio::test]
async fn skill_tag_soft_delete_filters() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let name = test_code("skill-tag-del");
    let id = insert_skill_tag(&pool, &name).await;

    assert_eq!(
        get_skill_tag_name(&pool, id).await,
        Some(name.clone()),
        "删除前应查到名称"
    );

    soft_delete_skill_tag(&pool, id).await;

    assert_eq!(
        get_skill_tag_name(&pool, id).await,
        None,
        "软删除后不应查到"
    );
}

#[tokio::test]
async fn skill_tag_list_counts() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let initial = count_skill_tags(&pool).await;

    insert_skill_tag(&pool, &test_code("tag-a")).await;
    insert_skill_tag(&pool, &test_code("tag-b")).await;

    assert_eq!(
        count_skill_tags(&pool).await,
        initial + 2,
        "插入 2 条后 LIST 计数应增加 2"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// 测试用例 — ApprovalRole CRUD
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn approval_role_create_and_read() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let name = test_code("approval-role");
    let id = insert_approval_role(&pool, &name).await;

    assert!(id > 0, "ApprovalRole 插入后应返回正数 ID");

    let fetched = get_approval_role_name(&pool, id).await;
    assert_eq!(fetched, Some(name), "READ 应返回刚插入的名称");
}

#[tokio::test]
async fn approval_role_update_name() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let original = test_code("approle-orig");
    let id = insert_approval_role(&pool, &original).await;

    let updated = test_code("approle-upd");
    update_approval_role_name(&pool, id, &updated).await;

    let fetched = get_approval_role_name(&pool, id).await;
    assert_eq!(fetched, Some(updated), "UPDATE 后名称应变更");
}

#[tokio::test]
async fn approval_role_soft_delete_filters() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let name = test_code("approle-del");
    let id = insert_approval_role(&pool, &name).await;

    assert_eq!(
        get_approval_role_name(&pool, id).await,
        Some(name.clone()),
        "删除前应查到名称"
    );

    soft_delete_approval_role(&pool, id).await;

    assert_eq!(
        get_approval_role_name(&pool, id).await,
        None,
        "软删除后不应查到"
    );
}

#[tokio::test]
async fn approval_role_list_counts() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let initial = count_approval_roles(&pool).await;

    insert_approval_role(&pool, &test_code("role-a")).await;
    insert_approval_role(&pool, &test_code("role-b")).await;
    insert_approval_role(&pool, &test_code("role-c")).await;

    assert_eq!(
        count_approval_roles(&pool).await,
        initial + 3,
        "插入 3 条后 LIST 计数应增加 3"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// 测试用例 — CCBMember CRUD
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn ccb_member_create_and_read() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let name = test_code("ccb-member");
    let id = insert_ccb_member(&pool, &name).await;

    assert!(id > 0, "CCBMember 插入后应返回正数 ID");

    let fetched = get_ccb_member_name(&pool, id).await;
    assert_eq!(fetched, Some(name), "READ 应返回刚插入的名称");
}

#[tokio::test]
async fn ccb_member_update_name() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let original = test_code("ccb-orig");
    let id = insert_ccb_member(&pool, &original).await;

    let updated = test_code("ccb-upd");
    update_ccb_member_name(&pool, id, &updated).await;

    let fetched = get_ccb_member_name(&pool, id).await;
    assert_eq!(fetched, Some(updated), "UPDATE 后名称应变更");
}

#[tokio::test]
async fn ccb_member_soft_delete_filters() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let name = test_code("ccb-del");
    let id = insert_ccb_member(&pool, &name).await;

    assert_eq!(
        get_ccb_member_name(&pool, id).await,
        Some(name.clone()),
        "删除前应查到名称"
    );

    soft_delete_ccb_member(&pool, id).await;

    assert_eq!(
        get_ccb_member_name(&pool, id).await,
        None,
        "软删除后不应查到"
    );
}

#[tokio::test]
async fn ccb_member_list_counts() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let initial = count_ccb_members(&pool).await;

    insert_ccb_member(&pool, &test_code("ccb-a")).await;
    insert_ccb_member(&pool, &test_code("ccb-b")).await;

    assert_eq!(
        count_ccb_members(&pool).await,
        initial + 2,
        "插入 2 条后 LIST 计数应增加 2"
    );
}
