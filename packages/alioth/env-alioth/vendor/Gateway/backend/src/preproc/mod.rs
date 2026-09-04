//! Pre-Proc 应用发现和路由模块
//!
//! 自动扫描 Pre-Proc/Apps/ 目录下的应用，并为每个应用创建路由规则

pub mod discovery;
// routes（/preproc 反代处理器）仅 preproc-proxy feature 编译：生产构建
// （build-ns.sh --no-default-features）不含该 feature → 路由不注册（404），
// 处理器代码亦不编译（避免 dead_code 告警）。
#[cfg(feature = "preproc-proxy")]
pub mod routes;

#[cfg(feature = "preproc-proxy")]
use actix_web::web;
pub use discovery::PreprocDiscovery;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreprocApp {
    /// 应用编码（从 app.json code 提取，或 fallback 到目录名）
    pub code: String,
    /// 应用展示名称（从 app.json name 提取，或 fallback 到目录名）
    pub name: String,
    pub namespace: Option<String>,
    pub path: PathBuf,
    pub has_backend: bool,
    pub has_frontend: bool,
    /// 模块列表（从 app.json config.modules 提取）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<String>,
    pub config: Option<AppConfig>,
    /// 导航分组（从 app.json `navigation` 提取）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation: Option<Vec<NavGroup>>,
    /// 路由配置（从 app.json `routing` 提取）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<Routing>,
    /// 权限配置（从 app.json `permissions` 提取，作为 NGAC 覆盖层）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,
    /// 品牌配置（从 app.json `brand` 提取）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<Brand>,
    /// 应用目标描述（从 app.json `goal` 提取）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// 应用范围外说明（从 app.json `nonScope` 提取）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_scope: Option<Vec<String>>,
    /// 应用目标描述（从 app.json 顶层 `description` 提取，前端 HomePage 卡片消费）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 部署模式（app.json `deploymentMode`：single_process/multi_process/remote；null=单进程）
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "deploymentMode"
    )]
    pub deployment_mode: Option<String>,
    /// 外部入口 URL（app.json `endpointUrl`，multi_process/remote 模式使用）
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "endpointUrl"
    )]
    pub endpoint_url: Option<String>,
}

/// 应用配置 (从 config/app.yaml 读取)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub backend_port: Option<u16>,
    pub frontend_port: Option<u16>,
}

/// 应用导航分组（从 app.json `navigation` 提取，对齐 AppAgent `NavGroupJson`）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NavGroup {
    pub group: String,
    pub icon: Option<String>,
    #[serde(default)]
    pub modules: Vec<String>,
}

/// 应用路由配置（从 app.json `routing` 提取，对齐 AppAgent `RoutingJson`）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Routing {
    pub base: Option<String>,
    pub default_route: Option<String>,
}

/// 应用权限配置（从 app.json `permissions` 提取，对齐 AppAgent `PermissionsJson`）
///
/// 注意：这是 NGAC PDP 之上的「覆盖层」，不取代 NGAC 的权威决策。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    #[serde(default)]
    pub default_roles: Vec<String>,
    #[serde(default)]
    pub public_paths: Vec<String>,
    #[serde(default)]
    pub admin_roles: Vec<String>,
}

/// 应用品牌配置（从 app.json `brand` 提取，对齐 AppAgent `BrandJson`）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Brand {
    pub primary: Option<String>,
    pub logo: Option<String>,
}

/// Pre-Proc 文件信息（仅 preproc-proxy 反代使用）
#[cfg(feature = "preproc-proxy")]
#[derive(Serialize)]
pub struct PreprocFile {
    pub path: String,
    pub content: String,
}

/// 应用列表响应（仅 preproc-proxy 反代使用）
#[cfg(feature = "preproc-proxy")]
#[derive(Serialize)]
pub struct AppListResponse {
    pub apps: Vec<PreprocApp>,
    pub total: usize,
}

/// 配置 Pre-Proc 路由
///
/// 支持两种路径模式：
/// 1. 兼容模式: /preproc/apps/{name}/{path}
/// 2. 统一 API 模式: /api/pre_proc/{app}/{path}
#[cfg(feature = "preproc-proxy")]
pub fn configure_preproc_routes(cfg: &mut web::ServiceConfig) {
    use self::routes::{
        get_preproc_app, list_preproc_app_files, list_preproc_apps, proxy_to_preproc_app,
        refresh_preproc_apps,
    };

    // 兼容模式：保留 /preproc 路由
    cfg.service(
        web::scope("/preproc")
            .route("/apps", web::get().to(list_preproc_apps))
            .route("/apps/refresh", web::post().to(refresh_preproc_apps))
            .route("/apps/{name}", web::get().to(get_preproc_app))
            .route("/apps/{name}/files", web::get().to(list_preproc_app_files))
            .route(
                "/apps/{name}/{path:.*}",
                web::route().to(proxy_to_preproc_app),
            ),
    );
    // 统一 API 模式：/api/pre_proc/{app}/{module}/{path}
    cfg.service(
        web::scope("/api/pre_proc")
            .route("/apps", web::get().to(list_preproc_apps))
            .route("/apps/refresh", web::post().to(refresh_preproc_apps))
            .route("/apps/{name}", web::get().to(get_preproc_app))
            .route("/apps/{name}/files", web::get().to(list_preproc_app_files))
            .route("/{name}/{path:.*}", web::route().to(proxy_to_preproc_app)),
    );
}
