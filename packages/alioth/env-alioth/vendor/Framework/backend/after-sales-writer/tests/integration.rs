//! 集成测试：售后订单子叶落库 / G3 合同校验负例 / 合同桥幂等 / 申诉来源指针。
//! 惯例：`common::testing::connect_test_db` + `#[tokio::test]`（TEST_INFRASTRUCTURE）；
//! 共享测试库，夹具以唯一 code 前缀 `AST-{nanos}-{seq}` 隔离（原子序号防并行纳秒碰撞）并定点清理。

use std::sync::atomic::{AtomicU64, Ordering};

use after_sales_writer::{
    bind_after_sales_contract_tx, insert_after_sales_order_tx, insert_appeal_tx,
    AfterSalesOrderInput, AppealInput,
};
use common::testing::connect_test_db;
use contract_writer::{insert_contract_row_tx, ContractLeaf, ContractParty, ContractRowInput};
use ontology_binding::resolve;
use sqlx::PgPool;

static TAG_SEQ: AtomicU64 = AtomicU64::new(0);

fn tag() -> String {
    let seq = TAG_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("AST-{nanos}-{seq}")
}

/// 确保 stus-contract 状态行存在（幂等），返回其 id。
async fn ensure_contract_status(pool: &PgPool, code: &str, notice: &str) -> i64 {
    let existing: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_stus-contract" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(code)
    .fetch_optional(pool)
    .await
    .unwrap()
    .flatten();
    if let Some(id) = existing {
        return id;
    }
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_stus-contract" (notice, code, flag)
           VALUES ($1, $2, 'doing') RETURNING id"#,
    )
    .bind(notice)
    .bind(code)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 造一张销售合同（actor=None 系统内部写），主状态 = `status_code`，含一方主体。
async fn seed_contract(pool: &PgPool, code: &str, status_code: &str) -> (i64, i64) {
    let subject_code = format!("SUBJ-{code}");
    let subject_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_subjects (id, notice, code, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), $1, $2,
                   (SELECT id FROM isahl.zc_id_scene WHERE code = 'TX' AND deleted_at IS NULL LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor WHERE code = 'FJA' AND deleted_at IS NULL LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↓_GG' AND deleted_at IS NULL LIMIT 1))
           RETURNING id"#,
    )
    .bind(format!("售后测试主体-{code}"))
    .bind(&subject_code)
    .fetch_one(pool)
    .await
    .unwrap();

    let status_id = ensure_contract_status(pool, status_code, status_code).await;
    let input = ContractRowInput {
        leaf: ContractLeaf::Sales,
        code,
        notice: "售后接线测试合同",
        comments: "",
        parties: vec![ContractParty {
            subject_id: Some(subject_id),
            name: format!("售后测试主体-{code}"),
            period_id: None,
        }],
        actor: None,
        require_view: None,
        fn_code: "↓_GG",
        scene_code: "TD",
        factor_code: "FJA",
        qk_date_id: None,
        qk_valid_segm_id: None,
        o_number: None,
        projection: None,
        tpl_id: None,
        lk_health: None,
        draft_status_id: Some(status_id),
        user_id: 1,
    };
    let mut tx = pool.begin().await.unwrap();
    let id = insert_contract_row_tx(&mut tx, &input).await.unwrap();
    tx.commit().await.unwrap();
    (id, subject_id)
}

/// 解析售后子叶坐标（TX/FJA/↓_BB：交易·单据·服务执行——首版占位口径，见 change design）。
async fn after_sales_coords(pool: &PgPool) -> (i64, i64, i64) {
    let (s, f, c) = resolve(pool, ("TX", "FJA", "↓_BB")).await.unwrap();
    (s.unwrap(), f.unwrap(), c.unwrap())
}

async fn cleanup(pool: &PgPool, t: &str) {
    for sql in [
        r#"DELETE FROM isahl."zc_id_order_rr_contract" WHERE code LIKE 'ORC-' || $1 || '%'"#,
        r#"DELETE FROM isahl."zc_id_order-after_sales" WHERE code LIKE $1 || '%'"#,
        r#"DELETE FROM isahl."zc_id_stat-appeal" WHERE code LIKE $1 || '%'"#,
        r#"DELETE FROM isahl."zc_id_contract_rr_party" WHERE ref_left IN (SELECT id FROM isahl.zc_id_contract WHERE code LIKE $1 || '%')"#,
        r#"DELETE FROM isahl."zc_id_lifecycle_r_primary-status" WHERE ref_left IN (SELECT id FROM isahl.zc_id_contract WHERE code LIKE $1 || '%')"#,
        r#"DELETE FROM isahl.zc_id_contract WHERE code LIKE $1 || '%'"#,
        r#"DELETE FROM isahl.zc_id_subjects WHERE code LIKE 'SUBJ-' || $1 || '%'"#,
    ] {
        let _ = sqlx::query(sql).bind(t).execute(pool).await;
    }
}

#[tokio::test]
async fn after_sales_order_lands_on_leaf_without_contract() {
    let pool = connect_test_db().await;
    let t = tag();
    let (dk_s, dk_f, dk_fn) = after_sales_coords(&pool).await;

    let input = AfterSalesOrderInput {
        code: format!("{t}-1"),
        notice: "测试售后单（无合同）".to_string(),
        comments: Some("纯文本摘要".into()),
        fk_subject: None,
        fk_object: None,
        qk_date: None,
        ck_category: None,
        fk_contract: None,
        ak_source: None,
        dk_scene: dk_s,
        dk_factor: dk_f,
        dk_function: dk_fn,
    };
    let mut tx = pool.begin().await.unwrap();
    let id = insert_after_sales_order_tx(&mut tx, &input, 1)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let (code, fk_contract): (String, Option<i64>) = sqlx::query_as(
        r#"SELECT code, fk_contract FROM isahl."zc_id_order-after_sales" WHERE id = $1"#,
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(code, format!("{t}-1"));
    assert!(fk_contract.is_none());

    let bridges: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl."zc_id_order_rr_contract" WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bridges, 0, "无合同售后单不应产生合同桥");

    cleanup(&pool, &t).await;
}

#[tokio::test]
async fn after_sales_order_rejects_draft_contract_zero_rows() {
    let pool = connect_test_db().await;
    let t = tag();
    let (dk_s, dk_f, dk_fn) = after_sales_coords(&pool).await;
    let (contract_id, subject_id) = seed_contract(&pool, &format!("{t}-CT"), "draft").await;

    let input = AfterSalesOrderInput {
        code: format!("{t}-2"),
        notice: "测试售后单（draft 合同应拒）".to_string(),
        fk_subject: Some(subject_id),
        fk_contract: Some(contract_id),
        dk_scene: dk_s,
        dk_factor: dk_f,
        dk_function: dk_fn,
        ..Default::default()
    };
    let mut tx = pool.begin().await.unwrap();
    let err = insert_after_sales_order_tx(&mut tx, &input, 1).await;
    tx.rollback().await.unwrap();
    assert!(err.is_err(), "draft 合同必须拒绝绑单（G3 口径）");

    let rows: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl."zc_id_order-after_sales" WHERE code = $1"#,
    )
    .bind(format!("{t}-2"))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 0, "拒绝路径零落库");

    cleanup(&pool, &t).await;
}

#[tokio::test]
async fn after_sales_order_binds_active_contract_and_junction_idempotent() {
    let pool = connect_test_db().await;
    let t = tag();
    let (dk_s, dk_f, dk_fn) = after_sales_coords(&pool).await;
    let (contract_id, subject_id) = seed_contract(&pool, &format!("{t}-CT"), "active").await;

    let input = AfterSalesOrderInput {
        code: format!("{t}-3"),
        notice: "测试售后单（active 合同双写）".to_string(),
        fk_subject: Some(subject_id),
        fk_contract: Some(contract_id),
        dk_scene: dk_s,
        dk_factor: dk_f,
        dk_function: dk_fn,
        ..Default::default()
    };
    let mut tx = pool.begin().await.unwrap();
    let id = insert_after_sales_order_tx(&mut tx, &input, 1)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let fk: Option<i64> = sqlx::query_scalar(
        r#"SELECT fk_contract FROM isahl."zc_id_order-after_sales" WHERE id = $1"#,
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(fk, Some(contract_id), "物理列双写口径（G5）");

    // 桥幂等：重复 bind 不产生第二行
    let mut tx = pool.begin().await.unwrap();
    bind_after_sales_contract_tx(&mut tx, id, &input.code, contract_id, 1)
        .await
        .unwrap();
    bind_after_sales_contract_tx(&mut tx, id, &input.code, contract_id, 1)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let bridges: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl."zc_id_order_rr_contract"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .bind(contract_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bridges, 1, "合同桥 NOT EXISTS 幂等");

    cleanup(&pool, &t).await;
}

#[tokio::test]
async fn appeal_carries_event_source_pointers() {
    let pool = connect_test_db().await;
    let t = tag();
    let (dk_s, dk_f, dk_fn) = after_sales_coords(&pool).await;

    let input = AppealInput {
        code: format!("{t}-AP"),
        notice: "货损转申诉".to_string(),
        comments: Some("纯文本摘要".into()),
        fk_subject: None,
        fk_object: None,
        qk_date: None,
        ak_source: Some(vec![101, 202]),
        dk_scene: dk_s,
        dk_factor: dk_f,
        dk_function: dk_fn,
    };
    let mut tx = pool.begin().await.unwrap();
    let id = insert_appeal_tx(&mut tx, &input, 1).await.unwrap();
    tx.commit().await.unwrap();

    let sources: Vec<i64> =
        sqlx::query_scalar(r#"SELECT ak_source FROM isahl."zc_id_stat-appeal" WHERE id = $1"#)
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(sources, vec![101, 202], "来源事件指针走 ak_source（D3）");

    cleanup(&pool, &t).await;
}
