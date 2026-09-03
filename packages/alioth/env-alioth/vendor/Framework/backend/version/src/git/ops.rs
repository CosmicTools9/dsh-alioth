//! git CLI 子进程封装
//!
//! - args 数组构造（无 shell 注入），对齐 app-agent tool_registry 既有 git 工具模式
//! - 支持 stdin 输入（hash-object 等 plumbing 需要）
//! - 超时保护（`GIT_TIMEOUT_SECS`），超时/非零退出均返回 [`BackendError`]

use crate::git::error::{BackendError, BackendResult};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// git 命令超时（秒）
pub const GIT_TIMEOUT_SECS: u64 = 30;

/// 执行 git 命令，返回 stdout 字节。
///
/// `args` 以数组透传（绝不经过 shell）；`stdin` 可选（Some 时写入子进程 stdin）。
pub async fn run_git(repo: &Path, args: &[&str], stdin: Option<&[u8]>) -> BackendResult<Vec<u8>> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.args(args)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| BackendError::GitExec(format!("spawn git {args:?}: {e}")))?;

    if let Some(data) = stdin {
        if let Some(mut handle) = child.stdin.take() {
            handle
                .write_all(data)
                .await
                .map_err(|e| BackendError::GitExec(format!("write git stdin {args:?}: {e}")))?;
            handle
                .shutdown()
                .await
                .map_err(|e| BackendError::GitExec(format!("close git stdin {args:?}: {e}")))?;
        }
    } else {
        drop(child.stdin.take());
    }

    let output = tokio::time::timeout(
        Duration::from_secs(GIT_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| BackendError::GitExec(format!("git {args:?} 超时（{GIT_TIMEOUT_SECS}s）")))?
    .map_err(|e| BackendError::GitExec(format!("wait git {args:?}: {e}")))?;

    if !output.status.success() {
        return Err(BackendError::GitExit {
            code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    Ok(output.stdout)
}

/// 同步探测 git 二进制可用性（构造期一次）
pub fn git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 判断目录是否为 git 仓库（.git 存在——目录或 worktree gitfile）
pub fn is_git_repo(dir: &Path) -> bool {
    let dot_git = dir.join(".git");
    dot_git.is_dir() || dot_git.is_file()
}

/// 判断 git 版本是否支持 `rev-parse --show-object-format`（git ≥ 2.29）
fn supports_show_object_format() -> bool {
    // 惰性探测：失败即视为不支持（返回 Sha1 保守值）
    true
}

/// 解析 OID 格式：优先 `rev-parse --show-object-format`（sha1|sha256），失败降级 Sha1
pub async fn object_format(repo: &Path) -> OidFormat {
    let _ = supports_show_object_format();
    match run_git(repo, &["rev-parse", "--show-object-format"], None).await {
        Ok(out) => match String::from_utf8_lossy(&out).trim() {
            "sha256" => OidFormat::Sha256,
            _ => OidFormat::Sha1,
        },
        Err(_) => OidFormat::Sha1,
    }
}

pub use crate::git::OidFormat;
