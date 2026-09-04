//! sso-remote 认证反代：将 Gateway 收到的 SSO 认证/NGAC 请求透明转发到独立部署的 SSO 服务。
//!
//! 适用场景（`sso-remote` feature）：Gateway 不内嵌 SSO，仅通过 `SSO_SERVICE_URL`
//! 将 PDP/JWT 校验委托给远程 SSO（见 `pep/middleware.rs` 的 `HttpNgacClient` /
//! `SsoJwksClient`）。此时 Gateway 进程内不再挂载 SSO 的认证路由，浏览器对
//! `/api/auth`、`/api/ngac` 的请求必须由本模块反向代理到 SSO。
//!
//! 注意：**不代理 `/auth/*`**（SSO 自身 UI 页面）——`sso-remote` 下由 Gateway 前端 SPA
//! 接管 `/auth/login` 等客户端路由；仅代理 API 前缀，避免把 SPA 导航误打到 SSO。
//!
//! Cookie 处理：SSO 签发 `access_token` / `refresh_token` 时**不设置 Domain 属性**
//! （见 `SSO/backend/src/auth/jwt.rs` 的 `set_cookie`），因此响应经本代理返回浏览器后，
//! 浏览器会将这些 HttpOnly cookie 绑定到 Gateway 的 origin（而非 SSO 的 host）。
//! 后续浏览器携带 cookie 访问 Gateway 受保护路由时，Gateway PEP 从 `access_token`
//! cookie 读取 JWT 并本地验签——链路天然闭合，无需改写 Set-Cookie 的 Domain。
//!
//! 已知限制：OAuth/OIDC 第三方登录的 `redirect_uri` 由 SSO 自身配置（指向 SSO host），
//! 远程模式下需将 SSO 的 redirect_uri 改为 Gateway 外部地址，否则回调会回到 SSO 而非
//! Gateway。基础账号密码 / 刷新 / 注销 / 会话（cookie）流程不受此限制影响。

use actix_web::{web, HttpRequest, HttpResponse};
use futures::StreamExt;

/// 将 actix 的 Method 映射到 reqwest 的 Method。
fn to_reqwest_method(m: &actix_web::http::Method) -> Option<reqwest::Method> {
    use actix_web::http::Method as M;
    Some(match *m {
        M::GET => reqwest::Method::GET,
        M::POST => reqwest::Method::POST,
        M::PUT => reqwest::Method::PUT,
        M::DELETE => reqwest::Method::DELETE,
        M::PATCH => reqwest::Method::PATCH,
        M::HEAD => reqwest::Method::HEAD,
        M::OPTIONS => reqwest::Method::OPTIONS,
        _ => return None,
    })
}

/// 跳过的逐跳（hop-by-hop）请求头，避免代理破坏连接语义。
fn is_hop_by_hop_request(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host" | "content-length" | "connection" | "transfer-encoding" | "upgrade"
    )
}

/// 透明转发 handler：保留原始 path（含 `/api/auth` 等前缀）与 query。
async fn proxy_handler(
    req: HttpRequest,
    body: web::Payload,
    client: web::Data<reqwest::Client>,
    base: web::Data<String>,
) -> HttpResponse {
    let path = req.path().to_string();
    let query = req.query_string();
    let target = format!("{}{}", base.trim_end_matches('/'), path);
    let target = if query.is_empty() {
        target
    } else {
        format!("{}?{}", target, query)
    };

    let method = match to_reqwest_method(req.method()) {
        Some(m) => m,
        None => return HttpResponse::BadRequest().body("unsupported method"),
    };

    // 收集请求体（认证端点 payload 小，缓冲即可，无需真流式）。
    let mut body_stream = body;
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = body_stream.next().await {
        match chunk {
            Ok(c) => buf.extend_from_slice(&c),
            Err(e) => return HttpResponse::BadGateway().body(format!("read body: {}", e)),
        }
    }
    let body_bytes = buf;

    // 用 owned HeaderName/HeaderValue 构建请求头（避开 reqwest header() 对 &str 生命周期的约束）。
    let mut header_map = reqwest::header::HeaderMap::new();
    for (k, v) in req.headers().iter() {
        if is_hop_by_hop_request(k.as_str()) {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(k.as_str().as_bytes()),
            reqwest::header::HeaderValue::from_bytes(v.as_bytes()),
        ) {
            header_map.insert(name, value);
        }
    }
    let rb = client.request(method, &target).headers(header_map);

    let upstream = if body_bytes.is_empty() {
        rb.send().await
    } else {
        rb.body(body_bytes).send().await
    };

    let upstream = match upstream {
        Ok(u) => u,
        Err(e) => {
            return HttpResponse::BadGateway().body(format!("sso upstream error: {}", e));
        }
    };

    // upstream.status() 是 reqwest(http crate) 的 StatusCode，需转换为 actix 的 StatusCode。
    let actix_status = actix_web::http::StatusCode::from_u16(upstream.status().as_u16())
        .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = HttpResponse::build(actix_status);
    for (k, v) in upstream.headers().iter() {
        if matches!(
            k.as_str().to_ascii_lowercase().as_str(),
            "connection" | "transfer-encoding"
        ) {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            actix_web::http::header::HeaderName::from_bytes(k.as_str().as_bytes()),
            actix_web::http::header::HeaderValue::from_bytes(v.as_bytes()),
        ) {
            builder.insert_header((name, value));
        }
    }

    match upstream.bytes().await {
        Ok(b) => builder.body(b.to_vec()),
        Err(e) => HttpResponse::BadGateway().body(format!("sso body error: {}", e)),
    }
}

/// 在指定 scope（/api/auth、/auth、/api/ngac）下挂载反代 handler，匹配全部子路径与方法。
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/{tail:.*}").route(web::route().to(proxy_handler)));
}
