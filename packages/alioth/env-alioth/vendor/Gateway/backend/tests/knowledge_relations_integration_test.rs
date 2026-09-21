//! 知识关系多跳端点回归护栏（change `extend-knowledge-graph-with-neighbor-derivation` C-3）
//!
//! 守门项来源：`openspec/specs/knowledge-graph-cypher/spec.md` 的 `bounded-multi-hop-retrieval`
//! （`depth ∈ [1,3]`、Cypher 优先 / SQL 递归降级同构、`via_chain`）。此前该守门项**无测试资产**
//! （`Gateway/backend/tests/**` 与源码测试模块均无 knowledge/relations/depth 用例）——本文件把
//! 它固化为可复跑基线：深度边界、缺省单跳逐字段兼容、多跳 `via_chain`、AGE 降级可用性。
//!
//! 数据自建自清（桥行 code 前缀 `KM-T-MH-`）：只插入/删除本文件创建的桥行，不动种子行。

use actix_web::{test, web, App};
use alioth_gateway::api::knowledge::configure_routes;
use common::testing::connect_test_db;
use serde_json::{json, Value};
use sqlx::PgPool;

const BASE: &str = "/knowledge";
const CODE_CONTRACT_BRIDGE: &str = "KM-T-MH-REL";
const CODE_REFERENCE_BRIDGE: &str = "KM-T-MH-REF";

/// 测试用合同 id（端点以 id 为锚，不要求合同行存在——投影节点集由桥左端补入）
const CONTRACT_ID: i64 = 999_000_000_000_001;

/// 两跳链：合同 --REL--> A（法规条文）--BRIDGE_REFERENCE--> B（适航条文）
async fn seed_chain(pool: &PgPool) -> (i64, i64) {
    let a: i64 = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_law-common-section" WHERE deleted_at IS NULL ORDER BY id LIMIT 1"#,
    )
    .fetch_one(pool)
    .await
    .expect("test 库须有 common-section 种子行");
    let b: i64 = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_stan-air-caac-article" WHERE deleted_at IS NULL ORDER BY id LIMIT 1"#,
    )
    .fetch_one(pool)
    .await
    .expect("test 库须有 caac 条文种子行");

    sqlx::query(
        r#"INSERT INTO isahl."zc_id_contract_rr_law" (code, ref_left, ref_right)
           VALUES ($1, $2, $3)"#,
    )
    .bind(CODE_CONTRACT_BRIDGE)
    .bind(CONTRACT_ID)
    .bind(a)
    .execute(pool)
    .await
    .expect("插入合同↔法条桥失败");

    sqlx::query(
        r#"INSERT INTO isahl."zc_id_standard_rr_reference" (code, ref_left, ref_right)
           VALUES ($1, $2, $3)"#,
    )
    .bind(CODE_REFERENCE_BRIDGE)
    .bind(a)
    .bind(b)
    .execute(pool)
    .await
    .expect("插入标准引用桥失败");

    (a, b)
}

async fn cleanup(pool: &PgPool) {
    let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_contract_rr_law" WHERE code = $1"#)
        .bind(CODE_CONTRACT_BRIDGE)
        .execute(pool)
        .await;
    let _ = sqlx::query(r#"DELETE FROM isahl."zc_id_standard_rr_reference" WHERE code = $1"#)
        .bind(CODE_REFERENCE_BRIDGE)
        .execute(pool)
        .await;
}

async fn send(pool: &PgPool, body: Value) -> (u16, Value) {
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .configure(configure_routes),
    )
    .await;
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("{BASE}/relations"))
            .set_json(body)
            .to_request(),
    )
    .await;
    let status = resp.status().as_u16();
    let value = test::read_body_json(resp).await;
    (status, value)
}

fn hit_ids(body: &Value) -> Vec<String> {
    body["hits"]
        .as_array()
        .expect("hits 数组")
        .iter()
        .map(|h| h["id"].as_str().expect("id 字符串化").to_string())
        .collect()
}

#[actix_web::test]
async fn relations_depth_out_of_range_rejected() {
    let pool = connect_test_db().await;
    for depth in [0_i64, 4_i64] {
        let (status, body) = send(
            &pool,
            json!({"entity": "contract", "entity_id": CONTRACT_ID.to_string(), "depth": depth}),
        )
        .await;
        assert_eq!(status, 400, "depth={depth} 越界必须 400，实际 {body}");
    }
}

#[actix_web::test]
async fn relations_default_equals_single_hop_and_multihop_carries_via_chain() {
    let pool = connect_test_db().await;
    cleanup(&pool).await;
    let (a, b) = seed_chain(&pool).await;

    let (status_default, default_body) = send(
        &pool,
        json!({"entity": "contract", "entity_id": CONTRACT_ID.to_string()}),
    )
    .await;
    assert_eq!(status_default, 200, "缺省请求必须可达：{default_body}");
    let (status_one, one_body) = send(
        &pool,
        json!({"entity": "contract", "entity_id": CONTRACT_ID.to_string(), "depth": 1}),
    )
    .await;
    assert_eq!(status_one, 200);
    assert_eq!(
        default_body, one_body,
        "缺省行为（注入器调用形态）必须与 depth=1 逐字段一致"
    );

    // 单跳：仅命中 A（via_chain 缺省——单跳不附路径）
    let one_ids = hit_ids(&one_body);
    assert_eq!(one_ids, vec![a.to_string()], "单跳必须命中 REL 目标");
    assert!(
        one_body["hits"][0].get("via_chain").is_none(),
        "单跳不得下发 via_chain（既有形态兼容）"
    );

    // 多跳：命中 A 与 B（B 带 via_chain = [contract, A.origin, B.origin]）
    let (status_two, two_body) = send(
        &pool,
        json!({"entity": "contract", "entity_id": CONTRACT_ID.to_string(), "depth": 2}),
    )
    .await;
    assert_eq!(status_two, 200, "depth=2 必须可达：{two_body}");
    let two_ids = hit_ids(&two_body);
    assert!(
        two_ids.contains(&a.to_string()) && two_ids.contains(&b.to_string()),
        "两跳链必须命中 A 与 B，实际 {two_ids:?}"
    );
    let b_hit = two_body["hits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["id"].as_str().unwrap() == b.to_string())
        .expect("两跳终点命中");
    let via = b_hit["via_chain"]
        .as_array()
        .expect("多跳命中必须携带 via_chain");
    assert_eq!(via.len(), 3, "两跳路径 via_chain = [起点, 中继, 终点]");
    assert_eq!(via[0], "zc_id_contract");
    assert_eq!(via[1], "zc_id_law-common-section");
    assert_eq!(via[2], "zc_id_stan-air-caac-article");

    cleanup(&pool).await;
}

#[actix_web::test]
async fn relations_unknown_entity_and_invalid_id_rejected() {
    let pool = connect_test_db().await;
    let (status, _) = send(&pool, json!({"entity": "nope", "entity_id": "1"})).await;
    assert_eq!(status, 400, "未登记实体必须 400");
    let (status, _) = send(&pool, json!({"entity": "contract", "entity_id": "abc"})).await;
    assert_eq!(status, 400, "非数值实体 id 必须 400");
}
