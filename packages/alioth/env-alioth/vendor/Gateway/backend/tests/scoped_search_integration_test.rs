//! 作用域检索收敛护栏（change `consolidate-scoped-search-module`）
//!
//! 守门面 = 三个内容检索端点收敛为 `common::scoped_search` 适配器后的可观察契约：
//! 命中 / 未命中 / scope 过滤 / `level` 过滤 / 真 `total`（大于返回页）/ 三档排序（全等优先）。
//! 数据自建自清（fixture code 前缀 `KM-T-SS-`）：只插入/删除本文件创建的行；
//! `level` 过滤用库内 `zc_id_stan-operation` 种子行（期望值由库内实况在 Rust 侧同谓词计数）。

use actix_web::{test, web, App};
use alioth_gateway::api::legal_search::configure_routes as legal_routes;
use alioth_gateway::api::standard_search::configure_routes as standard_routes;
use common::testing::connect_test_db;
use serde_json::{json, Value};
use sqlx::PgPool;

const LEGAL_KW: &str = "KM-T-SS-LEGAL-KW";
const STD_KW: &str = "KM-T-SS-STD-KW";

/// legal fixtures：civil 4 条（首条 notice 与关键词全等）+ intl 1 条（含关键词）
const LEGAL_CIVIL: &[(&str, &str)] = &[
    ("KM-T-SS-LEGAL-EXACT", LEGAL_KW),
    ("KM-T-SS-LEGAL-C1", "前缀 KM-T-SS-LEGAL-KW 后缀"),
    ("KM-T-SS-LEGAL-C2", "另一处 KM-T-SS-LEGAL-KW"),
    ("KM-T-SS-LEGAL-C3", "注解 KM-T-SS-LEGAL-KW 尾部"),
];
const LEGAL_INTL: &[(&str, &str)] = &[("KM-T-SS-LEGAL-INTL", "国际 KM-T-SS-LEGAL-KW 条款")];

/// standard fixtures：fin-ifrs 4 条（首条 notice 与关键词全等）+ fin-gaap 1 条（含关键词）
const STD_IFRS: &[(&str, &str)] = &[
    ("KM-T-SS-STD-EXACT", STD_KW),
    ("KM-T-SS-STD-C1", "前缀 KM-T-SS-STD-KW 后缀"),
    ("KM-T-SS-STD-C2", "另一处 KM-T-SS-STD-KW"),
    ("KM-T-SS-STD-C3", "注解 KM-T-SS-STD-KW 尾部"),
];
const STD_GAAP: &[(&str, &str)] = &[("KM-T-SS-STD-GAAP", "GAAP KM-T-SS-STD-KW 条款")];

async fn send(pool: &PgPool, uri: &str, body: Value) -> (u16, Value) {
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .configure(legal_routes)
            .configure(standard_routes),
    )
    .await;
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(uri)
            .set_json(body)
            .to_request(),
    )
    .await;
    (resp.status().as_u16(), test::read_body_json(resp).await)
}

fn hits(body: &Value) -> &Vec<Value> {
    body["hits"].as_array().expect("hits 数组")
}

fn total(body: &Value) -> i64 {
    body["total"].as_i64().expect("total 数字")
}

async fn cleanup(pool: &PgPool) {
    for table in [
        "zc_id_law-civil-article",
        "zc_id_law-intl-article",
        "zc_id_stan-fin-ifrs-article",
        "zc_id_stan-fin-gaap-article",
    ] {
        let sql = format!(r#"DELETE FROM isahl."{table}" WHERE code LIKE 'KM-T-SS-%'"#);
        let _ = sqlx::query(sqlx::AssertSqlSafe(sql)).execute(pool).await;
    }
}

async fn seed(pool: &PgPool, table: &str, rows: &[(&str, &str)]) {
    for (code, notice) in rows {
        let sql =
            format!(r#"INSERT INTO isahl."{table}" (code, notice, comments) VALUES ($1, $2, $3)"#);
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(code)
            .bind(notice)
            .bind(format!("fixture {code}"))
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("插入 {table} fixture {code} 失败: {e}"));
    }
}

#[actix_web::test]
async fn legal_search_scoped_contract() {
    let pool = connect_test_db().await;
    cleanup(&pool).await;
    seed(&pool, "zc_id_law-civil-article", LEGAL_CIVIL).await;
    seed(&pool, "zc_id_law-intl-article", LEGAL_INTL).await;

    // 命中 + 真 total（4 > 返回页 2）+ 三档排序（全等排在包含前）
    let (status, body) = send(
        &pool,
        "/legal/search",
        json!({"keywords": [LEGAL_KW], "scopes": ["civil"], "max_results": 2}),
    )
    .await;
    assert_eq!(status, 200, "legal 检索应 200: {body}");
    assert_eq!(total(&body), 4, "total MUST 为真实命中总数（非截断条数）");
    assert_eq!(hits(&body).len(), 2, "返回页 = max_results");
    assert_eq!(
        hits(&body)[0]["article_title"],
        LEGAL_KW,
        "全等命中的行 MUST 排在包含命中之前"
    );
    let first = &hits(&body)[0];
    assert_eq!(first["source_table"], "zc_id_law-civil-article");
    assert!(first["article_id"].is_string(), "zuid 序列化为字符串");
    for key in [
        "article_code",
        "article_title",
        "article_body",
        "jurisdiction",
    ] {
        assert!(first.get(key).is_some(), "响应字段 {key} MUST 保留");
    }
    assert_eq!(first["article_code"], "KM-T-SS-LEGAL-EXACT");

    // scope 过滤：intl 仅命中该域自身（跨域不串结果）
    let (status, body) = send(
        &pool,
        "/legal/search",
        json!({"keywords": [LEGAL_KW], "scopes": ["intl"]}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(total(&body), 1);
    assert_eq!(hits(&body)[0]["source_table"], "zc_id_law-intl-article");

    // 未命中：total = 0 且 hits 为空
    let (status, body) = send(
        &pool,
        "/legal/search",
        json!({"keywords": ["KM-T-SS-LEGAL-ABSENT"], "scopes": ["civil", "intl"]}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(total(&body), 0);
    assert!(hits(&body).is_empty());

    // 未登记 scope 忽略（不报错、无结果）
    let (status, body) = send(
        &pool,
        "/legal/search",
        json!({"keywords": [LEGAL_KW], "scopes": ["document"]}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(total(&body), 0);

    cleanup(&pool).await;
}

#[actix_web::test]
async fn standard_search_scoped_contract() {
    let pool = connect_test_db().await;
    cleanup(&pool).await;
    seed(&pool, "zc_id_stan-fin-ifrs-article", STD_IFRS).await;
    seed(&pool, "zc_id_stan-fin-gaap-article", STD_GAAP).await;

    // 命中 + 真 total（4 > 返回页 2）+ 三档排序（全等优先）
    let (status, body) = send(
        &pool,
        "/standard/search",
        json!({"keywords": [STD_KW], "scopes": ["fin-ifrs"], "max_results": 2}),
    )
    .await;
    assert_eq!(status, 200, "standard 检索应 200: {body}");
    assert_eq!(total(&body), 4, "total MUST 为真实命中总数（非截断条数）");
    assert_eq!(hits(&body).len(), 2);
    assert_eq!(
        hits(&body)[0]["article_title"],
        STD_KW,
        "全等命中的行 MUST 排在包含命中之前"
    );
    let first = &hits(&body)[0];
    assert_eq!(first["source_table"], "zc_id_stan-fin-ifrs-article");
    for key in [
        "article_id",
        "article_code",
        "article_title",
        "article_body",
        "level",
        "issuer",
    ] {
        assert!(first.get(key).is_some(), "响应字段 {key} MUST 保留");
    }

    // scope 过滤：fin-gaap 仅命中自身
    let (status, body) = send(
        &pool,
        "/standard/search",
        json!({"keywords": [STD_KW], "scopes": ["fin-gaap"]}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(total(&body), 1);
    assert_eq!(
        hits(&body)[0]["source_table"],
        "zc_id_stan-fin-gaap-article"
    );

    // 未命中
    let (status, body) = send(
        &pool,
        "/standard/search",
        json!({"keywords": ["KM-T-SS-STD-ABSENT"], "scopes": ["fin-ifrs", "fin-gaap"]}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(total(&body), 0);
    assert!(hits(&body).is_empty());

    // level 过滤：以 operation 种子行（notice 非空 ∧ `_t_` 非空）为关键词，
    // 期望值在 Rust 侧按同一谓词（大小写不敏感包含）对库内实况计数。
    let seeded: Option<(String, String)> = sqlx::query_as(
        r#"SELECT notice, "_t_" FROM isahl."zc_id_stan-operation"
           WHERE deleted_at IS NULL AND notice IS NOT NULL AND notice <> '' AND "_t_" IS NOT NULL
           ORDER BY id LIMIT 1"#,
    )
    .fetch_optional(&pool)
    .await
    .expect("operation 种子行查询失败");
    let (kw, level) = seeded.expect("test 库须有 operation 种子行（notice/_t_ 非空）");
    let rows: Vec<(Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as(r#"SELECT notice, comments, "_t_" FROM isahl."zc_id_stan-operation""#)
            .fetch_all(&pool)
            .await
            .expect("operation 全表读取失败");
    let needle = kw.to_lowercase();
    let matches = |fields: &[&Option<String>]| {
        fields
            .iter()
            .any(|f| f.as_deref().unwrap_or("").to_lowercase().contains(&needle))
    };
    let expected_all = rows.iter().filter(|(n, c, _)| matches(&[n, c])).count() as i64;
    let expected_level = rows
        .iter()
        .filter(|(n, c, t)| matches(&[n, c]) && t.as_deref() == Some(level.as_str()))
        .count() as i64;
    assert!(expected_all > 0 && expected_level > 0, "种子谓词应非空");

    let (status, body) = send(
        &pool,
        "/standard/search",
        json!({"keywords": [kw], "scopes": ["operation"], "max_results": 20}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(total(&body), expected_all, "无 level 过滤 = 全量命中");

    let (status, body) = send(
        &pool,
        "/standard/search",
        json!({"keywords": [kw], "scopes": ["operation"], "level": level, "max_results": 20}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        total(&body),
        expected_level,
        "level 过滤 MUST 收窄到 _t_ 命中集"
    );
    assert!(
        hits(&body)
            .iter()
            .all(|h| h["level"] == Value::String(level.clone())),
        "命中行 level MUST 为过滤值"
    );

    let (status, body) = send(
        &pool,
        "/standard/search",
        json!({"keywords": [kw], "scopes": ["operation"], "level": "KM-T-SS-NOPE"}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(total(&body), 0, "不存在的 level MUST 全量过滤");

    cleanup(&pool).await;
}
