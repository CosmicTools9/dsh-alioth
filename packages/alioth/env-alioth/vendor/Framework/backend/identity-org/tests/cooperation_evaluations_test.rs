//! 供应商合作评价（cooperation_r_evaluation 桥）集成测试
//! （wire-supplier-qualification-chain，B3 资质证照链）
//!
//! 直驱核心函数（handler 为薄壳）：登记评价 → 桥行落库 → 读谓词 JOIN 解析；
//! 调级软删替换（历史保留）；类型/等级/主体 fail-closed。
//!
//! fixture 自持（主体 empl-natural 叶 + 类型/等级字典行，code 前缀清理），
//! 不依赖种子执行状态。

use identity_org::handlers::cooperation_evaluations::{
    create, current, CreateCooperationEvaluationRequest,
};
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
async fn evaluation_lifecycle_and_read_predicate() {
    let pool = test_pool().await;
    let sfx = suffix();

    // fixture：主体（empl-natural 叶，dk 派生形态）+ 合作类型 + 资质等级
    let subject_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_empl-natural"
           (id, code, notice, dk_scene, dk_factor, dk_function, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, '合作评价测试主体',
                   (SELECT id FROM isahl.zc_id_scene LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code LIKE '↓\_%' LIMIT 1),
                   1) RETURNING id"#,
    )
    .bind(format!("T-COOP-SUBJ-{sfx}"))
    .fetch_one(&pool)
    .await
    .expect("主体 fixture");

    let coop_code = format!("COOP-T-{sfx}");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_cate-cooperation" (notice, code, created_by_id)
           VALUES ('测试运输合作', $1, 1)"#,
    )
    .bind(&coop_code)
    .execute(&pool)
    .await
    .expect("类型 fixture");
    let qual_a = format!("QUAL-T-A-{sfx}");
    let qual_b = format!("QUAL-T-B-{sfx}");
    for (notice, code, lv) in [
        ("优质", qual_a.as_str(), 4i32),
        ("合格", qual_b.as_str(), 3i32),
    ] {
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_leve-qualification" (notice, code, lv_value, created_by_id)
               VALUES ($1, $2, $3::numeric, 1)"#,
        )
        .bind(notice)
        .bind(code)
        .bind(lv)
        .execute(&pool)
        .await
        .expect("等级 fixture");
    }

    // 登记：初始评价 QUAL-A
    let eval_a = create(
        &pool,
        subject_id,
        &CreateCooperationEvaluationRequest {
            cooperation_code: coop_code.clone(),
            qualification_code: qual_a.clone(),
            remark: Some("首评".into()),
        },
        1,
    )
    .await
    .expect("登记评价 A");
    assert_eq!(eval_a.cooperation_name, "测试运输合作");
    assert_eq!(eval_a.qualification_name, "优质");
    assert!((eval_a.level_value - 4.0).abs() < 0.01);

    // 读谓词：当前 1 条，JOIN 解析齐
    let list = current(&pool, subject_id).await.expect("读谓词");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].qualification_code, qual_a);

    // 调级：QUAL-A → QUAL-B（旧行软删，历史保留）
    let eval_b = create(
        &pool,
        subject_id,
        &CreateCooperationEvaluationRequest {
            cooperation_code: coop_code.clone(),
            qualification_code: qual_b.clone(),
            remark: None,
        },
        1,
    )
    .await
    .expect("调级 B");
    assert_ne!(eval_b.id, eval_a.id, "调级应落新桥行");

    let list = current(&pool, subject_id).await.expect("读谓词 2");
    assert_eq!(list.len(), 1, "同合作类型仅当前等级可见");
    assert_eq!(list[0].qualification_code, qual_b);
    let soft_deleted: (i64,) = sqlx::query_as(
        r#"SELECT count(*) FROM isahl."zc_id_relation-cooperation_r_evaluation"
           WHERE ref_left = $1 AND deleted_at IS NOT NULL"#,
    )
    .bind(subject_id)
    .fetch_one(&pool)
    .await
    .expect("历史行数");
    assert_eq!(soft_deleted.0, 1, "调级历史行软删保留");

    // fail-closed：未知类型 / 未知等级 / 未知主体
    let bad_coop = create(
        &pool,
        subject_id,
        &CreateCooperationEvaluationRequest {
            cooperation_code: "COOP-NOT-EXIST".into(),
            qualification_code: qual_b.clone(),
            remark: None,
        },
        1,
    )
    .await;
    assert!(bad_coop.is_err(), "未知合作类型应 fail-closed");
    let bad_level = create(
        &pool,
        subject_id,
        &CreateCooperationEvaluationRequest {
            cooperation_code: coop_code.clone(),
            qualification_code: "QUAL-NOT-EXIST".into(),
            remark: None,
        },
        1,
    )
    .await;
    assert!(bad_level.is_err(), "未知资质等级应 fail-closed");
    let bad_subject = create(
        &pool,
        987_654_321,
        &CreateCooperationEvaluationRequest {
            cooperation_code: coop_code.clone(),
            qualification_code: qual_b.clone(),
            remark: None,
        },
        1,
    )
    .await;
    assert!(bad_subject.is_err(), "未知主体应 fail-closed");

    // 清理：桥行（含软删）+ fixture 字典 + 主体
    for sql in [
        format!(
            r#"DELETE FROM isahl."zc_id_relation-cooperation_r_evaluation" WHERE ref_left = {subject_id}"#
        ),
        format!(r#"DELETE FROM isahl."zc_id_cate-cooperation" WHERE code = '{coop_code}'"#),
        format!(
            r#"DELETE FROM isahl."zc_id_leve-qualification" WHERE code IN ('{qual_a}', '{qual_b}')"#
        ),
        format!(r#"DELETE FROM isahl."zc_id_empl-natural" WHERE id = {subject_id}"#),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .execute(&pool)
            .await
            .expect("cleanup");
    }
}
