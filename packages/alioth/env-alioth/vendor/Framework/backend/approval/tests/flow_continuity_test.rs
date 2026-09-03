//! flow-process-continuity 规约集成测试
//!
//! 验证（openspec/changes/fix-flow-process-continuity）：
//! 1. 审批流程定义 create 落子类表 `zc_id_proc-approve`，且经基表
//!    `zc_id_process` 查询（继承并集）可见——flow-definition-subtype-placement。
//! 2. 审批动作意见写时间锚：`zc_id_deta-opinion.qk_date` 非空且指向
//!    当日 `zc_id_scal-date` 行——flow-instance-temporal-anchor。
//!
//! 与其他套件共享 `aliothstudio_test`，单线程运行：
//!   cargo test --test flow_continuity_test -- --test-threads=1

use ::common::testing::connect_test_db;
use actix_web::{dev::Service, test, web, App, HttpMessage};
use approval::handlers;
use approval::models::CreateApprovalFlowRequest;
use approval::repositories::ApprovalFlowRepository;
use crud::repository::AliothRepository;

mod common;
use common::setup_test_schema;

#[tokio::test]
async fn approval_flow_create_lands_in_proc_approve_subtype() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    let repo = ApprovalFlowRepository::new(pool.clone());
    let name = common::test_code("流程落位测试");
    let created = repo
        .create(
            CreateApprovalFlowRequest {
                name: name.clone(),
                code: None,
                comments: None,
                meta: None,
                branch: None,
                context_id: None,
                context_table: None,
            },
            1,
        )
        .await
        .expect("create approval flow");

    // 1. 行存在于子类表 zc_id_proc-approve（定义落位）
    let in_subtype: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_proc-approve"
           WHERE id = $1 AND notice = $2 AND deleted_at IS NULL"#,
    )
    .bind(created.id)
    .bind(&name)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(
        in_subtype.is_some(),
        "create 必须落于 zc_id_proc-approve 子类表"
    );

    // 2. 基表查询（继承并集）可见——既有读路径不变
    let via_base = repo.get(created.id).await.expect("get via base table");
    assert!(via_base.is_some(), "基表 zc_id_process 查询必须并入子表行");
    assert_eq!(via_base.unwrap().name, name);

    // 3. dk 坐标不悬空：dk_* 为 NULL（维度未种子）或指向真实维度行（已种子），
    //    不得为历史硬编码 515/522/526
    let (dk_scene, dk_factor, dk_function): (Option<i64>, Option<i64>, Option<i64>) =
        sqlx::query_as(
            r#"SELECT dk_scene, dk_factor, dk_function
               FROM isahl."zc_id_proc-approve" WHERE id = $1"#,
        )
        .bind(created.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    for dk in [dk_scene, dk_factor, dk_function] {
        if let Some(v) = dk {
            assert!(
                v != 515 && v != 522 && v != 526,
                "dk 不得为历史悬空硬编码 ZUID"
            );
        }
    }

    // 清理
    sqlx::query(r#"UPDATE isahl."zc_id_proc-approve" SET deleted_at = NOW() WHERE id = $1"#)
        .bind(created.id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn approve_action_writes_qk_date_anchor() {
    let pool = connect_test_db().await;
    setup_test_schema(&pool).await.unwrap();

    // 夹具：工程师 + 审批事件 + 审批实例（对齐 approve_reject_test 范式）
    let eng_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_subj-employee" (notice, created_by_id)
           VALUES ('流程连续性测试工程师', 1) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let approve_event_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, created_by_id)
           VALUES ('流程连续性测试审批事项', 1) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let instance_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_oper-approve"
           (notice, fk_subject, fk_operator, created_by_id)
           VALUES ('节点1审核', $1, $1, 1) RETURNING id"#,
    )
    .bind(eng_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
    )
    .bind(instance_id)
    .bind(approve_event_id)
    .execute(&pool)
    .await
    .unwrap();

    common::grant_user_access(&pool, eng_id, "approval-instances", &["approve", "reject"])
        .await
        .expect("grant approval access");

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(std::sync::Arc::new(
                ::common::event_bus::InMemoryEventBus::new(),
            )
                as std::sync::Arc<dyn ::common::event_bus::DomainEventBus>))
            .configure(handlers::approve_reject::register),
    )
    .await;

    let req = test::TestRequest::post()
        .uri(&format!("/approval-instances/{}/approve", instance_id))
        .set_json(serde_json::json!({"opinion": "同意（连续性测试）"}))
        .to_request();
    req.extensions_mut().insert(eng_id);

    let resp = app.call(req).await.unwrap();
    assert!(resp.status().is_success(), "approve 调用必须成功");

    // 时间锚断言：qk_date 非空且指向当日 scal-date 行
    let (qk_date, date_val): (Option<i64>, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        r#"SELECT o.qk_date, sd.date
               FROM isahl."zc_id_deta-opinion" o
               LEFT JOIN isahl."zc_id_scal-date" sd ON sd.id = o.qk_date
               WHERE o.fk_list = $1 AND o.deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(qk_date.is_some(), "审批意见 qk_date 必须写入时间锚");
    let date_val = date_val.expect("qk_date 必须解析到 zc_id_scal-date 行");
    assert_eq!(
        date_val.date_naive(),
        chrono::Utc::now().date_naive(),
        "qk_date 锚定的日期必须为动作当日"
    );
}
