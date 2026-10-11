//! 发起收款端点集成测试（change `add-openapi-payment-channel` 第一阶段）
//!
//! 覆盖（禁真实微信调用——`PaymentChannel` 缝注入 stub）：
//!   1. 金额取自**计费聚合**（usage × 产品行 × 价标）；请求体含金额字段 MUST NOT 被采信；
//!   2. `out_trade_no = openapi-<client_id>-<month>`、`attach = {"client_id","month"}` 形态；
//!   3. 流水落 `ext_wechat_pay.transaction`（attach/金额/状态/prepay 句柄）；
//!   4. 未定价调用 ⇒ `409`（MUST NOT 以不完整金额发起）；零额 ⇒ `409`；
//!   5. 缺注册行 / 渠道不可用 ⇒ `503` 可读错误（MUST NOT 静默降级）；
//!   6. `notify_url` 注入：配置公开回调基址 ⇒ 下单形态 `{base}/api/openapi/billing/notify`；
//!      未配置 ⇒ `503` fail-loud（MUST NOT 静默留空 / 伪造域名）。
//!
//! 依赖真实测试库（`aliothstudio_test`）。测试数据自建自清；产品行写入遵守叶表坐标契约
//! （`ontology_binding::resolve`，禁硬编码 ZUID）。

use std::sync::Mutex;

use async_trait::async_trait;
use common::AliothError;
use sqlx::PgPool;

use alioth_gateway::openapi::payment::{
    initiate_payment, process_billing_notify, ChannelOrder, ChannelOrderResult, ChannelPrepay,
    PayRequest, PaymentChannel, WechatPayChannel,
};

/// 公开回调基址（测试注入；带尾斜杠以同时覆盖归一化）。
const TEST_PUBLIC_BASE: &str = "https://pay.example.test/";

// ============================================================================
// stub 渠道（唯一被抽象的是网络下单调用）
// ============================================================================

#[derive(Default)]
struct StubChannel {
    captured: Mutex<Vec<ChannelOrder>>,
}

impl StubChannel {
    fn orders(&self) -> Vec<ChannelOrder> {
        self.captured.lock().expect("stub mutex").clone()
    }
}

#[async_trait]
impl PaymentChannel for StubChannel {
    fn channel_code(&self) -> &str {
        "stub-wechat-pay"
    }

    async fn create_order(&self, req: &ChannelOrder) -> Result<ChannelOrderResult, AliothError> {
        self.captured.lock().expect("stub mutex").push(req.clone());
        Ok(ChannelOrderResult {
            prepay: ChannelPrepay::CodeUrl("weixin://wxpay/stub".to_string()),
            raw: serde_json::json!({ "code_url": "weixin://wxpay/stub" }),
        })
    }
}

// ============================================================================
// 夹具
// ============================================================================

struct Fixture {
    client_id: String,
    subscription_id: i64,
    service_user: i64,
    product_codes: Vec<String>,
}

async fn insert_price(pool: &PgPool, marker: &str, mark: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_scal-price\" (notice, code, mark) VALUES ($1, $2, $3::numeric) RETURNING id",
    )
    .bind(format!("pay-price-{}", marker))
    .bind(marker)
    .bind(mark)
    .fetch_one(pool)
    .await
    .expect("insert price scalar")
}

async fn insert_product(pool: &PgPool, code: &str, projection: &str, price_id: Option<i64>) -> i64 {
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("FF", "FJA", "↓.HA"))
        .await
        .expect("resolve coordinates");
    sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_prod-openapi-sales\"
           (notice, code, projection, qk_price, dk_scene, dk_factor, dk_function)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(format!("pay-product-{}", code))
    .bind(code)
    .bind(projection)
    .bind(price_id)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("insert product row")
}

async fn create_client(pool: &PgPool, marker: &str) -> Fixture {
    let client_id = format!("pay_test_{}", marker);
    let svc_user: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.auth_users
           (username, password_hash, user_type, is_active, status,
            created_at, updated_at, failed_login_attempts, notification_preferences)
         VALUES ($1, NULL, 'service', TRUE, 'active', NOW(), NOW(), 0, '{}'::jsonb)
         ON CONFLICT (username) DO UPDATE SET updated_at = NOW() RETURNING id",
    )
    .bind(format!("svc:{}", client_id))
    .fetch_one(pool)
    .await
    .expect("service user");
    let client_row: (i64,) = sqlx::query_as(
        "INSERT INTO isahl_auth.api_clients
           (client_id, client_type, client_name, secret_hash, scopes, fk_service_user, enabled)
         VALUES ($1, 'oauth2', 'pay-test', '', ARRAY[]::TEXT[], $2, TRUE) RETURNING id",
    )
    .bind(&client_id)
    .bind(svc_user)
    .fetch_one(pool)
    .await
    .expect("client");
    let plan: i64 = sqlx::query_scalar("SELECT id FROM isahl_auth.api_plans LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("a plan exists（套餐种子）");
    let sub: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.api_subscriptions (fk_client, fk_plan, status) \
         VALUES ($1, $2, 'active') RETURNING id",
    )
    .bind(client_row.0)
    .bind(plan)
    .fetch_one(pool)
    .await
    .expect("subscription");

    // 定价产品行（价格由调用方另插）
    let price_id = insert_price(pool, &format!("{}-priced", marker), "12.50").await;
    let priced_projection = format!("pay-{}/priced", marker);
    insert_product(
        pool,
        &format!("{}-priced", marker),
        &priced_projection,
        Some(price_id),
    )
    .await;

    Fixture {
        client_id,
        subscription_id: sub,
        service_user: svc_user,
        product_codes: vec![format!("{}-priced", marker)],
    }
}

async fn add_usage(
    pool: &PgPool,
    sub_id: i64,
    route: &str,
    times: i32,
    at: chrono::DateTime<chrono::Utc>,
) {
    for _ in 0..times {
        sqlx::query(
            "INSERT INTO isahl_auth.api_usage (fk_subscription, route, method, status, latency_ms, requested_at) \
             VALUES ($1, $2, 'GET', 200, 5, $3)",
        )
        .bind(sub_id)
        .bind(route)
        .bind(at)
        .execute(pool)
        .await
        .expect("insert usage");
    }
}

async fn cleanup(pool: &PgPool, fx: &Fixture, marker: &str, out_trade_no: &str) {
    let _ = sqlx::query("DELETE FROM ext_wechat_pay.transaction WHERE out_trade_no = $1")
        .bind(out_trade_no)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.api_usage WHERE fk_subscription = $1")
        .bind(fx.subscription_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.api_subscriptions WHERE id = $1")
        .bind(fx.subscription_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.api_clients WHERE client_id = $1")
        .bind(&fx.client_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(fx.service_user)
        .execute(pool)
        .await;
    for code in &fx.product_codes {
        let _ = sqlx::query("DELETE FROM isahl.\"zc_id_prod-openapi-sales\" WHERE code = $1")
            .bind(code)
            .execute(pool)
            .await;
    }
    let _ = sqlx::query("DELETE FROM isahl.\"zc_id_scal-price\" WHERE code = $1")
        .bind(format!("{}-priced", marker))
        .execute(pool)
        .await;
}

/// 上月 15 日 12:00（共享测试库中其它用例只写 NOW() ⇒ 该月无外部污染）。
fn target_month_and_time() -> (String, chrono::DateTime<chrono::Utc>) {
    use chrono::Datelike;
    let now = chrono::Utc::now();
    let prev_first = chrono::NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .expect("当月 1 日")
        .pred_opt()
        .expect("上月最后一天");
    let month = format!("{:04}-{:02}", prev_first.year(), prev_first.month());
    let at = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
        prev_first
            .with_day(15)
            .expect("15 日")
            .and_hms_opt(12, 0, 0)
            .expect("12:00"),
        chrono::Utc,
    );
    (month, at)
}

fn marker() -> String {
    format!(
        "p{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        std::process::id()
    )
}

// ============================================================================
// 用例
// ============================================================================

/// 金额取自计费聚合；`out_trade_no`/`attach` 形态；流水落库；请求体金额无效。
#[tokio::test]
async fn pay_uses_billing_amount_and_persists_transaction_row() {
    let pool = ::common::testing::connect_test_db().await;
    wechat_pay_adapter::ensure_wechat_pay_schema(&pool)
        .await
        .expect("ext_wechat_pay 表族自愈建表");

    let marker = marker();
    let fx = create_client(&pool, &marker).await;
    let (month, at) = target_month_and_time();
    // 3 次调用 × 12.50 = 37.50 ⇒ 3750 分
    add_usage(
        &pool,
        fx.subscription_id,
        &format!("/api/service/pay-{}/priced", marker),
        3,
        at,
    )
    .await;

    // 请求体**故意**带金额字段（含蛇形变体）——MUST NOT 被采信（结构上无该字段）
    let req: PayRequest = serde_json::from_value(serde_json::json!({
        "client_id": fx.client_id,
        "month": month,
        "amount": 99999,
        "amount_total": 99999,
    }))
    .expect("未知字段被忽略");

    let stub = StubChannel::default();
    let resp = initiate_payment(&pool, &stub, req, Some(TEST_PUBLIC_BASE))
        .await
        .expect("发起收款应成功");

    // 权威金额来自聚合（非请求体）
    assert_eq!(resp.amount_total, 3750, "金额 = 3 × 12.50 元 = 3750 分");
    assert_eq!(resp.currency, "CNY");
    let expected_trade_no = format!("openapi-{}-{}", fx.client_id, month);
    assert_eq!(resp.out_trade_no, expected_trade_no);
    assert_eq!(resp.channel, "stub-wechat-pay");
    assert_eq!(resp.month, month);
    assert_eq!(
        resp.prepay["code_url"].as_str(),
        Some("weixin://wxpay/stub")
    );

    // attach 形态
    let attach: serde_json::Value = serde_json::from_str(&resp.attach).expect("attach 为 JSON 串");
    assert_eq!(attach["client_id"].as_str(), Some(fx.client_id.as_str()));
    assert_eq!(attach["month"].as_str(), Some(month.as_str()));

    // 渠道收到同一形态（金额 MUST NOT 由渠道改写）
    let orders = stub.orders();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].out_trade_no, expected_trade_no);
    assert_eq!(orders[0].amount_total, 3750);
    assert_eq!(orders[0].attach, resp.attach);
    assert_eq!(orders[0].currency, "CNY");
    // notify_url 由公开回调基址推导（尾斜杠归一化；指向公开回调路径）
    assert_eq!(
        orders[0].notify_url.as_deref(),
        Some("https://pay.example.test/api/openapi/billing/notify"),
        "notify_url MUST 为 {{公开基址}}/api/openapi/billing/notify"
    );

    // 流水落 ext_wechat_pay.transaction
    let row: (Option<String>, Option<i64>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT attach, amount_total, trade_state, code_url \
         FROM ext_wechat_pay.transaction WHERE out_trade_no = $1",
    )
    .bind(&expected_trade_no)
    .fetch_one(&pool)
    .await
    .expect("交易流水行应存在");
    assert_eq!(row.0.as_deref(), Some(resp.attach.as_str()), "attach 落库");
    assert_eq!(row.1, Some(3750), "金额（分）落库");
    assert_eq!(row.2.as_deref(), Some("NOTPAY"), "下单后状态 NOTPAY");
    assert_eq!(
        row.3.as_deref(),
        Some("weixin://wxpay/stub"),
        "prepay 句柄落库"
    );

    cleanup(&pool, &fx, &marker, &expected_trade_no).await;
}

/// 未定价调用（未命中任何产品行）⇒ 409，MUST NOT 以不完整金额发起。
#[tokio::test]
async fn unpriced_usage_is_rejected() {
    let pool = ::common::testing::connect_test_db().await;
    wechat_pay_adapter::ensure_wechat_pay_schema(&pool)
        .await
        .expect("ext_wechat_pay 表族自愈建表");

    let marker = marker();
    let fx = create_client(&pool, &marker).await;
    let (month, at) = target_month_and_time();
    // 1 次定价调用 + 1 次未命中任何产品行的调用
    add_usage(
        &pool,
        fx.subscription_id,
        &format!("/api/service/pay-{}/priced", marker),
        1,
        at,
    )
    .await;
    add_usage(
        &pool,
        fx.subscription_id,
        &format!("/api/service/unmatched-{}", marker),
        1,
        at,
    )
    .await;

    let stub = StubChannel::default();
    let err = initiate_payment(
        &pool,
        &stub,
        PayRequest {
            client_id: fx.client_id.clone(),
            month: Some(month.clone()),
        },
        Some(TEST_PUBLIC_BASE),
    )
    .await
    .expect_err("存在未定价调用 MUST 拒绝");
    assert!(matches!(err, AliothError::Conflict(_)), "{err:?}");
    assert!(
        err.to_string().contains("UNPRICED_USAGE"),
        "错误消息应含标记：{err}"
    );
    assert!(stub.orders().is_empty(), "拒绝时 MUST NOT 发起下单");

    let trade_no = format!("openapi-{}-{}", fx.client_id, month);
    cleanup(&pool, &fx, &marker, &trade_no).await;
}

/// 无应付金额 ⇒ 409（MUST NOT 以 0 元发起）。
#[tokio::test]
async fn nothing_to_bill_is_rejected() {
    let pool = ::common::testing::connect_test_db().await;
    wechat_pay_adapter::ensure_wechat_pay_schema(&pool)
        .await
        .expect("ext_wechat_pay 表族自愈建表");

    let marker = marker();
    let fx = create_client(&pool, &marker).await;
    let (month, _) = target_month_and_time();
    // 无 usage ⇒ 应付金额 0

    let stub = StubChannel::default();
    let err = initiate_payment(
        &pool,
        &stub,
        PayRequest {
            client_id: fx.client_id.clone(),
            month: Some(month.clone()),
        },
        Some(TEST_PUBLIC_BASE),
    )
    .await
    .expect_err("零额 MUST 拒绝");
    assert!(matches!(err, AliothError::Conflict(_)), "{err:?}");
    assert!(
        err.to_string().contains("NOTHING_TO_BILL"),
        "错误消息应含标记：{err}"
    );
    assert!(stub.orders().is_empty());

    let trade_no = format!("openapi-{}-{}", fx.client_id, month);
    cleanup(&pool, &fx, &marker, &trade_no).await;
}

/// 缺注册行 / 渠道不可用 ⇒ 503 可读错误（MUST NOT 静默降级）。
#[tokio::test]
async fn missing_registry_row_yields_readable_503() {
    let pool = ::common::testing::connect_test_db().await;

    // ① 不存在的注册行 code ⇒ 可读 503（确定性：随机 code 必无注册行）
    let absent = format!("wechat-pay-absent-{}", marker());
    let err = match WechatPayChannel::for_code(&pool, &absent).await {
        Ok(_) => panic!("不存在的注册行 MUST 报错"),
        Err(e) => e,
    };
    assert!(matches!(err, AliothError::ServiceUnavailable(_)), "{err:?}");
    assert!(
        err.to_string().contains("PAYMENT_CHANNEL"),
        "错误消息应可读且含标记：{err}"
    );

    // ② provider 无启用行（测试库无 wechat-pay 凭据；真相源 = 注册表实查）⇒ 503 UNCONFIGURED
    let enabled = wechat_pay_adapter::config::first_enabled_code(&pool)
        .await
        .expect("注册表可查");
    if enabled.is_none() {
        let err = match WechatPayChannel::from_registry(&pool).await {
            Ok(_) => panic!("无启用行 MUST 报错"),
            Err(e) => e,
        };
        assert!(matches!(err, AliothError::ServiceUnavailable(_)), "{err:?}");
        let msg = err.to_string();
        assert!(
            msg.contains("PAYMENT_CHANNEL_UNCONFIGURED"),
            "错误消息应含标记：{msg}"
        );
    }
}

/// 未配置公开回调基址 ⇒ 下单 fail-loud（MUST NOT 静默留空 / 伪造域名）。
#[tokio::test]
async fn missing_public_base_url_is_rejected() {
    let pool = ::common::testing::connect_test_db().await;
    let (month, _) = target_month_and_time();

    let stub = StubChannel::default();
    let err = initiate_payment(
        &pool,
        &stub,
        PayRequest {
            client_id: "pay_test_no_base".to_string(),
            month: Some(month),
        },
        None,
    )
    .await
    .expect_err("未配置回调基址 MUST 拒绝");
    assert!(matches!(err, AliothError::ServiceUnavailable(_)), "{err:?}");
    assert!(
        err.to_string()
            .contains("OPENAPI_PUBLIC_BASE_URL_UNCONFIGURED"),
        "错误消息应含标记：{err}"
    );
    assert!(stub.orders().is_empty(), "拒绝时 MUST NOT 发起下单");
}

// ============================================================================
// 到账回调（第二阶段）：验签 → 解密 → notify_log 幂等 → 订阅状态联动
// ============================================================================

/// 测试用自签 RSA PKCS#8 私钥（**仅测试面**；`tests/` 面免 check-secrets-staged）。
const TEST_NOTIFY_PRIVATE_KEY: &str = include_str!("fixtures/openapi_notify_test_key.pem");
/// APIv3 密钥（AES-256-GCM，32 字符）。
const TEST_API_V3_KEY: &str = "0123456789abcdef0123456789abcdef";
/// 平台公钥序列号（验签时与 `Wechatpay-Serial` 比对）。
const TEST_PLATFORM_SERIAL: &str = "TESTPLATFORMSERIAL0001";

/// 由测试私钥构造 notifier（公钥由私钥导出 ⇒ 与签名同一身份）。
fn test_notifier() -> wechat_pay_adapter::WechatPayNotifier {
    let public_pem =
        wechat_pay_adapter::crypto::public_key_pem_from_private(TEST_NOTIFY_PRIVATE_KEY)
            .expect("导出测试公钥");
    let cfg = wechat_pay_adapter::WechatPayConfig {
        base_url: wechat_pay_adapter::DEFAULT_BASE_URL.to_string(),
        appid: "wxtestappid".to_string(),
        mchid: "10000100".to_string(),
        serial_no: "MCHSERIAL0001".to_string(),
        private_key_pem: TEST_NOTIFY_PRIVATE_KEY.to_string(),
        api_v3_key: TEST_API_V3_KEY.to_string(),
        platform_public_key_pem: Some(public_pem),
        platform_serial_no: Some(TEST_PLATFORM_SERIAL.to_string()),
    };
    wechat_pay_adapter::WechatPayNotifier::new(cfg).expect("构造测试 notifier")
}

/// AES-256-GCM 加密 resource 明文（与适配器解密同参：nonce 12 字节、AAD = associated_data）。
fn encrypt_resource(plaintext: &str, nonce: &str) -> String {
    use aes_gcm::aead::{Aead, KeyInit, Nonce, Payload};
    use aes_gcm::Aes256Gcm;
    use base64::Engine as _;
    let cipher = Aes256Gcm::new_from_slice(TEST_API_V3_KEY.as_bytes()).expect("APIv3 密钥");
    let nonce = Nonce::<Aes256Gcm>::try_from(nonce.as_bytes()).expect("12 字节 nonce");
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext.as_bytes(),
                aad: b"transaction",
            },
        )
        .expect("AES-256-GCM 加密");
    base64::engine::general_purpose::STANDARD.encode(ct)
}

/// 组装并签名一条 `TRANSACTION.SUCCESS` 回调（信封 + 请求头）。
fn signed_notify(
    notify_id: &str,
    tx_json: &serde_json::Value,
) -> (String, wechat_pay_adapter::model::NotifyHeaders) {
    let resource_nonce = "ab12cd34ef56"; // 12 ASCII 字符（AES-GCM nonce）
    let ciphertext = encrypt_resource(&tx_json.to_string(), resource_nonce);
    let body = serde_json::json!({
        "id": notify_id,
        "create_time": "2026-10-08T00:00:00+08:00",
        "resource_type": "encrypt-resource",
        "event_type": "TRANSACTION.SUCCESS",
        "summary": "支付成功",
        "resource": {
            "original_type": "transaction",
            "algorithm": "AEAD_AES_256_GCM",
            "ciphertext": ciphertext,
            "associated_data": "transaction",
            "nonce": resource_nonce,
        }
    })
    .to_string();
    let timestamp = "1700000000";
    let nonce = "NOTIFYHEADERNONCE";
    let message = wechat_pay_adapter::crypto::response_signature_message(timestamp, nonce, &body);
    let signature =
        wechat_pay_adapter::crypto::sign_base64(TEST_NOTIFY_PRIVATE_KEY, &message).expect("签名");
    let headers = wechat_pay_adapter::model::NotifyHeaders {
        timestamp: timestamp.to_string(),
        nonce: nonce.to_string(),
        signature,
        serial: TEST_PLATFORM_SERIAL.to_string(),
    };
    (body, headers)
}

/// 成功回调 ⇒ 交易落定 + 订阅续期/激活；同 `notify_id` 重放 ⇒ 零副作用（幂等）。
#[tokio::test]
async fn billing_notify_success_links_subscription_and_is_idempotent() {
    let pool = ::common::testing::connect_test_db().await;
    wechat_pay_adapter::ensure_wechat_pay_schema(&pool)
        .await
        .expect("ext_wechat_pay 表族自愈建表");

    let marker = marker();
    let fx = create_client(&pool, &marker).await;
    let (month, _) = target_month_and_time();
    let out_trade_no = format!("openapi-{}-{}", fx.client_id, month);
    let attach = serde_json::json!({ "client_id": fx.client_id, "month": month }).to_string();

    // 发起阶段形态：交易行 NOTPAY + attach；订阅置 suspended（验证「激活/续期」语义）
    sqlx::query(
        "INSERT INTO ext_wechat_pay.transaction \
           (out_trade_no, trade_state, attach, amount_total, amount_currency, raw_payload) \
         VALUES ($1, 'NOTPAY', $2, 3750, 'CNY', '{}'::jsonb)",
    )
    .bind(&out_trade_no)
    .bind(&attach)
    .execute(&pool)
    .await
    .expect("初始交易行");
    sqlx::query(
        "UPDATE isahl_auth.api_subscriptions SET status='suspended', expires_at=NULL WHERE id=$1",
    )
    .bind(fx.subscription_id)
    .execute(&pool)
    .await
    .expect("置 suspended");

    let notify_id = format!("NOTIFY-{}", marker);
    let tx_json = serde_json::json!({
        "out_trade_no": out_trade_no,
        "transaction_id": format!("WX-{}", marker),
        "trade_state": "SUCCESS",
        "trade_state_desc": "支付成功",
        "attach": attach,
        "success_time": "2026-10-08T00:00:01+08:00",
        "amount": { "total": 3750, "payer_total": 3750, "currency": "CNY", "payer_currency": "CNY" }
    });
    let notifier = test_notifier();
    let (body, headers) = signed_notify(&notify_id, &tx_json);

    // ① 首次回调：施加副作用
    let out = process_billing_notify(&pool, &notifier, &headers, &body)
        .await
        .expect("首次回调应成功");
    assert!(!out.idempotent, "首次回调非重放");
    assert_eq!(out.out_trade_no.as_deref(), Some(out_trade_no.as_str()));
    assert_eq!(out.trade_state.as_deref(), Some("SUCCESS"));
    assert_eq!(out.subscription_id, Some(fx.subscription_id));

    let log_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM ext_wechat_pay.notify_log WHERE notify_id = $1",
    )
    .bind(&notify_id)
    .fetch_one(&pool)
    .await
    .expect("notify_log 计数");
    assert_eq!(log_count, 1, "首条通知落单行");
    let (status, verified): (String, bool) = sqlx::query_as(
        "SELECT process_status, signature_verified FROM ext_wechat_pay.notify_log WHERE notify_id = $1",
    )
    .bind(&notify_id)
    .fetch_one(&pool)
    .await
    .expect("通知行");
    assert_eq!(status, "processed");
    assert!(verified, "验签通过 MUST 落 true");

    let state: String = sqlx::query_scalar(
        "SELECT trade_state FROM ext_wechat_pay.transaction WHERE out_trade_no = $1",
    )
    .bind(&out_trade_no)
    .fetch_one(&pool)
    .await
    .expect("交易行");
    assert_eq!(state, "SUCCESS", "回调落定 trade_state");

    let (sub_status, expires): (String, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("SELECT status, expires_at FROM isahl_auth.api_subscriptions WHERE id = $1")
            .bind(fx.subscription_id)
            .fetch_one(&pool)
            .await
            .expect("订阅行");
    assert_eq!(sub_status, "active", "到账 ⇒ 订阅激活");
    let window = openapi_billing::month_window(Some(&month), chrono::Utc::now()).expect("月窗口");
    let expected =
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(window.end_dt(), chrono::Utc);
    assert_eq!(expires, Some(expected), "到期边界 = 次月 0 点（绝对派生）");

    // ② 重放同一 notify_id：零副作用
    let out2 = process_billing_notify(&pool, &notifier, &headers, &body)
        .await
        .expect("重放应成功");
    assert!(out2.idempotent, "同 notify_id 重放 MUST 识别为幂等");
    let log_count2: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM ext_wechat_pay.notify_log WHERE notify_id = $1",
    )
    .bind(&notify_id)
    .fetch_one(&pool)
    .await
    .expect("notify_log 计数");
    assert_eq!(log_count2, 1, "重放 MUST NOT 产生重复通知行");
    let expires2: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT expires_at FROM isahl_auth.api_subscriptions WHERE id = $1")
            .bind(fx.subscription_id)
            .fetch_one(&pool)
            .await
            .expect("订阅行");
    assert_eq!(expires2, Some(expected), "重放 MUST NOT 改变到期边界");

    // 清理
    let _ = sqlx::query("DELETE FROM ext_wechat_pay.notify_log WHERE notify_id = $1")
        .bind(&notify_id)
        .execute(&pool)
        .await;
    cleanup(&pool, &fx, &marker, &out_trade_no).await;
}

/// 验签失败 ⇒ 400 且消息不含内部细节（MUST NOT 泄露）；不产生通知行/副作用。
#[tokio::test]
async fn billing_notify_bad_signature_is_rejected_without_detail_leak() {
    let pool = ::common::testing::connect_test_db().await;

    let marker = marker();
    let notify_id = format!("NOTIFY-BAD-{}", marker);
    let tx_json = serde_json::json!({ "out_trade_no": "x", "trade_state": "SUCCESS" });
    let notifier = test_notifier();
    let (body, mut headers) = signed_notify(&notify_id, &tx_json);
    // 篡改签名（保持长度）⇒ 验签必失败
    let mut bad = headers.signature.clone().into_bytes();
    bad[0] = if bad[0] == b'A' { b'B' } else { b'A' };
    headers.signature = String::from_utf8(bad).expect("utf8");

    let err = process_billing_notify(&pool, &notifier, &headers, &body)
        .await
        .expect_err("篡改签名 MUST 拒绝");
    assert!(matches!(err, AliothError::BadRequest(_)), "{err:?}");
    let msg = err.to_string();
    assert!(msg.contains("BILLING_NOTIFY_INVALID"), "{msg}");
    // MUST NOT 泄露内部细节（公钥/序列号/解密机制/验签实现等内部措辞）
    for leak in [
        "platform_public_key",
        "序列号",
        "APIv3",
        "AES-256-GCM",
        "verify_base64",
        "resource 解密失败",
    ] {
        assert!(
            !msg.contains(leak),
            "错误消息 MUST NOT 泄露内部细节（{leak}）：{msg}"
        );
    }

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM ext_wechat_pay.notify_log WHERE notify_id = $1",
    )
    .bind(&notify_id)
    .fetch_one(&pool)
    .await
    .expect("notify_log 计数");
    assert_eq!(count, 0, "验签失败 MUST NOT 落通知行");
}
