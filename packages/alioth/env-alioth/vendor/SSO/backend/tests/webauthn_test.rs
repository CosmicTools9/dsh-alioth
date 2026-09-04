//! WebAuthn / Passkey 集成测试（使用内存模拟认证器）
//!
//! 模拟认证器自行构造有效的 attestation / assertion：
//! - 注册（"none" attestation）只需合法的 authData + CBOR，无需签名；
//! - 认证需要以私钥对 `authData || SHA256(clientDataJSON)` 做 ES256 签名。
//!
//! 服务器按 webauthn-rs 0.5 的公开 API 校验，登录需已知用户（邮箱定位）。

mod common;

use actix_web::{http::header, test, web, App};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use gateway_sso::auth::jwt::{encode_access_token, Claims};
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::SigningKey;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

const RP_ID: &str = "sso.localhost";
const ORIGIN: &str = "http://sso.localhost:8080";

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    let o = h.finalize();
    let mut a = [0u8; 32];
    a.copy_from_slice(&o);
    a
}

fn b64(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

/// 编码一段字节为 CBOR byte string（支持 1/2/4 字节长度前缀）
fn cbor_bytes(data: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    match data.len() {
        n if n <= 23 => v.push(0x40 | n as u8),
        n if n <= 0xFF => {
            v.push(0x58);
            v.push(n as u8);
        }
        n if n <= 0xFFFF => {
            v.push(0x59);
            v.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            v.push(0x5A);
            v.extend_from_slice(&(n as u32).to_be_bytes());
        }
    }
    v.extend_from_slice(data);
    v
}

/// EC2 P-256 / ES256 的 COSE 公钥
fn encode_cose_ec2(x: &[u8; 32], y: &[u8; 32]) -> Vec<u8> {
    let mut v = vec![
        0xa5, // map(5)
        0x01, 0x02, // 1 => 2 (kty EC2)
        0x03, 0x26, // 3 => -7 (alg ES256)
        0x20, 0x01, // -1 => 1 (crv P-256)
        0x21,
    ];
    v.extend(cbor_bytes(x)); // -2 => x
    v.push(0x22);
    v.extend(cbor_bytes(y)); // -3 => y
    v
}

fn encode_attestation_object(auth_data: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.push(0xa3); // map(3)
    v.push(0x63);
    v.extend_from_slice(b"fmt");
    v.push(0x64);
    v.extend_from_slice(b"none");
    v.push(0x67);
    v.extend_from_slice(b"attStmt");
    v.push(0xa0);
    v.push(0x68);
    v.extend_from_slice(b"authData");
    v.extend(cbor_bytes(auth_data));
    v
}

fn make_register_auth_data(rp_id_hash: &[u8; 32], cred_id: &[u8], cose: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(rp_id_hash);
    v.push(0x45); // flags: UP | UV | AT
    v.extend_from_slice(&0u32.to_be_bytes());
    v.extend_from_slice(&[0u8; 16]); // aaguid
    v.extend_from_slice(&(cred_id.len() as u16).to_be_bytes());
    v.extend_from_slice(cred_id);
    v.extend_from_slice(cose);
    v
}

fn make_auth_auth_data(rp_id_hash: &[u8; 32]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(rp_id_hash);
    v.push(0x05); // flags: UP | UV
    v.extend_from_slice(&0u32.to_be_bytes());
    v
}

/// 内存模拟认证器（单个凭据）
struct FakeAuth {
    signing_key: SigningKey,
    cred_id: Vec<u8>,
}

impl FakeAuth {
    fn new() -> Self {
        let bytes = rand::random::<[u8; 32]>();
        let signing_key = SigningKey::from_slice(&bytes).expect("valid signing key");
        let mut cred_id = [0u8; 32];
        for b in &mut cred_id {
            *b = rand::random::<u8>();
        }
        FakeAuth {
            signing_key,
            cred_id: cred_id.to_vec(),
        }
    }

    fn rp_id_hash(&self) -> [u8; 32] {
        sha256(RP_ID.as_bytes())
    }

    fn coords(&self) -> ([u8; 32], [u8; 32]) {
        let pt = self.signing_key.verifying_key().to_sec1_point(false);
        let bytes = pt.as_bytes();
        eprintln!(
            "[debug] sec1 len={} first_byte={:02x}",
            bytes.len(),
            bytes.first().copied().unwrap_or(0)
        );
        let mut x = [0u8; 32];
        let mut y = [0u8; 32];
        x.copy_from_slice(&bytes[1..33]);
        y.copy_from_slice(&bytes[33..65]);
        (x, y)
    }

    fn register_response(&self, challenge_b64: &str) -> Value {
        let (x, y) = self.coords();
        let cose = encode_cose_ec2(&x, &y);
        let auth_data = make_register_auth_data(&self.rp_id_hash(), &self.cred_id, &cose);
        let attestation = encode_attestation_object(&auth_data);
        let client_data = format!(
            r#"{{"type":"webauthn.create","challenge":"{}","origin":"{}","crossOrigin":false}}"#,
            challenge_b64, ORIGIN
        );
        let cred_id_b64 = b64(&self.cred_id);
        json!({
            "id": cred_id_b64,
            "rawId": cred_id_b64,
            "type": "public-key",
            "response": {
                "clientDataJSON": b64(client_data.as_bytes()),
                "attestationObject": b64(&attestation),
                "transports": ["internal"]
            }
        })
    }

    fn auth_response(&self, challenge_b64: &str) -> Value {
        let auth_data = make_auth_auth_data(&self.rp_id_hash());
        let client_data = format!(
            r#"{{"type":"webauthn.get","challenge":"{}","origin":"{}","crossOrigin":false}}"#,
            challenge_b64, ORIGIN
        );
        let client_data_b64 = b64(client_data.as_bytes());
        let client_data_hash = sha256(client_data.as_bytes());
        let mut msg = auth_data.clone();
        msg.extend_from_slice(&client_data_hash);
        let sig: p256::ecdsa::Signature = self.signing_key.sign(&msg);
        // WebAuthn 标准：raw 64 字节 r||s（真实浏览器语义；
        // webauthn-rs-core 的 raw→DER 兼容由 vendor/webauthn-rs-core patch 提供）。
        let sig_bytes = sig.to_bytes();
        eprintln!("[debug] sig len={}", sig_bytes.len());
        let vk = self.signing_key.verifying_key();
        match vk.verify(&msg, &sig) {
            Ok(_) => eprintln!("[debug] LOCAL verify(msg) OK"),
            Err(e) => eprintln!("[debug] LOCAL verify(msg) FAILED: {:?}", e),
        }
        let pre = sha256(&msg);
        match vk.verify(&pre, &sig) {
            Ok(_) => eprintln!("[debug] verify(prehash) OK -> p256 does NOT hash"),
            Err(_) => eprintln!("[debug] verify(prehash) FAILED -> p256 hashes"),
        }
        let cred_id_b64 = b64(&self.cred_id);
        json!({
            "id": cred_id_b64,
            "rawId": cred_id_b64,
            "type": "public-key",
            "response": {
                "authenticatorData": b64(&auth_data),
                "clientDataJSON": client_data_b64,
                "signature": b64(&sig_bytes)
            }
        })
    }
}

fn challenge_from_options(body: &[u8]) -> String {
    let v: Value = serde_json::from_slice(body).expect("options should be json");
    v["publicKey"]["challenge"]
        .as_str()
        .expect("challenge should be present")
        .to_string()
}

async fn setup_pool() -> PgPool {
    // 统一走共享测试库连接（含 OS 用户注入），避免 postgres://localhost/... 被
    // sqlx 解析为 anonymous 角色导致连接失败（与 admin_api_test 一致）。
    ::common::testing::connect_test_db().await
}

async fn create_user(pool: &PgPool, email: &str) -> i64 {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.auth_users (name, username, email, password_hash, status, is_active, created_at, updated_at) \
         VALUES ($1, $2, $3, 'dummyhash', 'active', TRUE, NOW(), NOW()) RETURNING id",
    )
    .bind(email)
    .bind(email)
    .bind(email)
    .fetch_one(pool)
    .await
    .expect("Failed to create test user");
    id
}

/// 构建测试 app（返回 `impl Service`，类型由各调用点推断，避免显式命名 `actix_http::Request`）
macro_rules! build_app {
    ($pool:expr) => {{
        let auth_state = common::test_auth_state();
        test::init_service(
            App::new()
                .app_data(web::Data::new($pool.clone()))
                .app_data(web::Data::new(auth_state))
                .service(web::scope("/auth").configure(gateway_sso::auth::webauthn::configure)),
        )
    }};
}

fn auth_header(sub: i64, auth_state: &gateway_sso::auth::AuthState) -> String {
    let claims = Claims::with_expiry_seconds(&sub.to_string(), "", false, 900);
    let token = encode_access_token(&claims, &auth_state.jwt_private_key).expect("token");
    format!("Bearer {}", token)
}

#[tokio::test]
async fn test_passkey_register_and_login_roundtrip() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema");
    common::cleanup_test_users(&pool).await.ok();

    let email = "webauthn-roundtrip@alioth.test";
    common::cleanup_user_by_email(&pool, email).await.ok();
    let user_id = create_user(&pool, email).await;

    let app = build_app!(pool.clone()).await;
    let auth_state = common::test_auth_state();
    let bearer = auth_header(user_id, &auth_state);

    let fake = FakeAuth::new();

    // 1. 注册开始
    let begin_req = test::TestRequest::post()
        .uri("http://sso.localhost:8080/auth/webauthn/register/begin")
        .insert_header((header::AUTHORIZATION, bearer.clone()))
        .set_json(json!({"credential_type": "passkey"}))
        .to_request();
    let begin_resp = test::call_service(&app, begin_req).await;
    assert_eq!(begin_resp.status(), 200, "register begin should succeed");
    let begin_body = test::read_body(begin_resp).await;
    let challenge = challenge_from_options(&begin_body);

    // 2. 注册完成
    let reg_body = fake.register_response(&challenge);
    let complete_req = test::TestRequest::post()
        .uri("http://sso.localhost:8080/auth/webauthn/register/complete")
        .insert_header((header::AUTHORIZATION, bearer.clone()))
        .set_json(reg_body)
        .to_request();
    let complete_resp = test::call_service(&app, complete_req).await;
    assert_eq!(
        complete_resp.status(),
        200,
        "register complete should succeed"
    );

    // DEBUG: inspect stored credential
    let row: (Vec<u8>,) = sqlx::query_as::<_, (Vec<u8>,)>(
        "SELECT public_key_cose FROM isahl_auth.webauthn_credentials WHERE credential_id = $1",
    )
    .bind(&fake.cred_id)
    .fetch_one(&pool)
    .await
    .expect("fetch stored cred");
    eprintln!("[debug] stored blob len={}", row.0.len());
    let v: serde_json::Value = serde_json::from_slice(&row.0).expect("parse blob as value");
    let dump = serde_json::to_string(&v).unwrap();
    eprintln!("[debug] blob head: {}", &dump[..dump.len().min(400)]);
    let (cx, cy) = fake.coords();
    let my_cose = encode_cose_ec2(&cx, &cy);
    eprintln!("[debug] my_cose len={}", my_cose.len());
    eprintln!("[debug] my cred_id b64 = {}", b64(&fake.cred_id));
    eprintln!("[debug] my x b64 = {}", b64(&cx));
    eprintln!("[debug] my y b64 = {}", b64(&cy));

    // DIAGNOSTIC: reload stored credential, ensure COSE key round-trips and validates.
    {
        let stored: (Vec<u8>,) = sqlx::query_as(
            "SELECT public_key_cose FROM isahl_auth.webauthn_credentials WHERE credential_id = $1",
        )
        .bind(fake.cred_id.clone())
        .fetch_one(&pool)
        .await
        .expect("fetch stored credential");
        let pk: webauthn_rs::prelude::Passkey =
            serde_json::from_slice(&stored.0).expect("deserialize Passkey");
        pk.get_public_key()
            .get_openssl_pkey()
            .expect("DIAGNOSTIC: stored COSE key failed to build OpenSSL key after round-trip");
        eprintln!("DIAGNOSTIC: stored COSE key validates OK");
    }

    // 3. 登录开始（按邮箱定位用户）
    let login_begin_req = test::TestRequest::post()
        .uri("http://sso.localhost:8080/auth/webauthn/login/begin")
        .set_json(json!({"email": email}))
        .to_request();
    let login_begin_resp = test::call_service(&app, login_begin_req).await;
    assert_eq!(login_begin_resp.status(), 200, "login begin should succeed");
    let login_begin_body = test::read_body(login_begin_resp).await;
    let login_challenge = challenge_from_options(&login_begin_body);

    // 4. 登录完成
    let auth_body = fake.auth_response(&login_challenge);

    // DIAGNOSTIC: locally re-derive verification_data and verify the test's own signature.
    {
        use p256::ecdsa::signature::Verifier as _;
        let ad = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(auth_body["response"]["authenticatorData"].as_str().unwrap())
            .unwrap();
        let cd = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(auth_body["response"]["clientDataJSON"].as_str().unwrap())
            .unwrap();
        let mut vd = ad.clone();
        vd.extend_from_slice(&sha256(&cd));
        let sig_raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(auth_body["response"]["signature"].as_str().unwrap())
            .unwrap();
        let sig = p256::ecdsa::Signature::from_slice(&sig_raw).unwrap();
        let local_ok = fake.signing_key.verifying_key().verify(&vd, &sig).is_ok();
        eprintln!(
            "DIAGNOSTIC local verify={} sig_len={} ad_len={} cd_len={}",
            local_ok,
            sig_raw.len(),
            ad.len(),
            cd.len()
        );
    }

    let login_complete_req = test::TestRequest::post()
        .uri("http://sso.localhost:8080/auth/webauthn/login/complete")
        .set_json(auth_body)
        .to_request();
    let login_complete_resp = test::call_service(&app, login_complete_req).await;
    let status = login_complete_resp.status();
    let login_complete_body = test::read_body(login_complete_resp).await;
    if status != 200 {
        panic!(
            "login complete failed: status={} body={}",
            status,
            String::from_utf8_lossy(&login_complete_body)
        );
    }
    let v: Value = serde_json::from_slice(&login_complete_body).unwrap();
    assert!(
        v["access_token"].as_str().is_some(),
        "login complete should return access_token"
    );

    common::cleanup_user_by_email(&pool, email).await.ok();
}

#[tokio::test]
async fn test_duplicate_credential_rejected() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema");
    common::cleanup_test_users(&pool).await.ok();

    let email = "webauthn-dup@alioth.test";
    common::cleanup_user_by_email(&pool, email).await.ok();
    let user_id = create_user(&pool, email).await;

    let app = build_app!(pool.clone()).await;
    let auth_state = common::test_auth_state();
    let bearer = auth_header(user_id, &auth_state);
    let fake = FakeAuth::new();

    let begin_req = test::TestRequest::post()
        .uri("http://sso.localhost:8080/auth/webauthn/register/begin")
        .insert_header((header::AUTHORIZATION, bearer.clone()))
        .set_json(json!({"credential_type": "passkey"}))
        .to_request();
    let begin_body = test::read_body(test::call_service(&app, begin_req).await).await;
    let challenge = challenge_from_options(&begin_body);

    let reg_body = fake.register_response(&challenge);
    let complete_req = test::TestRequest::post()
        .uri("http://sso.localhost:8080/auth/webauthn/register/complete")
        .insert_header((header::AUTHORIZATION, bearer.clone()))
        .set_json(reg_body)
        .to_request();
    assert_eq!(
        test::call_service(&app, complete_req).await.status(),
        200,
        "first registration should succeed"
    );

    // 第二次用同一凭据注册 —— 应被拒绝（409）
    let begin_req2 = test::TestRequest::post()
        .uri("http://sso.localhost:8080/auth/webauthn/register/begin")
        .insert_header((header::AUTHORIZATION, bearer.clone()))
        .set_json(json!({"credential_type": "passkey"}))
        .to_request();
    let begin_body2 = test::read_body(test::call_service(&app, begin_req2).await).await;
    let challenge2 = challenge_from_options(&begin_body2);
    let reg_body2 = fake.register_response(&challenge2);
    let complete_req2 = test::TestRequest::post()
        .uri("http://sso.localhost:8080/auth/webauthn/register/complete")
        .insert_header((header::AUTHORIZATION, bearer))
        .set_json(reg_body2)
        .to_request();
    let status = test::call_service(&app, complete_req2).await.status();
    assert_eq!(status, 409, "duplicate credential should be rejected");

    common::cleanup_user_by_email(&pool, email).await.ok();
}

#[tokio::test]
async fn test_unknown_credential_rejected() {
    let pool = setup_pool().await;
    common::setup_schema(&pool).await.expect("schema");
    common::cleanup_test_users(&pool).await.ok();

    let email = "webauthn-unknown@alioth.test";
    common::cleanup_user_by_email(&pool, email).await.ok();
    let user_id = create_user(&pool, email).await;

    let app = build_app!(pool.clone()).await;
    let auth_state = common::test_auth_state();
    let bearer = auth_header(user_id, &auth_state);

    // 注册一个合法凭据
    let fake = FakeAuth::new();
    let begin_req = test::TestRequest::post()
        .uri("http://sso.localhost:8080/auth/webauthn/register/begin")
        .insert_header((header::AUTHORIZATION, bearer.clone()))
        .set_json(json!({"credential_type": "passkey"}))
        .to_request();
    let begin_body = test::read_body(test::call_service(&app, begin_req).await).await;
    let challenge = challenge_from_options(&begin_body);
    let reg_body = fake.register_response(&challenge);
    let complete_req = test::TestRequest::post()
        .uri("http://sso.localhost:8080/auth/webauthn/register/complete")
        .insert_header((header::AUTHORIZATION, bearer.clone()))
        .set_json(reg_body)
        .to_request();
    assert_eq!(
        test::call_service(&app, complete_req).await.status(),
        200,
        "registration should succeed"
    );

    // 登录开始（基于该用户凭据）
    let login_begin_req = test::TestRequest::post()
        .uri("http://sso.localhost:8080/auth/webauthn/login/begin")
        .set_json(json!({"email": email}))
        .to_request();
    let login_begin_body = test::read_body(test::call_service(&app, login_begin_req).await).await;
    let login_challenge = challenge_from_options(&login_begin_body);

    // 用一个不同的（未知）凭据提交 assertion —— 应被拒绝
    let attacker = FakeAuth::new();
    let auth_body = attacker.auth_response(&login_challenge);
    let login_complete_req = test::TestRequest::post()
        .uri("http://sso.localhost:8080/auth/webauthn/login/complete")
        .set_json(auth_body)
        .to_request();
    let status = test::call_service(&app, login_complete_req).await.status();
    assert_ne!(status, 200, "unknown credential must be rejected");

    common::cleanup_user_by_email(&pool, email).await.ok();
}
