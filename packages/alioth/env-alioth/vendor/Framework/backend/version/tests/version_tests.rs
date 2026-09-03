use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use version::*;

#[test]
fn test_version_error_display() {
    let err = VersionError::NotFound("entity 42".to_string());
    assert!(err.to_string().contains("entity 42"));

    let err = VersionError::InvalidOperation("cannot rollback".to_string());
    assert!(err.to_string().contains("cannot rollback"));
}

#[test]
fn test_version_record_construction() {
    let now = Utc::now();
    let record = VersionRecord {
        id: 1,
        tk_version: Some(2),
        x_version: Some("v2.0".to_string()),
        reversion: Some(1),
        fk_previous: Some(0),
        ck_branch: None,
        majority: Some("release".to_string()),
        sprint: Some("sprint-1".to_string()),
        git_ref: None,
        git_oid: None,
        created_at: now,
        created_by_id: Some(42),
    };

    assert_eq!(record.id, 1);
    assert_eq!(record.tk_version, Some(2));
    assert_eq!(record.x_version.as_deref(), Some("v2.0"));
}

#[test]
fn test_version_record_serde() {
    let now = Utc::now();
    let record = VersionRecord {
        id: 1,
        tk_version: Some(1),
        x_version: None,
        reversion: Some(0),
        fk_previous: None,
        ck_branch: None,
        majority: None,
        sprint: None,
        git_ref: None,
        git_oid: None,
        created_at: now,
        created_by_id: None,
    };
    let json = serde_json::to_string(&record).unwrap();
    let back: VersionRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, 1);
    assert_eq!(back.reversion, Some(0));
}

#[test]
fn test_version_diff_construction() {
    let diff = VersionDiff {
        field: "notice".to_string(),
        old_value: Some(json!("old")),
        new_value: Some(json!("new")),
    };
    assert_eq!(diff.field, "notice");
    assert_eq!(
        diff.old_value.as_ref().and_then(|v| v.as_str()),
        Some("old")
    );
}

#[test]
fn test_version_diff_serde() {
    let diff = VersionDiff {
        field: "code".to_string(),
        old_value: None,
        new_value: Some(json!("NEW-001")),
    };
    let json = serde_json::to_string(&diff).unwrap();
    let back: VersionDiff = serde_json::from_str(&json).unwrap();
    assert_eq!(back.field, "code");
    assert!(back.old_value.is_none());
}

#[test]
fn test_version_service_is_object_safe() {
    // VersionService must be object-safe (Send + Sync)
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<&dyn VersionService>();
}

// ── diff 集成测试（DB）────────────────────────────────────────────
// implement-framework-version-diff：两版本已知变更 → 字段级 diff；相同版本 → 空

struct TestVersionService;

#[async_trait::async_trait]
impl VersionService for TestVersionService {
    fn version_table_name(&self) -> &'static str {
        "zc_id_version"
    }

    async fn create_version(
        &self,
        _pool: &PgPool,
        _entity_id: i64,
        _x_version: Option<String>,
        _comment: Option<String>,
    ) -> VersionResult<VersionRecord> {
        unimplemented!("create_version not exercised")
    }

    async fn list_versions(
        &self,
        _pool: &PgPool,
        _entity_id: i64,
    ) -> VersionResult<Vec<VersionRecord>> {
        unimplemented!("list_versions not exercised")
    }

    async fn rollback(
        &self,
        _pool: &PgPool,
        _entity_id: i64,
        _target_version_id: i64,
    ) -> VersionResult<VersionRecord> {
        unimplemented!("rollback not exercised")
    }
}

async fn insert_version_row(pool: &PgPool, notice: &str) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_version (notice, created_at) VALUES ($1, NOW()) RETURNING id"#,
    )
    .bind(notice)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn diff_between_two_versions_reports_field_changes() {
    let pool = common::testing::connect_test_db().await;
    let id_a = insert_version_row(&pool, "版本A-初始").await;
    let id_b = insert_version_row(&pool, "版本B-变更").await;

    let svc = TestVersionService;
    let diffs = svc.diff(&pool, id_a, id_b).await.unwrap();

    // notice 变更应产生一条 diff
    let notice_diff = diffs
        .iter()
        .find(|d| d.field == "notice")
        .expect("notice 字段应有 diff");
    assert_eq!(
        notice_diff.old_value.as_ref().and_then(|v| v.as_str()),
        Some("版本A-初始")
    );
    assert_eq!(
        notice_diff.new_value.as_ref().and_then(|v| v.as_str()),
        Some("版本B-变更")
    );

    sqlx::query("DELETE FROM isahl.zc_id_version WHERE id = ANY($1)")
        .bind([id_a, id_b])
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn diff_between_identical_versions_is_empty() {
    let pool = common::testing::connect_test_db().await;
    let id = insert_version_row(&pool, "相同版本").await;

    let svc = TestVersionService;
    let diffs = svc.diff(&pool, id, id).await.unwrap();
    assert!(diffs.is_empty(), "相同版本 diff 应为空，实际: {:?}", diffs);

    sqlx::query("DELETE FROM isahl.zc_id_version WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
}
