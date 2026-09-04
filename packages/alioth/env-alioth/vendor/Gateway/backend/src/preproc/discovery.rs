use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{Brand, NavGroup, Permissions, PreprocApp, Routing};

/// Pre-Proc 应用发现服务
///
/// 启动时扫描 Pre-Proc 目录，按 namespace 过滤应用。
/// 每个 Gateway 实例只运行一个 namespace。
pub struct PreprocDiscovery {
    base_path: PathBuf,
    /// 当前 Gateway 实例绑定的 namespace（None = 未绑定，加载所有）
    namespace: Option<String>,
    apps: HashMap<String, PreprocApp>,
}

impl PreprocDiscovery {
    /// 创建新的发现服务
    ///
    /// `namespace` — 若为 Some，只加载匹配 namespace 的应用；
    /// 若为 None，加载所有应用（开发模式回退）。
    pub fn new(base_path: impl AsRef<Path>, namespace: Option<String>) -> Self {
        Self {
            base_path: base_path.as_ref().to_path_buf(),
            namespace,
            apps: HashMap::new(),
        }
    }

    /// 扫描 Pre-Proc 目录，按 namespace 过滤。
    ///
    /// 若 self.namespace 为 None（未指定），会先检测所有 namespace：
    ///   - 0 个 → 警告，跳过
    ///   - 1 个 → 自动绑定（WARN 日志提示）
    ///   - N>1 个 → 返回错误，列出所有 namespace 要求用户指定
    pub fn scan(&mut self) -> anyhow::Result<Vec<PreprocApp>> {
        self.apps.clear();

        // 若未指定 namespace，检测歧义
        if self.namespace.is_none() {
            self.detect_and_bind_namespace()?;
        }

        let mut discovered = Vec::new();
        let mut skipped_wrong_namespace = Vec::new();

        if !self.base_path.exists() {
            common::telemetry::warn!(
                "Pre-Proc directory does not exist: {}",
                self.base_path.display()
            );
            return Ok(discovered);
        }

        for entry in std::fs::read_dir(&self.base_path)? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            // 跳过特殊目录
            if dir_name.starts_with('_') || dir_name.starts_with('.') || dir_name == "apps.json" {
                continue;
            }

            // 若 namespace 已指定且不匹配则跳过
            if let Some(ref gateway_ns) = self.namespace {
                if dir_name != gateway_ns.as_str() {
                    continue;
                }
            }

            // 新结构：namespace 目录下有 Apps/ 子目录
            let apps_dir = path.join("Apps");
            if !apps_dir.exists() || !apps_dir.is_dir() {
                continue;
            }

            // 扫描 Apps/ 下的每个应用目录
            for sub in std::fs::read_dir(&apps_dir)? {
                let sub = sub?;
                let sub_path = sub.path();
                if !sub_path.is_dir() {
                    continue;
                }
                let app_dir_name = sub_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if app_dir_name.starts_with('.') {
                    continue;
                }
                if let Some(app) = self.discover_app(&sub_path)? {
                    let app_ns = app
                        .namespace
                        .clone()
                        .unwrap_or_else(|| "(none)".to_string());
                    match &self.namespace {
                        Some(gateway_ns) if app_ns != gateway_ns.as_str() => {
                            skipped_wrong_namespace
                                .push(format!("{} (namespace={})", app.name, app_ns));
                            continue;
                        }
                        _ => {}
                    }
                    let name = app.name.clone();
                    self.apps.insert(name.clone(), app.clone());
                    discovered.push(app);
                    common::telemetry::info!(
                        "Discovered Pre-Proc app: {} (namespace={}) at {}",
                        name,
                        app_ns,
                        sub_path.display()
                    );
                }
            }
        }

        // ── Samples 目录发现（开发级默认样例，Pre-Proc 优先）──
        // Samples 与 Pre-Proc 同级（项目根目录下），结构为 Samples/{app}/app.json。
        // 仅当 Samples 目录存在时才扫描；同名应用已被 Pre-Proc 覆盖的则跳过。
        let samples_dir = self
            .base_path
            .parent()
            .map(|p| p.join("Samples"))
            .unwrap_or_default();
        if samples_dir.is_dir() {
            for sub in std::fs::read_dir(&samples_dir)? {
                let sub = sub?;
                let sub_path = sub.path();
                if !sub_path.is_dir() {
                    continue;
                }
                let app_dir_name = sub_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if app_dir_name.starts_with('.') || app_dir_name.starts_with('_') {
                    continue;
                }
                if let Some(app) = self.discover_app(&sub_path)? {
                    let app_ns = app
                        .namespace
                        .clone()
                        .unwrap_or_else(|| "(none)".to_string());
                    if let Some(ref gateway_ns) = self.namespace {
                        if app_ns != gateway_ns.as_str() {
                            continue;
                        }
                    }
                    let name = app.name.clone();
                    // Pre-Proc 优先：同名应用已被 Pre-Proc 发现则跳过 Samples
                    if self.apps.contains_key(&name) {
                        common::telemetry::debug!(
                            "Skipping Samples app '{}' (overridden by Pre-Proc)",
                            name
                        );
                        continue;
                    }
                    self.apps.insert(name.clone(), app.clone());
                    discovered.push(app);
                    common::telemetry::info!(
                        "Discovered Samples app: {} (namespace={}) at {}",
                        name,
                        app_ns,
                        sub_path.display()
                    );
                }
            }
        }

        if !skipped_wrong_namespace.is_empty() {
            common::telemetry::warn!(
                "Skipped {} apps from other namespaces: {}",
                skipped_wrong_namespace.len(),
                skipped_wrong_namespace.join(", ")
            );
        }

        Ok(discovered)
    }

    /// 发现单个应用（含 namespace 检测）
    fn discover_app(&self, path: &Path) -> anyhow::Result<Option<PreprocApp>> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid directory name"))?
            .to_string();

        let app_json_path = path.join("app.json");
        let backend_path = path.join("backend");
        let frontend_path = path.join("frontend");

        let has_backend = backend_path.exists();
        let has_frontend = frontend_path.exists();

        // 有 app.json 即为有效应用（即使尚未生成 backend/frontend）
        if !app_json_path.exists() {
            return Ok(None);
        }

        // 读取 app.json 获取 namespace
        let app_json_path = path.join("app.json");
        let namespace = if app_json_path.exists() {
            match std::fs::read_to_string(&app_json_path) {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(json) => json
                        .get("namespace")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    Err(e) => {
                        common::telemetry::warn!("Failed to parse app.json for {}: {}", name, e);
                        None
                    }
                },
                Err(e) => {
                    common::telemetry::warn!("Failed to read app.json for {}: {}", name, e);
                    None
                }
            }
        } else {
            common::telemetry::warn!("App '{}' has no app.json — namespace unknown, will be skipped if namespace filter is active", name);
            None
        };

        // 若 app.json 中有 name 字段，覆盖目录名
        let name = if app_json_path.exists() {
            match std::fs::read_to_string(&app_json_path) {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(json) => json
                        .get("name")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .unwrap_or(name),
                    Err(_) => name,
                },
                Err(_) => name,
            }
        } else {
            name
        };

        // 若 app.json 中有 code 字段，用它；否则用目录名
        let code = if app_json_path.exists() {
            match std::fs::read_to_string(&app_json_path) {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(json) => json
                        .get("code")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| name.clone()),
                    Err(_) => name.clone(),
                },
                Err(_) => name.clone(),
            }
        } else {
            name.clone()
        };

        // 从 app.json 提取模块列表
        let modules = if app_json_path.exists() {
            match std::fs::read_to_string(&app_json_path) {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(json) => json
                        .get("config")
                        .and_then(|c| c.get("modules"))
                        .and_then(|m| m.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default(),
                    Err(e) => {
                        common::telemetry::warn!("Failed to parse app.json for {}: {}", name, e);
                        vec![]
                    }
                },
                Err(e) => {
                    common::telemetry::warn!("Failed to read app.json for {}: {}", name, e);
                    vec![]
                }
            }
        } else {
            vec![]
        };

        // 一次性解析 app.json 以提取富字段（navigation/routing/permissions/brand/goal/non_scope）
        // 注意：保留上方 namespace/name/code/modules 的既有解析逻辑不变，这里仅补充消费。
        let rich: Option<serde_json::Value> = if app_json_path.exists() {
            std::fs::read_to_string(&app_json_path)
                .ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        } else {
            None
        };

        let navigation = rich
            .as_ref()
            .and_then(|j| j.get("navigation"))
            .and_then(|v| serde_json::from_value::<Vec<NavGroup>>(v.clone()).ok())
            .filter(|n| !n.is_empty());
        let routing = rich
            .as_ref()
            .and_then(|j| j.get("routing"))
            .and_then(|v| serde_json::from_value::<Routing>(v.clone()).ok());
        let permissions = rich
            .as_ref()
            .and_then(|j| j.get("permissions"))
            .and_then(|v| serde_json::from_value::<Permissions>(v.clone()).ok());
        let brand = rich
            .as_ref()
            .and_then(|j| j.get("brand"))
            .and_then(|v| serde_json::from_value::<Brand>(v.clone()).ok());
        let goal = rich
            .as_ref()
            .and_then(|j| j.get("goal"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let non_scope = rich
            .as_ref()
            .and_then(|j| j.get("non_scope"))
            .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
            .filter(|n| !n.is_empty());
        let description = rich
            .as_ref()
            .and_then(|j| j.get("description"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let deployment_mode = rich
            .as_ref()
            .and_then(|j| j.get("deploymentMode"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let endpoint_url = rich
            .as_ref()
            .and_then(|j| j.get("endpointUrl"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let config = {
            let config_path = path.join("config/app.yaml");
            if config_path.exists() {
                match std::fs::read_to_string(&config_path) {
                    Ok(content) => yaml_serde::from_str(&content).ok(),
                    Err(e) => {
                        common::telemetry::warn!("Failed to read config for {}: {}", name, e);
                        None
                    }
                }
            } else {
                None
            }
        };
        Ok(Some(PreprocApp {
            code,
            name,
            namespace,
            path: path.to_path_buf(),
            has_backend,
            has_frontend,
            modules,
            config,
            navigation,
            routing,
            permissions,
            brand,
            goal,
            non_scope,
            description,
            deployment_mode,
            endpoint_url,
        }))
    }

    /// 获取所有发现的应用
    pub fn get_apps(&self) -> &HashMap<String, PreprocApp> {
        &self.apps
    }

    /// 获取单个应用（仅 preproc-proxy 反代路由消费；standalone 构建不编译）
    #[cfg(feature = "preproc-proxy")]
    pub fn get_app(&self, name: &str) -> Option<&PreprocApp> {
        self.apps.get(name)
    }

    /// 确保已扫描（懒加载）
    ///
    /// 若 apps 为空且 base_path 存在则自动执行一次扫描。
    /// 避免启动时集中扫描阻塞服务器就绪。
    pub fn ensure_scanned(&mut self) -> anyhow::Result<bool> {
        if !self.apps.is_empty() {
            return Ok(false); // 已扫描，无需重复
        }
        if !self.base_path.exists() {
            common::telemetry::warn!(
                "Pre-Proc/Apps/ directory does not exist: {}",
                self.base_path.display()
            );
            return Ok(false);
        }
        let count = self.scan()?.len();
        if count > 0 {
            common::telemetry::info!("Pre-Proc lazy scan: {} apps loaded", count);
        }
        Ok(true)
    }

    /// 检测所有 namespace 并自动绑定（若无歧义）
    ///
    /// 扫描 Pre-Proc 中所有 namespace 目录下的 Apps/ 内 app.json，收集 namespace：
    ///   - 0 个 → 警告
    ///   - 1 个 → 自动绑定
    ///   - N>1 个 → 返回错误
    fn detect_and_bind_namespace(&mut self) -> anyhow::Result<()> {
        use std::collections::BTreeSet;

        if !self.base_path.exists() {
            return Ok(());
        }

        let mut namespaces = BTreeSet::new();

        // 扫描 base_path 下所有 namespace 目录中的 Apps/ 子目录下的 app.json
        fn collect_namespaces(base_path: &Path, namespaces: &mut BTreeSet<String>) {
            if let Ok(entries) = std::fs::read_dir(base_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }

                    // 路径权威：namespace 目录名即真相源（与 app.json 字段交叉校验见下）
                    let ns_from_path = entry.file_name().to_str().unwrap_or("").to_string();
                    if ns_from_path.is_empty() {
                        continue;
                    }

                    // 新结构：namespace 目录下有 Apps/ 子目录
                    let apps_dir = path.join("Apps");
                    if !apps_dir.exists() || !apps_dir.is_dir() {
                        continue;
                    }

                    // 扫描 Apps/ 下各 app 的 app.json
                    if let Ok(subs) = std::fs::read_dir(&apps_dir) {
                        for sub in subs.flatten() {
                            let sub_path = sub.path();
                            if !sub_path.is_dir() {
                                continue;
                            }
                            let app_dir_name =
                                sub_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                            if app_dir_name.starts_with('.') {
                                continue;
                            }
                            let app_json = sub_path.join("app.json");
                            if app_json.exists() {
                                if let Ok(content) = std::fs::read_to_string(&app_json) {
                                    if let Ok(json) =
                                        serde_json::from_str::<serde_json::Value>(&content)
                                    {
                                        // 路径权威收集；字段存在且不一致时 warn（漂移由检查脚本阻断）
                                        if let Some(field_ns) =
                                            json.get("namespace").and_then(|v| v.as_str())
                                        {
                                            if field_ns != ns_from_path {
                                                common::telemetry::warn!(
                                                    "app.json namespace '{}' 与路径 '{}' 不一致（{})",
                                                    field_ns,
                                                    ns_from_path,
                                                    app_json.display()
                                                );
                                            }
                                        }
                                        namespaces.insert(ns_from_path.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        collect_namespaces(&self.base_path, &mut namespaces);

        match namespaces.len() {
            0 => {
                common::telemetry::warn!(
                    "No namespaces detected in Pre-Proc — Gateway will run without App loading"
                );
                Ok(())
            }
            1 => {
                let ns = namespaces.into_iter().next().unwrap();
                common::telemetry::warn!("NAMESPACE not specified — auto-detected '{}'. Set NAMESPACE env var to silence this warning.",
                ns);
                self.namespace = Some(ns);
                Ok(())
            }
            _n => {
                let list: Vec<_> = namespaces.into_iter().collect();
                common::telemetry::warn!(
                    "Multiple namespaces detected in Pre-Proc: {}. \
                 NAMESPACE not set — loading ALL apps (development mode). \
                 Set NAMESPACE env var to restrict to one namespace.",
                    list.join(", ")
                );
                // 开发模式：不绑定 namespace，加载所有应用
                self.namespace = None;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preproc_app_serializes_camel_case_keys() {
        // 回归：deploymentMode/endpointUrl 必须输出 camelCase（前端 AppInfo 消费），
        // 禁止 snake_case 泄漏（deployment_mode/endpoint_url）
        let app = PreprocApp {
            code: "test-app".into(),
            name: "测试应用".into(),
            namespace: Some("WZ".into()),
            path: PathBuf::from("/tmp/test-app"),
            has_backend: true,
            has_frontend: true,
            modules: vec!["logistics-wz".into()],
            config: None,
            navigation: None,
            routing: None,
            permissions: None,
            brand: None,
            goal: None,
            non_scope: None,
            description: Some("测试描述".into()),
            deployment_mode: Some("multi_process".into()),
            endpoint_url: Some("http://localhost:9999".into()),
        };
        let v = serde_json::to_value(&app).unwrap();
        assert_eq!(
            v["deploymentMode"],
            json!("multi_process"),
            "deploymentMode 键名"
        );
        assert_eq!(
            v["endpointUrl"],
            json!("http://localhost:9999"),
            "endpointUrl 键名"
        );
        assert_eq!(v["description"], json!("测试描述"));
        assert!(
            v.get("deployment_mode").is_none(),
            "不得输出 snake_case deployment_mode"
        );
        assert!(
            v.get("endpoint_url").is_none(),
            "不得输出 snake_case endpoint_url"
        );
    }

    #[test]
    fn discover_app_extracts_rich_fields_from_app_json() {
        // 回归：cb-01 曾"记录已接通但 discover 未实现"——deploymentMode/endpointUrl/description
        // 必须真实从 app.json 解析，而非仅序列化 rename。
        let tmp = std::env::temp_dir().join(format!("cb01-discover-{}", std::process::id()));
        let app_dir = tmp.join("test-multi");
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(
            app_dir.join("app.json"),
            r#"{
              "code": "test-multi",
              "name": "多进程测试",
              "description": "多进程部署样本",
              "namespace": "WZ",
              "deploymentMode": "multi_process",
              "endpointUrl": "http://localhost:9999",
              "config": { "modules": ["logistics-wz"] }
            }"#,
        )
        .unwrap();

        let discovery = PreprocDiscovery::new(&tmp, Some("WZ".into()));
        let app = discovery
            .discover_app(&app_dir)
            .unwrap()
            .expect("discover_app 应返回应用");
        assert_eq!(app.description.as_deref(), Some("多进程部署样本"));
        assert_eq!(app.deployment_mode.as_deref(), Some("multi_process"));
        assert_eq!(app.endpoint_url.as_deref(), Some("http://localhost:9999"));
        assert_eq!(app.modules, vec!["logistics-wz"]);

        let v = serde_json::to_value(&app).unwrap();
        assert_eq!(v["deploymentMode"], json!("multi_process"));
        assert_eq!(v["endpointUrl"], json!("http://localhost:9999"));
        assert!(v.get("deployment_mode").is_none());

        std::fs::remove_dir_all(&tmp).ok();
    }
}
