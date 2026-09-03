//! flow-lifecycle-split 集成测试 — 设计·实例 / 实现·范例 分离契约
//!
//! 守护可观察行为：
//! 1. `POST /approval-flows/{id}/generate-template`：有效态（published）设计·实例
//!    克隆为实现·范例——notice/code 原样、tpl_id → 设计行、`_f_/_t_` 派生为
//!    实现·范例（function 码 `↑_` → `↓.` 前缀换码）；
//! 2. 幂等守卫：同源同名范例已存在 → 400；
//! 3. `GET /approval-flows/lifecycle/{class}`：类过滤（设计·实例 / 实现·范例
//!    互不串页）、非法类 400；
//! 4. 未认证写请求 → 非 2xx（require_auth 对齐 initiate/publish）。

use ::common::context::RequestContext;
use ::common::testing::connect_test_db;
use actix_web::{dev::Service, test, web, App, HttpMessage};
use approval::handlers;
use serde_json::{json, Value};
mod common;
use common::setup_test_schema;

const USER_ID: i64 = 424242;

fn test_ctx() -> RequestContext {
    RequestContext::with_username(USER_ID, "flow-lifecycle@test.local", "flow-lifecycle-test")
}

macro_rules! test_app {
    ($pool:expr, $authed:expr) => {{
        let ctx = test_ctx();
        let authed: bool = $authed;
        test::init_service(
            App::new().app_data(web::Data::new($pool)).service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        if authed {
                            req.extensions_mut().insert(ctx.clone());
                        }
                        srv.call(req)
                    })
                    .configure(handlers::validate::register)
                    .configure(handlers::flow_lifecycle::register)
                    .configure(handlers::approval_flow::register),
            ),
        )
        .await
    }};
}

/// 种子用户 + 设计·实例行（↑_NA 坐标）+ published 状态桥；返回 design_id。
async fn seed_design(pool: &sqlx::PgPool, notice: &str, code: &str) -> i64 {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, 'flow-lifecycle-test', 'flow-lifecycle-test', 'flow-lifecycle@test.local',
                   'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(USER_ID)
    .execute(pool)
    .await
    .unwrap();

    // dk 坐标（JC/FTA/↑_NA）解析 —— 字典在册码
    let (scene_id, factor_id, fn_id): (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT id FROM isahl.zc_id_scene WHERE code = 'JC' AND deleted_at IS NULL LIMIT 1),
             (SELECT id FROM isahl.zc_id_factor WHERE code = 'FTA' AND deleted_at IS NULL LIMIT 1),
             (SELECT id FROM isahl.zc_id_function WHERE code = '↑_NA' AND deleted_at IS NULL LIMIT 1)"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();

    let design_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve"
           (notice, code, comments, dk_scene, dk_factor, dk_function, created_by_id,
            _f_, _t_)
           VALUES ($1, $2, '设计说明', $3, $4, $5, $6, '设计', '实例')
           RETURNING id"#,
    )
    .bind(notice)
    .bind(code)
    .bind(scene_id)
    .bind(factor_id)
    .bind(fn_id)
    .bind(USER_ID)
    .fetch_one(pool)
    .await
    .unwrap();

    // published 桥（「有效」态）
    let status_id: i64 = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_stus-process" WHERE code = 'published' LIMIT 1"#,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (id, ref_left, ref_right)
           VALUES (isahl.gen_next_zuid(), $1, $2)"#,
    )
    .bind(design_id)
    .bind(status_id)
    .execute(pool)
    .await
    .unwrap();

    design_id
}

#[tokio::test]
async fn generate_template_clones_design_to_exemplar() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let design_id = seed_design(&pool, "生命周期克隆测试流程", "FLOW-LC-1").await;
    let app = test_app!(pool.clone(), true);

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/test/approval-flows/{design_id}/generate-template"
            ))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "首次生成应成功");
    let body: Value = test::read_body_json(resp).await;
    let tpl_id: i64 = body["data"]["tpl_id"]
        .as_str()
        .expect("tpl_id in data")
        .parse()
        .expect("zuid numeric string");
    assert_eq!(tpl_id, design_id, "tpl_id 必须指向设计·实例行");
    let exemplar_id: i64 = body["data"]["id"]
        .as_str()
        .expect("id in data")
        .parse()
        .expect("zuid numeric string");
    // DB 断言：同表物化 + 类派生 + 原样复制
    let row: (Option<String>, Option<String>, Option<i64>) = sqlx::query_as(
        r#"SELECT code, comments, dk_function FROM isahl.zc_id_process WHERE id = $1"#,
    )
    .bind(exemplar_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let design_code: Option<String> =
        sqlx::query_scalar(r#"SELECT code FROM isahl.zc_id_process WHERE id = $1"#)
            .bind(design_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row.0, design_code, "code 原样复制");
    assert!(row.1.as_deref() == Some("设计说明"), "comments 原样复制");
    let fn_code: String =
        sqlx::query_scalar(r#"SELECT code FROM isahl.zc_id_function WHERE id = $1"#)
            .bind(row.2.expect("dk_function"))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(fn_code, "↓.NA", "function 码换 ↓. 前缀（实现·范例）");
    let (f, t): (String, String) =
        sqlx::query_as(r#"SELECT _f_, _t_ FROM isahl.zc_id_process WHERE id = $1"#)
            .bind(exemplar_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((f.as_str(), t.as_str()), ("实现", "范例"));

    // 幂等守卫：同名在册范例已存在 → 400
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/test/approval-flows/{design_id}/generate-template"
            ))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 400, "重复生成应被守卫拒绝");
}

#[tokio::test]
async fn lifecycle_list_filters_by_class() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let design_id = seed_design(&pool, "类过滤设计流程", "FLOW-LC-2").await;
    // 设计·模板行（_t_=NULL，标准审批流程模板语义——如 FLOW-STD 标准三级审批）
    let (scene_id, factor_id, fn_id): (i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT id FROM isahl.zc_id_scene WHERE code = 'JC' AND deleted_at IS NULL LIMIT 1),
             (SELECT id FROM isahl.zc_id_factor WHERE code = 'FTA' AND deleted_at IS NULL LIMIT 1),
             (SELECT id FROM isahl.zc_id_function WHERE code = '↑_NA' AND deleted_at IS NULL LIMIT 1)"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let template_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve"
           (notice, code, comments, dk_scene, dk_factor, dk_function, created_by_id,
            _f_, _t_)
           VALUES ('设计模板-类过滤', 'FLOW-TPL-LC', '模板说明', $1, $2, $3, $4, '设计', NULL)
           RETURNING id"#,
    )
    .bind(scene_id)
    .bind(factor_id)
    .bind(fn_id)
    .bind(USER_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    let app = test_app!(pool.clone(), true);

    // 经生成端点产出范例行
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/test/approval-flows/{design_id}/generate-template"
            ))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = test::read_body_json(resp).await;
    let exemplar_id: i64 = body["data"]["id"]
        .as_str()
        .expect("id in data")
        .parse()
        .expect("zuid numeric string");

    // design-instance 页只含设计行
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/test/approval-flows/lifecycle/design-instance")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = test::read_body_json(resp).await;
    let ids: Vec<i64> = body["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r["id"]
                .as_str()
                .expect("zuid string id")
                .parse::<i64>()
                .expect("zuid numeric")
        })
        .collect();
    assert!(ids.contains(&design_id), "设计页应含设计行");
    assert!(
        ids.contains(&template_id),
        "设计页应含设计·模板行（_t_=NULL，标准审批流程模板）"
    );
    assert!(!ids.contains(&exemplar_id), "设计页不得串入范例行");

    // impl-exemplar 页只含范例行
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/test/approval-flows/lifecycle/impl-exemplar")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = test::read_body_json(resp).await;
    let ids: Vec<i64> = body["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r["id"]
                .as_str()
                .expect("zuid string id")
                .parse::<i64>()
                .expect("zuid numeric")
        })
        .collect();
    assert!(ids.contains(&exemplar_id), "模板页应含范例行");
    assert!(!ids.contains(&design_id), "模板页不得串入设计行");

    // 非法类 → 400
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/test/approval-flows/lifecycle/bogus-class")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn generate_template_requires_auth() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let design_id = seed_design(&pool, "鉴权设计流程", "FLOW-LC-3").await;
    let app = test_app!(pool, false);

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/test/approval-flows/{design_id}/generate-template"
            ))
            .to_request(),
    )
    .await;
    assert!(
        resp.status().is_client_error(),
        "未认证写请求必须被拒绝（require_auth）"
    );
}

/// 回归守护（flow-lifecycle-split）：crud 创建的新流程必须落 设计·实例 类，
/// 否则会被流程定义页的类过滤隐藏——用户视角即「创建失败」。
#[tokio::test]
async fn crud_created_flow_visible_in_design_list() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let app = test_app!(pool.clone(), true);

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/test/approval-flows")
            .set_json(json!({"name": "crud创建可见性测试"}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 201, "crud 创建应成功");
    let body: Value = test::read_body_json(resp).await;
    // crud create 201 返回裸实体（无 {success,data} 信封）——两种形状都取 id
    let flow_id: i64 = body["data"]["id"]
        .as_str()
        .or_else(|| body["id"].as_str())
        .expect("zuid string id")
        .parse()
        .expect("zuid numeric");

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/test/approval-flows/lifecycle/design-instance")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = test::read_body_json(resp).await;
    let ids: Vec<i64> = body["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r["id"]
                .as_str()
                .expect("zuid string id")
                .parse::<i64>()
                .expect("zuid numeric")
        })
        .collect();
    assert!(
        ids.contains(&flow_id),
        "crud 创建的设计·实例必须出现在流程定义页数据源中"
    );

    // 回归守护（引用连字符叶表名必须双引号）：project 分支创建同样成功且可见
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/test/approval-flows")
            .set_json(json!({"name": "project分支可见性测试", "branch": "zc_id_proc-project"}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 201, "project 分支创建应成功");
    let body: Value = test::read_body_json(resp).await;
    let project_flow_id: i64 = body["data"]["id"]
        .as_str()
        .or_else(|| body["id"].as_str())
        .expect("zuid string id")
        .parse()
        .expect("zuid numeric");
    assert!(
        ids.is_empty() || !ids.contains(&project_flow_id),
        "创建后需重新查询（本断言仅确认创建成功）"
    );
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/test/approval-flows/lifecycle/design-instance")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = test::read_body_json(resp).await;
    let ids_after: Vec<i64> = body["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r["id"]
                .as_str()
                .expect("zuid string id")
                .parse::<i64>()
                .expect("zuid numeric")
        })
        .collect();
    assert!(
        ids_after.contains(&project_flow_id),
        "project 分支创建的流程必须可见（连字符表名需引号）"
    );
}

/// 保存链路回归：crud PUT（设计器画布/表单保存）须能持久化 comments 信封
/// 并返回 200；validate 预检端点对合法图放行。
#[tokio::test]
async fn save_flow_updates_comments_via_crud_put() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();
    let app = test_app!(pool.clone(), true);

    // 1. crud 创建
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/test/approval-flows")
            .set_json(json!({"name": "保存链路测试"}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 201, "crud 创建应成功");
    let body: Value = test::read_body_json(resp).await;
    let flow_id: i64 = body["data"]["id"]
        .as_str()
        .or_else(|| body["id"].as_str())
        .expect("zuid string id")
        .parse()
        .expect("zuid numeric");

    // 2. 保存（PUT comments 信封，AVIC withCommentsMerge 持久化模式）
    let graph = json!({
        "version": 1,
        "nodes": [
            {"id": "n0", "type": "start", "label": "提交"},
            {"id": "n1", "type": "end", "label": "结束"}
        ],
        "meta": {"category": "测试", "entityType": "task"}
    });
    let resp = test::call_service(
        &app,
        test::TestRequest::put()
            .uri(&format!("/test/approval-flows/{flow_id}"))
            .set_json(json!({"name": "保存链路测试-改名", "meta": graph}))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "PUT 保存应成功");

    // 3. 持久化断言：设计图 meta jsonb + 类标记不回退 + mermaid 自动生成
    let row: (Option<Value>, Option<String>, Option<String>) =
        sqlx::query_as(r#"SELECT meta, _f_, _t_ FROM isahl.zc_id_process WHERE id = $1"#)
            .bind(flow_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        row.0
            .as_ref()
            .and_then(|m| m.get("nodes"))
            .and_then(|n| n.as_array())
            .map(|arr| arr.iter().any(|nd| nd["id"] == "n0"))
            .unwrap_or(false),
        "meta 应持久化设计图信封"
    );
    assert_eq!(
        (row.1.as_deref(), row.2.as_deref()),
        (Some("设计"), Some("实例")),
        "保存不得回退生命周期类标记"
    );

    // 4. validate 预检（保存前 D8 检查走同一端点）
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/test/approval-flows/validate")
            .set_json(json!({
                "nodes": [
                    {"id": "n0", "type": "start", "label": "提交", "drive": "event", "eventLeaf": "zc_id_even-accident"},
                    {"id": "n1", "type": "end", "label": "结束", "statementLeaf": "zc_id_stat-appeal"}
                ]
            }))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "合法图 validate 应放行");
}
