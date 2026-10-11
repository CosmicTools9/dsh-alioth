//! 凭据来源判定与 CSRF 校验 —— **单一实现**
//!
//! 模块名 `csrf` 而非 `auth`：`common/src/auth.rs` 是**另一份既有文件**（HEAD 既存、`lib.rs` 未声明、
//! 无消费方的 JWT 工具孤儿）——本模块 MUST NOT 占用该路径（2026-10-03 实测教训：`write` 会静默覆盖
//! 已跟踪文件；新增模块前 MUST 先 `git ls-files` 核该路径是否已跟踪）。
//!
//! change: `implement-csrf-protection`（design.md §3 凭据来源唯一实现、§6 cookie 契约、§7 豁免面、§8 错误契约）。
//!
//! 存在理由（spec-audit F1）：本仓原有**三份**令牌提取实现且优先级互相冲突
//! （`common::middleware`＝cookie→Bearer；Gateway PEP＝Bearer→cookie；SSO `jwt::extract_token`
//! ＝scoped cookie→Bearer→环境 cookie）。CSRF 的豁免判据必须等于「**实际完成鉴权的凭据来源**」，
//! 故本模块给出唯一实现，各消费方 MUST 委托之，MUST NOT 另立「存在某头即豁免」类启发式。

use actix_web::cookie::{time::Duration, Cookie, SameSite};
use actix_web::http::Method;
use actix_web::{HttpRequest, HttpResponse};
use subtle::ConstantTimeEq;

/// 应用 cookie 命名空间请求头（`{s}_access_token` / `{s}_refresh_token` / `{s}_csrf_token`）。
pub const SCOPE_HEADER: &str = "x-auth-cookie-scope";
/// CSRF double-submit 请求头。
pub const CSRF_HEADER: &str = "x-csrf-token";
/// 开关环境变量（`on|off`，缺省 off）。
pub const CSRF_ENFORCE_ENV: &str = "CSRF_ENFORCE";
/// 错误码：无 CSRF 凭据。
pub const CODE_CSRF_MISSING: &str = "CSRF_TOKEN_MISSING";
/// 错误码：header 与 cookie 不等。
pub const CODE_CSRF_MISMATCH: &str = "CSRF_TOKEN_MISMATCH";
/// CSRF cookie 生命周期（秒）—— MUST ≥ refresh 周期（SSO `jwt_refresh_expiry` 名义值 604800），
/// 否则登录后第 16 分钟起的静默写请求会全部 403（spec-audit ⑥a）。
pub const CSRF_MAX_AGE_SECS: i64 = 604_800;

/// 实际完成鉴权的凭据来源（CSRF 豁免判据的唯一输入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    /// `x-auth-cookie-scope` 声明的应用命名空间 cookie。
    ScopedCookie,
    /// `Authorization: Bearer`（攻击者无法跨站注入自定义头 ⇒ 非 CSRF 面）。
    Bearer,
    /// 无前缀环境 cookie（浏览器同站会话）。
    Cookie,
}

/// CSRF 校验失败类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsrfFailure {
    Missing,
    Mismatch,
}

// ---------------------------------------------------------------- scope 与命名

/// 解析并校验 `x-auth-cookie-scope`（合法字符 `[A-Za-z0-9_-]`、长度 ≤32；缺失/非法 → None）。
///
/// 唯一实现：SSO 处理器必须委托本函数（原 `login::handlers::cookie_scope` 为私有副本）。
pub fn cookie_scope(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get(SCOPE_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 32
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

/// `{s}_access_token` / `access_token`。
pub fn access_cookie_name(scope: Option<&str>) -> String {
    scoped_name(scope, "access_token")
}

/// `{s}_refresh_token` / `refresh_token`。
pub fn refresh_cookie_name(scope: Option<&str>) -> String {
    scoped_name(scope, "refresh_token")
}

/// `{s}_csrf_token` / `csrf_token`（design.md §6：与令牌同 scope 规则，
/// 避免 GW 与 OA 共用一枚导致「一方登出打断另一方写操作」）。
pub fn csrf_cookie_name(scope: Option<&str>) -> String {
    scoped_name(scope, "csrf_token")
}

fn scoped_name(scope: Option<&str>, base: &str) -> String {
    match scope {
        Some(s) if !s.is_empty() => format!("{s}_{base}"),
        _ => base.to_string(),
    }
}

// ---------------------------------------------------------------- 凭据来源

fn cookie_value(req: &HttpRequest, name: &str) -> Option<String> {
    req.cookie(name)
        .map(|c| c.value().trim().to_string())
        .filter(|v| !v.is_empty())
}

fn bearer_value(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get(actix_web::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|auth| auth.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// 令牌提取 + 来源判定（顺序 = `auth` capability「访问令牌提取优先级」：
/// scoped cookie → Bearer → 环境 cookie）。
pub fn resolve_credential(req: &HttpRequest) -> Option<(String, CredentialSource)> {
    if let Some(scope) = req
        .headers()
        .get(SCOPE_HEADER)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(token) = cookie_value(req, &access_cookie_name(Some(scope.trim()))) {
            return Some((token, CredentialSource::ScopedCookie));
        }
    }
    if let Some(token) = bearer_value(req) {
        return Some((token, CredentialSource::Bearer));
    }
    cookie_value(req, &access_cookie_name(None)).map(|t| (t, CredentialSource::Cookie))
}

/// 兼容包装：仅取令牌（来源丢弃）。
pub fn extract_token(req: &HttpRequest) -> Option<String> {
    resolve_credential(req).map(|(token, _)| token)
}

// ---------------------------------------------------------------- 校验判据

/// 安全方法（不触发 CSRF 校验）。
pub fn is_safe_method(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// 显式豁免面 = 会话自举/终止端点（design.md §7）。**MUST NOT 扩至业务写端点。**
///
/// 覆盖 `/api/auth/{refresh,logout}`（SSO standalone 与 Gateway 两前缀形态）。
pub fn is_csrf_exempt_path(path: &str) -> bool {
    path.ends_with("/auth/refresh") || path.ends_with("/auth/logout")
}

/// 开关是否开启（唯一读点）。生产环境「常开」由启动 fail-loud 保证（见 `assert_csrf_enforced_in_prod`）。
pub fn csrf_enforced() -> bool {
    csrf_enforced_from(std::env::var(CSRF_ENFORCE_ENV).ok().as_deref())
}

/// 纯函数形态（可测）。
pub fn csrf_enforced_from(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("on") | Some("1") | Some("true")
    )
}

/// 该请求是否需要执行 CSRF 校验（开关 ∧ 非安全方法 ∧ 非豁免路径）。
pub fn csrf_requires_check(req: &HttpRequest) -> bool {
    csrf_enforced() && !is_safe_method(req.method()) && !is_csrf_exempt_path(req.path())
}

/// 校验（cookie 名参数化）——供自有 cookie 命名的消费方（如 Meta `meta_csrf_token`）复用同一判据。
pub fn check_csrf_named(
    req: &HttpRequest,
    cookie_name: &str,
    source: CredentialSource,
) -> Result<(), CsrfFailure> {
    if source == CredentialSource::Bearer {
        return Ok(());
    }
    let cookie = cookie_value(req, cookie_name);
    let header = req
        .headers()
        .get(CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match (cookie, header) {
        (Some(c), Some(h)) if c.as_bytes().ct_eq(h.as_bytes()).into() => Ok(()),
        (Some(_), Some(_)) => Err(CsrfFailure::Mismatch),
        _ => Err(CsrfFailure::Missing),
    }
}

/// 校验：`Bearer` 来源豁免；cookie 来源 MUST 常量时间比较 header 与 cookie。
pub fn check_csrf(req: &HttpRequest, source: CredentialSource) -> Result<(), CsrfFailure> {
    let scope = cookie_scope(req);
    check_csrf_named(req, &csrf_cookie_name(scope.as_deref()), source)
}

/// 失败响应 = 统一平铺错误体（`ErrorResponse`，`API_DESIGN_SPEC §3.2`）。
///
/// **MUST NOT** 使用旧式嵌套 `{"error": ...}` 体（spec-audit F6 陷阱）。
pub fn csrf_failure_response(failure: CsrfFailure) -> HttpResponse {
    let (code, message) = match failure {
        CsrfFailure::Missing => (CODE_CSRF_MISSING, "CSRF token missing"),
        CsrfFailure::Mismatch => (CODE_CSRF_MISMATCH, "CSRF token mismatch"),
    };
    let body = crate::error::ErrorResponse {
        code: code.to_string(),
        message: message.to_string(),
        details: None,
    };
    HttpResponse::Forbidden().json(body)
}

/// 登录/签发时的站点钩子：插入点调用一次即可（校验 ∧ 失败即返回）。
///
/// 返回 `Some(response)` 表示 MUST 短路返回该 403；`None` 表示放行。
/// 调用点 MUST 在**鉴权成功之后**调用（保证失效会话仍为 401，见 design.md §4）。
pub fn csrf_gate(req: &HttpRequest, source: CredentialSource) -> Option<HttpResponse> {
    if !csrf_requires_check(req) {
        return None;
    }
    check_csrf(req, source).err().map(csrf_failure_response)
}

/// 同 [`csrf_gate`]，cookie 名参数化（自有命名的消费方）。
pub fn csrf_gate_named(
    req: &HttpRequest,
    cookie_name: &str,
    source: CredentialSource,
) -> Option<HttpResponse> {
    if !csrf_requires_check(req) {
        return None;
    }
    check_csrf_named(req, cookie_name, source)
        .err()
        .map(csrf_failure_response)
}

// ---------------------------------------------------------------- cookie 属性

/// cookie 是否带 `Secure`（与 SSO 同源判据）。
pub fn cookie_secure_from_env() -> bool {
    std::env::var("COOKIE_SECURE")
        .ok()
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// `SameSite` 档位（Secure ⇒ Strict，否则 Lax）——与认证 cookie 同档。
pub fn same_site_for(secure: bool) -> SameSite {
    if secure {
        SameSite::Strict
    } else {
        SameSite::Lax
    }
}

/// 构造 CSRF cookie（**非 HttpOnly** —— double-submit 前提，design.md §6）。
pub fn csrf_cookie(scope: Option<&str>, token: &str, secure: bool) -> Cookie<'static> {
    let mut cookie = Cookie::new(csrf_cookie_name(scope), token.to_string());
    cookie.set_path("/");
    cookie.set_http_only(false);
    cookie.set_secure(secure);
    cookie.set_same_site(same_site_for(secure));
    cookie.set_max_age(Duration::seconds(CSRF_MAX_AGE_SECS));
    cookie
}

/// 构造清除 CSRF cookie 的 cookie（Max-Age=0）。
pub fn csrf_clear_cookie(scope: Option<&str>, secure: bool) -> Cookie<'static> {
    let mut cookie = Cookie::new(csrf_cookie_name(scope), String::new());
    cookie.set_path("/");
    cookie.set_http_only(false);
    cookie.set_secure(secure);
    cookie.set_same_site(same_site_for(secure));
    cookie.set_max_age(Duration::ZERO);
    cookie
}

/// 生成 CSRF 令牌（≥256 bit CSPRNG，base64url 无填充）。
pub fn new_csrf_token() -> String {
    use base64::Engine as _;
    let bytes: [u8; 32] = rand::random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// 生产环境启动自检：识别为生产而开关未开 ⇒ 拒启（design.md §9）。
///
/// 返回 `Err(原因)` 时调用方 MUST 终止启动。
pub fn assert_csrf_enforced_in_prod(is_production: bool) -> Result<(), String> {
    if is_production && !csrf_enforced() {
        return Err(format!(
            "生产环境 MUST 开启 {CSRF_ENFORCE_ENV}=on（当前未开启）——\
             跨站请求伪造防护为发布面硬要求；如需临时关闭请在非生产环境操作。"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test::TestRequest;

    #[test]
    fn scope_parsing_is_strict() {
        let ok = TestRequest::default()
            .insert_header((SCOPE_HEADER, "oa"))
            .to_http_request();
        assert_eq!(cookie_scope(&ok).as_deref(), Some("oa"));

        for bad in ["bad scope", "a".repeat(33).as_str(), "空格 "] {
            let r = TestRequest::default()
                .insert_header((SCOPE_HEADER, bad))
                .to_http_request();
            assert!(cookie_scope(&r).is_none(), "非法 scope 应回退缺省: {bad:?}");
        }
    }

    #[test]
    fn cookie_names_follow_scope() {
        assert_eq!(access_cookie_name(Some("oa")), "oa_access_token");
        assert_eq!(csrf_cookie_name(Some("oa")), "oa_csrf_token");
        assert_eq!(csrf_cookie_name(None), "csrf_token");
    }

    #[test]
    fn credential_source_priority() {
        // scoped cookie 优先于 Bearer（auth「访问令牌提取优先级」）
        let r = TestRequest::default()
            .insert_header((SCOPE_HEADER, "oa"))
            .insert_header(("Authorization", "Bearer bearer-token"))
            .cookie(Cookie::new("oa_access_token", "scoped-token"))
            .to_http_request();
        assert_eq!(
            resolve_credential(&r),
            Some(("scoped-token".to_string(), CredentialSource::ScopedCookie))
        );

        // Bearer 优先于环境 cookie
        let r = TestRequest::default()
            .insert_header(("Authorization", "Bearer bearer-token"))
            .cookie(Cookie::new("access_token", "env-cookie"))
            .to_http_request();
        assert_eq!(
            resolve_credential(&r),
            Some(("bearer-token".to_string(), CredentialSource::Bearer))
        );

        // 仅环境 cookie
        let r = TestRequest::default()
            .cookie(Cookie::new("access_token", "env-cookie"))
            .to_http_request();
        assert_eq!(
            resolve_credential(&r),
            Some(("env-cookie".to_string(), CredentialSource::Cookie))
        );
    }

    #[test]
    fn bearer_is_exempt_but_scoped_cookie_is_not() {
        let r = TestRequest::default()
            .insert_header(("Authorization", "Bearer x"))
            .insert_header((CSRF_HEADER, "whatever"))
            .to_http_request();
        assert_eq!(check_csrf(&r, CredentialSource::Bearer), Ok(()));

        // 同请求若按 scoped cookie 来源判定（存在 scope 头 ⇒ 实际以 cookie 鉴权）⇒ MUST 校验
        let r = TestRequest::default()
            .insert_header((SCOPE_HEADER, "oa"))
            .insert_header(("Authorization", "Bearer x"))
            .cookie(Cookie::new("oa_csrf_token", "abc"))
            .to_http_request();
        assert_eq!(
            check_csrf(&r, CredentialSource::ScopedCookie),
            Err(CsrfFailure::Missing)
        );
    }

    #[test]
    fn csrf_matrix() {
        // 缺失
        let r = TestRequest::default().to_http_request();
        assert_eq!(
            check_csrf(&r, CredentialSource::Cookie),
            Err(CsrfFailure::Missing)
        );

        // 不等
        let r = TestRequest::default()
            .cookie(Cookie::new("csrf_token", "abc"))
            .insert_header((CSRF_HEADER, "abd"))
            .to_http_request();
        assert_eq!(
            check_csrf(&r, CredentialSource::Cookie),
            Err(CsrfFailure::Mismatch)
        );

        // 相等
        let r = TestRequest::default()
            .cookie(Cookie::new("csrf_token", "abc"))
            .insert_header((CSRF_HEADER, "abc"))
            .to_http_request();
        assert_eq!(check_csrf(&r, CredentialSource::Cookie), Ok(()));

        // scoped 命名
        let r = TestRequest::default()
            .insert_header((SCOPE_HEADER, "oa"))
            .cookie(Cookie::new("oa_csrf_token", "abc"))
            .insert_header((CSRF_HEADER, "abc"))
            .to_http_request();
        assert_eq!(check_csrf(&r, CredentialSource::Cookie), Ok(()));
    }

    #[test]
    fn safe_methods_and_exempt_paths() {
        assert!(is_safe_method(&Method::GET));
        assert!(is_safe_method(&Method::HEAD));
        assert!(is_safe_method(&Method::OPTIONS));
        assert!(!is_safe_method(&Method::POST));

        assert!(is_csrf_exempt_path("/api/auth/refresh"));
        assert!(is_csrf_exempt_path("/auth/logout"));
        assert!(is_csrf_exempt_path("/api/auth/logout"));
        assert!(!is_csrf_exempt_path("/api/orders"));
        assert!(!is_csrf_exempt_path("/api/auth/refresh-all"));
    }

    #[test]
    fn enforce_switch_parsing() {
        assert!(!csrf_enforced_from(None));
        assert!(!csrf_enforced_from(Some("off")));
        assert!(csrf_enforced_from(Some("on")));
        assert!(csrf_enforced_from(Some(" TRUE ")));
        assert!(!csrf_enforced_from(Some("yes")));
    }

    #[test]
    fn prod_without_switch_refuses_start() {
        assert!(assert_csrf_enforced_in_prod(false).is_ok());
        // 生产 + 未开（测试进程内 env 未设）⇒ 拒启
        assert!(assert_csrf_enforced_in_prod(true).is_err());
    }

    #[test]
    fn csrf_cookie_is_readable_by_js() {
        let c = csrf_cookie(Some("oa"), "tok", false);
        assert!(
            !c.http_only().unwrap_or(false),
            "CSRF cookie MUST NOT 置 HttpOnly"
        );
        assert_eq!(c.name(), "oa_csrf_token");
        assert_eq!(c.path(), Some("/"));
        assert_eq!(c.max_age(), Some(Duration::seconds(CSRF_MAX_AGE_SECS)));
        assert_eq!(c.same_site(), Some(SameSite::Lax));
        assert_eq!(
            csrf_cookie(None, "tok", true).same_site(),
            Some(SameSite::Strict)
        );
        assert_eq!(
            csrf_clear_cookie(None, false).max_age(),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn gate_short_circuits_only_when_enforced() {
        // 默认 off ⇒ 放行
        let r = TestRequest::default()
            .method(Method::POST)
            .uri("/api/orders")
            .to_http_request();
        assert!(csrf_gate(&r, CredentialSource::Cookie).is_none());
    }

    #[test]
    fn token_is_256bit_base64url() {
        let t = new_csrf_token();
        assert_eq!(t.len(), 43, "32 字节 base64url 无填充 = 43 字符");
        assert_ne!(new_csrf_token(), t, "MUST 每次随机");
    }
}
