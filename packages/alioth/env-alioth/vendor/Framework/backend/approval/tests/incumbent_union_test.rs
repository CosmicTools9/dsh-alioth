//! 审批节点「岗位任职账号集合」回归（fix-approver-incumbent-source）
//!
//! 覆盖：任职账号集合 = 岗位标量 `fk_user` ∪ 任职桥 `zc_id_subj-post_rr_employee`
//! 派生的 `zc_id_empl-natural`/`zc_id_empl-agent`.`fk_user`（去重、仅活跃；唯一实现 =
//! `common::position_incumbent_accounts_sql!()`）。
//!
//! 修复前（仅认标量）：
//! - 运行时待办解析 `resolve_node_assign` 对「仅经组织管理挂桥任职」的岗位产出空审批人；
//! - 发布期 `resolve_approver_positions` 解析为空 → 节点落桥 0 行（无人可审）。
//!
//! 依赖：isahl_auth.auth_users、`zc_id_operation_rr_approve`、`zc_id_subj-position`、
//! `zc_id_empl-natural`、`zc_id_subj-post_rr_employee`。

use ::common::testing::connect_test_db;
use actix_web::{dev::Service, test, web, App, HttpMessage};
use approval::handlers;
use approval::node_meta::resolve_node_assign;
use serde_json::{json, Value};
use sqlx::PgPool;

mod common;
use common::setup_test_schema;

/// 动态测试 id 段（进程+纳秒派生，跨运行不冲突；测试不清理数据）
fn tid(base: i64) -> i64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    ((nanos % 1_000_000) as i64) * 100 + base
}

/// 活跃账号
async fn seed_account(pool: &PgPool, id: i64, username: &str) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, $2, $3, $3 || '@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(id)
    .bind(username)
    .bind(username)
    .execute(pool)
    .await
    .expect("seed account");
}

/// 岗位 + 任职自然人（持账号）+ 任职桥；岗位行**不写标量** `fk_user`（组织管理挂人只写桥）
async fn seed_bridge_only_position(
    pool: &PgPool,
    pos_id: i64,
    person_id: i64,
    account: i64,
    tag: &str,
) {
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve dk coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-position" (id, notice, code, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(pos_id)
    .bind(format!("T-POS-{tag}"))
    .bind(format!("T-POSC-{tag}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(pool)
    .await
    .expect("seed position");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_empl-natural" (id, notice, fk_user, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(person_id)
    .bind(format!("T-EMP-{tag}"))
    .bind(account)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(pool)
    .await
    .expect("seed natural person");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-post_rr_employee" (ref_left, ref_right, notice)
           VALUES ($1, $2, $3) ON CONFLICT DO NOTHING"#,
    )
    .bind(pos_id)
    .bind(person_id)
    .bind(format!("T-BRIDGE-{tag}"))
    .execute(pool)
    .await
    .expect("seed employment bridge");
}

/// 操作行（节点）+ 岗位桥 `rr_approve`（ck_cate-role NULL = 直管）
async fn seed_operation_with_position(pool: &PgPool, pos_id: i64, tag: &str) -> i64 {
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve dk coords");
    let op_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve" (notice, code, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, 1, $3, $4, $5) RETURNING id"#,
    )
    .bind(format!("T-OP-{tag}"))
    .bind(format!("T-OPC-{tag}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("seed operation");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_operation_rr_approve" (ref_left, ref_right, created_by_id)
           VALUES ($1, $2, 1)"#,
    )
    .bind(op_id)
    .bind(pos_id)
    .execute(pool)
    .await
    .expect("seed rr_approve bridge");
    op_id
}

/// 运行时解析：仅桥任职（标量空）的岗位 MUST 解析出其任职账号
#[tokio::test]
async fn resolve_node_assign_includes_bridge_only_incumbent() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let base = tid(8100);
    let (account, person, pos_bridged, pos_bare) = (base, base + 1, base + 2, base + 3);
    seed_account(&pool, account, &format!("t-inc-{base}")).await;
    seed_bridge_only_position(&pool, pos_bridged, person, account, &format!("{base}")).await;
    // 无任何任职账号的岗位（对照组）
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("TX", "FJA", "↓_GG"))
            .await
            .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-position" (id, notice, code, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(pos_bare)
    .bind(format!("T-POS-BARE-{base}"))
    .bind(format!("T-POSC-BARE-{base}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .unwrap();

    let op_bridged = seed_operation_with_position(&pool, pos_bridged, &format!("{base}")).await;
    let op_bare = seed_operation_with_position(&pool, pos_bare, &format!("{base}b")).await;

    let assign = resolve_node_assign(&pool, op_bridged)
        .await
        .expect("resolve bridged node");
    assert!(
        assign.assignees.contains(&account),
        "仅桥任职（标量空）的岗位 MUST 解析出任职账号，实测: {:?}",
        assign.assignees
    );

    let bare = resolve_node_assign(&pool, op_bare)
        .await
        .expect("resolve bare node");
    assert!(
        bare.assignees.is_empty(),
        "无任职账号的岗位 MUST 解析为空（admin 兜底路径），实测: {:?}",
        bare.assignees
    );
}

/// 发布期：仅桥任职岗位放行并落桥；无活跃任职账号岗位同样放行但**不落桥**（空审批人 degrade）
#[tokio::test]
async fn publish_accepts_bridge_only_position_and_degrades_bare_position() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let base = tid(8200);
    let (account, person, pos_bridged, pos_bare) = (base, base + 1, base + 2, base + 3);
    seed_account(&pool, account, &format!("t-pub-{base}")).await;
    seed_bridge_only_position(&pool, pos_bridged, person, account, &format!("p{base}")).await;
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("TX", "FJA", "↓_GG"))
            .await
            .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-position" (id, notice, code, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(pos_bare)
    .bind(format!("T-POS-NOINC-{base}"))
    .bind(format!("T-POSC-NOINC-{base}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .unwrap();

    // 发布用户（require_auth 需要）
    seed_account(&pool, base + 9, &format!("t-publisher-{base}")).await;
    let ctx = ::common::context::RequestContext::with_username(
        base + 9,
        "publisher@test.local",
        "publisher",
    );
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/test")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .configure(handlers::publish::register),
        ),
    )
    .await;

    // 流程坐标（与节点无关，取流程域坐标）
    let (f_scene, f_factor, f_function) = ontology_binding::resolve(&pool, ("JC", "FTA", "↑_NA"))
        .await
        .unwrap();

    // ① 仅桥任职岗位 → 发布成功且落 rr_approve 桥
    let graph_ok = json!({
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "n-appr", "type": "approve", "label": "审批",
             "direct": {"pos": format!("T-POS-p{base}")}},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"},
        ],
        "edges": [
            {"source": "n-start", "target": "n-appr"},
            {"source": "n-appr", "target": "n-end"}
        ]
    });
    let flow_ok: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve" (notice, meta, code, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2::jsonb, 'draft', 1, $3, $4, $5) RETURNING id"#,
    )
    .bind(format!("T-FLOW-OK-{base}"))
    .bind(graph_ok.to_string())
    .bind(f_scene)
    .bind(f_factor)
    .bind(f_function)
    .fetch_one(&pool)
    .await
    .unwrap();

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-flows/{}/publish", flow_ok))
            .to_request(),
    )
    .await;
    let status = resp.status();
    let body: Value = test::read_body_json(resp).await;
    assert!(
        status.is_success(),
        "仅桥任职岗位 MUST 可发布（修复前解析为空 → 落桥 0 行）：status={status} body={body:?}"
    );
    let bridged: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_operation_rr_approve ra
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = ra.ref_left AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.ref_left = $1 AND rro.deleted_at IS NULL
           WHERE ra.ref_right = $2 AND ra.deleted_at IS NULL"#,
    )
    .bind(flow_ok)
    .bind(pos_bridged)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bridged, 1, "发布 MUST 把仅桥任职岗位落入 rr_approve 桥");

    // ② 无活跃任职账号岗位 → 仍可发布（空审批人 = approval-node-degrade 已声明降级形态），
    //    该岗位不落桥（桥 0 行），运行期待办集合为空 + admin 兜底；发布期 MUST NOT 拒绝。
    let graph_bare = json!({
        "nodes": [
            {"id": "n-start", "type": "start", "label": "提交", "eventLeaf": "zc_id_even-accident", "next": [{"to": 1}]},
            {"id": "n-appr", "type": "approve", "label": "审批",
             "direct": {"pos": format!("T-POS-NOINC-{base}")}},
            {"id": "n-end", "type": "end", "label": "完成", "statementLeaf": "zc_id_stat-inspection"},
        ],
        "edges": [
            {"source": "n-start", "target": "n-appr"},
            {"source": "n-appr", "target": "n-end"}
        ]
    });
    let flow_bare: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_proc-approve" (notice, meta, code, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2::jsonb, 'draft', 1, $3, $4, $5) RETURNING id"#,
    )
    .bind(format!("T-FLOW-BARE-{base}"))
    .bind(graph_bare.to_string())
    .bind(f_scene)
    .bind(f_factor)
    .bind(f_function)
    .fetch_one(&pool)
    .await
    .unwrap();

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/test/approval-flows/{}/publish", flow_bare))
            .to_request(),
    )
    .await;
    let status = resp.status();
    let body: Value = test::read_body_json(resp).await;
    assert!(
        status.is_success(),
        "无活跃任职账号的岗位 MUST 仍可发布（空审批人 = 已声明 degrade 形态），实测 status={status} body={body:?}"
    );
    let bridged_bare: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl.zc_id_operation_rr_approve ra
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_left = ra.ref_left AND oe.deleted_at IS NULL
           JOIN isahl.zc_id_process_rr_operation rro
             ON rro.ref_right = oe.ref_left AND rro.ref_left = $1 AND rro.deleted_at IS NULL
           WHERE ra.ref_right = $2 AND ra.deleted_at IS NULL"#,
    )
    .bind(flow_bare)
    .bind(pos_bare)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        bridged_bare, 0,
        "无任职账号岗位 MUST 不落桥（运行期按空审批人 + admin 可见回退，不得静默挂到无关岗位）"
    );
}
