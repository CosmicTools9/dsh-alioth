//! 供应商合作评价（挂合作关系桥）集成测试
//! （add-supplier-qualification-certificates：评估 ref_left = 桥行 id，MUST NOT 直挂主体）
//!
//! 直驱核心函数（handler 为薄壳）：找或建桥（幂等）→ 评估落桥 → 读谓词桥两跳；
//! 调级软删替换（历史保留）；同等级唯一键预检；证照挂载（已批/未批/异属）。
//!
//! fixture 自持（主体对 empl-natural 叶 + 类型/等级字典行 + 证照叶行 + 状态桥，
//! code 前缀清理），不依赖种子执行状态。

use identity_org::handlers::cooperation_evaluations::{
    create_anchored, current, CreateCooperationEvaluationRequest,
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
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}", nanos % 0xF_FFFF_FFFF)
}

/// 主体 fixture（empl-natural 叶，dk 派生形态）。
async fn fixture_subject(pool: &PgPool, code: String, notice: &str) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_empl-natural"
           (id, code, notice, dk_scene, dk_factor, dk_function, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2,
                   (SELECT id FROM isahl.zc_id_scene LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code LIKE '↓\_%' LIMIT 1),
                   1) RETURNING id"#,
    )
    .bind(code)
    .bind(notice)
    .fetch_one(pool)
    .await
    .expect("主体 fixture")
}

#[tokio::test]
async fn evaluation_lifecycle_bridge_anchor_and_read_predicate() {
    let pool = test_pool().await;
    let sfx = suffix();

    // fixture：评估方主体 + 被评估主体 + 合作类型 + 资质等级
    let anchor_org = fixture_subject(&pool, format!("T-COOP-ANCHOR-{sfx}"), "评估方主体").await;
    let subject_id = fixture_subject(&pool, format!("T-COOP-SUBJ-{sfx}"), "合作评价测试主体").await;

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

    // 登记：初始评价 QUAL-A（挂桥）
    let eval_a = create_anchored(
        &pool,
        subject_id,
        anchor_org,
        &CreateCooperationEvaluationRequest {
            cooperation_code: coop_code.clone(),
            qualification_code: qual_a.clone(),
            remark: Some("首评".into()),
            certificate_ids: None,
        },
        1,
    )
    .await
    .expect("登记评价 A");
    assert_eq!(eval_a.cooperation_name, "测试运输合作");
    assert_eq!(eval_a.qualification_name, "优质");
    assert!((eval_a.level_value - 4.0).abs() < 0.01);

    // 挂桥断言：评估行 ref_left = 桥行 id（MUST NOT 直挂主体）；
    // 桥 = (评估方, 被评估主体)，qk_period NULL
    let (bridge_ref_left, bridge_left, bridge_right): (i64, i64, i64) = sqlx::query_as(
        r#"SELECT e.ref_left, b.ref_left, b.ref_right
           FROM isahl."zc_id_relation-cooperation_r_evaluation" e
           JOIN isahl."zc_id_subjects_rr_partner" b ON b.id = e.ref_left
           WHERE e.id = $1"#,
    )
    .bind(eval_a.id)
    .fetch_one(&pool)
    .await
    .expect("挂桥断言");
    assert_eq!(bridge_left, anchor_org, "桥 ref_left = 评估方主体");
    assert_eq!(bridge_right, subject_id, "桥 ref_right = 被评估主体");
    assert_ne!(
        bridge_ref_left, subject_id,
        "评估行 MUST NOT 直挂被评估主体"
    );

    // 读谓词：被评估主体侧两跳取当前 1 条（JOIN 解析齐）
    let list = current(&pool, subject_id)
        .await
        .expect("读谓词（被评估侧）");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].qualification_code, qual_a);
    // 评估方主体侧同可读（桥任一端）
    let anchor_side = current(&pool, anchor_org)
        .await
        .expect("读谓词（评估方侧）");
    assert_eq!(anchor_side.len(), 1, "桥任一端可读同合作关系评价");
    assert_eq!(anchor_side[0].id, eval_a.id);

    // 二次登记（同类型调级）：复用既有桥（不重复建桥）+ 旧行软删（历史保留）
    let bridges_before: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl."zc_id_subjects_rr_partner"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(anchor_org)
    .bind(subject_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bridges_before, 1);

    let eval_b = create_anchored(
        &pool,
        subject_id,
        anchor_org,
        &CreateCooperationEvaluationRequest {
            cooperation_code: coop_code.clone(),
            qualification_code: qual_b.clone(),
            remark: None,
            certificate_ids: None,
        },
        1,
    )
    .await
    .expect("调级 B");
    assert_ne!(eval_b.id, eval_a.id, "调级应落新评估行");

    let bridges_after: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl."zc_id_subjects_rr_partner"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(anchor_org)
    .bind(subject_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bridges_after, 1, "同主体对复用既有桥（幂等不重建）");

    let list = current(&pool, subject_id).await.expect("读谓词 2");
    assert_eq!(list.len(), 1, "同合作类型仅当前等级可见");
    assert_eq!(list[0].qualification_code, qual_b);
    let soft_deleted: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl."zc_id_relation-cooperation_r_evaluation"
           WHERE ref_left = (SELECT id FROM isahl."zc_id_subjects_rr_partner"
                              WHERE ref_left = $1 AND ref_right = $2
                                AND qk_period IS NULL AND deleted_at IS NULL)
             AND deleted_at IS NOT NULL"#,
    )
    .bind(anchor_org)
    .bind(subject_id)
    .fetch_one(&pool)
    .await
    .expect("历史行数");
    assert_eq!(soft_deleted, 1, "调级历史行软删保留");

    // 同等级唯一键预检：异合作类型复用同等级 → 显式 400（不裸撞唯一键）
    let coop_code2 = format!("COOP-T2-{sfx}");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_cate-cooperation" (notice, code, created_by_id)
           VALUES ('测试仓储合作', $1, 1)"#,
    )
    .bind(&coop_code2)
    .execute(&pool)
    .await
    .expect("类型2 fixture");
    let clash = create_anchored(
        &pool,
        subject_id,
        anchor_org,
        &CreateCooperationEvaluationRequest {
            cooperation_code: coop_code2,
            qualification_code: qual_b.clone(),
            remark: None,
            certificate_ids: None,
        },
        1,
    )
    .await;
    assert!(clash.is_err(), "同桥异类型复用同等级应显式拒绝");

    // 自指拒绝：评估方 = 被评估主体
    let self_ref = create_anchored(
        &pool,
        subject_id,
        subject_id,
        &CreateCooperationEvaluationRequest {
            cooperation_code: coop_code.clone(),
            qualification_code: qual_b.clone(),
            remark: None,
            certificate_ids: None,
        },
        1,
    )
    .await;
    assert!(self_ref.is_err(), "合作关系桥禁止自指");

    // fail-closed：未知类型 / 未知等级 / 未知主体
    let bad_coop = create_anchored(
        &pool,
        subject_id,
        anchor_org,
        &CreateCooperationEvaluationRequest {
            cooperation_code: "COOP-NOT-EXIST".into(),
            qualification_code: qual_b.clone(),
            remark: None,
            certificate_ids: None,
        },
        1,
    )
    .await;
    assert!(bad_coop.is_err(), "未知合作类型应 fail-closed");
    let bad_level = create_anchored(
        &pool,
        subject_id,
        anchor_org,
        &CreateCooperationEvaluationRequest {
            cooperation_code: coop_code.clone(),
            qualification_code: "QUAL-NOT-EXIST".into(),
            remark: None,
            certificate_ids: None,
        },
        1,
    )
    .await;
    assert!(bad_level.is_err(), "未知资质等级应 fail-closed");
    let bad_subject = create_anchored(
        &pool,
        987_654_321,
        anchor_org,
        &CreateCooperationEvaluationRequest {
            cooperation_code: coop_code.clone(),
            qualification_code: qual_b.clone(),
            remark: None,
            certificate_ids: None,
        },
        1,
    )
    .await;
    assert!(bad_subject.is_err(), "未知主体应 fail-closed");

    // 清理：评估行 + 桥行 + fixture 字典 + 主体对
    for sql in [
        format!(
            r#"DELETE FROM isahl."zc_id_relation-cooperation_r_evaluation" WHERE ref_left IN
               (SELECT id FROM isahl."zc_id_subjects_rr_partner" WHERE ref_left = {anchor_org} AND ref_right = {subject_id})"#
        ),
        format!(
            r#"DELETE FROM isahl."zc_id_subjects_rr_partner" WHERE ref_left = {anchor_org} AND ref_right = {subject_id}"#
        ),
        format!(
            r#"DELETE FROM isahl."zc_id_cate-cooperation" WHERE code IN ('{coop_code}', 'COOP-T2-{sfx}')"#
        ),
        format!(
            r#"DELETE FROM isahl."zc_id_leve-qualification" WHERE code IN ('{qual_a}', '{qual_b}')"#
        ),
        format!(
            r#"DELETE FROM isahl."zc_id_empl-natural" WHERE id IN ({anchor_org}, {subject_id})"#
        ),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .execute(&pool)
            .await
            .expect("cleanup");
    }
}

/// 证照挂载面：已批（cert-state-approved + 属主匹配）可挂；未批/异属拒绝。
/// 依赖评估表 `ak_attachment` 列——测试库模型快照落后时打印说明并跳过（重建即恢复全断言）。
#[tokio::test]
async fn certificate_mounting_requires_approved_and_owned_certs() {
    let pool = test_pool().await;
    let has_column: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (SELECT 1 FROM information_schema.columns
            WHERE table_schema = 'isahl'
              AND table_name = 'zc_id_relation-cooperation_r_evaluation'
              AND column_name = 'ak_attachment')"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    if !has_column {
        eprintln!(
            "SKIP: 测试库评估表缺 ak_attachment 列（模型快照落后，重建测试库后本用例恢复全断言）"
        );
        return;
    }

    let sfx = suffix();
    let anchor_org = fixture_subject(&pool, format!("T-CERT-ANCHOR-{sfx}"), "评估方主体").await;
    let subject_id = fixture_subject(&pool, format!("T-CERT-SUBJ-{sfx}"), "证照挂载主体").await;
    let other_org = fixture_subject(&pool, format!("T-CERT-OTHER-{sfx}"), "异属主体").await;

    let coop_code = format!("COOP-CT-{sfx}");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_cate-cooperation" (notice, code, created_by_id)
           VALUES ('测试运输合作', $1, 1)"#,
    )
    .bind(&coop_code)
    .execute(&pool)
    .await
    .expect("类型 fixture");
    let qual = format!("QUAL-CT-{sfx}");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_leve-qualification" (notice, code, lv_value, created_by_id)
           VALUES ('合格', $1, 3::numeric, 1)"#,
    )
    .bind(&qual)
    .execute(&pool)
    .await
    .expect("等级 fixture");

    // 证照 fixture：证照叶行（type_cert-sales；provider=被评估主体）+ 状态桥
    let approved_cert: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_prod-type_cert-sales"
           (notice, o_number, "fk_subj-provider", dk_scene, dk_factor, dk_function, created_by_id)
           VALUES ('测试营业执照', 'T-CERT-NO-1', $1,
                   (SELECT id FROM isahl.zc_id_scene LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code LIKE '↓\_%' LIMIT 1),
                   1) RETURNING id"#,
    )
    .bind(subject_id)
    .fetch_one(&pool)
    .await
    .expect("已批证照 fixture");
    let unapproved_cert: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_prod-type_cert-sales"
           (notice, o_number, "fk_subj-provider", dk_scene, dk_factor, dk_function, created_by_id)
           VALUES ('测试未批证照', 'T-CERT-NO-2', $1,
                   (SELECT id FROM isahl.zc_id_scene LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code LIKE '↓\_%' LIMIT 1),
                   1) RETURNING id"#,
    )
    .bind(subject_id)
    .fetch_one(&pool)
    .await
    .expect("未批证照 fixture");
    let foreign_cert: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_prod-type_cert-sales"
           (notice, o_number, "fk_subj-provider", dk_scene, dk_factor, dk_function, created_by_id)
           VALUES ('异属证照', 'T-CERT-NO-3', $1,
                   (SELECT id FROM isahl.zc_id_scene LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code LIKE '↓\_%' LIMIT 1),
                   1) RETURNING id"#,
    )
    .bind(other_org)
    .fetch_one(&pool)
    .await
    .expect("异属证照 fixture");

    let approved_status: i64 = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_stus-certification"
           WHERE code = 'cert-state-approved' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_one(&pool)
    .await
    .expect("状态字典行");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status"
           (id, ref_left, ref_right, status_date, code, created_by_id, updated_by_id)
           VALUES (isahl.gen_next_uid(260), $1, $2, NOW(), 'cert-state-approved', 1, 1)"#,
    )
    .bind(approved_cert)
    .bind(approved_status)
    .execute(&pool)
    .await
    .expect("状态桥 fixture");

    // 未批证照 → 拒绝
    let bad = create_anchored(
        &pool,
        subject_id,
        anchor_org,
        &CreateCooperationEvaluationRequest {
            cooperation_code: coop_code.clone(),
            qualification_code: qual.clone(),
            remark: None,
            certificate_ids: Some(vec![unapproved_cert]),
        },
        1,
    )
    .await;
    assert!(
        bad.is_err(),
        "未批证照（无 cert-state-approved 状态桥）应拒绝"
    );

    // 异属证照 → 拒绝
    let bad2 = create_anchored(
        &pool,
        subject_id,
        anchor_org,
        &CreateCooperationEvaluationRequest {
            cooperation_code: coop_code.clone(),
            qualification_code: qual.clone(),
            remark: None,
            certificate_ids: Some(vec![foreign_cert]),
        },
        1,
    )
    .await;
    assert!(bad2.is_err(), "异属证照（provider 非被评估主体）应拒绝");

    // 已批 + 属主匹配 → 挂载成功；响应含证照摘要
    let ok = create_anchored(
        &pool,
        subject_id,
        anchor_org,
        &CreateCooperationEvaluationRequest {
            cooperation_code: coop_code.clone(),
            qualification_code: qual.clone(),
            remark: None,
            certificate_ids: Some(vec![approved_cert]),
        },
        1,
    )
    .await
    .expect("已批证照挂载");
    assert_eq!(ok.certificates.len(), 1);
    assert_eq!(ok.certificates[0].cert_no.as_deref(), Some("T-CERT-NO-1"));
    assert_eq!(ok.certificates[0].issuer.as_deref(), Some("测试营业执照"));

    // 读径回读证照
    let list = current(&pool, subject_id).await.expect("读谓词");
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].certificates.len(),
        1,
        "读径解析评估行 ak_attachment"
    );
    assert_eq!(list[0].certificates[0].id, approved_cert);

    // 清理
    for sql in [
        format!(
            r#"DELETE FROM isahl."zc_id_relation-cooperation_r_evaluation" WHERE ref_left IN
               (SELECT id FROM isahl."zc_id_subjects_rr_partner" WHERE ref_left = {anchor_org} AND ref_right = {subject_id})"#
        ),
        format!(
            r#"DELETE FROM isahl."zc_id_subjects_rr_partner" WHERE ref_left = {anchor_org} AND ref_right = {subject_id}"#
        ),
        format!(
            r#"DELETE FROM isahl."zc_id_lifecycle_r_primary-status" WHERE ref_left IN ({approved_cert})"#
        ),
        format!(
            r#"DELETE FROM isahl."zc_id_prod-type_cert-sales" WHERE id IN ({approved_cert}, {unapproved_cert}, {foreign_cert})"#
        ),
        format!(r#"DELETE FROM isahl."zc_id_cate-cooperation" WHERE code = '{coop_code}'"#),
        format!(r#"DELETE FROM isahl."zc_id_leve-qualification" WHERE code = '{qual}'"#),
        format!(
            r#"DELETE FROM isahl."zc_id_empl-natural" WHERE id IN ({anchor_org}, {subject_id}, {other_org})"#
        ),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .execute(&pool)
            .await
            .expect("cleanup");
    }
}
