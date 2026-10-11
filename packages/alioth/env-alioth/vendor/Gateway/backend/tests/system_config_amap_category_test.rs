//! amap 分类收编（change amap-system-config-category）集成测试：
//!
//! 1. **env 族 code 作用域互斥**：`integration:amap` 行经 list/find 路由为 `amap` 分类，
//!    MUST NOT 标为 `approval`（修复前 list 的 UNION 以 approval 标签覆盖整张 env 表）；
//!    `approval:%` 行保持 `approval`；
//! 2. **探测草稿分流**：amap 分类敏感键集为空（key/secret 落 settings 明文，构建期
//!    消费链直读）⇒ 草稿探测时 key MUST 留在 settings、不进 credentials。
//!
//! fixture 自建自清；探测用 FakeProbe（零外联、只记录调用）。

use actix_web::body::to_bytes;
use actix_web::{web, HttpMessage, HttpRequest};
use async_trait::async_trait;
use common::context::RequestContext;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use system_config::handlers::probe_category;
use system_config::probe::{ProbeOutcome, ProbeRegistry, ProviderProbe};

use ::common::testing::connect_test_db;
use alioth_gateway::system_config_repo::SystemConfigRepo;

const AMAP_CODE: &str = "integration:amap";
const APPROVAL_CODE: &str = "approval:scope-test";
const TEST_USER_ID: i64 = 9_900_432;

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
        ProbeOutcome::ok("amap-ok")
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
    sqlx::query(r#"DELETE FROM isahl."zc_id_prot-env_config" WHERE code = ANY($1)"#)
        .bind(vec![AMAP_CODE, APPROVAL_CODE])
        .execute(pool)
        .await
        .ok();
}

async fn body_json(resp: actix_web::HttpResponse) -> Value {
    let bytes = to_bytes(resp.into_body()).await.expect("response body");
    serde_json::from_slice(&bytes).expect("json body")
}

#[tokio::test]
async fn amap_env_family_code_scoping_and_probe_draft() {
    let pool = connect_test_db().await;
    cleanup(&pool).await;
    let repo = SystemConfigRepo::new(pool.clone());

    // ── fixture：env 表一行 amap + 一行 approval（scope-test 专用 code，不碰 auto-approve）──
    system_config::SystemConfigRepository::insert(
        &repo,
        &system_config::CreateSystemConfigRequest {
            notice: Some("amap 夹具".to_string()),
            code: Some(AMAP_CODE.to_string()),
            _f_: Some("amap".to_string()),
            _t_: Some("amap".to_string()),
            comments: None,
            credentials: None,
            settings: Some(json!({"key": "k-fix", "secret": "s-fix", "enabled": true})),
            enabled: true,
            is_default: false,
            domain_: None,
        },
        Some(TEST_USER_ID),
    )
    .await
    .expect("insert amap fixture");

    system_config::SystemConfigRepository::insert(
        &repo,
        &system_config::CreateSystemConfigRequest {
            notice: Some("approval 夹具".to_string()),
            code: Some(APPROVAL_CODE.to_string()),
            _f_: Some("approval".to_string()),
            _t_: Some("platform".to_string()),
            comments: None,
            credentials: None,
            settings: Some(json!({"enabled": false})),
            enabled: false,
            is_default: false,
            domain_: None,
        },
        Some(TEST_USER_ID),
    )
    .await
    .expect("insert approval fixture");

    // ── 1. list 按 code 作用域打标：amap 行 ≠ approval 行 ──
    let all = system_config::SystemConfigRepository::list(&repo, 200, 0)
        .await
        .expect("list");
    let amap_rows: Vec<&system_config::SystemConfig> = all
        .iter()
        .filter(|c| c._f_.as_deref() == Some("amap"))
        .collect();
    let approval_rows: Vec<&system_config::SystemConfig> = all
        .iter()
        .filter(|c| c._f_.as_deref() == Some("approval"))
        .collect();
    assert!(
        amap_rows
            .iter()
            .any(|c| c.code.as_deref() == Some(AMAP_CODE)),
        "integration:amap 行 MUST 列于 amap 分类"
    );
    assert!(
        !approval_rows
            .iter()
            .any(|c| c.code.as_deref() == Some(AMAP_CODE)),
        "integration:amap 行 MUST NOT 混入 approval 分类（修复前的缺陷形态）"
    );
    assert!(
        approval_rows
            .iter()
            .any(|c| c.code.as_deref() == Some(APPROVAL_CODE)),
        "approval:% 行 MUST 列于 approval 分类"
    );
    assert!(
        !amap_rows
            .iter()
            .any(|c| c.code.as_deref() == Some(APPROVAL_CODE)),
        "approval:% 行 MUST NOT 混入 amap 分类"
    );

    // ── 2. find_by_code 按作用域路由 ──
    let amap_by_code = system_config::SystemConfigRepository::find_by_code(&repo, AMAP_CODE)
        .await
        .expect("find amap")
        .expect("amap row exists");
    assert_eq!(amap_by_code._f_.as_deref(), Some("amap"));
    let appr_by_code = system_config::SystemConfigRepository::find_by_code(&repo, APPROVAL_CODE)
        .await
        .expect("find approval")
        .expect("approval row exists");
    assert_eq!(appr_by_code._f_.as_deref(), Some("approval"));

    // ── 3. 草稿探测：amap 敏感键集为空 ⇒ key 留 settings、不进 credentials ──
    let rec = Recorder::default();
    let probes = web::Data::new(ProbeRegistry::new().with("amap", FakeProbe { rec: rec.clone() }));
    let pool_data = web::Data::new(pool.clone());
    let resp = probe_category(
        pool_data,
        probes,
        authed_req("amap-admin"),
        web::Path::from("amap".to_string()),
        web::Json(system_config::handlers::ProbeRequest {
            id: None,
            config: Some(json!({"provider": "amap", "key": "k-draft", "secret": "s-draft"})),
        }),
    )
    .await
    .expect("amap draft probe must succeed");
    assert_eq!(resp.status(), 200);
    let envelope = body_json(resp).await;
    assert_eq!(envelope["data"]["success"], json!(true));
    {
        let calls = rec.calls.lock().expect("recorder lock");
        let (provider, settings, credentials) = calls.last().expect("probe recorded");
        assert_eq!(provider, "amap");
        assert_eq!(
            settings["key"],
            json!("k-draft"),
            "key MUST 留在 settings（明文承载）"
        );
        assert_eq!(settings["secret"], json!("s-draft"));
        assert!(
            credentials
                .as_object()
                .map(|o| o.is_empty())
                .unwrap_or(true),
            "amap 零敏感键 ⇒ credentials MUST 为空"
        );
    }

    cleanup(&pool).await;
}
