use actix_web::{web, HttpRequest, HttpResponse, Result};

use super::discovery::PreprocDiscovery;
use super::{AppListResponse, PreprocApp, PreprocFile};

/// 获取 Pre-Proc 应用列表处理器
pub async fn list_preproc_apps(
    discovery: web::Data<std::sync::Mutex<PreprocDiscovery>>,
) -> Result<HttpResponse> {
    let mut discovery = discovery
        .lock()
        .map_err(|e| actix_web::error::ErrorInternalServerError(format!("Lock error: {}", e)))?;

    // 懒加载：首次访问时自动扫描
    if let Err(e) = discovery.ensure_scanned() {
        common::telemetry::warn!("Pre-Proc lazy scan failed: {}", e);
    }

    let apps: Vec<PreprocApp> = discovery.get_apps().values().cloned().collect();

    Ok(HttpResponse::Ok().json(AppListResponse {
        total: apps.len(),
        apps,
    }))
}

/// 获取单个 Pre-Proc 应用处理器
pub async fn get_preproc_app(
    path: web::Path<String>,
    discovery: web::Data<std::sync::Mutex<PreprocDiscovery>>,
) -> Result<HttpResponse> {
    let mut discovery = discovery
        .lock()
        .map_err(|e| actix_web::error::ErrorInternalServerError(format!("Lock error: {}", e)))?;

    // 懒加载：首次访问时自动扫描
    if let Err(e) = discovery.ensure_scanned() {
        common::telemetry::warn!("Pre-Proc lazy scan failed: {}", e);
    }

    match discovery.get_app(&path.into_inner()) {
        Some(app) => Ok(HttpResponse::Ok().json(app)),
        None => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "App not found"
        }))),
    }
}

/// 刷新 Pre-Proc 应用列表处理器
pub async fn refresh_preproc_apps(
    discovery: web::Data<std::sync::Mutex<PreprocDiscovery>>,
) -> Result<HttpResponse> {
    let mut discovery = discovery
        .lock()
        .map_err(|e| actix_web::error::ErrorInternalServerError(format!("Lock error: {}", e)))?;

    match discovery.scan() {
        Ok(apps) => Ok(HttpResponse::Ok().json(AppListResponse {
            total: apps.len(),
            apps,
        })),
        Err(e) => Ok(HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("Scan failed: {}", e)
        }))),
    }
}

/// 获取 Pre-Proc 应用文件列表处理器
pub async fn list_preproc_app_files(
    path: web::Path<String>,
    discovery: web::Data<std::sync::Mutex<PreprocDiscovery>>,
) -> Result<HttpResponse> {
    let app_name = path.into_inner();

    let discovery = discovery
        .lock()
        .map_err(|e| actix_web::error::ErrorInternalServerError(format!("Lock error: {}", e)))?;

    let app = match discovery.get_app(&app_name) {
        Some(app) => app,
        None => {
            return Ok(HttpResponse::NotFound().json(serde_json::json!({
                "error": format!("App '{}' not found", app_name)
            })));
        }
    };

    // 扫描应用目录下的所有文件
    let mut files = Vec::new();

    if app.has_backend {
        let backend_path = app.path.join("backend");
        if let Ok(entries) = std::fs::read_dir(&backend_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let relative_path = path
                            .strip_prefix(&app.path)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .to_string();
                        files.push(PreprocFile {
                            path: relative_path,
                            content,
                        });
                    }
                }
            }
        }
    }

    if app.has_frontend {
        let frontend_path = app.path.join("frontend");
        if let Ok(entries) = std::fs::read_dir(&frontend_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let relative_path = path
                            .strip_prefix(&app.path)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .to_string();
                        files.push(PreprocFile {
                            path: relative_path,
                            content,
                        });
                    }
                }
            }
        }
    }

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "app_name": app_name,
        "files": files,
        "total": files.len(),
    })))
}

/// 从 .mise.toml 文件中解析 SERVER_ADDR
fn parse_mise_server_addr(app_path: &std::path::Path) -> Option<String> {
    let mise_path = app_path.join(".mise.toml");
    if !mise_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&mise_path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("SERVER_ADDR") {
            if let Some(eq_pos) = trimmed.find('=') {
                let value = trimmed[eq_pos + 1..].trim().trim_matches('"');
                return Some(value.to_string());
            }
        }
    }
    None
}

/// 构建应用后端目标地址
fn resolve_app_backend_url(app: &PreprocApp) -> Option<String> {
    // 1. 优先从 .mise.toml 的 SERVER_ADDR 解析
    if let Some(addr) = parse_mise_server_addr(&app.path) {
        return Some(format!("http://{}", addr));
    }

    // 2. 从 app.config.backend_port 构建
    if let Some(config) = &app.config {
        if let Some(port) = config.backend_port {
            return Some(format!("http://127.0.0.1:{}", port));
        }
    }

    None
}

/// 代理请求到 Pre-Proc 应用
///
/// 将 /preproc/apps/{name}/{path:.*} 转发到应用实例的后端服务。
/// 临时方案：基于 HTTP 反向代理。未来应改为 Library crate 直接注册到 Gateway。
pub async fn proxy_to_preproc_app(
    req: HttpRequest,
    payload: web::Payload,
    path: web::Path<(String, String)>,
    discovery: web::Data<std::sync::Mutex<PreprocDiscovery>>,
) -> Result<HttpResponse> {
    let (app_name, app_path) = path.into_inner();

    let base_url = {
        let discovery = discovery.lock().map_err(|e| {
            actix_web::error::ErrorInternalServerError(format!("Lock error: {}", e))
        })?;

        let app = match discovery.get_app(&app_name) {
            Some(app) => app,
            None => {
                return Ok(HttpResponse::NotFound().json(serde_json::json!({
                    "error": format!("App '{}' not found", app_name)
                })));
            }
        };

        match resolve_app_backend_url(app) {
            Some(url) => url,
            None => {
                return Ok(HttpResponse::ServiceUnavailable().json(serde_json::json!({
                    "error": format!(
                        "App '{}' has no configured backend endpoint",
                        app_name
                    )
                })));
            }
        }
    };

    let query = req
        .uri()
        .query()
        .map(|q| format!("?{}", q))
        .unwrap_or_default();
    let target_url = format!("{}/{}{}", base_url, app_path, query);

    common::telemetry::debug!("Pre-Proc proxy: {} -> {}", req.uri(), target_url);

    let client = req
        .app_data::<web::Data<awc::Client>>()
        .cloned()
        .unwrap_or_else(|| web::Data::new(awc::Client::new()));
    let mut forwarded_req = client.request_from(target_url, req.head());

    for (header_name, header_value) in req.headers() {
        if header_name != actix_web::http::header::HOST {
            forwarded_req =
                forwarded_req.insert_header((header_name.clone(), header_value.clone()));
        }
    }

    let resp = forwarded_req
        .send_stream(payload)
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?;

    let status = resp.status();
    let mut client_resp = HttpResponse::build(status);

    for (header_name, header_value) in resp.headers() {
        if header_name != actix_web::http::header::TRANSFER_ENCODING
            && header_name != actix_web::http::header::CONTENT_LENGTH
        {
            if header_name == actix_web::http::header::SET_COOKIE {
                client_resp.append_header((header_name.clone(), header_value.clone()));
            } else {
                client_resp.insert_header((header_name.clone(), header_value.clone()));
            }
        }
    }

    Ok(client_resp.streaming(resp))
}
