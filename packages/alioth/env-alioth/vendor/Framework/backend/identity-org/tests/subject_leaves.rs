//! identity-org 主体域叶表集成测试（strengthen-identity-org 8.1）
//!
//! 覆盖：8 个新叶表 CRUD 全周期（subj-group/subj-employee/empl-agent/subj-country/
//! subj-bank/subj-ministry/subj-sovereign/subj-supranational）+ subjects_rr_place
//! 桥语义 + dk 坐标注入非空。
//! 直接驱动 Repository（handler 为薄壳：auth + 调用 repository）。
//!
//! 依赖：test 库存在上述叶表 + zc_id_scene/zc_id_factor/zc_id_function 维度行
//! （ZB/ZJ/ZH/UB/LNC/LNK/↓_DA/↓_EH 已种子）。

use crud::repository::AliothRepository as _;
use identity_org::models::*;
use identity_org::repository::*;
use sqlx::AssertSqlSafe;
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

/// 动态测试标识后缀（纳秒派生，跨运行不冲突）
fn suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", nanos % 0xF_FFFF_FFFF)
}

macro_rules! leaf_crud_test {
    ($test_fn:ident, $repo:ident, $create:ident, $update:ident, $table:literal) => {
        #[tokio::test]
        async fn $test_fn() {
            let pool = test_pool().await;
            let repo = $repo::new(pool.clone());
            let sfx = suffix();
            let code = format!("T-{}-{}", stringify!($test_fn), sfx);

            // create（dk 坐标注入）
            let created = repo
                .create(
                    $create {
                        code: Some(code.clone()),
                        notice: Some(format!("测试-{}", stringify!($test_fn))),
                        o_number: None,
                        comments: Some("integration test".into()),
                    },
                    1,
                )
                .await
                .expect(concat!("create ", $table));
            assert_eq!(created.code.as_deref(), Some(code.as_str()));

            // dk 坐标非空（维度行已种子 → resolve 必须命中）
            let dk: (Option<i64>, Option<i64>) = sqlx::query_as(AssertSqlSafe(format!(
                "SELECT dk_scene, dk_factor FROM isahl.\"{}\" WHERE id = $1",
                $table
            )))
            .bind(created.id)
            .fetch_one(&pool)
            .await
            .expect("dk columns");
            assert!(dk.0.is_some(), "dk_scene must be resolved");
            assert!(dk.1.is_some(), "dk_factor must be resolved");

            // get
            let got = repo.get(created.id).await.expect("get").expect("found");
            assert_eq!(got.code.as_deref(), Some(code.as_str()));

            // update
            let updated = repo
                .update(
                    created.id,
                    $update {
                        code: None,
                        notice: Some("已更新".into()),
                        o_number: None,
                        comments: None,
                    },
                    1,
                )
                .await
                .expect("update")
                .expect("updated row");
            assert_eq!(updated.notice.as_deref(), Some("已更新"));

            // delete（软删）
            repo.delete(created.id, 1).await.expect("delete");
            let gone = repo.get(created.id).await.expect("get after delete");
            assert!(gone.is_none(), "soft-deleted row must not be returned");
        }
    };
}

leaf_crud_test!(
    subject_group_crud_lifecycle,
    SubjectGroupRepository,
    CreateSubjectGroupRequest,
    UpdateSubjectGroupRequest,
    "zc_id_subj-group"
);
leaf_crud_test!(
    subject_employee_crud_lifecycle,
    SubjectEmployeeRepository,
    CreateSubjectEmployeeRequest,
    UpdateSubjectEmployeeRequest,
    "zc_id_subj-employee"
);
leaf_crud_test!(
    employment_agent_crud_lifecycle,
    EmploymentAgentRepository,
    CreateEmploymentAgentRequest,
    UpdateEmploymentAgentRequest,
    "zc_id_empl-agent"
);
leaf_crud_test!(
    subject_country_crud_lifecycle,
    SubjectCountryRepository,
    CreateSubjectCountryRequest,
    UpdateSubjectCountryRequest,
    "zc_id_subj-country"
);
leaf_crud_test!(
    subject_bank_crud_lifecycle,
    SubjectBankRepository,
    CreateSubjectBankRequest,
    UpdateSubjectBankRequest,
    "zc_id_subj-bank"
);
leaf_crud_test!(
    subject_ministry_crud_lifecycle,
    SubjectMinistryRepository,
    CreateSubjectMinistryRequest,
    UpdateSubjectMinistryRequest,
    "zc_id_subj-ministry"
);
leaf_crud_test!(
    subject_sovereign_crud_lifecycle,
    SubjectSovereignRepository,
    CreateSubjectSovereignRequest,
    UpdateSubjectSovereignRequest,
    "zc_id_subj-sovereign"
);
leaf_crud_test!(
    subject_supranational_crud_lifecycle,
    SubjectSupranationalRepository,
    CreateSubjectSupranationalRequest,
    UpdateSubjectSupranationalRequest,
    "zc_id_subj-supranational"
);

/// subjects_rr_place 桥语义：插入 → 幂等检查 → 软删（复制 handler add/delete 的 SQL 语义，
/// handler DTO 字段私有不可外部构造，SQL 即行为防漂移）。
#[tokio::test]
async fn subjects_rr_place_bridge_semantics() {
    let pool = test_pool().await;
    let sfx = suffix();

    // 准备：一个主体 + 一个 place 目标（lifecycle 叶）
    let subject_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_subjects\" (notice, code, created_by_id) \
         VALUES ($1, $2, 1) RETURNING id",
    )
    .bind(format!("测试主体-{sfx}"))
    .bind(format!("T-SUBJ-{sfx}"))
    .fetch_one(&pool)
    .await
    .expect("insert subject");

    let place_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_lifecycle\" (notice, code, created_by_id) \
         VALUES ($1, $2, 1) RETURNING id",
    )
    .bind(format!("测试场所-{sfx}"))
    .bind(format!("T-PLACE-{sfx}"))
    .fetch_one(&pool)
    .await
    .expect("insert place");

    // 插入桥行（ref_left=主体, ref_right=目标）
    let rel_id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_subjects_rr_place\" \
         (notice, ref_left, ref_right, created_by_id, updated_by_id) \
         VALUES ($1, $2, $3, 1, 1) RETURNING id",
    )
    .bind(format!("subject-{subject_id} 场所"))
    .bind(subject_id)
    .bind(place_id)
    .fetch_one(&pool)
    .await
    .expect("insert bridge");

    // 幂等检查（handler 的 existing 查询语义）
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl.\"zc_id_subjects_rr_place\" \
         WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(subject_id)
    .bind(place_id)
    .fetch_optional(&pool)
    .await
    .expect("idempotent check");
    assert_eq!(existing, Some(rel_id));

    // 软删
    let rows = sqlx::query(
        "UPDATE isahl.\"zc_id_subjects_rr_place\" SET deleted_at = NOW(), deleted_by_id = 1 \
         WHERE id = $1 AND ref_left = $2 AND deleted_at IS NULL",
    )
    .bind(rel_id)
    .bind(subject_id)
    .execute(&pool)
    .await
    .expect("soft delete bridge");
    assert_eq!(rows.rows_affected(), 1);

    // 软删后幂等检查不再命中
    let gone: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl.\"zc_id_subjects_rr_place\" \
         WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(subject_id)
    .bind(place_id)
    .fetch_optional(&pool)
    .await
    .expect("check after delete");
    assert!(gone.is_none());
}
