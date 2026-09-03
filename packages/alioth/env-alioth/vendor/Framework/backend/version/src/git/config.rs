//! 版本后端配置（env + JSON 文件）
//!
//! 配置格式为 JSON（serde_json 已在 workspace 依赖；serde_yaml 不在依赖树，
//! 避免为配置新增外部依赖）。env 覆盖文件默认值。

use crate::git::error::{BackendError, BackendResult};
use crate::git::BackendKind;
use serde::Deserialize;
use std::env;
use std::path::{Path, PathBuf};

/// 配置文件默认路径（相对项目根）
pub const DEFAULT_CONFIG_PATH: &str = ".alioth/version-backend.json";

/// ECR branch adapter 配置（opt-in）
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EcrBranchConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_worktree_dir")]
    pub worktree_dir: PathBuf,
    #[serde(default)]
    pub path_allowlist: Vec<String>,
}

fn default_worktree_dir() -> PathBuf {
    PathBuf::from(".alioth/worktrees")
}

/// 版本后端配置
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionBackendConfig {
    /// 显式后端类型（git | db | memory）；缺省走自动探测
    pub backend_type: Option<BackendKind>,
    /// git 仓库路径（默认项目根）
    #[serde(default = "default_repo")]
    pub repo: PathBuf,
    /// 模型 tag 前缀（默认 `model/`）
    #[serde(default = "default_tag_prefix")]
    pub tag_prefix: String,
    /// 可选远端（autoPush 时使用）
    pub remote: Option<String>,
    /// 是否自动推送（默认 false）
    #[serde(default)]
    pub auto_push: bool,
    /// ECR branch adapter（未配置 = 不启用）
    pub ecr_branch: Option<EcrBranchConfig>,
}

impl Default for VersionBackendConfig {
    fn default() -> Self {
        Self {
            backend_type: None,
            repo: default_repo(),
            tag_prefix: default_tag_prefix(),
            remote: None,
            auto_push: false,
            ecr_branch: None,
        }
    }
}

fn default_repo() -> PathBuf {
    PathBuf::from(".")
}

fn default_tag_prefix() -> String {
    "model/".into()
}

impl VersionBackendConfig {
    /// 从 JSON 文件加载（不存在返回默认配置，不报错）
    pub fn from_file(path: &Path) -> BackendResult<Self> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(e) => return Err(BackendError::Config(format!("读取 {path:?}: {e}"))),
        };
        let cfg: VersionBackendConfig = serde_json::from_str(&content)
            .map_err(|e| BackendError::Config(format!("解析 {path:?}: {e}")))?;
        Ok(cfg)
    }

    /// 从 env 覆盖字段（仅当对应 env 存在时覆盖）
    pub fn apply_env(&mut self) {
        if let Ok(v) = env::var("ALIOTH_VERSION_BACKEND") {
            match v.trim() {
                "git" => self.backend_type = Some(BackendKind::Git),
                "db" => self.backend_type = Some(BackendKind::Db),
                "memory" => self.backend_type = Some(BackendKind::Memory),
                other => log::warn!("ALIOTH_VERSION_BACKEND 未知值 {other:?}（忽略）"),
            }
        }
        if let Ok(v) = env::var("ALIOTH_GIT_REPO") {
            if !v.trim().is_empty() {
                self.repo = PathBuf::from(v.trim());
            }
        }
        if let Ok(v) = env::var("ALIOTH_GIT_TAG_PREFIX") {
            if !v.trim().is_empty() {
                self.tag_prefix = v.trim().to_string();
            }
        }
        if let Ok(v) = env::var("ALIOTH_GIT_AUTO_PUSH") {
            self.auto_push = matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes");
        }
    }

    /// 加载配置：文件默认 + env 覆盖
    pub fn load() -> BackendResult<Self> {
        let mut cfg = Self::from_file(Path::new(DEFAULT_CONFIG_PATH))?;
        cfg.apply_env();
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_memory_auto() {
        let cfg = VersionBackendConfig::default();
        assert!(cfg.backend_type.is_none());
        assert_eq!(cfg.tag_prefix, "model/");
        assert!(cfg.ecr_branch.is_none());
    }

    #[test]
    fn parse_config_json() {
        let dir = std::env::temp_dir().join(format!("vb-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("version-backend.json");
        std::fs::write(
            &p,
            r#"{"backendType":"git","repo":"/tmp/x","tagPrefix":"rel/","ecrBranch":{"enabled":true,"worktreeDir":".alioth/wt","pathAllowlist":["Pre-Proc/**"]}}"#,
        )
        .unwrap();
        let cfg = VersionBackendConfig::from_file(&p).unwrap();
        assert_eq!(cfg.backend_type, Some(BackendKind::Git));
        assert_eq!(cfg.tag_prefix, "rel/");
        let ecr = cfg.ecr_branch.unwrap();
        assert!(ecr.enabled);
        assert_eq!(ecr.worktree_dir, PathBuf::from(".alioth/wt"));
        assert_eq!(ecr.path_allowlist, vec!["Pre-Proc/**".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
