//! ECR → git branch 生命周期 adapter（opt-in，默认不启用）
//!
//! 设计约束（spec `version-backend-ecr-branch-opt-in`）：
//! - 每 ECR 使用独立 worktree（`git worktree add`），禁止共享工作树并发 checkout/merge
//! - 每次操作前校验 ECR 快照 manifest（变更路径）⊆ pathAllowlist，超范围拒绝
//! - git 操作失败返回真实 [`BackendError`]，禁止静默成功/伪称已同步
//! - 未装配（未启用）时调用方跳过并 warn，不阻断业务

use crate::git::error::{BackendError, BackendResult};
use crate::git::ops::run_git;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// ECR 状态机 transition 事件（对齐 extensions/statemachines.yaml）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcrEvent {
    /// Draft → Submitted：创建分支 + worktree
    Submit,
    /// InReview → Approved：合并回主分支
    Approve,
    /// Implemented/Rejected → Closed：清理 worktree + 分支
    Close,
}

impl EcrEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            EcrEvent::Submit => "submit",
            EcrEvent::Approve => "approve",
            EcrEvent::Close => "close",
        }
    }
}

/// 分支命名：`ecr/{code}`（ecr_code 已由调用方校验为业务编码）
pub fn ecr_branch_name(ecr_code: &str) -> String {
    format!("ecr/{ecr_code}")
}

/// ECR branch adapter（独立于 `VersionBackend` 核心，opt-in 装配）
#[derive(Debug, Clone)]
pub struct EcrBranchAdapter {
    repo: PathBuf,
    worktree_dir: PathBuf,
    path_allowlist: Vec<String>,
    /// 仓库级互斥锁：同一时刻仅一个 ECR 操作，杜绝共享仓库并发竞态
    lock: Arc<Mutex<()>>,
}

impl EcrBranchAdapter {
    pub fn new(
        repo: impl Into<PathBuf>,
        worktree_dir: impl Into<PathBuf>,
        path_allowlist: Vec<String>,
    ) -> Self {
        Self {
            repo: repo.into(),
            worktree_dir: worktree_dir.into(),
            path_allowlist,
            lock: Arc::new(Mutex::new(())),
        }
    }

    fn branch(&self, ecr_code: &str) -> String {
        ecr_branch_name(ecr_code)
    }

    fn worktree_path(&self, ecr_code: &str) -> PathBuf {
        // ecr_code 已过滤路径穿越（见 validate_ecr_code）
        self.worktree_dir.join(format!("ecr-{ecr_code}"))
    }

    /// 校验 ECR 编码：仅允许字母数字与 `-`/`_`（防分支名注入与路径穿越）
    fn validate_ecr_code(&self, ecr_code: &str) -> BackendResult<()> {
        if ecr_code.is_empty()
            || ecr_code
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        {
            return Err(BackendError::InvalidInput(format!(
                "ECR 编码非法（仅字母数字/-/_）: {ecr_code:?}"
            )));
        }
        Ok(())
    }

    /// 范围校验：manifest 每个路径 ⊆ pathAllowlist。
    /// allowlist 支持 `**` 通配（前缀/后缀匹配，非正则——NO_REGEX 允许的路径匹配场景）。
    fn validate_manifest(&self, manifest: &[String]) -> BackendResult<()> {
        if manifest.is_empty() {
            return Err(BackendError::InvalidInput(
                "ECR 快照 manifest 为空：必须声明影响文件范围".into(),
            ));
        }
        for path in manifest {
            let allowed = self.path_allowlist.iter().any(|pat| glob_match(pat, path));
            if !allowed {
                return Err(BackendError::InvalidInput(format!(
                    "ECR manifest 路径超出 pathAllowlist: {path}（allowlist: {:?}）",
                    self.path_allowlist
                )));
            }
        }
        Ok(())
    }

    /// 处理 ECR 状态转换（旁路钩子：失败返回真实错误，由调用方决定策略）
    pub async fn on_transition(
        &self,
        ecr_code: &str,
        event: EcrEvent,
        manifest: &[String],
    ) -> BackendResult<()> {
        self.validate_ecr_code(ecr_code)?;
        // 范围校验：submit/approve 前的变更范围必须声明且合规
        self.validate_manifest(manifest)?;

        let _guard = self.lock.lock().await;
        let branch = self.branch(ecr_code);
        let wt = self.worktree_path(ecr_code);

        match event {
            EcrEvent::Submit => {
                if wt.exists() {
                    return Err(BackendError::InvalidInput(format!(
                        "ECR worktree 已存在: {wt:?}"
                    )));
                }
                std::fs::create_dir_all(&self.worktree_dir).map_err(BackendError::Io)?;
                // git worktree add <path> -b ecr/{code}（独立工作树，不触碰共享工作树）
                run_git(
                    &self.repo,
                    &[
                        "worktree",
                        "add",
                        wt.to_str().ok_or_else(|| {
                            BackendError::InvalidInput("worktree 路径非 UTF-8".into())
                        })?,
                        "-b",
                        &branch,
                    ],
                    None,
                )
                .await?;
                Ok(())
            }
            EcrEvent::Approve => {
                // 校验 worktree 存在（未 submit 不能 approve）
                if !wt.exists() {
                    return Err(BackendError::NotFound(format!(
                        "ECR worktree 不存在（先 submit）: {wt:?}"
                    )));
                }
                // worktree 内提交 ECR 变更（add + commit，仅限该 worktree 的工作树——隔离的）
                run_git(&wt, &["add", "-A"], None).await?;
                run_git(
                    &wt,
                    &[
                        "commit",
                        "-m",
                        &format!("ecr/{ecr_code}: 合并 ECR 变更"),
                        "--allow-empty",
                    ],
                    None,
                )
                .await?;
                // 主仓库 merge --no-ff
                run_git(
                    &self.repo,
                    &[
                        "merge",
                        "--no-ff",
                        &branch,
                        "-m",
                        &format!("merge ecr/{ecr_code}"),
                    ],
                    None,
                )
                .await?;
                Ok(())
            }
            EcrEvent::Close => {
                if wt.exists() {
                    run_git(
                        &self.repo,
                        &[
                            "worktree",
                            "remove",
                            wt.to_str().ok_or_else(|| {
                                BackendError::InvalidInput("worktree 路径非 UTF-8".into())
                            })?,
                            "--force",
                        ],
                        None,
                    )
                    .await?;
                }
                // 分支已 merge 时 -d 成功；未 merge 用 -D 会丢工作——用 -d，失败如实返回
                run_git(&self.repo, &["branch", "-d", &branch], None).await?;
                Ok(())
            }
        }
    }
}

/// 简单 glob 匹配（支持 `**` / `*` 通配，前后缀匹配——非正则，NO_REGEX 允许的路径匹配场景）
fn glob_match(pattern: &str, path: &str) -> bool {
    if pattern == "**" {
        return true;
    }
    if pattern.contains("**") {
        // ** 表示任意深度：拆成前缀 + 后缀
        let mut parts = pattern.split("**");
        let prefix = parts.next().unwrap_or("");
        let suffix = parts.collect::<Vec<_>>().join("**");
        if suffix.is_empty() {
            return path.starts_with(prefix);
        }
        return path.starts_with(prefix) && path.ends_with(&suffix);
    }
    if pattern.ends_with("/*") {
        // 单层通配：目录前缀 + 最后一段任意（剩余须为 "/<单段>"）
        let dir = pattern.strip_suffix("/*").unwrap_or("");
        let rest = &path[dir.len()..];
        return path.starts_with(dir)
            && rest.starts_with('/')
            && !rest[1..].contains('/')
            && !rest[1..].is_empty();
    }
    if pattern.contains('*') {
        // 通用 *：前缀 + 后缀匹配（`*` 不跨目录；多个 * 时取首尾段）
        let prefix = pattern.split('*').next().unwrap_or("");
        let suffix = pattern.rsplit('*').next().unwrap_or("");
        if !path.starts_with(prefix) || !path.ends_with(&suffix) {
            return false;
        }
        let middle = &path[prefix.len()..path.len() - suffix.len()];
        return !middle.contains('/');
    }
    pattern == path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matching_rules() {
        assert!(glob_match(
            "Pre-Proc/**",
            "Pre-Proc/AVIC-CAASEC/Sources/x.rs"
        ));
        assert!(glob_match("Pre-Proc/**", "Pre-Proc/AVIC-CAASEC"));
        assert!(!glob_match("Pre-Proc/**", "Gateway/backend/x"));
        assert!(glob_match("src/*", "src/lib.rs"));
        assert!(!glob_match("src/*", "src/sub/lib.rs"));
        assert!(glob_match("**", "anything/at/all"));
        assert!(glob_match(
            "docs/specs/*.md",
            "docs/specs/API_DESIGN_SPEC.md"
        ));
        assert!(!glob_match("docs/specs/*.md", "docs/specs/sub/x.md"));
    }

    #[test]
    fn ecr_code_validation() {
        let a = EcrBranchAdapter::new("/tmp/x", "/tmp/wt", vec!["**".into()]);
        assert!(a.validate_ecr_code("ECR-001").is_ok());
        assert!(a.validate_ecr_code("ecr_abc").is_ok());
        assert!(a.validate_ecr_code("a/b").is_err());
        assert!(a.validate_ecr_code("..").is_err());
        assert!(a.validate_ecr_code("").is_err());
    }

    #[test]
    fn manifest_scope_rejected() {
        let a = EcrBranchAdapter::new(
            "/tmp/x",
            "/tmp/wt",
            vec!["Pre-Proc/AVIC-CAASEC/Sources/**".into()],
        );
        let ok = vec![
            "Pre-Proc/AVIC-CAASEC/Sources/Modules/system-dev/frontend/src/pages/x.tsx".to_string(),
        ];
        assert!(a.validate_manifest(&ok).is_ok());
        let bad = vec!["Gateway/backend/src/main.rs".to_string()];
        assert!(a.validate_manifest(&bad).is_err());
        assert!(a.validate_manifest(&[]).is_err());
    }
}
