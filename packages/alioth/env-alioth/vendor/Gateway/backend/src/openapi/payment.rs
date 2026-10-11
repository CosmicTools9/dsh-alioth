//! 发起收款端点（`openapi-external-access :: openapi-billing-payment`）。
//!
//! `POST /api/openapi/billing/pay`（body `{client_id, month?}`；PDP `openapi_admin:0`）：
//! 以**计费聚合**的权威金额向调用方发起月度收款。
//!
//! 关键契约（change `add-openapi-payment-channel` design）：
//! - **金额单源**：金额只取 `openapi_billing::month_totals(.., Some(client_id))`；
//!   请求体 MUST NOT 含金额（结构上无该字段 ⇒ 客户端传金额无效）；
//!   存在未定价调用（命中产品行但缺价 / 未命中任何产品行）⇒ `409`，MUST NOT 以不完整金额发起；
//!   应付金额为 0 ⇒ `409`。
//! - **幂等键**：`out_trade_no = openapi-<client_id>-<month>`；`attach = {"client_id","month"}`（JSON 串）。
//! - **协议面复用**：下单/签名/流水写径/自愈建表全部经 `wechat-pay-adapter`（MUST NOT 复制）。
//! - **可测性缝**：唯一被抽象的是**网络下单调用**（[`PaymentChannel`]）；落库与金额换算走生产代码路径，
//!   测试注入 stub channel 即可覆盖（禁真实微信调用）。
//! - **fail-loud**：无 `provider='wechat-pay'` 启用注册行 / 凭据缺项 / 渠道不可用 ⇒ `503` 可读消息，
//!   MUST NOT 静默降级。

use actix_web::{web, HttpResponse};
use async_trait::async_trait;
use common::AliothError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use wechat_pay_adapter::model::{
    Amount, CreateOrderRequest, NotifyEnvelope, NotifyHeaders, OrderAmount, TradeState, Transaction,
};
use wechat_pay_adapter::store::{self, NotifyRecord, PrepayHandle};
use wechat_pay_adapter::{config as wp_config, WechatPayClient, WechatPayNotifier};

/// 微信支付 provider 代号（与适配器注册键同源，MUST NOT 手写第二份）。
const WECHAT_PAY_PROVIDER: &str = wp_config::PROVIDER;

/// 公开回调基址环境变量名（**单一配置面**；见 change `add-openapi-payment-channel` design §I）。
///
/// 值形如 `https://gateway.example.com`（可含路径前缀；尾斜杠被归一化）。
/// 未配置 / 空串 ⇒ 下单 fail-loud（MUST NOT 静默留空或伪造域名）。
pub const PUBLIC_BASE_URL_ENV: &str = "ALIOTH_OPENAPI_PUBLIC_BASE_URL";

/// 到账回调路径（相对公开基址；与 `mod.rs` 路由注册同源字面量）。
const NOTIFY_PATH: &str = "/api/openapi/billing/notify";

/// 由公开回调基址推导微信 `notify_url` = `{base}/api/openapi/billing/notify`。
///
/// - 未配置 / 空串 / 纯空白 ⇒ `503 OPENAPI_PUBLIC_BASE_URL_UNCONFIGURED`（可读消息，fail-loud）；
/// - 尾斜杠归一化（`https://h/` ⇒ `https://h/api/openapi/billing/notify`）。
pub fn notify_url_from_base(base: Option<&str>) -> Result<String, AliothError> {
    let base = base
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AliothError::ServiceUnavailable(format!(
                "OPENAPI_PUBLIC_BASE_URL_UNCONFIGURED: 未配置公开回调基址环境变量 {PUBLIC_BASE_URL_ENV}，无法注入微信 notify_url"
            ))
        })?;
    Ok(format!("{}{NOTIFY_PATH}", base.trim_end_matches('/')))
}

// ============================================================================
// 可测性缝：PaymentChannel
// ============================================================================

/// 一次下单的入参（渠道无关；金额由计费聚合给出，渠道 MUST NOT 自行计算）。
#[derive(Debug, Clone)]
pub struct ChannelOrder {
    pub out_trade_no: String,
    /// 应付金额（**分**，整数；由字符串金额 × 100 取整而来）。
    pub amount_total: i64,
    pub currency: String,
    pub description: String,
    /// `attach`（JSON 串；`{"client_id","month"}`）。
    pub attach: String,
    /// 回调地址（由公开回调基址推导，见 [`notify_url_from_base`]；未配置基址 ⇒ 下单 fail-loud）。
    pub notify_url: Option<String>,
}

/// 下单应答句柄（三选一，与 `wechat_pay_adapter::store::PrepayHandle` 同形）。
#[derive(Debug, Clone)]
pub enum ChannelPrepay {
    PrepayId(String),
    CodeUrl(String),
    H5Url(String),
}

impl ChannelPrepay {
    /// 归一到落库句柄形态（唯一映射点）。
    fn as_store_handle(&self) -> PrepayHandle<'_> {
        match self {
            ChannelPrepay::PrepayId(v) => PrepayHandle::PrepayId(v),
            ChannelPrepay::CodeUrl(v) => PrepayHandle::CodeUrl(v),
            ChannelPrepay::H5Url(v) => PrepayHandle::H5Url(v),
        }
    }

    /// 对外 prepay 参数（响应体 `prepay` 字段）。
    fn to_json(&self) -> serde_json::Value {
        match self {
            // `prepay_id` 是微信侧字符串句柄（枚举载荷即 String，非数值 id）⇒ 无精度面
            ChannelPrepay::PrepayId(v) => serde_json::json!({ "prepay_id": v }), // id-json-ok
            ChannelPrepay::CodeUrl(v) => serde_json::json!({ "code_url": v }),
            ChannelPrepay::H5Url(v) => serde_json::json!({ "h5_url": v }),
        }
    }
}

/// 下单结果（句柄 + 渠道应答原文，原文用于流水 `raw_payload`）。
#[derive(Debug, Clone)]
pub struct ChannelOrderResult {
    pub prepay: ChannelPrepay,
    pub raw: serde_json::Value,
}

/// 收款渠道：唯一被抽象的是**网络下单调用**（落库/金额/形态判断全在 handler 侧生产路径）。
#[async_trait]
pub trait PaymentChannel: Send + Sync {
    /// 渠道注册行 code（诊断与响应回显；如 `wechat-pay-10000100`）。
    fn channel_code(&self) -> &str;

    /// 发起下单。金额由调用方给出（渠道 MUST NOT 自行计算或改写）。
    async fn create_order(&self, req: &ChannelOrder) -> Result<ChannelOrderResult, AliothError>;
}

// ============================================================================
// 微信支付渠道实现（复用适配器，MUST NOT 复制协议面）
// ============================================================================

/// 微信支付渠道：包 `WechatPayClient`（Native 下单 ⇒ 扫码支付，无需 payer openid）。
pub struct WechatPayChannel {
    client: WechatPayClient,
    code: String,
}

/// 无启用注册行 ⇒ 可读 `503`（MUST NOT 静默降级）；纯函数，供单测确定性覆盖。
fn require_code(code: Option<String>) -> Result<String, AliothError> {
    code.ok_or_else(|| {
        AliothError::ServiceUnavailable(format!(
            "PAYMENT_CHANNEL_UNCONFIGURED: 无 provider='{WECHAT_PAY_PROVIDER}' 的启用注册行（isahl_auth.outbound_client）"
        ))
    })
}

impl WechatPayChannel {
    /// 按 provider 首个 enabled 注册行构造（渠道不可用 ⇒ `503` 可读消息）。
    pub async fn from_registry(pool: &PgPool) -> Result<Self, AliothError> {
        let code = require_code(
            wp_config::first_enabled_code(pool)
                .await
                .map_err(|e| {
                    AliothError::ServiceUnavailable(format!(
                        "PAYMENT_CHANNEL_LOOKUP_FAILED: 读取 provider='{WECHAT_PAY_PROVIDER}' 注册行失败：{e}"
                    ))
                })?,
        )?;
        Self::for_code(pool, &code).await
    }

    /// 指定注册行 code 构造（`from_registry` 的单点装配；测试亦用此验证「注册行缺失 ⇒ 可读错误」）。
    pub async fn for_code(pool: &PgPool, code: &str) -> Result<Self, AliothError> {
        let client = WechatPayClient::from_registry(pool, code)
            .await
            .map_err(|e| {
                AliothError::ServiceUnavailable(format!(
                    "PAYMENT_CHANNEL_UNUSABLE: 微信支付渠道不可用（注册行 {code}）：{e}"
                ))
            })?;
        Ok(Self {
            client,
            code: code.to_string(),
        })
    }

    /// 通知验签/解密器（与下单同一注册行凭据；回调端点复用，MUST NOT 另建配置装载面）。
    pub fn notifier(&self) -> &WechatPayNotifier {
        self.client.notifier()
    }
}

#[async_trait]
impl PaymentChannel for WechatPayChannel {
    fn channel_code(&self) -> &str {
        &self.code
    }

    async fn create_order(&self, req: &ChannelOrder) -> Result<ChannelOrderResult, AliothError> {
        let cfg = self.client.config();
        let order = CreateOrderRequest {
            appid: cfg.appid.clone(),
            mchid: cfg.mchid.clone(),
            description: req.description.clone(),
            out_trade_no: req.out_trade_no.clone(),
            time_expire: None,
            attach: Some(req.attach.clone()),
            notify_url: req.notify_url.clone(),
            goods_tag: None,
            support_fapiao: None,
            amount: OrderAmount {
                total: req.amount_total,
                currency: Some(req.currency.clone()),
            },
            payer: None,
            detail: None,
            scene_info: None,
            settle_info: None,
        };
        let resp = self.client.create_native_order(&order).await?;
        // 应答原文（`CodeUrlResponse` 唯一字段；未建模字段由适配器 `parse` 丢弃，此处如实重述）
        let raw = serde_json::json!({ "code_url": resp.code_url });
        Ok(ChannelOrderResult {
            prepay: ChannelPrepay::CodeUrl(resp.code_url),
            raw,
        })
    }
}

// ============================================================================
// 请求 / 响应
// ============================================================================

/// 发起收款入参。**无金额字段**（权威金额 = 计价聚合；客户端传金额结构上不可达）。
#[derive(Debug, Deserialize)]
pub struct PayRequest {
    pub client_id: String,
    #[serde(default)]
    pub month: Option<String>,
}

/// 发起收款响应。
#[derive(Debug, Serialize)]
pub struct PayResponse {
    pub out_trade_no: String,
    /// 渠道注册行 code。
    pub channel: String,
    pub month: String,
    /// 应付金额（分）。
    pub amount_total: i64,
    pub currency: String,
    pub attach: String,
    pub prepay: serde_json::Value,
}

// ============================================================================
// 业务逻辑（生产路径；HTTP handler 仅做渠道装配）
// ============================================================================

/// 字符串金额（numeric 精度）→ **分**（整数）。MUST NOT 走 f64。
fn yuan_to_cents(amount: &str) -> Result<i64, String> {
    use rust_decimal::prelude::ToPrimitive;
    use rust_decimal::Decimal;
    let yuan = amount
        .trim()
        .parse::<Decimal>()
        .map_err(|e| format!("金额 '{amount}' 非合法 numeric：{e}"))?;
    let cents = (yuan * Decimal::from(100)).round_dp(0);
    cents
        .to_i64()
        .ok_or_else(|| format!("金额 '{amount}' 超出分值 i64 范围"))
}

/// 发起收款（业务主体；渠道经 [`PaymentChannel`] 注入 ⇒ 可测）。
///
/// `public_base` = 公开回调基址（调用方从 [`PUBLIC_BASE_URL_ENV`] 注入；测试可直接给定）。
/// 未配置 ⇒ `503`（fail-loud，见 [`notify_url_from_base`]）。
///
/// 预条件校验 → 回调基址解析 → 计费聚合 → 未定价/零额拒绝 → 下单 → 生产落库（attach + prepay）。
pub async fn initiate_payment(
    pool: &PgPool,
    channel: &dyn PaymentChannel,
    req: PayRequest,
    public_base: Option<&str>,
) -> Result<PayResponse, AliothError> {
    let client_id = req.client_id.trim();
    if client_id.is_empty() {
        return Err(AliothError::BadRequest("client_id 不能为空".to_string()));
    }

    let window = openapi_billing::month_window(req.month.as_deref(), chrono::Utc::now())
        .map_err(|e| AliothError::BadRequest(format!("month 非法：{e}")))?;
    let (start, end) = (window.start_dt(), window.end_dt());

    // 部署配置面 fail-loud：公开回调基址未配置 ⇒ 明确错误（MUST NOT 静默留空 / 伪造域名）。
    let notify_url = notify_url_from_base(public_base)?;

    // 产品/价标面未就绪 ⇒ 全部调用视为未定价 ⇒ 不可发起（fail-loud，MUST NOT 以 0 元收款）
    if !openapi_billing::products_face_present(pool).await {
        return Err(AliothError::Conflict(
            "BILLING_PRODUCTS_UNAVAILABLE: 产品/价标面未就绪，无法计价发起收款".to_string(),
        ));
    }

    let totals = openapi_billing::month_totals(pool, start, end, Some(client_id))
        .await
        .map_err(|e| AliothError::Internal(format!("计费聚合失败：{e}")))?;
    let unmatched = openapi_billing::client_unmatched_calls(pool, start, end, client_id)
        .await
        .map_err(|e| AliothError::Internal(format!("未定价命中统计失败：{e}")))?;

    let unpriced = totals.unpriced_hits + unmatched;
    if unpriced > 0 {
        return Err(AliothError::Conflict(format!(
            "UNPRICED_USAGE: {unpriced} 次调用未定价（命中产品行但缺价或未命中任何产品行），MUST NOT 以不完整金额发起收款"
        )));
    }

    let currency = totals.currency.clone().unwrap_or_else(|| "CNY".to_string());
    let cents = yuan_to_cents(&totals.total_amount)
        .map_err(|e| AliothError::Internal(format!("金额换算失败：{e}")))?;
    if cents <= 0 {
        return Err(AliothError::Conflict(format!(
            "NOTHING_TO_BILL: client='{client_id}' 月份 {} 应付金额为 0",
            window.month
        )));
    }

    let out_trade_no = format!("openapi-{client_id}-{}", window.month);
    // `client_id` 是 `&str`（`BillingAttach.client_id` 同源为 String 字段）⇒ 无精度面
    let attach = serde_json::json!({ "client_id": client_id, "month": window.month }).to_string(); // id-json-ok
    let description = format!("OpenAPI 计费 {client_id} {}", window.month);

    let order = ChannelOrder {
        out_trade_no: out_trade_no.clone(),
        amount_total: cents,
        currency: currency.clone(),
        description: description.clone(),
        attach: attach.clone(),
        notify_url: Some(notify_url),
    };
    let result = channel.create_order(&order).await?;

    // 落库（生产路径，非测试替身）：
    // ① `upsert_transaction` **先**写：业务身份（`attach`/`description`/`amount`）在 INSERT 时落定
    //    （该 UPSERT 的 `DO UPDATE` 不含 `attach`/`description` ⇒ 只有首写能落这两列）；
    // ② `store_prepay` 后写：补齐下单句柄列（`prepay_id`/`code_url`/`h5_url`），不触碰 ① 的列。
    let tx = Transaction {
        appid: None,
        mchid: None,
        out_trade_no: out_trade_no.clone(),
        transaction_id: None,
        trade_type: None,
        trade_state: Some(TradeState::Notpay),
        trade_state_desc: None,
        bank_type: None,
        attach: Some(attach.clone()),
        success_time: None,
        payer: None,
        amount: Some(Amount {
            total: Some(cents),
            currency: Some(currency.clone()),
            ..Default::default()
        }),
        scene_info: None,
        promotion_detail: None,
        extra: [(
            "description".to_string(),
            serde_json::Value::String(description.clone()),
        )]
        .into_iter()
        .collect(),
    };
    store::upsert_transaction(pool, &tx, &result.raw)
        .await
        .map_err(|e| AliothError::Internal(format!("交易流水落库失败：{e}")))?;
    store::store_prepay(
        pool,
        &out_trade_no,
        &result.prepay.as_store_handle(),
        Some(&description),
        None,
        &result.raw,
    )
    .await
    .map_err(|e| AliothError::Internal(format!("下单句柄落库失败：{e}")))?;

    Ok(PayResponse {
        out_trade_no,
        channel: channel.channel_code().to_string(),
        month: window.month,
        amount_total: cents,
        currency,
        attach,
        prepay: result.prepay.to_json(),
    })
}

/// `POST /api/openapi/billing/pay`——发起月度收款（PDP `openapi_admin:0`）。
///
/// 公开回调基址从 [`PUBLIC_BASE_URL_ENV`] 读取（单一配置面）；未配置 ⇒ `503`（fail-loud）。
pub async fn post_billing_pay(
    pool: web::Data<PgPool>,
    body: web::Json<PayRequest>,
) -> Result<HttpResponse, AliothError> {
    let channel = WechatPayChannel::from_registry(pool.get_ref()).await?;
    let public_base = std::env::var(PUBLIC_BASE_URL_ENV).ok();
    let resp = initiate_payment(
        pool.get_ref(),
        &channel,
        body.into_inner(),
        public_base.as_deref(),
    )
    .await?;
    Ok(HttpResponse::Ok().json(resp))
}

// ============================================================================
// 到账回调（第二阶段）：验签 / 解密 / notify_log 幂等 / 订阅状态联动
// ============================================================================

/// 微信支付成功应答体（微信契约：`{"code":"SUCCESS","message":"成功"}`；非 2xx/非 SUCCESS 触发重推）。
#[derive(Debug, Serialize)]
pub struct NotifyAck {
    pub code: &'static str,
    pub message: &'static str,
}

/// 回调处理结果（幂等诊断面；`idempotent=true` 表示同 `notify_id` 重放、未施加任何副作用）。
#[derive(Debug, Clone, Serialize)]
pub struct NotifyOutcome {
    pub notify_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_trade_no: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trade_state: Option<String>,
    pub idempotent: bool,
    /// 成功到账时联动续期/激活的订阅 id（无联动对象时为 `None`）。
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "common::serde_zuid::opt"
    )]
    pub subscription_id: Option<i64>,
}

/// `attach` 承载的计费身份（发起端写入，MUST 同源：`{"client_id","month"}`）。
#[derive(Debug, Deserialize)]
struct BillingAttach {
    client_id: String,
    month: String,
}

/// 到账 → 订阅续期/激活（`api_subscriptions`）。
///
/// 语义（change `add-openapi-payment-channel` §5.3）：
/// - 按 `attach.month` 派生**绝对**到期边界（次月 1 日 00:00 UTC；单源 = `openapi_billing::month_window`）；
///   `status='active'`、`expires_at = GREATEST(现值, 边界)`、`starts_at = LEAST(现值, 月初)`
///   ——**绝对派生 + LEAST/GREATEST ⇒ 重放幂等**（同回调重放取值不变）。
/// - 目标订阅 = 该 client 最新有效订阅；仅存软删订阅时**复活**（`deleted_at=NULL` ⇒ 激活语义）。
/// - 无 client / 无任何订阅记录 ⇒ fail-loud（发起收款阶段已隐含二者存在，缺失即异常）。
async fn link_subscription(pool: &PgPool, attach: &str) -> Result<i64, AliothError> {
    let parsed: BillingAttach = serde_json::from_str(attach).map_err(|e| {
        AliothError::Internal(format!("BILLING_NOTIFY_BAD_ATTACH: attach 非计费形态：{e}"))
    })?;
    let window = openapi_billing::month_window(Some(&parsed.month), chrono::Utc::now())
        .map_err(|e| AliothError::Internal(format!("BILLING_NOTIFY_BAD_ATTACH: {e}")))?;

    let client_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.api_clients WHERE client_id = $1 AND deleted_at IS NULL",
    )
    .bind(&parsed.client_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AliothError::Internal(format!("回调客户解析失败：{e}")))?;
    let client_id = client_id.ok_or_else(|| {
        AliothError::Internal(format!(
            "BILLING_NOTIFY_CLIENT_UNKNOWN: attach.client_id='{}' 无对应 api_clients 行",
            parsed.client_id
        ))
    })?;

    let month_start =
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(window.start_dt(), chrono::Utc);
    let expires_at =
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(window.end_dt(), chrono::Utc);

    // 优先最新有效订阅；无有效订阅则取最新软删订阅并复活（激活语义）。
    let updated: Option<i64> = sqlx::query_scalar(
        "UPDATE isahl_auth.api_subscriptions \
         SET status = 'active', deleted_at = NULL, \
             starts_at = LEAST(starts_at, $2), \
             expires_at = GREATEST(COALESCE(expires_at, $2), $3) \
         WHERE id = (SELECT id FROM isahl_auth.api_subscriptions \
                     WHERE fk_client = $1 \
                     ORDER BY (deleted_at IS NULL) DESC, id DESC LIMIT 1) \
         RETURNING id",
    )
    .bind(client_id)
    .bind(month_start)
    .bind(expires_at)
    .fetch_optional(pool)
    .await
    .map_err(|e| AliothError::Internal(format!("订阅联动失败：{e}")))?;

    updated.ok_or_else(|| {
        AliothError::Internal(format!(
            "BILLING_NOTIFY_SUBSCRIPTION_UNRESOLVED: fk_client={client_id} 无订阅记录可续期/激活"
        ))
    })
}

/// 计费到账回调处理（业务主体；HTTP handler 仅做装配 ⇒ 可测）。
///
/// 链路：验签 → 解密 → 解析（适配器 `parse_notify`，fail-closed）→ `notify_log` 幂等
/// （同 `notify_id` 已 `processed` ⇒ 直接应答，MUST NOT 重复施加副作用）→ 交易落定
/// （`ext_wechat_pay.transaction`，单调更新）→ 成功时联动订阅。
///
/// 错误面：验签/解密失败 ⇒ `400` 且消息**不含内部细节**（内部错误仅入日志）。
pub async fn process_billing_notify(
    pool: &PgPool,
    notifier: &WechatPayNotifier,
    headers: &NotifyHeaders,
    body: &str,
) -> Result<NotifyOutcome, AliothError> {
    // ① 验签 → 解密 → 解析（fail-closed）。对外统一为 400 通用消息（MUST NOT 泄露内部细节）。
    let event = notifier.parse_notify(headers, body).map_err(|e| {
        log::warn!("billing notify 验签/解密失败：{e}");
        AliothError::BadRequest("BILLING_NOTIFY_INVALID: 回调验签或解密失败".to_string())
    })?;

    let notify_id = event.id.clone();

    // ② 幂等：同 notify_id 已处理 ⇒ 零副作用直接应答（微信重推不重复续期）。
    let prior: Option<String> = sqlx::query_scalar(
        "SELECT process_status FROM ext_wechat_pay.notify_log WHERE notify_id = $1",
    )
    .bind(&notify_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AliothError::Internal(format!("通知幂等查询失败：{e}")))?;
    if prior.as_deref() == Some("processed") {
        return Ok(NotifyOutcome {
            notify_id,
            out_trade_no: None,
            trade_state: None,
            idempotent: true,
            subscription_id: None,
        });
    }

    // ③ 落通知流水（`notify_id` 唯一；失败行同样留原文，可回溯）。
    let envelope: NotifyEnvelope = serde_json::from_str(body).map_err(|e| {
        log::warn!("billing notify 信封解析失败：{e}");
        AliothError::BadRequest("BILLING_NOTIFY_INVALID: 回调体非法".to_string())
    })?;
    let txn = event.transaction.clone();
    let out_trade_no = txn.as_ref().map(|t| t.out_trade_no.clone());
    let trade_state = txn
        .as_ref()
        .and_then(|t| t.trade_state)
        .map(|s| s.as_str().to_string());
    let transaction_id = txn.as_ref().and_then(|t| t.transaction_id.clone());
    store::insert_notify(
        pool,
        &NotifyRecord {
            envelope: &envelope,
            headers: Some(headers),
            signature_verified: true,
            plaintext: Some(&event.plaintext),
            out_trade_no: out_trade_no.as_deref(),
            transaction_id: transaction_id.as_deref(),
        },
    )
    .await
    .map_err(|e| AliothError::Internal(format!("通知流水落库失败：{e}")))?;

    // ④ 施加业务副作用（失败 ⇒ 记 failed 并上抛，微信将重推）。
    let mut subscription_id = None;
    let applied: Result<(), AliothError> = async {
        if let Some(t) = txn.as_ref() {
            store::upsert_transaction(pool, t, &event.plaintext)
                .await
                .map_err(|e| AliothError::Internal(format!("交易状态落库失败：{e}")))?;
            if t.trade_state == Some(TradeState::Success) {
                if let Some(attach) = t.attach.as_deref() {
                    subscription_id = Some(link_subscription(pool, attach).await?);
                }
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = applied {
        log::warn!("billing notify 处理失败（notify_id={notify_id}）：{e}");
        let _ =
            store::mark_notify_processed(pool, &notify_id, "failed", Some(&e.to_string())).await;
        return Err(e);
    }

    // ⑤ 标记处理结果。
    store::mark_notify_processed(pool, &notify_id, "processed", None)
        .await
        .map_err(|e| AliothError::Internal(format!("通知处理结果标记失败：{e}")))?;

    Ok(NotifyOutcome {
        notify_id,
        out_trade_no,
        trade_state,
        idempotent: false,
        subscription_id,
    })
}

/// 收集请求头为原始名值对（`NotifyHeaders::from_pairs` 的输入形态；大小写不敏感）。
fn collect_notify_headers(req: &actix_web::HttpRequest) -> Vec<(String, String)> {
    req.headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|s| (k.as_str().to_string(), s.to_string()))
        })
        .collect()
}

/// `POST /api/openapi/billing/notify`——微信支付到账回调（**公开路径**，无 JWT）。
///
/// 安全补偿 = 服务侧 `verify_notify`（WECHATPAY2-SHA256-RSA2048 验签）+ AES-256-GCM 解密，
/// 二者 fail-closed（`wechat-pay-adapter` 协议面，MUST NOT 复制）。
/// 验签/解密失败 ⇒ `400` 通用消息（MUST NOT 泄露内部细节）；成功 ⇒ 微信契约 `{"code":"SUCCESS"}`。
pub async fn post_billing_notify(
    req: actix_web::HttpRequest,
    body: web::Bytes,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, AliothError> {
    let headers = NotifyHeaders::from_pairs(&collect_notify_headers(&req)).map_err(|e| {
        log::warn!("billing notify 缺签名头：{e}");
        AliothError::BadRequest("BILLING_NOTIFY_INVALID: 缺少微信支付签名头".to_string())
    })?;
    let body_str = std::str::from_utf8(&body).map_err(|e| {
        log::warn!("billing notify 请求体非 UTF-8：{e}");
        AliothError::BadRequest("BILLING_NOTIFY_INVALID: 回调体非 UTF-8".to_string())
    })?;
    let channel = WechatPayChannel::from_registry(pool.get_ref()).await?;
    process_billing_notify(pool.get_ref(), channel.notifier(), &headers, body_str).await?;
    Ok(HttpResponse::Ok().json(NotifyAck {
        code: "SUCCESS",
        message: "成功",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yuan_to_cents_is_exact_not_float() {
        assert_eq!(yuan_to_cents("37.5").unwrap(), 3750);
        assert_eq!(yuan_to_cents("12.50").unwrap(), 1250);
        assert_eq!(yuan_to_cents("0").unwrap(), 0);
        assert_eq!(yuan_to_cents("0.01").unwrap(), 1);
        assert_eq!(yuan_to_cents("1234567.89").unwrap(), 123456789);
        assert!(yuan_to_cents("not-a-number").is_err());
    }

    #[test]
    fn missing_registry_row_is_readable_503() {
        let err = require_code(None).expect_err("缺注册行 MUST 报错");
        assert!(matches!(err, AliothError::ServiceUnavailable(_)), "{err:?}");
        let msg = err.to_string();
        assert!(
            msg.contains("PAYMENT_CHANNEL_UNCONFIGURED") && msg.contains(WECHAT_PAY_PROVIDER),
            "错误消息 MUST 可读且含标记：{msg}"
        );
    }

    #[test]
    fn prepay_json_shape_per_variant() {
        assert_eq!(
            ChannelPrepay::CodeUrl("weixin://x".into()).to_json(),
            serde_json::json!({ "code_url": "weixin://x" })
        );
        assert_eq!(
            ChannelPrepay::PrepayId("p1".into()).to_json(),
            serde_json::json!({ "prepay_id": "p1" })
        );
        assert_eq!(
            ChannelPrepay::H5Url("https://h".into()).to_json(),
            serde_json::json!({ "h5_url": "https://h" })
        );
    }

    #[test]
    fn pay_request_has_no_amount_field() {
        // 客户端传金额 → serde 忽略未知字段（结构上无 amount ⇒ MUST NOT 被采信）
        let parsed: PayRequest = serde_json::from_value(serde_json::json!({
            "client_id": "c1",
            "month": "2026-01",
            "amount": 99999,
            "amount_total": 99999,
        }))
        .expect("未知字段被忽略");
        assert_eq!(parsed.client_id, "c1");
        assert_eq!(parsed.month.as_deref(), Some("2026-01"));
    }

    #[test]
    fn notify_url_derived_from_base() {
        assert_eq!(
            notify_url_from_base(Some("https://gateway.example.com")).unwrap(),
            "https://gateway.example.com/api/openapi/billing/notify"
        );
        // 尾斜杠归一化
        assert_eq!(
            notify_url_from_base(Some("https://gateway.example.com/")).unwrap(),
            "https://gateway.example.com/api/openapi/billing/notify"
        );
        // 可含路径前缀
        assert_eq!(
            notify_url_from_base(Some("https://gw.example.com/alioth")).unwrap(),
            "https://gw.example.com/alioth/api/openapi/billing/notify"
        );
    }

    #[test]
    fn notify_url_missing_base_is_readable_503() {
        for absent in [None, Some(""), Some("   ")] {
            let err = notify_url_from_base(absent).expect_err("未配置基址 MUST 报错");
            assert!(matches!(err, AliothError::ServiceUnavailable(_)), "{err:?}");
            let msg = err.to_string();
            assert!(
                msg.contains("OPENAPI_PUBLIC_BASE_URL_UNCONFIGURED")
                    && msg.contains(PUBLIC_BASE_URL_ENV),
                "错误消息 MUST 可读且含标记与环境变量名：{msg}"
            );
        }
    }
}
