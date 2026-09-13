//! Gateway PEP 操作者标识接缝测试（血缘审计口径：`SECURITY_SPEC §10.1`）。
//!
//! 证明链条：PEP 判定通过 → PEP 以 `resolve_subject` 结果开启 actor 作用域 →
//! **内层 handler** 触发的 `crud::audit_outbox::enqueue` 自动带上该标识
//! （真实 DB：`isahl_audit.audit_outbox`）。
//!
//! 与 `pep_decision_path_test.rs`（无 DB，判定语义）互补：本文件覆盖
//! 「身份如何从 JWT 走到审计写入」这一跨层接缝。
//!
//! 依赖：`common::testing::connect_test_db`（aliothstudio_test）+ 本地 stub PDP。

use actix_web::{http::StatusCode, test, web, App, HttpResponse, HttpServer};
use alioth_gateway::pep::NgacEnforcer;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Duration;

const TEST_SSO_JWT_PRIVATE_KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgD/UpJ7dxbI+3BhJs\
dDIxSFS+tdT9wSzVVS8z+Au6MRahRANCAATEcFhYPhVkFdIGNAiBwxQpu0cYRXc0\
roJB3RHF1LfIsaCxcnVep0snC4+8StUixIjfLAZ8Mc8+uqa43ndeNEFm\
-----END PRIVATE KEY-----";

/// stub PDP：decide/list/columns 全部放行（本测试只关心标识传递，不关心判定）。
async fn stub_permit(_state: web::Data<Arc<AtomicUsize>>) -> HttpResponse {
    HttpResponse::Ok().json(json!({
        "permitted": true,
        "reason": "stub",
        "columns": ["*"],
        "visible_ids": null,
    }))
}

async fn stub_columns(_state: web::Data<Arc<AtomicUsize>>) -> HttpResponse {
    HttpResponse::Ok().json(json!({ "permitted": true, "reason": "stub", "columns": ["*"] }))
}

fn spawn_stub_pdp() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("stub addr").to_string();
    drop(listener);
    let bind_addr = addr.clone();
    std::thread::spawn(move || {
        let system = actix_web::rt::System::new();
        system.block_on(async move {
            let _ = HttpServer::new(|| {
                App::new()
                    .app_data(web::Data::new(Arc::new(AtomicUsize::new(0))))
                    .route("/api/ngac/pdp/decide", web::post().to(stub_permit))
                    .route("/api/ngac/pdp/list", web::post().to(stub_permit))
                    .route("/api/ngac/pdp/columns", web::post().to(stub_columns))
            })
            .workers(1)
            .bind(&bind_addr)
            .expect("bind stub")
            .run()
            .await;
        });
    });
    for _ in 0..100 {
        if std::net::TcpStream::connect(&addr).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    format!("http://{addr}")
}

/// 签发测试 JWT（可带/不带 username，验证回落规则）。
fn issue_token(sub: &str, username: &str) -> String {
    let now = chrono::Utc::now().timestamp() as usize;
    let claims = json!({
        "sub": sub,
        "exp": now + 3600,
        "iat": now,
        "email": "",
        "username": username,
        "sid": "",
        "iss": "http://localhost:9002",
        "aud": "http://localhost:9002",
        "scope": "",
        "svc_user_id": 0,
    });
    encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(TEST_SSO_JWT_PRIVATE_KEY).expect("test ec key"),
    )
    .expect("sign token")
}

/// 内层 handler：模拟业务写路径经 crud 审计 outbox（不带显式操作者 → 取作用域值）。
async fn enqueue_probe(pool: web::Data<sqlx::PgPool>, table: web::Data<String>) -> HttpResponse {
    let event = crud::audit_outbox::OutboxEvent::new(
        table.get_ref().clone(),
        1,
        crud::audit_outbox::AuditAction::Insert,
    );
    match crud::audit_outbox::enqueue(&pool, &event).await {
        Ok(id) => HttpResponse::Ok().json(json!({ "outbox_id": id })),
        Err(e) => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR)
            .json(json!({ "error": e.to_string() })),
    }
}

async fn operator_identity(pool: &sqlx::PgPool, table: &str) -> Option<String> {
    sqlx::query_scalar(
        "SELECT performed_by_email FROM isahl_audit.audit_outbox WHERE table_name = $1",
    )
    .bind(table)
    .fetch_optional(pool)
    .await
    .expect("fetch outbox identity")
    .flatten()
}

async fn run_case(sub: &str, username: &str) -> Option<String> {
    let pool = ::common::testing::connect_test_db().await;
    let table = format!(
        "test_audit_pep_{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(table.clone()))
            .service(
                web::scope("/api")
                    .wrap(NgacEnforcer::new_without_pool_with_pdp(spawn_stub_pdp()))
                    .route("/engineers", web::post().to(enqueue_probe)),
            ),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/api/engineers")
        .insert_header((
            "Authorization",
            format!("Bearer {}", issue_token(sub, username)),
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200, "PEP 放行后内层写路径应成功");

    let identity = operator_identity(&pool, &table).await;
    sqlx::query("DELETE FROM isahl_audit.audit_outbox WHERE table_name = $1")
        .bind(&table)
        .execute(&pool)
        .await
        .expect("cleanup outbox");
    identity
}

#[tokio::test]
async fn pep_scopes_username_identity_into_lineage_audit() {
    // username 非空 → 标识 = username（不是 email，也不是 id）
    assert_eq!(
        run_case("340569801117730", "tester").await.as_deref(),
        Some("tester"),
        "经 PEP 的写路径应带上 username 作为操作者标识"
    );
}

#[tokio::test]
async fn pep_scopes_unique_fallback_when_username_absent() {
    // username 空 → 唯一回落 user:{id}（禁止共享常量/空值）
    assert_eq!(
        run_case("340569801117730", "").await.as_deref(),
        Some("user:340569801117730"),
        "无 username 时应回落唯一标识（user: 前缀 + id）"
    );
}

#[tokio::test]
async fn unscoped_write_keeps_row_without_identity() {
    // 作用域外（异步子任务等）：行仍须写入，标识允许为空（不因缺失丢事件）
    let pool = ::common::testing::connect_test_db().await;
    let table = format!(
        "test_audit_unscoped_{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    crud::audit_outbox::enqueue(
        &pool,
        &crud::audit_outbox::OutboxEvent::new(
            table.clone(),
            1,
            crud::audit_outbox::AuditAction::Insert,
        ),
    )
    .await
    .expect("enqueue outside scope");
    assert_eq!(operator_identity(&pool, &table).await, None);
    sqlx::query("DELETE FROM isahl_audit.audit_outbox WHERE table_name = $1")
        .bind(&table)
        .execute(&pool)
        .await
        .expect("cleanup");
}
