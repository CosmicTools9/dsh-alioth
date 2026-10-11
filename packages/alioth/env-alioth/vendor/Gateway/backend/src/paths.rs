//! namespace 基目录解析（**单一实现**——openapi-external-access / change
//! fix-openapi-architecture-gaps C1）。
//!
//! 语义（= 原 `Gateway/host/src/main.rs::resolve_preproc_path` 的 ns 基目录判定，
//! 文档端点与 host 同源消费）：
//!   1. `DEPLOY_PATH`（存在即用）——release 模式，**已含 ns 层级**（`Deploy/{namespace}/`），
//!      MUST NOT 再拼 namespace。2026-10-07 修复：`openapi/mod.rs` 旧实现在此二次拼接，
//!      产出 `Deploy/{ns}/{ns}/openapi.json`（双拼 bug，生产形态恒 404）。
//!   2. `PREPROC_APPS_PATH`（存在即用）——dev 模式覆盖，拼接 `/{namespace}`。
//!   3. 相对探测（按 cwd）：`Pre-Proc/{ns}` → `../Pre-Proc/{ns}` → `../../Pre-Proc/{ns}`。
//!   4. 兜底 = `../../Pre-Proc/{ns}`（host 原语义：Gateway/backend 相对 cwd）。
//!
//! 消费方（MUST 经本函数，MUST NOT 手抄第二份探测）：`Gateway/host/src/main.rs`
//! 的 `load_extensions`（Apps 发现）与 `Gateway/backend/src/openapi/mod.rs`
//! 的 `/api/openapi.json`（文档端点）。

use std::path::{Path, PathBuf};

/// 解析 namespace 基目录（`Apps/`、`openapi.json` 等的父目录）。
pub fn resolve_ns_base_dir(namespace: &str) -> PathBuf {
    let deploy_path = std::env::var("DEPLOY_PATH").ok();
    let preproc_apps_path = std::env::var("PREPROC_APPS_PATH").ok();
    let cwd = std::env::current_dir().ok();
    resolve_ns_base_dir_at(
        namespace,
        deploy_path.as_deref(),
        preproc_apps_path.as_deref(),
        cwd.as_deref(),
    )
}

/// 纯判定核（env/cwd 注入）：不读环境、不依赖进程状态 —— 单测直接注入夹具路径。
fn resolve_ns_base_dir_at(
    namespace: &str,
    deploy_path: Option<&str>,
    preproc_apps_path: Option<&str>,
    cwd: Option<&Path>,
) -> PathBuf {
    if let Some(p) = deploy_path {
        if Path::new(p).exists() {
            common::telemetry::info!("Using DEPLOY_PATH: {}", p);
            return PathBuf::from(p);
        }
        common::telemetry::warn!("DEPLOY_PATH set but not found: {}, falling back", p);
    }
    if let Some(p) = preproc_apps_path {
        if Path::new(p).exists() {
            return Path::new(p).join(namespace);
        }
    }
    let fallback = Path::new("../../Pre-Proc").join(namespace);
    let Some(cwd) = cwd else {
        return fallback;
    };
    for cand in [
        cwd.join("Pre-Proc").join(namespace),
        cwd.join("../Pre-Proc").join(namespace),
        cwd.join("../../Pre-Proc").join(namespace),
    ] {
        if cand.exists() {
            return cand;
        }
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 夹具目录（进程 id + 标签唯一化；用后即删）。
    fn fixture(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("alioth-gateway-paths-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("fixture dir");
        dir
    }

    #[test]
    fn deploy_path_is_base_verbatim_no_ns_append() {
        // release 语义：DEPLOY_PATH 已含 ns 层级（Deploy/{ns}/）——契约断言：不再拼 ns
        let deploy = fixture("deploy");
        let ns_dir = deploy.join("WZ");
        std::fs::create_dir_all(&ns_dir).unwrap();
        let got = resolve_ns_base_dir_at("WZ", Some(deploy.to_str().unwrap()), None, None);
        assert_eq!(got, deploy);
        assert!(
            !got.ends_with("WZ"),
            "DEPLOY_PATH MUST NOT 二次拼接 namespace"
        );
        std::fs::remove_dir_all(&deploy).ok();
    }

    #[test]
    fn missing_deploy_falls_back_to_preproc_apps_path_with_ns() {
        // DEPLOY_PATH 指向**不存在**的路径（fixture 建出的目录本身存在 ⇒ 需显式取不存在子路径）
        let missing = fixture("missing-deploy").join("does-not-exist");
        let preproc = fixture("preproc");
        let got = resolve_ns_base_dir_at(
            "Alioth",
            Some(missing.to_str().unwrap()),
            Some(preproc.to_str().unwrap()),
            None,
        );
        assert_eq!(got, preproc.join("Alioth"));
        std::fs::remove_dir_all(missing.parent().unwrap()).ok();
        std::fs::remove_dir_all(&preproc).ok();
    }

    #[test]
    fn relative_probe_prefers_nearest_preproc_ns() {
        let root = fixture("probe");
        let cwd = root.join("Gateway/backend");
        std::fs::create_dir_all(root.join("Pre-Proc/Alioth")).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        // cwd = <root>/Gateway/backend ⇒ ../../Pre-Proc/{ns} 命中
        let got = resolve_ns_base_dir_at("Alioth", None, None, Some(cwd.as_path()));
        assert_eq!(got, cwd.join("../../Pre-Proc/Alioth"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn fallback_is_dotted_preproc_when_nothing_resolves() {
        let empty = fixture("empty");
        let got = resolve_ns_base_dir_at("WZ", None, None, Some(empty.as_path()));
        assert_eq!(got, Path::new("../../Pre-Proc").join("WZ"));
        std::fs::remove_dir_all(&empty).ok();
    }
}
