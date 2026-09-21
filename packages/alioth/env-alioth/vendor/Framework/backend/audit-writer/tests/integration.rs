//! 集成测试：审计项目全生命周期（主档 → 受审主体多挂幂等 → 结论闭环）。
//! 惯例：`common::testing::connect_test_db` + `#[tokio::test]`（TEST_INFRASTRUCTURE）；
//! 夹具 `ABT-{nanos}-{seq}` 原子序号防并行碰撞，定点清理。

use std::sync::atomic::{AtomicU64, Ordering};

use audit_writer::{
    attach_auditee_tx, insert_audit_project_tx, set_conclusion_tx, AuditProjectInput,
};
use common::testing::connect_test_db;
use ontology_binding::resolve;
use sqlx::PgPool;

static TAG_SEQ: AtomicU64 = AtomicU64::new(0);

fn tag() -> String {
    let seq = TAG_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("ABT-{nanos}-{seq}")
}

/// 受审主体夹具（带 dk 派生源——叶表铁律）。
async fn seed_subject(pool: &PgPool, code: &str) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_subjects (id, notice, code, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), $1, $2,
                   (SELECT id FROM isahl.zc_id_scene WHERE code = 'TX' AND deleted_at IS NULL LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor WHERE code = 'FJA' AND deleted_at IS NULL LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↓_GG' AND deleted_at IS NULL LIMIT 1))
           RETURNING id"#,
    )
    .bind(format!("审计测试主体-{code}"))
    .bind(code)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 审计项目坐标（首版占位 JC/FTA/↑_NA——管理·审批处理·管理职能，见 change design）。
async fn audit_coords(pool: &PgPool) -> (i64, i64, i64) {
    let (s, f, c) = resolve(pool, ("JC", "FTA", "↑_NA")).await.unwrap();
    (s.unwrap(), f.unwrap(), c.unwrap())
}

async fn cleanup(pool: &PgPool, t: &str) {
    for sql in [
        r#"DELETE FROM isahl."zc_id_audit_rr_auditee" WHERE code LIKE 'AAB-' || $1 || '%' OR ref_left IN (SELECT id FROM isahl."zc_id_audit" WHERE code LIKE $1 || '%')"#,
        r#"DELETE FROM isahl."zc_id_audit_rr_conclusion" WHERE ref_left IN (SELECT id FROM isahl."zc_id_audit" WHERE code LIKE $1 || '%')"#,
        r#"DELETE FROM isahl."zc_id_audit" WHERE code LIKE $1 || '%'"#,
        r#"DELETE FROM isahl.zc_id_subjects WHERE code LIKE 'SUBJ-' || $1 || '%'"#,
    ] {
        let _ = sqlx::query(sql).bind(t).execute(pool).await;
    }
}

#[tokio::test]
async fn audit_project_lifecycle_auditees_idempotent_conclusion_single() {
    let pool = connect_test_db().await;
    let t = tag();
    let (dk_s, dk_f, dk_fn) = audit_coords(&pool).await;
    let subj1 = seed_subject(&pool, &format!("SUBJ-{t}-1")).await;
    let subj2 = seed_subject(&pool, &format!("SUBJ-{t}-2")).await;

    // 主档（id 省略——列默认 gen_next_zuid）
    let input = AuditProjectInput {
        code: format!("{t}-1"),
        notice: "测试审计项目（会计审计）".to_string(),
        comments: Some("纯文本摘要".into()),
        fk_launcher: Some(subj1),
        ak_source: None,
        dk_scene: dk_s,
        dk_factor: dk_f,
        dk_function: dk_fn,
    };
    let mut tx = pool.begin().await.unwrap();
    let audit_id = insert_audit_project_tx(&mut tx, &input, 1).await.unwrap();
    tx.commit().await.unwrap();

    let (code, launcher): (String, Option<i64>) =
        sqlx::query_as(r#"SELECT code, fk_launcher FROM isahl."zc_id_audit" WHERE id = $1"#)
            .bind(audit_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(code, format!("{t}-1"));
    assert_eq!(launcher, Some(subj1), "发起人落 fk_launcher");

    // 受审主体多挂 + 幂等（重复挂同一主体 → 仍 1 行）
    let mut tx = pool.begin().await.unwrap();
    attach_auditee_tx(&mut tx, audit_id, subj1, 1)
        .await
        .unwrap();
    attach_auditee_tx(&mut tx, audit_id, subj1, 1)
        .await
        .unwrap();
    attach_auditee_tx(&mut tx, audit_id, subj2, 1)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let bridges: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl."zc_id_audit_rr_auditee"
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(audit_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bridges, 2, "两主体各一行（同主体重复挂接幂等）");

    // 结论闭环：单结论位（ref_left 唯一持有），重复设结论零副作用
    let conclusion_id = 9_000_000_000_000_001i64; // prod-conclusion 行 id 由调用方解析；测试用占位指针
    let mut tx = pool.begin().await.unwrap();
    set_conclusion_tx(&mut tx, audit_id, conclusion_id, 1)
        .await
        .unwrap();
    set_conclusion_tx(&mut tx, audit_id, conclusion_id, 1)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let conclusions: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl."zc_id_audit_rr_conclusion"
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(audit_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(conclusions, 1, "结论桥 NOT EXISTS 幂等");

    cleanup(&pool, &t).await;
}

#[tokio::test]
async fn oper_rows_confirm_bill_and_smtv_review_land_with_coords() {
    let pool = connect_test_db().await;
    let t = tag();

    // 账单确认（核销留痕；D5——调用方在核销事务边界之外落行）
    let mut conn = pool.acquire().await.unwrap();
    let confirm_id = audit_writer::insert_confirm_bill_tx(
        &mut conn,
        &audit_writer::OperationRowInput {
            code: format!("{t}-CFB"),
            notice: "付款核销确认".to_string(),
            comments: Some("来源核销单 BIL-TEST".to_string()),
            fk_subject: None,
            fk_operator: Some(1),
        },
        1,
    )
    .await
    .unwrap();
    drop(conn);

    let (code, has_coords): (String, bool) = sqlx::query_as(
        r#"SELECT code, (dk_scene IS NOT NULL AND dk_factor IS NOT NULL AND dk_function IS NOT NULL)
           FROM isahl."zc_id_oper-confirm_bill" WHERE id = $1"#,
    )
    .bind(confirm_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(code, format!("{t}-CFB"));
    assert!(has_coords, "操作行坐标经 resolve_conn 落库（JC/FTA/↓_EZ）");

    // 结算复盘 + 读面回显
    let mut conn = pool.acquire().await.unwrap();
    let review_id = audit_writer::insert_smtv_review_tx(
        &mut conn,
        &audit_writer::OperationRowInput {
            code: format!("{t}-SRV"),
            notice: "月度结算复盘".to_string(),
            comments: None,
            fk_subject: None,
            fk_operator: Some(1),
        },
        1,
    )
    .await
    .unwrap();
    drop(conn);

    let reviews = audit_writer::read::list_smtv_reviews(&pool).await.unwrap();
    let hit = reviews
        .iter()
        .find(|v| v.get("code").and_then(|c| c.as_str()) == Some(format!("{t}-SRV").as_str()));
    let row = hit.expect("复盘读面应含新登记行");
    assert_eq!(row["id"].as_str(), Some(review_id.to_string().as_str()));

    let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_oper-confirm_bill" WHERE id = $1"#)
        .bind(confirm_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_oper-smtv_review" WHERE id = $1"#)
        .bind(review_id)
        .execute(&pool)
        .await;
}

#[tokio::test]
async fn supervision_bridge_links_even_approve_idempotent() {
    let pool = connect_test_db().await;
    let t = tag();
    let (dk_s, dk_f, dk_fn) = audit_coords(&pool).await;

    // 审计项目 + 模拟 even-approve 行 id（桥接面只依赖指针，不需要真实 even-approve 行）
    let input = AuditProjectInput {
        code: format!("{t}-1"),
        notice: "桥接测试审计项目".to_string(),
        fk_launcher: None,
        dk_scene: dk_s,
        dk_factor: dk_f,
        dk_function: dk_fn,
        ..Default::default()
    };
    let mut tx = pool.begin().await.unwrap();
    let audit_id = insert_audit_project_tx(&mut tx, &input, 1).await.unwrap();
    tx.commit().await.unwrap();
    let approve_row_id = 9_000_000_000_000_777i64;

    let mut tx = pool.begin().await.unwrap();
    let first = audit_writer::link_supervision_audit_tx(&mut tx, audit_id, approve_row_id, 1)
        .await
        .unwrap();
    let second = audit_writer::link_supervision_audit_tx(&mut tx, audit_id, approve_row_id, 1)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(first > 0, "首次桥接落操作行");
    assert_eq!(second, 0, "重复桥接幂等 no-op");

    // 双侧指针：操作行 ak_source = [approve_row]；审计项目 ak_source 含同指针
    let oper_src: Option<Vec<i64>> =
        sqlx::query_scalar(r#"SELECT ak_source FROM isahl."zc_id_oper-audit_prj" WHERE id = $1"#)
            .bind(first)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(oper_src, Some(vec![approve_row_id]));
    let audit_src: Option<Vec<i64>> =
        sqlx::query_scalar(r#"SELECT ak_source FROM isahl."zc_id_audit" WHERE id = $1"#)
            .bind(audit_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(audit_src.unwrap_or_default().contains(&approve_row_id));

    let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_oper-audit_prj" WHERE id = $1"#)
        .bind(first)
        .execute(&pool)
        .await;
    let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_audit" WHERE id = $1"#)
        .bind(audit_id)
        .execute(&pool)
        .await;
}
