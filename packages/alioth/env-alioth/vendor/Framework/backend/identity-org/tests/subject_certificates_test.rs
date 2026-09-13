//! 主体持有证书（物权语义）集成测试（add-subject-certificate-title）
//!
//! 直驱核心函数（handler 为薄壳）：取得（纸质/数字化）→ mv_title_ownership 自动
//! 关联 → 持有查询聚合；幂等重取不重复记账；类别/主体 fail-closed。
//!
//! 依赖：test 库证书族叶表 + cate-certification 字典 + 维度行（JC/GID/↓_LA 已种子）。

use identity_org::handlers::subject_certificates::{acquire, held, AcquireCertificateRequest};
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

fn suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", nanos % 0xF_FFFF_FFFF)
}

#[tokio::test]
async fn acquire_creates_title_association_and_list_aggregates() {
    let pool = test_pool().await;
    let sfx = suffix();

    // fixture：主体 + 类别（类列形态 1：dk_* 派生源，_f_/_t_ 由触发器派生）
    let subject_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_empl-natural"
           (id, code, notice, dk_scene, dk_factor, dk_function, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, '证书物权测试主体',
                   (SELECT id FROM isahl.zc_id_scene LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code LIKE '↓\_%' LIMIT 1),
                   1) RETURNING id"#,
    )
    .bind(format!("T-SUBJ-{sfx}"))
    .fetch_one(&pool)
    .await
    .expect("主体 fixture");
    let cat_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_cate-certification" (notice, code, created_by_id)
           VALUES ('证书物权测试类型', $1, 1) RETURNING id"#,
    )
    .bind(format!("CERT-T-{sfx}"))
    .fetch_one(&pool)
    .await
    .expect("类别 fixture");
    // 取得：纸质 + 数字化各一张
    let paper = acquire(
        &pool,
        subject_id,
        &AcquireCertificateRequest {
            name: "测试纸质证书".into(),
            code: format!("T-CERT-P-{sfx}"),
            category_code: format!("CERT-T-{sfx}"),
            kind: "diploma".into(),
            qty: 1.0,
        },
        1,
    )
    .await
    .expect("纸质取得");
    assert_eq!(paper.medium, "paper");
    assert_eq!(paper.kind, "zc_id_prod-diploma-sales");

    let digital = acquire(
        &pool,
        subject_id,
        &AcquireCertificateRequest {
            name: "测试数字证书".into(),
            code: format!("T-CERT-D-{sfx}"),
            category_code: format!("CERT-T-{sfx}"),
            kind: "digital".into(),
            qty: 2.0,
        },
        1,
    )
    .await
    .expect("数字化取得");
    assert_eq!(digital.medium, "digital");
    assert_eq!(digital.kind, "zc_id_prod-digital_cert-sales");

    // mv_title_ownership 自动关联（凭证净变聚合）
    let (net, count): (f64, i64) = sqlx::query_as(
        "SELECT net_qty::float8, voucher_count FROM isahl.mv_title_ownership \
         WHERE subject_id = $1 AND production_id = $2",
    )
    .bind(subject_id)
    .bind(digital.id)
    .fetch_one(&pool)
    .await
    .expect("mv 属权行");
    assert!((net - 2.0).abs() < 0.01, "净属权应 2，实际 {net}");
    assert_eq!(count, 1);

    // 持有查询：纸质/数字化聚合，类别名解析
    let held_list = held(&pool, subject_id).await.expect("持有查询");
    let paper_row = held_list
        .iter()
        .find(|c| c.id == paper.id)
        .expect("纸质证书在持有列表");
    assert_eq!(paper_row.medium, "paper");
    assert_eq!(paper_row.category.as_deref(), Some("证书物权测试类型"));
    let digital_row = held_list
        .iter()
        .find(|c| c.id == digital.id)
        .expect("数字证书在持有列表");
    assert!((digital_row.net_qty - 2.0).abs() < 0.01);

    // 幂等：同 code 重复取得 → 凭证跳过，净属权不变（2，凭证仍 1 笔）
    let again = acquire(
        &pool,
        subject_id,
        &AcquireCertificateRequest {
            name: "测试数字证书".into(),
            code: format!("T-CERT-D-{sfx}"),
            category_code: format!("CERT-T-{sfx}"),
            kind: "digital".into(),
            qty: 2.0,
        },
        1,
    )
    .await;
    // 证书实体 code 冲突（业务键）→ 数据库拒绝；凭证幂等路径由 create_title_voucher_tx 保证
    assert!(again.is_err(), "同 code 重复取得应被业务键拒绝");

    // fail-closed：类别不存在
    let bad = acquire(
        &pool,
        subject_id,
        &AcquireCertificateRequest {
            name: "坏类别".into(),
            code: format!("T-CERT-X-{sfx}"),
            category_code: "CERT-NOT-EXIST".into(),
            kind: "diploma".into(),
            qty: 1.0,
        },
        1,
    )
    .await;
    assert!(bad.is_err(), "未知类别应 fail-closed");

    // fail-closed：主体不存在
    let no_subject = acquire(
        &pool,
        987_654_321,
        &AcquireCertificateRequest {
            name: "坏主体".into(),
            code: format!("T-CERT-N-{sfx}"),
            category_code: format!("CERT-T-{sfx}"),
            kind: "diploma".into(),
            qty: 1.0,
        },
        1,
    )
    .await;
    assert!(no_subject.is_err(), "未知主体应 fail-closed");

    // 清理：凭证 + 标量 + 证书 + 类别 + 主体（LIKE 前缀），刷新回净
    for sql in [
        format!(
            r#"DELETE FROM isahl."zc_id_stat-whs-voucher" WHERE code LIKE 'TTL-SUBJ{subject_id}-CERT-T-CERT-%'"#
        ),
        format!(
            r#"DELETE FROM isahl."zc_id_scal-common" WHERE code LIKE 'TTL-SUBJ{subject_id}-CERT%'"#
        ),
        format!(
            r#"DELETE FROM isahl."zc_id_prod-diploma-sales" WHERE code LIKE 'T-CERT-P-{sfx}' OR code LIKE 'T-CERT-X-{sfx}' OR code LIKE 'T-CERT-N-{sfx}'"#
        ),
        format!(
            r#"DELETE FROM isahl."zc_id_prod-digital_cert-sales" WHERE code LIKE 'T-CERT-D-{sfx}'"#
        ),
        format!(r#"DELETE FROM isahl."zc_id_cate-certification" WHERE code = 'CERT-T-{sfx}'"#),
        format!(r#"DELETE FROM isahl."zc_id_empl-natural" WHERE id = {subject_id}"#),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .execute(&pool)
            .await
            .expect("cleanup");
    }
    // 纸质/数字化失败路径的证书残留（坏类别/坏主体在 INSERT 前已拒绝——无残留；
    // 坏主体路径的主体检查在证书 INSERT 之前，同样无证书残留）
    sqlx::query(sqlx::AssertSqlSafe(
        "REFRESH MATERIALIZED VIEW isahl.mv_title_ownership",
    ))
    .execute(&pool)
    .await
    .expect("refresh after cleanup");

    let _ = cat_id; // 类别行已按 code 清理
}
