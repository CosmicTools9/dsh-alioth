//! 连通性探测（统一 seam）集成测试（change unify-provider-probe）
//!
//! 直调 crate `probe_category` handler（FakeProbe 注入 `ProbeRegistry`）：
//! - `{id}` 已存行：经 `config_resolution::load_by_id` 读取（凭证过解密器）、`config` 覆盖字段合并；
//! - `{config}` 草稿：敏感键按 schema 分流进 credentials、其余留 settings；
//! - 未支持分类 ⇒ 400；无 `RequestContext` ⇒ 401。
//!
//! fixture 自建自清；探测本身零副作用（FakeProbe 只记录调用）。

use actix_web::body::to_bytes;
use actix_web::{web, HttpMessage, HttpRequest};
use async_trait::async_trait;
use common::context::RequestContext;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use system_config::handlers::probe_category;
use system_config::probe::{ProbeOutcome, ProbeRegistry, ProviderProbe};
use system_config::CreateSystemConfigRequest;

use ::common::testing::connect_test_db;
use alioth_gateway::system_config_repo::SystemConfigRepo;

const CODE: &str = "test:probe-config";
const TEST_USER_ID: i64 = 9_900_422;

/// 调用记录：`(provider, settings, credentials)`
#[derive(Clone, Default)]
struct Recorder {
    calls: Arc<Mutex<Vec<(String, Value, Value)>>>,
}

#[derive(Clone)]
struct FakeProbe {
    rec: Recorder,
}

#[async_trait]
impl ProviderProbe for FakeProbe {
    async fn probe(&self, provider: &str, settings: &Value, credentials: &Value) -> ProbeOutcome {
        self.rec.calls.lock().expect("recorder lock").push((
            provider.to_string(),
            settings.clone(),
            credentials.clone(),
        ));
        ProbeOutcome::ok("probe-ok")
    }
}

fn authed_req(username: &str) -> HttpRequest {
    let req = actix_web::test::TestRequest::default().to_http_request();
    req.extensions_mut().insert(RequestContext::with_username(
        TEST_USER_ID,
        format!("{username}@test.local"),
        username,
    ));
    req
}

async fn cleanup(pool: &sqlx::PgPool) {
    sqlx::query(r#"DELETE FROM isahl."zc_id_prot-llm_config" WHERE code = $1"#)
        .bind(CODE)
        .execute(pool)
        .await
        .ok();
}

async fn body_json(resp: actix_web::HttpResponse) -> Value {
    let bytes = to_bytes(resp.into_body()).await.expect("response body");
    serde_json::from_slice(&bytes).expect("json body")
}

#[tokio::test]
async fn probe_seam_end_to_end() {
    let pool = connect_test_db().await;
    cleanup(&pool).await;

    let rec = Recorder::default();
    let probes = web::Data::new(
        ProbeRegistry::new()
            .with("llm", FakeProbe { rec: rec.clone() })
            .with("email", FakeProbe { rec: rec.clone() }),
    );
    let pool_data = web::Data::new(pool.clone());

    // ── fixture：已存 LLM 行（enc_fields.api_key 为 enc: 形态；本测试用非合法密文验证「过解密器」路径）──
    let repo = SystemConfigRepo::new(pool.clone());
    let created = system_config::SystemConfigRepository::insert(
        &repo,
        &CreateSystemConfigRequest {
            notice: Some("探测夹具".to_string()),
            code: Some(CODE.to_string()),
            _f_: Some("llm".to_string()),
            _t_: Some("deepseek".to_string()),
            comments: None,
            credentials: Some(json!({"api_key": "enc:fixture-cipher"})),
            settings: Some(json!({"enabled": true, "is_default": true, "model": "m-base"})),
            enabled: true,
            is_default: true,
            domain_: None,
        },
        Some(TEST_USER_ID),
    )
    .await
    .expect("insert fixture");

    // ── 1. 已存行：凭证过解密器 + config 覆盖合并（不落库）──
    let resp = probe_category(
        pool_data.clone(),
        probes.clone(),
        authed_req("probe-admin"),
        web::Path::from("llm".to_string()),
        web::Json(system_config::handlers::ProbeRequest {
            id: Some(created.id),
            config: Some(json!({"model": "m-override"})),
        }),
    )
    .await
    .expect("probe by id must succeed");
    assert_eq!(resp.status(), 200);
    let envelope = body_json(resp).await;
    assert_eq!(envelope["data"]["success"], json!(true));
    assert_eq!(envelope["data"]["message"], json!("probe-ok"));

    {
        let calls = rec.calls.lock().expect("recorder lock");
        let (provider, settings, credentials) = calls.last().expect("probe recorded");
        assert_eq!(provider, "deepseek");
        assert_eq!(
            settings["model"],
            json!("m-override"),
            "config 覆盖字段必须合并"
        );
        assert_eq!(
            credentials["api_key"],
            json!("enc:fixture-cipher"),
            "凭证必须过解密器（坏密文回退原值）"
        );
    }
    // 覆盖不落库（guard 已释放后再 await）
    let row = system_config::SystemConfigRepository::find_by_id(&repo, created.id)
        .await
        .expect("reread")
        .expect("row exists");
    assert_eq!(row.settings.unwrap()["model"], json!("m-base"));

    // ── 2. 草稿：敏感键分流（password → credentials）──
    let resp = probe_category(
        pool_data.clone(),
        probes.clone(),
        authed_req("probe-admin"),
        web::Path::from("email".to_string()),
        web::Json(system_config::handlers::ProbeRequest {
            id: None,
            config: Some(json!({
                "provider": "smtp",
                "smtp_host": "smtp.example.com",
                "smtp_port": 587,
                "password": "p@ss"
            })),
        }),
    )
    .await
    .expect("draft probe must succeed");
    assert_eq!(resp.status(), 200);
    {
        let calls = rec.calls.lock().expect("recorder lock");
        let (provider, settings, credentials) = calls.last().expect("probe recorded");
        assert_eq!(provider, "smtp");
        assert_eq!(
            credentials["password"],
            json!("p@ss"),
            "敏感键必须进 credentials"
        );
        assert!(
            settings.get("password").is_none(),
            "settings 不得残留敏感键"
        );
        assert_eq!(settings["smtp_host"], json!("smtp.example.com"));
    }

    // ── 3. 未支持分类 ⇒ 400 ──
    let err = probe_category(
        pool_data.clone(),
        probes.clone(),
        authed_req("probe-admin"),
        web::Path::from("sms".to_string()),
        web::Json(system_config::handlers::ProbeRequest {
            id: None,
            config: Some(json!({"provider": "aliyun"})),
        }),
    )
    .await
    .expect_err("unsupported category must fail");
    assert!(
        matches!(err, common::AliothError::BadRequest(_)),
        "应为 400：{err:?}"
    );

    // ── 4. 无上下文 ⇒ 401 ──
    let bare = actix_web::test::TestRequest::default().to_http_request();
    let err = probe_category(
        pool_data.clone(),
        probes.clone(),
        bare,
        web::Path::from("llm".to_string()),
        web::Json(system_config::handlers::ProbeRequest {
            id: Some(created.id),
            config: None,
        }),
    )
    .await
    .expect_err("no context must fail");
    assert!(
        matches!(err, common::AliothError::Unauthorized(_)),
        "应为 401：{err:?}"
    );

    cleanup(&pool).await;
}
