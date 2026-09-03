//! 临时诊断探针（定位 approve 500 响应体；定位后删除）
mod common;

use actix_web::{test, web, App};
use common::testing::connect_test_db;
use serde_json::json;

const APP1: i64 = 2001;
const M1: i64 = 2002;

async fn call_approve_body(
    pool: &sqlx::PgPool,
    actor: i64,
    instance_id: i64,
) -> (actix_web::http::StatusCode, String) {
    let ctx = ::common::context::RequestContext::with_username(actor, "actor@test", "actor");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(std::sync::Arc::new(
                ::common::event_bus::InMemoryEventBus::new(),
            )
                as std::sync::Arc<dyn ::common::event_bus::DomainEventBus>))
            .service(
                web::scope("/test")
                    .wrap_fn(move |req, srv| {
                        req.extensions_mut().insert(ctx.clone());
                        srv.call(req)
                    })
                    .configure(approval::handlers::approve_reject::register),
            ),
    )
    .await;
    let req = test::TestRequest::post()
        .uri(&format!("/test/approval-instances/{}/approve", instance_id))
        .set_json(json!({"opinion": "同意"}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let status = resp.status();
    let bytes = actix_web::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap_or_default();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

#[tokio::test]
async fn zz_probe_approve_500() {
    let pool = connect_test_db().await;
    common::setup_test_schema(&pool).await.unwrap();
    common::ensure_role_member(&pool, "r_role_a", M1).await.unwrap();
    common::grant_user_access(&pool, APP1, "approval-instances", &["approve"])
        .await
        .unwrap();

    let (flow, ids) = common::make_flow(
        &pool,
        "FLOW-ZZPROBE",
        &[common::NodeSpec {
            code: "N1",
            node_type: "approve",
            next: vec![],
        }],
    )
    .await;
    let n2 = common::add_approve_node(&pool, flow, "N2", "岗位节点", &[M1], "sequential").await;
    sqlx::query(
        r#"UPDATE isahl.zc_id_process_rr_operation
           SET "next-ops" = $2::jsonb WHERE ref_left = $1 AND ref_right = $3"#,
    )
    .bind(flow)
    .bind(json!([n2]))
    .bind(ids[0])
    .execute(&pool)
    .await
    .unwrap();

    // 复刻 create_first_instance
    let instance_id: i64 = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_oper-approve" WHERE deleted_at IS NULL ORDER BY id DESC LIMIT 1"#,
    )
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(0);
    eprintln!("probe instance_id placeholder={}", instance_id);

    // 直接调用权限决策（approve handler 第一道闸）验证 SQL
    let inst_probe: i64 = sqlx::query_scalar(
        r#"SELECT COALESCE(MAX(id),0)+1 FROM isahl."zc_id_oper-approve""#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    match common::permissions::require_resource_access(
        &pool,
        APP1,
        "approval-instances",
        inst_probe,
        "approve",
    )
    .await
    {
        Ok(_) => eprintln!("require_resource_access OK"),
        Err(e) => eprintln!("require_resource_access ERR: {:?}", e),
    }

    // 直接跑决策 SQL（复刻 permissions.rs 拼装后形态，但带真实 resource_attrs）
    let resource_attrs_check: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl_auth.ngac_object_attribute
           WHERE resource_type = 'approval-instances' AND fk_resource = $1 AND deleted_at IS NULL"#,
    )
    .bind(inst_probe)
    .fetch_one(&pool)
    .await
    .unwrap();
    eprintln!("OA rows for approval-instances:{} = {}", inst_probe, resource_attrs_check);
    let _ = instance_id;
}
