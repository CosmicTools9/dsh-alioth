use actix_web::{web, HttpResponse, Result};
use serde::Serialize;
use std::collections::HashMap;

/// 按 namespace 分组的路由响应
#[derive(Serialize)]
pub struct ActiveRoutesResponse {
    pub namespaces: Vec<NamespaceRoutes>,
}

#[derive(Serialize)]
pub struct NamespaceRoutes {
    pub namespace: String,
    pub modules: Vec<ModuleEntry>,
}

#[derive(Serialize)]
pub struct ModuleEntry {
    pub module_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_capabilities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_blocks: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_declarations: Option<serde_json::Value>,
}

/// GET /api/apps/overrides 的响应条目
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCapabilityOverride {
    pub app_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_capabilities: Option<HashMap<String, bool>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppInstance {
    pub namespace: String,
    pub modules: Vec<String>,
    pub app_code: String,
    pub app_capability_overrides: Option<HashMap<String, bool>>,
    /// 应用展示名称（app.json `name`）
    pub name: Option<String>,
    /// 应用目标描述（app.json `description`，前端 HomePage 卡片消费）
    pub description: Option<String>,
    /// 状态（app.json `status`，如 developing）
    pub status: Option<String>,
    /// 版本（app.json `version`）
    pub version: Option<String>,
    /// 部署模式（app.json `deploymentMode`：single_process/multi_process/remote；null=单进程）
    pub deployment_mode: Option<String>,
    /// 外部入口 URL（app.json `endpointUrl`，multi_process/remote 模式使用）
    pub endpoint_url: Option<String>,
}

/// GET /api/apps 响应条目（与前端 AppInfo 契约对齐）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfoResponse {
    pub code: String,
    pub name: Option<String>,
    pub namespace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment_mode: Option<String>,
    pub modules: Vec<String>,
    pub config: AppInfoConfig,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfoConfig {
    pub modules: Vec<String>,
}

/// 查找 Pre-Proc/ 目录（支持多种 CWD + DEPLOY_PATH 优先）
fn resolve_pre_proc_dir() -> Option<std::path::PathBuf> {
    // 1. DEPLOY_PATH 的父目录下的 Pre-Proc（release 模式）
    if let Ok(p) = std::env::var("DEPLOY_PATH") {
        let path = std::path::Path::new(&p).join("Pre-Proc");
        if path.is_dir() {
            return Some(path);
        }
    }

    // 2. 自动探测 Pre-Proc/ 目录
    let cwd = std::env::current_dir().ok()?;
    let candidates = [
        cwd.join("Pre-Proc"),       // from project root
        cwd.join("../Pre-Proc"),    // from Gateway/
        cwd.join("../../Pre-Proc"), // from Gateway/backend/
    ];
    candidates.into_iter().find(|p| p.is_dir())
}

/// 从单个 app JSON 对象解析 AppInstance
/// fallback_ns: 开发模式扫描时传入路径推导的 namespace（字段缺失时兜底，不一致时 warn）；
/// None = 聚合模式（apps.json 无路径上下文），字段缺失保持丢弃。
fn parse_single_app(value: &serde_json::Value, fallback_ns: Option<&str>) -> Option<AppInstance> {
    let namespace = match value["namespace"].as_str() {
        Some(field_ns) => {
            if let Some(path_ns) = fallback_ns {
                if field_ns != path_ns {
                    common::telemetry::warn!(
                        "app.json namespace '{}' 与路径 '{}' 不一致（漂移由检查脚本阻断）",
                        field_ns,
                        path_ns
                    );
                }
            }
            field_ns.to_string()
        }
        None => {
            let path_ns = fallback_ns?;
            common::telemetry::warn!("app.json 缺 namespace，按路径 '{}' 兜底", path_ns);
            path_ns.to_string()
        }
    };
    let mut modules = Vec::new();
    if let Some(config) = value["config"].as_object() {
        if let Some(mods) = config["modules"].as_array() {
            for m in mods {
                if let Some(name) = m.as_str() {
                    modules.push(name.to_string());
                }
            }
        }
    }
    let app_code = value["code"]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| {
            // 历史兼容：旧产物使用 appCode（schema 已废弃该字段）
            if let Some(old) = value["appCode"].as_str() {
                common::telemetry::warn!("app.json 使用废弃字段 appCode='{}'，应迁移为 code", old);
                Some(old.to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| namespace.clone());
    let app_capability_overrides = value
        .get("appCapabilityOverrides")
        .and_then(|v| serde_json::from_value::<HashMap<String, bool>>(v.clone()).ok());
    let name = value["name"].as_str().map(|s| s.to_string());
    let description = value["description"].as_str().map(|s| s.to_string());
    let status = value["status"].as_str().map(|s| s.to_string());
    let version = value["version"].as_str().map(|s| s.to_string());
    let deployment_mode = value["deploymentMode"].as_str().map(|s| s.to_string());
    let endpoint_url = value["endpointUrl"].as_str().map(|s| s.to_string());
    Some(AppInstance {
        namespace,
        modules,
        app_code,
        app_capability_overrides,
        name,
        description,
        status,
        version,
        deployment_mode,
        endpoint_url,
    })
}

/// 解析聚合格式 `{ "appInstances": [...] }` 的 {DEPLOY_PATH}/apps.json（生产模式）
fn parse_apps_json(content: &str) -> Vec<AppInstance> {
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(root) => {
            let mut instances = Vec::new();
            if let Some(arr) = root["appInstances"].as_array() {
                for app in arr {
                    if let Some(instance) = parse_single_app(app, None) {
                        instances.push(instance);
                    }
                }
            }
            instances
        }
        Err(e) => {
            common::telemetry::warn!("Failed to parse apps.json: {}", e);
            vec![]
        }
    }
}

/// 从 Pre-Proc/ 目录树读取所有 app 实例
/// 从 Pre-Proc/ 目录树读取所有 app 实例
///
/// 两种模式：
/// - DEPLOY_PATH 环境变量（生产）：读取 {DEPLOY_PATH}/apps.json 聚合文件
/// - 其他（开发）：扫描 Pre-Proc/{namespace}/Apps/*/app.json 目录树自动发现
///   从 `{deploy_path}/Apps/*/app.json` 扫描聚合 App 实例（生产模式自愈源）。
///   语义对齐 dev 模式扫描：有 app.json 即为有效应用；namespace 字段缺失按 None 丢弃
///   （apps.json 聚合无路径上下文，与 parse_apps_json 契约一致）。
fn aggregate_apps_from_dir(deploy_path: &str) -> Vec<AppInstance> {
    let apps_dir = std::path::Path::new(deploy_path).join("Apps");
    let mut instances = Vec::new();
    let Ok(entries) = std::fs::read_dir(&apps_dir) else {
        return instances;
    };
    for entry in entries.flatten() {
        // 跳过特殊目录（对齐 preproc/discovery.rs 语义 + shell glob 行为）：
        // 隐藏备份（.xxx.bak）与下划线目录不参与聚合——start.sh/release 脚本
        // 的 compgen 同样不会匹配，避免自愈与 release 产物 app 集合不一致。
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name.starts_with('_') {
            continue;
        }
        let app_json = entry.path().join("app.json");
        if !app_json.exists() {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&app_json) {
            if let Ok(root) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(app) = parse_single_app(&root, None) {
                    instances.push(app);
                }
            }
        }
    }
    instances
}

/// 扫描 `{deploy_path}/Apps/*/app.json` 返回**原始 JSON 内容**（apps.json 写回专用）。
///
/// 写回必须保持原始 app.json 结构（`config.modules` 等 parse_single_app 契约字段），
/// 与 release-to-namespace.sh / build-ns.sh / start.sh 的 `jq -n '{appInstances: [inputs]}'`
/// 输出同构。禁止用 AppInstance 序列化（顶层扁平 `modules`/`appCode`）写回——扁平格式
/// 与 parse_single_app 读取契约不匹配，自愈写回后下次启动模块列表丢失
/// （2026-08-22 验证环境事故实证，openspec fix-apps-json-self-heal-roundtrip）。
fn aggregate_raw_apps_from_dir(deploy_path: &str) -> Vec<serde_json::Value> {
    let apps_dir = std::path::Path::new(deploy_path).join("Apps");
    let mut raw = Vec::new();
    let Ok(entries) = std::fs::read_dir(&apps_dir) else {
        return raw;
    };
    for entry in entries.flatten() {
        // 跳过特殊目录（.xxx.bak 等），与 aggregate_apps_from_dir / shell glob 一致
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name.starts_with('_') {
            continue;
        }
        let app_json = entry.path().join("app.json");
        if !app_json.exists() {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&app_json) {
            if let Ok(root) = serde_json::from_str::<serde_json::Value>(&content) {
                raw.push(root);
            }
        }
    }
    raw
}

fn read_apps_data() -> Vec<AppInstance> {
    // Phase 1: DEPLOY_PATH 模式（production/release）—— 从聚合文件读取
    if let Ok(deploy_path) = std::env::var("DEPLOY_PATH") {
        let path = std::path::Path::new(&deploy_path).join("apps.json");
        match std::fs::read_to_string(&path) {
            Ok(content) => return parse_apps_json(&content),
            Err(read_err) => {
                // 自愈（add-gateway-apps-json-self-heal，对齐 ensure_gateway_seed_self_check 哲学）：
                // apps.json 缺失/不可读 → 从 {DEPLOY_PATH}/Apps/*/app.json 扫描聚合，
                // 写回 apps.json（幂等持久化）后返回——/api/apps 与 /api/apps/routes 即刻修复，
                // 不依赖人工（2026-08-20 验证环境 rsync --delete 误删 apps.json 事故）。
                common::telemetry::warn!(
                    "read_apps_data: apps.json 不可读（{read_err}），尝试从 Apps/ 目录自愈聚合"
                );
                let instances = aggregate_apps_from_dir(&deploy_path);
                if instances.is_empty() {
                    common::telemetry::warn!(
                        "apps.json 自愈失败：{}/Apps 无有效 app.json",
                        deploy_path
                    );
                    return instances;
                }
                // 写回 MUST 为原始 app.json 嵌入（与 release 脚本 jq 同构）——
                // AppInstance 序列化（扁平 modules）与 parse_single_app 的
                // config.modules 契约不匹配，会破坏 round-trip（2026-08-22 事故）。
                let agg = serde_json::json!({ "appInstances": aggregate_raw_apps_from_dir(&deploy_path) });
                match std::fs::write(
                    &path,
                    serde_json::to_string_pretty(&agg).unwrap_or_default(),
                ) {
                    Ok(()) => common::telemetry::info!(
                        "apps.json 自愈成功：从 Apps/ 聚合 {} 个 App，已写回 {}",
                        instances.len(),
                        path.display()
                    ),
                    Err(e) => common::telemetry::warn!(
                        "apps.json 自愈聚合成功但写回失败（不影响本次返回）: {}",
                        e
                    ),
                }
                return instances;
            }
        }
    }

    // Phase 2: Dev 模式 —— 扫描 Pre-Proc/{namespace}/Apps/*/app.json
    let pre_proc_dir = match resolve_pre_proc_dir() {
        Some(d) => d,
        None => {
            common::telemetry::warn!("Pre-Proc/ directory not found");
            return vec![];
        }
    };
    common::telemetry::info!(
        "read_apps_data scanning pre_proc_dir: {}",
        pre_proc_dir.display()
    );

    let mut instances = Vec::new();
    let namespace = std::env::var("NAMESPACE").unwrap_or_default();

    // 当 NAMESPACE 设置时，只扫描该 namespace 目录；
    // 未设置时扫描全部 namespace（向后兼容）
    let ns_dirs: Vec<std::path::PathBuf> = if !namespace.is_empty() {
        let ns_dir = pre_proc_dir.join(&namespace);
        if ns_dir.is_dir() {
            vec![ns_dir]
        } else {
            vec![]
        }
    } else if let Ok(entries) = std::fs::read_dir(&pre_proc_dir) {
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect()
    } else {
        vec![]
    };

    for ns_dir in ns_dirs {
        let apps_dir = ns_dir.join("Apps");
        if !apps_dir.is_dir() {
            continue;
        }
        if let Ok(app_entries) = std::fs::read_dir(&apps_dir) {
            for app_entry in app_entries.flatten() {
                // 跳过特殊目录（对齐 aggregate_apps_from_dir / preproc/discovery.rs 语义）：
                // 隐藏备份（.xxx.bak）与下划线目录不参与注册——否则备份的旧版 app.json
                // 会被当成合法应用，/api/apps 出现重复应用且模块列表错乱。
                let dir_name = app_entry.file_name();
                let dir_name = dir_name.to_string_lossy();
                if dir_name.starts_with('.') || dir_name.starts_with('_') {
                    continue;
                }
                let app_json = app_entry.path().join("app.json");
                if !app_json.exists() {
                    continue;
                }

                if let Ok(content) = std::fs::read_to_string(&app_json) {
                    if let Ok(root) = serde_json::from_str::<serde_json::Value>(&content) {
                        // 路径权威兜底：ns 来自目录名（字段缺失不再丢弃 app，不一致 warn）
                        let ns_from_path = ns_dir
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_string();
                        if let Some(app) = parse_single_app(&root, Some(&ns_from_path)) {
                            instances.push(app);
                        }
                    }
                }
            }
        }
    }

    if instances.is_empty() {
        if !namespace.is_empty() {
            common::telemetry::warn!("No apps found in Pre-Proc/{}/Apps/", namespace);
        } else {
            common::telemetry::warn!("No apps found in Pre-Proc directory scan");
        }
    }

    instances
}

/// GET /api/apps/routes
///
/// 返回按 namespace 分组活跃模块：
/// ```json
/// { "namespaces": [{ "namespace": "AVIC-CAASEC", "modules": [{ "module_id": "system-dev" }] }] }
/// ```
pub async fn get_active_routes() -> Result<HttpResponse> {
    let apps = read_apps_data();

    // 按 namespace 分组去重
    let mut namespace_map: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for app in &apps {
        let entry = namespace_map.entry(app.namespace.clone()).or_default();
        for m in &app.modules {
            if !entry.contains(m) {
                entry.push(m.clone());
            }
        }
    }

    // 若设置了 NAMESPACE 则按 namespace 过滤；未设置时返回所有 namespace
    if let Ok(gateway_ns) = std::env::var("NAMESPACE") {
        if !gateway_ns.is_empty() {
            namespace_map.retain(|k, _| k == &gateway_ns);
        }
    }

    let preproc_dir = resolve_pre_proc_dir();

    let namespaces: Vec<NamespaceRoutes> = namespace_map
        .into_iter()
        .map(|(namespace, mut modules)| {
            modules.sort();
            NamespaceRoutes {
                namespace: namespace.clone(),
                modules: modules
                    .into_iter()
                    .map(|id| {
                        let ns = namespace.clone();
                        // 读取 module.json 提取 name 和 workspace 能力声明
                        let mut name = None;
                        let mut workspace_capabilities = None;
                        let mut workspace_blocks = None;
                        let mut workspace_declarations = None;

                        if let Some(base) = preproc_dir.as_ref() {
                            // Sources 全量镜像：优先 Sources/Apps（Gateway 侧），
                            // 未迁移 namespace 回退扁平 Sources。
                            let ns_root = base.join(&ns);
                            let sources = {
                                let apps = ns_root.join("Sources").join("Apps");
                                if apps.is_dir() {
                                    apps
                                } else {
                                    ns_root.join("Sources")
                                }
                            };
                            let path = sources.join("Modules").join(&id).join("module.json");
                            if let Ok(content) = std::fs::read_to_string(path) {
                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                                    name = v["name"].as_str().map(|s| s.to_string());
                                    workspace_capabilities = v
                                        .get("workspaceCapabilities")
                                        .and_then(|c| serde_json::from_value(c.clone()).ok());
                                    workspace_blocks = v.get("workspaceBlocks").cloned();
                                    workspace_declarations =
                                        v.get("workspaceDeclarations").cloned();
                                }
                            }
                        }

                        ModuleEntry {
                            module_id: id,
                            name,
                            workspace_capabilities,
                            workspace_blocks,
                            workspace_declarations,
                        }
                    })
                    .collect(),
            }
        })
        .collect();
    let response = ActiveRoutesResponse { namespaces };
    Ok(HttpResponse::Ok().json(response))
}

/// GET /api/apps/overrides
///
/// 返回所有 app 的能力覆盖配置（来自 app.json 中的 appCapabilityOverrides）：
/// ```json
/// { "apps": [{ "appCode": "...", "workspaceCapabilities": { "approval": true, "ai": false } }] }
/// ```
pub async fn get_app_overrides() -> Result<HttpResponse> {
    let apps = read_apps_data();
    let overrides: Vec<AppCapabilityOverride> = apps
        .into_iter()
        .filter(|a| a.app_capability_overrides.is_some())
        .map(|a| AppCapabilityOverride {
            app_code: a.app_code,
            workspace_capabilities: a.app_capability_overrides,
        })
        .collect();
    Ok(HttpResponse::Ok().json(serde_json::json!({ "apps": overrides })))
}
/// GET /api/apps
///
/// 返回应用实例列表（standalone 构建可用——不依赖 preproc-proxy feature）。
/// 响应字段与前端 `AppInfo` 类型对齐：
/// ```json
/// { "apps": [{ "code": "...", "name": "...", "namespace": "WZ",
///              "modules": ["..."], "config": { "modules": [...] } }] }
/// ```
pub async fn list_apps() -> Result<HttpResponse> {
    let apps: Vec<AppInfoResponse> = read_apps_data()
        .into_iter()
        .map(|a| AppInfoResponse {
            code: a.app_code,
            name: a.name,
            namespace: a.namespace,
            description: a.description,
            status: a.status,
            version: a.version,
            endpoint_url: a.endpoint_url,
            deployment_mode: a.deployment_mode,
            modules: a.modules.clone(),
            config: AppInfoConfig { modules: a.modules },
        })
        .collect();
    Ok(HttpResponse::Ok().json(serde_json::json!({ "apps": apps })))
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/apps", web::get().to(list_apps));
    cfg.route("/apps/routes", web::get().to(get_active_routes));
    cfg.route("/apps/overrides", web::get().to(get_app_overrides));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_missing_namespace_falls_back_to_path() {
        // 开发模式：字段缺失 → 路径兜底（不丢弃 app）
        let app = parse_single_app(&json!({ "code": "ai-x" }), Some("WZ"));
        let inst = app.expect("字段缺失 + 路径兜底应加载");
        assert_eq!(inst.namespace, "WZ");
        // code 字段优先（schema 权威）
        assert_eq!(inst.app_code, "ai-x");
    }

    #[test]
    fn parse_app_code_legacy_app_code_compat() {
        // 历史兼容：仅 appCode 字段时采用之（warn 提示迁移）
        let app = parse_single_app(&json!({ "appCode": "legacy-app" }), Some("WZ"));
        let inst = app.expect("appCode 兼容路径应加载");
        assert_eq!(inst.app_code, "legacy-app");
    }

    #[test]
    fn parse_app_code_missing_falls_back_to_namespace() {
        // 双字段缺失 → namespace 兜底
        let app = parse_single_app(&json!({ "namespace": "WZ" }), Some("WZ"));
        let inst = app.expect("字段缺失应兜底加载");
        assert_eq!(inst.app_code, "WZ");
    }

    #[test]
    fn parse_missing_namespace_aggregate_mode_rejects() {
        // 聚合模式（apps.json 无路径上下文）：字段缺失保持丢弃
        let app = parse_single_app(&json!({ "code": "ai-x" }), None);
        assert!(app.is_none());
    }

    #[test]
    fn parse_field_matches_path() {
        let app = parse_single_app(&json!({ "namespace": "WZ" }), Some("WZ"));
        let inst = app.expect("字段与路径一致应加载");
        assert_eq!(inst.namespace, "WZ");
    }

    #[test]
    fn parse_field_mismatch_path_warns_but_loads() {
        // 字段≠路径：warn 不阻断运行时（漂移由检查脚本阻断）
        let app = parse_single_app(&json!({ "namespace": "Alioth" }), Some("WZ"));
        let inst = app.expect("字段不一致应仍加载（warn）");
        assert_eq!(inst.namespace, "Alioth"); // 字段优先，路径为兜底
    }

    // ── apps.json 自愈（add-gateway-apps-json-self-heal）──

    #[test]
    fn aggregate_apps_from_dir_loads_apps() {
        let dir = std::env::temp_dir().join(format!("ctg-heal-agg-{}", std::process::id()));
        let apps_dir = dir.join("Apps").join("ct-git");
        std::fs::create_dir_all(&apps_dir).expect("mkdir");
        std::fs::write(
            apps_dir.join("app.json"),
            json!({ "code": "ct-git", "namespace": "Cosmic-Tools", "config": { "modules": ["repositories"] } }).to_string(),
        )
        .expect("write app.json");
        let instances = aggregate_apps_from_dir(dir.to_str().unwrap());
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].app_code, "ct-git");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn aggregate_apps_from_dir_empty_when_no_apps() {
        let dir = std::env::temp_dir().join(format!("ctg-heal-empty-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Apps")).expect("mkdir");
        let instances = aggregate_apps_from_dir(dir.to_str().unwrap());
        assert!(instances.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn self_heal_writeback_roundtrip_preserves_modules() {
        // 回归 2026-08-22 事故：自愈写回 MUST 保持原始 app.json 结构
        // （config.modules 契约），禁止 AppInstance 扁平序列化——否则
        // 下次启动 parse_apps_json 解析 modules 为空。
        let dir = std::env::temp_dir().join(format!("ctg-heal-rt-{}", std::process::id()));
        let apps_dir = dir.join("Apps").join("ct-git");
        std::fs::create_dir_all(&apps_dir).expect("mkdir");
        std::fs::write(
            apps_dir.join("app.json"),
            json!({
                "code": "ct-git",
                "namespace": "Cosmic-Tools",
                "config": { "modules": ["repositories", "pipelines", "reviews"] }
            })
            .to_string(),
        )
        .expect("write app.json");

        // 自愈读取：实例模块正确
        let instances = aggregate_apps_from_dir(dir.to_str().unwrap());
        assert_eq!(instances.len(), 1);
        assert_eq!(
            instances[0].modules,
            vec!["repositories", "pipelines", "reviews"]
        );

        // 隐藏备份目录（.xxx.bak）不参与聚合（对齐 shell glob / discovery.rs）
        let bak_dir = dir.join("Apps").join(".ct-git.bak");
        std::fs::create_dir_all(&bak_dir).expect("mkdir bak");
        std::fs::write(
            bak_dir.join("app.json"),
            json!({ "code": "ct-git-old", "namespace": "Cosmic-Tools", "config": { "modules": ["stale"] } })
                .to_string(),
        )
        .expect("write bak app.json");
        assert_eq!(aggregate_apps_from_dir(dir.to_str().unwrap()).len(), 1);
        assert_eq!(aggregate_raw_apps_from_dir(dir.to_str().unwrap()).len(), 1);

        // 写回内容 = 原始 app.json 嵌入（契约格式）
        let agg = json!({ "appInstances": aggregate_raw_apps_from_dir(dir.to_str().unwrap()) });
        assert_eq!(
            agg["appInstances"][0]["config"]["modules"][0],
            "repositories"
        );

        // 重启后读取（apps.json 存在路径）：round-trip 不丢模块
        let written = dir.join("apps.json");
        std::fs::write(&written, agg.to_string()).expect("write apps.json");
        let reparsed = parse_apps_json(&std::fs::read_to_string(&written).unwrap());
        assert_eq!(reparsed.len(), 1);
        assert_eq!(
            reparsed[0].modules,
            vec!["repositories", "pipelines", "reviews"]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
