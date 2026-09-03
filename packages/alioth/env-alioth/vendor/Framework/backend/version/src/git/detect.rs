//! 自适应后端探测（构造期一次）
//!
//! 决策链：显式配置（env / .alioth/version-backend.json）→ 自动探测（git 可用 + 仓库）
//! → 降级 MemoryBackend。探测失败仅 warn，绝不 fail 启动。

use crate::git::backend::{git_backend_available, GitBackend, MemoryBackend};
use crate::git::config::VersionBackendConfig;
use crate::git::{BackendKind, VersionBackend};
use std::path::Path;

/// 项目根：优先 env `PROJECT_ROOT`，否则当前目录（与 app-agent resolve_project_root 同思路）
pub fn project_root() -> std::path::PathBuf {
    std::env::var("PROJECT_ROOT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// 根据配置选择后端（显式配置优先，其次自动探测，最后降级）
pub fn detect_backend(config: Option<VersionBackendConfig>) -> Box<dyn VersionBackend> {
    let cfg = match config {
        Some(c) => c,
        None => match VersionBackendConfig::load() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[version-backend] 配置加载失败，降级 MemoryBackend: {e}");
                return Box::new(MemoryBackend);
            }
        },
    };

    // 1. 显式配置
    if let Some(kind) = cfg.backend_type {
        return match kind {
            BackendKind::Git => {
                let repo = resolve_repo(&cfg.repo);
                if !git_backend_available(&repo) {
                    eprintln!(
                        "[version-backend] 显式 git 但不可用（git 缺失或非仓库 {repo:?}），降级 MemoryBackend"
                    );
                    return Box::new(MemoryBackend);
                }
                Box::new(build_git_backend(&repo, &cfg))
            }
            BackendKind::Db | BackendKind::Remote => {
                eprintln!("[version-backend] {kind:?} 后端尚未实现，降级 MemoryBackend");
                Box::new(MemoryBackend)
            }
            BackendKind::Memory => Box::new(MemoryBackend),
        };
    }

    // 2. 自动探测
    let repo = resolve_repo(&cfg.repo);
    if git_backend_available(&repo) {
        return Box::new(build_git_backend(&repo, &cfg));
    }

    // 3. 降级
    Box::new(MemoryBackend)
}

/// 构造 GitBackend：注入 config 的 remote/autoPush（RemoteSync 能力）
fn build_git_backend(repo: &std::path::Path, cfg: &VersionBackendConfig) -> GitBackend {
    let mut backend = GitBackend::new(repo, cfg.tag_prefix.clone());
    if let Some(remote) = &cfg.remote {
        if !remote.trim().is_empty() {
            backend = backend.with_remote(remote.clone(), cfg.auto_push);
        }
    }
    backend
}

/// 解析仓库路径：相对路径基于项目根
fn resolve_repo(repo: &Path) -> std::path::PathBuf {
    if repo.is_absolute() {
        repo.to_path_buf()
    } else {
        project_root().join(repo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn explicit_memory_wins_over_git() {
        let cfg = VersionBackendConfig {
            backend_type: Some(BackendKind::Memory),
            ..Default::default()
        };
        // 即使当前目录是 git 仓库，显式 memory 也优先
        let backend = detect_backend(Some(cfg));
        assert_eq!(backend.kind(), BackendKind::Memory);
    }

    #[test]
    fn auto_detects_non_git_dir_falls_back() {
        // 临时非 git 目录：自动探测 → Memory 降级（现有行为不变）
        let dir = std::env::temp_dir().join(format!("vb-nongit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = VersionBackendConfig {
            repo: dir.clone(),
            ..Default::default()
        };
        let backend = detect_backend(Some(cfg));
        assert_eq!(backend.kind(), BackendKind::Memory);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn explicit_git_unavailable_falls_back() {
        let cfg = VersionBackendConfig {
            backend_type: Some(BackendKind::Git),
            repo: PathBuf::from("/nonexistent/definitely-not-a-repo"),
            ..Default::default()
        };
        let backend = detect_backend(Some(cfg));
        assert_eq!(backend.kind(), BackendKind::Memory);
    }

    #[test]
    fn auto_detects_git_repo() {
        // 临时 git 仓库
        let dir = std::env::temp_dir().join(format!("vb-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            let _ = std::fs::remove_dir_all(&dir);
            return; // 无 git 环境跳过
        }
        let cfg = VersionBackendConfig {
            repo: dir.clone(),
            ..Default::default()
        };
        let backend = detect_backend(Some(cfg));
        assert_eq!(backend.kind(), BackendKind::Git);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
