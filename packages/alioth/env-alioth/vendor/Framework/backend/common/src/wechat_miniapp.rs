//! 微信小程序登录 seam
//!
//! 微信小程序一键登录的**扩展点**：不同 namespace 的小程序身份模型不同
//! （司机端=手机号→司机表；C 端=openid 即账户），但「凭据解析 → 身份锚定 →
//! 签发平台 JWT」的后段流程是平台公共能力。
//!
//! 本模块只声明 trait 与来源注册表，**不含任何来源的领域概念**：
//! - [`WechatMiniappLogin::credentials`]：来源 → (appid, secret)
//! - [`WechatMiniappLogin::resolve_identity`]：来源 + openid → 平台用户 id
//!
//! 签发（ES256 JWT + refresh + session）由 SSO 公共段完成，provider 只负责
//! 「把这个人映射到平台哪个用户」。因此新增一个小程序 = 实现本 trait +
//! 在组合根注册一行，**SSO 源码零改动**。

use async_trait::async_trait;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;

/// 微信登录来源的失败语义（收敛为三态，由 SSO 映射为 HTTP 状态码）。
#[derive(Debug, thiserror::Error)]
pub enum WechatLoginError {
    /// 该来源无此用户（来源可自行兜底注册，故仅在明确拒绝时抛出）
    #[error("该来源无此用户")]
    NotFound,
    /// 命中实体但未绑定登录账号
    #[error("未绑定登录账号")]
    Unbound,
    /// 下游依赖不可用（DB/外部服务）
    #[error("下游依赖不可用：{0}")]
    Unavailable(String),
}

/// 一次微信登录的来源侧上下文。
///
/// 携带**两种身份锚点**与对应的一次性凭证，因为不同 namespace 的锚点不同：
/// - C 端（openid 即账户）：用 `js_code` 换到的 `openid`，无需手机号授权；
/// - 司机端（手机号→司机表）：用 `phone_code` 换到的明文手机号。
///
/// 字段全部为 `Option` / 空串表示「该来源不需要」，由 provider 自行取用；
/// SSO 公共段负责**填充微信侧换码结果**，不解释其业务含义。
#[derive(Debug, Clone, Default)]
pub struct WechatLoginContext {
    /// `jscode2session` 得到的 openid（C 端锚点）
    pub openid: String,
    /// `jscode2session` 得到的 unionid：同一微信开放平台账号（主体）下跨应用一致，
    /// 是**跨 AppID 识别同一人**的唯一依据（openid 的作用域只是单个 AppID）。
    /// 空串 = 微信未返回（该 AppID 未绑同一开放平台账号）⇒ 来源侧 MUST 退化为
    /// 各自的 openid 锚定，MUST NOT 推导或伪造。
    pub unionid: String,
    /// `getuserphonenumber` 得到的明文手机号（司机端锚点），数字归一后
    pub phone: String,
    /// 该来源是否要求手机号授权（true ⇒ 缺 `phone_code` 直接 400）
    pub requires_phone: bool,
}

/// 微信小程序登录来源。
#[async_trait]
pub trait WechatMiniappLogin: Send + Sync + 'static {
    /// 本 provider 支持的 `fromType` 标识（如 `driverWeChatApp`）。
    fn login_source(&self) -> &str;

    /// 该来源的凭据对；`None` = 未配置 ⇒ 调用方 400 fail-closed。
    fn credentials(&self) -> Option<(String, String)>;

    /// 该来源是否要求手机号授权。
    ///
    /// - `true`（司机端：身份锚点是手机号）：`phone_code` 必填，SSO 走
    ///   `getuserphonenumber` 换明文号码。
    /// - `false`（C 端：身份锚点是 openid）：前端不弹手机号授权框，
    ///   `WechatLoginContext::phone` 为空串。
    fn requires_phone(&self) -> bool;

    /// 身份锚定：微信侧换码结果 → 平台 `auth_users.id`。
    ///
    /// 账号状态门禁（`is_active` / MFA / status）由 SSO 公共段统一执行，
    /// provider 无需重复。
    async fn resolve_identity(
        &self,
        pool: &PgPool,
        ctx: &WechatLoginContext,
    ) -> Result<i64, WechatLoginError>;
}

/// 来源注册表：按 `fromType` 索引已注册的 provider。
///
/// 加一个小程序 = 向 [`WechatMiniappRegistry::with_drivers`] 追加一个
/// feature-gated 的 provider 实例。注册表本身不认识任何具体来源。
pub struct WechatMiniappRegistry {
    by_from_type: HashMap<String, Arc<dyn WechatMiniappLogin>>,
}

impl WechatMiniappRegistry {
    /// 由组合根装配的 provider 列表建索引。重复 `fromType` 后者覆盖前者
    /// （组合根是唯一装配点，重复即配置错误，由调用方保证）。
    pub fn with_drivers(drivers: Vec<Box<dyn WechatMiniappLogin>>) -> Self {
        let mut by_from_type = HashMap::new();
        for d in drivers {
            by_from_type.insert(d.login_source().to_string(), Arc::from(d));
        }
        Self { by_from_type }
    }

    /// 查该 `fromType` 的 provider；未注册 ⇒ `None`（调用方 400 INVALID_FROM_TYPE）。
    pub fn resolve(&self, from_type: &str) -> Option<Arc<dyn WechatMiniappLogin>> {
        self.by_from_type.get(from_type.trim()).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct Dummy(&'static str, bool);

    #[async_trait]
    impl WechatMiniappLogin for Dummy {
        fn login_source(&self) -> &str {
            self.0
        }
        fn credentials(&self) -> Option<(String, String)> {
            if self.1 {
                Some(("appid".into(), "secret".into()))
            } else {
                None
            }
        }
        fn requires_phone(&self) -> bool {
            self.1
        }
        async fn resolve_identity(
            &self,
            _pool: &PgPool,
            _ctx: &WechatLoginContext,
        ) -> Result<i64, WechatLoginError> {
            Ok(42)
        }
    }

    #[test]
    fn resolves_by_exact_from_type() {
        let reg = WechatMiniappRegistry::with_drivers(vec![
            Box::new(Dummy("driverWeChatApp", true)),
            Box::new(Dummy("bvLocalHomeApp", false)),
        ]);
        assert!(reg.resolve("driverWeChatApp").is_some());
        assert!(reg.resolve("bvLocalHomeApp").is_some());
        // 空白容错：注册表按 trim 后的键命中
        assert!(reg.resolve("  bvLocalHomeApp  ").is_some());
        // 未注册来源 fail-closed
        assert!(reg.resolve("unknownApp").is_none());
    }

    #[test]
    fn unconfigured_credentials_are_none() {
        let reg =
            WechatMiniappRegistry::with_drivers(vec![Box::new(Dummy("bvLocalHomeApp", false))]);
        assert!(reg
            .resolve("bvLocalHomeApp")
            .unwrap()
            .credentials()
            .is_none());
    }
}
