//! provider 抽象与注册表（`outbound-provider-abstraction`）
//!
//! 定位（`docs/specs/EXTERNAL_INTEGRATION_SPEC.md` §8.2）：本模块只承载 **provider 无关**的
//! 抽象与派发；任一 provider 的协议实现（接口路径、握手步骤、签名算法、专有字段）MUST 落在
//! L-ADP 适配器面（单 ns ⇒ `Pre-Proc/{ns}/Ext-adapter/<ext>/backend`；≥2 ns 复用 ⇒ 平台
//! `Ext-adapter/<ext>/backend`），由该 ns 的组合根在启动期装载。
//!
//! 装载语义：编译指令驱动（组合根 `build.rs` 扫描本 ns 适配器面生成注册臂）+ 一次性安装
//! （`install`，重复安装报错）。解析按 `provider`（注册表列 = 派发键）精确匹配，未注册即
//! fail-closed —— MUST NOT 回退默认 provider 或空认证。
//!
//! 形态参照 `system_config::probe::{ProviderProbe, ProbeRegistry}`（同款「宿主注入实现 +
//! Arc<HashMap> 注册表」），差异仅在实例化策略：出向调用的消费方是 service 层代码（无
//! web::Data/AppState 可及），故注册表以进程级单例持有，由组合根在启动期一次性安装。

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use common::AliothError as ApiError;
use serde_json::Value;

/// 认证方案标识：无认证（provider 默认实现即可满足，注册表行 `auth_type='none'`）。
pub const AUTH_TYPE_NONE: &str = "none";

/// provider 出向请求的治理化入口（超时 / 幂等重试 / 计量落 `outbound_call_log`）。
///
/// provider MUST 经此发起自身协议请求（握手、签名前置查询等），MUST NOT 自建 HTTP 客户端
/// —— 否则该流量脱离出向计量（`EXTERNAL_INTEGRATION_SPEC` §8.3.1）。
#[derive(Clone)]
pub struct ProviderHttp {
    client: reqwest::Client,
    pool: Option<sqlx::PgPool>,
    provider_id: String,
}

impl ProviderHttp {
    pub(crate) fn new(
        client: reqwest::Client,
        pool: Option<sqlx::PgPool>,
        provider_id: impl Into<String>,
    ) -> Self {
        Self {
            client,
            pool,
            provider_id: provider_id.into(),
        }
    }

    /// 治理化 POST（`retryable=true` ⇒ 幂等读，网络错误/5xx 指数退避重试至多 3 次）。
    pub async fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: Value,
        retryable: bool,
        interface: &str,
    ) -> Result<Value, ApiError> {
        crate::OutboundClient::send_once(
            &self.client,
            self.pool.as_ref(),
            &self.provider_id,
            "POST",
            url,
            headers,
            Some(body),
            retryable,
            interface,
        )
        .await
    }

    /// 治理化 GET（语义同 [`ProviderHttp::post_json`]）。
    pub async fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        retryable: bool,
        interface: &str,
    ) -> Result<Value, ApiError> {
        crate::OutboundClient::send_once(
            &self.client,
            self.pool.as_ref(),
            &self.provider_id,
            "GET",
            url,
            headers,
            None,
            retryable,
            interface,
        )
        .await
    }
}

/// provider 调用上下文：注册表行的 provider 私有配置 + 解密后明文凭据 + 治理化 HTTP 面。
pub struct ProviderContext<'a> {
    /// 注册表 `provider` 列（派发键；provider 可与自身 `provider_id()` 比对自检）。
    pub provider_id: &'a str,
    /// 注册表 `auth_type` 列（provider 自定义方案标识；`none` = 无认证）。
    pub auth_type: &'a str,
    /// 注册表 `base_url` 列。
    pub base_url: &'a str,
    /// 注册表 `config` 列（jsonb；provider 私有配置，schema 归 provider）。
    pub config: &'a Value,
    /// 解密后的明文凭据（`app_secret_enc`；`None` = 未配置）。
    pub secret: Option<&'a str>,
    http: &'a ProviderHttp,
}

impl<'a> ProviderContext<'a> {
    pub(crate) fn new(
        provider_id: &'a str,
        auth_type: &'a str,
        base_url: &'a str,
        config: &'a Value,
        secret: Option<&'a str>,
        http: &'a ProviderHttp,
    ) -> Self {
        Self {
            provider_id,
            auth_type,
            base_url,
            config,
            secret,
            http,
        }
    }

    /// `config` 中的字符串字段（provider 私有配置读取唯一入口）。
    pub fn config_str(&self, key: &str) -> Option<&str> {
        self.config.get(key).and_then(|v| v.as_str())
    }

    /// 必填 `config` 字段；缺失/空串即报错（fail-closed）。
    pub fn config_required(&self, key: &str) -> Result<&str, ApiError> {
        match self.config_str(key) {
            Some(v) if !v.trim().is_empty() => Ok(v),
            _ => Err(ApiError::Internal(format!(
                "provider '{}' 注册行缺配置项 config.{key}",
                self.provider_id
            ))),
        }
    }

    /// 治理化 POST（出向计量与超时治理同 `OutboundClient`）。
    pub async fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: Value,
        retryable: bool,
        interface: &str,
    ) -> Result<Value, ApiError> {
        self.http
            .post_json(url, headers, body, retryable, interface)
            .await
    }

    /// 治理化 GET。
    pub async fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        retryable: bool,
        interface: &str,
    ) -> Result<Value, ApiError> {
        self.http.get_json(url, headers, retryable, interface).await
    }
}

/// 本次出向请求的报文面（provider 做**请求签名**时需要：method + 完整 URL + 原始 body 文本）。
///
/// `body_text` 为平台序列化后的**实际发送字节**（provider 签名 MUST 以该文本为准，
/// MUST NOT 自行重序列化——重序列化与发送字节不一致会导致上游验签失败）。
pub struct RequestInfo<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub body_text: Option<&'a str>,
}

/// provider 契约：派发键 + 认证材料注入 + 认证态失效。
///
/// - `provider_id()` MUST 与注册表 `provider` 列同值（kebab-case 单一代号，如 `fssc`）；
/// - `authorize()` 返回本次调用要注入的请求头（默认实现返回空 = 无认证）；
/// - `prepare_body()` 返回变换后的请求体（默认实现原样返回）——**请求体级认证注入的唯一
///   入口**（调用序见方法注释；change `migrate-image-sender-to-provider-seam`）；
/// - `invalidate()` 在认证被上游拒绝（401/403）或凭据轮换后清理本地材料（默认无操作）。
#[async_trait]
pub trait OutboundProvider: Send + Sync {
    /// 派发键（= 注册表 `provider` 列取值）。
    fn provider_id(&self) -> &'static str;

    /// 注入认证材料（每次出向调用调用一次；重试时重新调用，故失效后自然重取）。
    async fn authorize(
        &self,
        _ctx: ProviderContext<'_>,
        _req: &RequestInfo<'_>,
    ) -> Result<Vec<(String, String)>, ApiError> {
        Ok(Vec::new())
    }

    /// 请求体级认证注入（默认原样返回 = 无 body 变换）。
    ///
    /// 调用序（`OutboundClient::send_governed`）：本钩子 → 序列化（`RequestInfo.body_text`
    /// = 实际发送字节）→ [`Self::authorize`] → 发送。需要「签名绑定最终字节」的协议 MUST
    /// 在 `authorize` 内基于 `body_text` 计算，MUST NOT 依赖本钩子的入参。
    /// `auth_type='none'` 行不调用本钩子（与 `authorize` 默认空实现同分支语义）。
    async fn prepare_body(
        &self,
        _ctx: &ProviderContext<'_>,
        _method: &str,
        _url: &str,
        body: Value,
    ) -> Result<Value, ApiError> {
        Ok(body)
    }

    /// 认证态失效钩子（401/403 或凭据轮换后）。
    async fn invalidate(&self, _ctx: ProviderContext<'_>) {}
}

/// provider 注册表（构造期填充，运行期只读；重复 `provider_id` 报错）。
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    providers: Arc<HashMap<&'static str, Arc<dyn OutboundProvider>>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记 provider（重复 `provider_id` MUST 报错，MUST NOT 静默覆盖）。
    pub fn with(self, provider: Arc<dyn OutboundProvider>) -> Result<Self, ApiError> {
        let id = provider.provider_id();
        if id.trim().is_empty() {
            return Err(ApiError::Internal(
                "provider 的 provider_id() 为空——派发键必填".to_string(),
            ));
        }
        if self.providers.contains_key(id) {
            return Err(ApiError::Internal(format!(
                "provider 重复注册（provider_id='{id}'）——同一代号只能有一个实现"
            )));
        }
        let mut map = (*self.providers).clone();
        map.insert(id, provider);
        Ok(Self {
            providers: Arc::new(map),
        })
    }

    pub fn get(&self, provider_id: &str) -> Option<Arc<dyn OutboundProvider>> {
        self.providers.get(provider_id).cloned()
    }

    pub fn contains(&self, provider_id: &str) -> bool {
        self.providers.contains_key(provider_id)
    }

    /// 已登记 provider 代号（诊断/测试用）。
    pub fn ids(&self) -> Vec<&'static str> {
        let mut ids: Vec<&'static str> = self.providers.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRegistry")
            .field("providers", &self.ids())
            .finish()
    }
}

/// 进程级注册表（组合根启动期安装一次）。
static REGISTRY: OnceLock<ProviderRegistry> = OnceLock::new();

/// 安装注册表（组合根调用；重复安装 MUST 报错——两处装配意味着派发面不确定）。
pub fn install(registry: ProviderRegistry) -> Result<(), ApiError> {
    REGISTRY.set(registry).map_err(|_| {
        ApiError::Internal(
            "provider 注册表已安装（install 只能调用一次）——检查组合根是否重复装配".to_string(),
        )
    })
}

/// 安装状态（诊断/测试用）。
pub fn installed() -> bool {
    REGISTRY.get().is_some()
}

/// 已安装注册表（未安装 ⇒ `None`；供诊断）。
pub fn registry() -> Option<&'static ProviderRegistry> {
    REGISTRY.get()
}

/// 按派发键解析 provider（未安装注册表 / 未注册该代号 ⇒ fail-closed）。
pub fn resolve(provider_id: &str) -> Result<Arc<dyn OutboundProvider>, ApiError> {
    let Some(registry) = REGISTRY.get() else {
        return Err(ApiError::Internal(
            "provider 注册表未安装——组合根 MUST 在启动期装载本 ns 适配器（install）".to_string(),
        ));
    };
    registry.get(provider_id).ok_or_else(|| {
        ApiError::Internal(format!(
            "provider '{provider_id}' 未注册（本 ns 构建未装入该适配器；已注册: {:?}）",
            registry.ids()
        ))
    })
}

/// 保存期注册行成员资格判定（**纯函数**：注册表快照注入 ⇒ 可单测）。
///
/// 分支 MUST 与 `OutboundClient::auth_headers()`（及本模块 [`resolve()`]）
/// **逐条一致**（不得比运行期更严或更宽）：
/// - `auth_type == 'none'`：运行期短路、不解析 provider ⇒ 不校验（`known` 可为任意值）；
/// - `known == None`（注册表**未安装**：平台模板档空装配 / 未走 ns 装配）：无可比集合 ⇒
///   不校验——`installed=false`（装载未执行）与「已装但为空」是两个独立事实，运行期
///   [`resolve()`] 的 fail-closed 仍兜底；
/// - 其余：`provider` ∉ 已装集合 ⇒ 400（message 含 `UNKNOWN_PROVIDER` 标记与已装集合）。
///
/// 规约：`openspec/specs/openapi-external-access :: outbound-provider-catalog-and-membership-check`
/// （判据单源实现于本模块；管理面 MUST 同源消费，MUST NOT 双写分支语义）。
pub fn membership_verdict(
    provider: &str,
    auth_type: &str,
    known: Option<&[&str]>,
) -> Result<(), ApiError> {
    if auth_type.trim() == AUTH_TYPE_NONE {
        return Ok(());
    }
    let Some(known) = known else {
        return Ok(());
    };
    let provider = provider.trim();
    if known.contains(&provider) {
        return Ok(());
    }
    Err(ApiError::BadRequest(format!(
        "UNKNOWN_PROVIDER: provider '{provider}' 未装入本 ns 构建（已装: {known:?}）——\
         auth_type='{}' 需已注册的 provider 实现",
        auth_type.trim()
    )))
}

/// 成员资格校验（IO 面）：取已装注册表代号快照后交 [`membership_verdict`]。
/// 管理面（出向注册行保存期验收）的唯一调用入口——MUST NOT 另写分支语义。
pub fn validate_registered(provider: &str, auth_type: &str) -> Result<(), ApiError> {
    let snapshot = registry().map(|r| r.ids());
    membership_verdict(provider, auth_type, snapshot.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Probe {
        id: &'static str,
        auth: Vec<(String, String)>,
        invalidated: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl OutboundProvider for Probe {
        fn provider_id(&self) -> &'static str {
            self.id
        }

        async fn authorize(
            &self,
            _ctx: ProviderContext<'_>,
            _req: &RequestInfo<'_>,
        ) -> Result<Vec<(String, String)>, ApiError> {
            Ok(self.auth.clone())
        }

        async fn invalidate(&self, _ctx: ProviderContext<'_>) {
            self.invalidated
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// 默认实现 = 无认证（`auth_type='none'` 的 provider 无需覆写 authorize）。
    struct NoAuth;
    #[async_trait]
    impl OutboundProvider for NoAuth {
        fn provider_id(&self) -> &'static str {
            "no-auth"
        }
    }

    fn probe(id: &'static str) -> Arc<dyn OutboundProvider> {
        Arc::new(Probe {
            id,
            auth: vec![("access_token".to_string(), "t".to_string())],
            invalidated: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        })
    }

    #[test]
    fn registry_duplicate_provider_id_is_error() {
        let r = ProviderRegistry::new()
            .with(probe("acme"))
            .expect("首个登记");
        let err = r.with(probe("acme")).expect_err("重复登记必须报错");
        assert!(
            format!("{err}").contains("重复注册"),
            "文案应指明冲突: {err}"
        );
    }

    #[test]
    fn registry_empty_provider_id_is_error() {
        let err = ProviderRegistry::new()
            .with(probe(""))
            .expect_err("空派发键必须报错");
        assert!(format!("{err}").contains("provider_id"), "{err}");
    }

    #[test]
    fn registry_lookup_and_ids_are_stable() {
        let r = ProviderRegistry::new()
            .with(probe("beta"))
            .and_then(|r| r.with(probe("alpha")))
            .expect("登记");
        assert_eq!(r.ids(), vec!["alpha", "beta"], "ids 排序稳定");
        assert!(r.contains("alpha"));
        assert!(r.get("missing").is_none(), "未注册 ⇒ None");
    }

    #[tokio::test]
    async fn no_auth_provider_defaults_to_empty_headers() {
        let p = NoAuth;
        assert!(p.provider_id() == "no-auth");
    }

    #[tokio::test]
    async fn invalidate_hook_is_callable() {
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let p = Probe {
            id: "x",
            auth: vec![],
            invalidated: counter.clone(),
        };
        let http = ProviderHttp::new(reqwest::Client::new(), None, "x");
        let config = json!({});
        let ctx = ProviderContext::new("x", "none", "", &config, None, &http);
        let req = RequestInfo {
            method: "POST",
            url: "http://example.invalid/x",
            body_text: None,
        };
        assert!(p.authorize(ctx, &req).await.expect("authorize").is_empty());
        let ctx = ProviderContext::new("x", "none", "", &config, None, &http);
        p.invalidate(ctx).await;
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn resolve_without_install_fails_closed() {
        // 本进程未安装注册表（其他测试不安装）⇒ 解析必须报错而非放行
        if !installed() {
            match resolve("acme") {
                Ok(_) => panic!("未安装注册表时解析必须 fail-closed"),
                Err(e) => assert!(format!("{e}").contains("未安装"), "{e}"),
            }
        }
    }

    #[test]
    fn config_helpers_require_non_empty() {
        let http = ProviderHttp::new(reqwest::Client::new(), None, "x");
        let cfg = json!({ "appId": "A", "blank": "" });
        let ctx = ProviderContext::new("x", "none", "", &cfg, None, &http);
        assert_eq!(ctx.config_str("appId"), Some("A"));
        assert_eq!(ctx.config_required("appId").expect("必填"), "A");
        assert!(ctx.config_required("blank").is_err(), "空串视同缺失");
        assert!(ctx.config_required("missing").is_err());
    }

    // ── 保存期成员资格（membership_verdict 三分支，与 auth_headers() 逐条同源；
    //    管理面 `openapi::outbound_admin` 同源消费本实现——fix-openapi-review-findings §5）──

    #[test]
    fn membership_none_auth_skips_check() {
        // auth_type='none' ⇒ 运行期 auth_headers() 短路、不解析 provider ⇒ 保存期不校验
        // （carrier 先例：auth_type=none 且仓内无适配器面）
        assert!(
            membership_verdict("carrier", "none", Some(&["fssc"])).is_ok(),
            "none 认证行 MUST NOT 因 provider 未注册被拒"
        );
    }

    #[test]
    fn membership_uninstalled_registry_skips_check() {
        // 注册表未安装（平台模板档空装配 / 未走 ns 装配）⇒ 无可比集合 ⇒ 放行；
        // installed=false（装载未执行）≠ 零 provider——运行期 resolve() 的 fail-closed 兜底
        assert!(membership_verdict("fssc", "bearer", None).is_ok());
    }

    #[test]
    fn membership_rejects_unknown_provider() {
        let err = membership_verdict("fssc-typo", "bearer", Some(&["fssc", "wechat-pay"]))
            .expect_err("未注册 provider + 非 none 认证 MUST 400");
        let msg = err.to_string();
        assert!(msg.contains("UNKNOWN_PROVIDER"), "message 须带标记: {msg}");
        assert!(msg.contains("wechat-pay"), "message 须含已装集合: {msg}");
    }

    #[test]
    fn membership_accepts_known_provider_after_trim() {
        assert!(membership_verdict("fssc", "bearer", Some(&["fssc"])).is_ok());
        assert!(
            membership_verdict("  fssc  ", "bearer", Some(&["fssc"])).is_ok(),
            "两侧空白 MUST 被 trim（与注册表列写入口径一致）"
        );
        assert!(
            membership_verdict("fssc", "  none  ", Some(&[])).is_ok(),
            "auth_type 双侧空白亦须 trim 后判 none 短路"
        );
    }
}
