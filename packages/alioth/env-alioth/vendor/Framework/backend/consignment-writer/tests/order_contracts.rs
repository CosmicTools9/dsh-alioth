//! consignment-writer 下单合同对集成测试（真库：`common::testing::connect_test_db` → `*_test` 库）。
//!
//! 覆盖 `insert_order_contracts_tx`：诉求合同（`zc_id_cont-request` 主 + **同表**镜像）+
//! 销售合同（`zc_id_cont-sales` 主 + 相反叶表 `zc_id_cont-purchase` 镜像）+
//! `zc_id_order_rr_contract` 桥四行（`ref_left` = 订单）。

use sqlx::PgPool;

use common::testing::connect_test_db;
use consignment_writer::{insert_order_contracts_tx, OrderContractsInput};

/// 测试订单 id（桥为叶表行，不依赖真实委托行存在）。
const ORDER_ID: i64 = 9_000_100_001;

fn unique_consign_code() -> String {
    format!(
        "T-OC-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_millis()
    )
}

/// 测试主体（幂等：按 code 判重）。落**叶** `zc_id_orga-non-banking-legal`（法律主体），
/// 并同批绑定坐标三元组 TX/FJA/↓_GG（§6.12；`zc_id_subj-org` 为父表，禁直写）。
async fn ensure_subject(pool: &PgPool, code: &str) -> i64 {
    let id: Option<i64> = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_orga-non-banking-legal" (notice, code, created_by_id, updated_by_id, dk_scene, dk_factor, dk_function)
           SELECT $1, $2, 1, 1,
                  (SELECT id FROM isahl."zc_id_scene"    WHERE code = 'TX'   AND deleted_at IS NULL LIMIT 1),
                  (SELECT id FROM isahl."zc_id_factor"   WHERE code = 'FJA'  AND deleted_at IS NULL LIMIT 1),
                  (SELECT id FROM isahl."zc_id_function" WHERE code = '↓_GG' AND deleted_at IS NULL LIMIT 1)
           WHERE NOT EXISTS (SELECT 1 FROM isahl."zc_id_orga-non-banking-legal" WHERE code = $2 AND deleted_at IS NULL)
           RETURNING id"#,
    )
    .bind(format!("下单合同测试主体-{code}"))
    .bind(code)
    .fetch_optional(pool)
    .await
    .expect("insert subject");
    match id {
        Some(id) => id,
        None => sqlx::query_scalar(
            r#"SELECT id FROM isahl."zc_id_subj-org" WHERE code = $1 AND deleted_at IS NULL"#,
        )
        .bind(code)
        .fetch_one(pool)
        .await
        .expect("fetch subject"),
    }
}

#[tokio::test]
async fn order_contracts_generate_request_and_sales_pairs_with_bridge() {
    let pool = connect_test_db().await;
    // 漂移自检（缺诉求叶表时自解释失败，不静默跳过）：
    let has_request_leaf: bool =
        sqlx::query_scalar(r#"SELECT to_regclass('"isahl"."zc_id_cont-request"') IS NOT NULL"#)
            .fetch_one(&pool)
            .await
            .expect("regclass probe");
    assert!(
        has_request_leaf,
        "isahl.zc_id_cont-request 缺失 = 参考测试库 schema 漂移（ns 库均有该叶表）——\
         本用例须在已同步库运行；解除条件见 change 归档 tasks §6.4/6.5，非代码缺陷"
    );
    let consign_code = unique_consign_code();
    let buyer = ensure_subject(&pool, "T-OC-SUBJ-A").await;
    let seller = ensure_subject(&pool, "T-OC-SUBJ-B").await;
    let mut conn = pool.acquire().await.expect("acquire");

    // 前置清理：同 id 的旧桥残留（合同行随后按编号前缀清理）
    sqlx::query(r#"DELETE FROM isahl."zc_id_order_rr_contract" WHERE ref_left = $1"#)
        .bind(ORDER_ID)
        .execute(&pool)
        .await
        .expect("pre-clean bridge");

    let (request_id, request_mirror_id, sales_id, sales_mirror_id) = insert_order_contracts_tx(
        &mut conn,
        ORDER_ID,
        &OrderContractsInput {
            consign_code: &consign_code,
            buyer,
            seller,
            fn_code: "↓.GG",
            user_id: 1,
        },
    )
    .await
    .expect("order contracts");

    // 诉求合同：主 + 镜像落**同一叶表**（`zc_id_cont-request`），编号 `-REQ` / `-REQ-R`
    let request_rows: Vec<(i64, String)> = sqlx::query_as(
        r#"SELECT id, code FROM isahl."zc_id_cont-request" WHERE id = ANY($1) ORDER BY code"#,
    )
    .bind(vec![request_id, request_mirror_id])
    .fetch_all(&pool)
    .await
    .expect("request rows");
    assert_eq!(
        request_rows,
        vec![
            (request_id, format!("CT-{consign_code}-REQ")),
            (request_mirror_id, format!("CT-{consign_code}-REQ-R")),
        ],
        "诉求主/镜像须同叶表且编号成对"
    );

    // 销售合同：主在销售叶、镜像在**相反**叶表（采购）
    let sales_code: String =
        sqlx::query_scalar(r#"SELECT code FROM isahl."zc_id_cont-sales" WHERE id = $1"#)
            .bind(sales_id)
            .fetch_one(&pool)
            .await
            .expect("sales row");
    assert_eq!(sales_code, format!("CT-{consign_code}"));
    let sales_mirror_code: String =
        sqlx::query_scalar(r#"SELECT code FROM isahl."zc_id_cont-purchase" WHERE id = $1"#)
            .bind(sales_mirror_id)
            .fetch_one(&pool)
            .await
            .expect("sales mirror row");
    assert_eq!(sales_mirror_code, format!("CT-{consign_code}-R"));

    // 桥：订单 → 四张合同各一行（`ref_left` = 订单）
    let mut bridged: Vec<i64> = sqlx::query_scalar(
        r#"SELECT ref_right FROM isahl."zc_id_order_rr_contract"
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(ORDER_ID)
    .fetch_all(&pool)
    .await
    .expect("bridge rows");
    bridged.sort_unstable();
    let mut expected = vec![request_id, request_mirror_id, sales_id, sales_mirror_id];
    expected.sort_unstable();
    assert_eq!(bridged, expected, "四张合同须各挂一行订单↔合同桥");

    // 合同方：主/镜像 P1=客户 / P2=运营组织（甲/乙不互换——镜像是同一单据的对方账副本）
    let main_parties: Vec<Option<i64>> = sqlx::query_scalar(
        r#"SELECT ref_right FROM isahl."zc_id_contract_rr_party"
           WHERE ref_left = $1 AND deleted_at IS NULL ORDER BY code"#,
    )
    .bind(request_id)
    .fetch_all(&pool)
    .await
    .expect("request parties");
    assert_eq!(main_parties, vec![Some(buyer), Some(seller)]);
    let mirror_parties: Vec<Option<i64>> = sqlx::query_scalar(
        r#"SELECT ref_right FROM isahl."zc_id_contract_rr_party"
           WHERE ref_left = $1 AND deleted_at IS NULL ORDER BY code"#,
    )
    .bind(request_mirror_id)
    .fetch_all(&pool)
    .await
    .expect("request mirror parties");
    assert_eq!(mirror_parties, vec![Some(buyer), Some(seller)]);

    drop(conn);

    // 清理：桥 → MIR 桥/合同方 → 合同行（诉求 / 销售 / 采购）
    sqlx::query(r#"DELETE FROM isahl."zc_id_order_rr_contract" WHERE ref_left = $1"#)
        .bind(ORDER_ID)
        .execute(&pool)
        .await
        .expect("clean bridge");
    let like = format!("CT-{consign_code}%");
    for sql in [
        r#"DELETE FROM isahl."zc_id_contract_rr_symmetry"
           WHERE ref_left IN (SELECT id FROM isahl."zc_id_contract" WHERE code LIKE $1)"#,
        r#"DELETE FROM isahl."zc_id_contract_rr_party"
           WHERE ref_left IN (SELECT id FROM isahl."zc_id_contract" WHERE code LIKE $1)"#,
        r#"DELETE FROM isahl."zc_id_cont-request" WHERE code LIKE $1"#,
        r#"DELETE FROM isahl."zc_id_cont-sales" WHERE code LIKE $1"#,
        r#"DELETE FROM isahl."zc_id_cont-purchase" WHERE code LIKE $1"#,
    ] {
        sqlx::query(sql)
            .bind(&like)
            .execute(&pool)
            .await
            .expect("clean contracts");
    }
}
