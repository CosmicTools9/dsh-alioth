//! EcrBranchAdapter 集成测试（临时 git 仓库 + 独立 worktree）
//!
//! 覆盖 T2.4：worktree 生命周期（submit→approve→close）/ manifest 超范围拒绝 /
//! 未 submit 先 approve 失败 / git 操作失败返回真实错误。

use std::path::{Path, PathBuf};
use std::process::Command;
use version::git::{EcrBranchAdapter, EcrEvent};

fn temp_repo(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vb-ecr-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ok = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        panic!("git 不可用");
    }
    git(&dir, &["config", "user.name", "test"]);
    git(&dir, &["config", "user.email", "t@t"]);
    // 基线文件 + commit
    std::fs::write(dir.join("base.txt"), "base").unwrap();
    git(&dir, &["add", "base.txt"]);
    git(&dir, &["commit", "-m", "base"]);
    dir
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git run");
    assert!(
        out.status.success(),
        "git {args:?} 失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn manifest(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|s| s.to_string()).collect()
}

#[tokio::test]
async fn ecr_worktree_lifecycle() {
    let repo = temp_repo("lifecycle");
    let wt_dir = std::env::temp_dir().join(format!("vb-ecr-wt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&wt_dir);
    let adapter = EcrBranchAdapter::new(
        &repo,
        &wt_dir,
        vec!["Pre-Proc/**".into(), "base.txt".into()],
    );

    // submit：创建分支 + 独立 worktree
    adapter
        .on_transition(
            "ECR-001",
            EcrEvent::Submit,
            &manifest(&["Pre-Proc/AVIC-CAASEC/Sources/x.rs"]),
        )
        .await
        .expect("submit");
    let branch = git(&repo, &["branch", "--list", "ecr/ECR-001"]);
    assert!(branch.contains("ecr/ECR-001"), "分支应已创建: {branch}");
    assert!(wt_dir.join("ecr-ECR-001").exists(), "worktree 应已创建");

    // approve：worktree 内提交 + 主仓库 merge --no-ff
    adapter
        .on_transition(
            "ECR-001",
            EcrEvent::Approve,
            &manifest(&["Pre-Proc/AVIC-CAASEC/Sources/x.rs"]),
        )
        .await
        .expect("approve");
    let merged = git(&repo, &["log", "--oneline", "--all", "--max-count=5"]);
    assert!(
        merged.contains("merge ecr/ECR-001"),
        "应存在 merge commit: {merged}"
    );

    // close：worktree 移除 + 分支删除
    adapter
        .on_transition(
            "ECR-001",
            EcrEvent::Close,
            &manifest(&["Pre-Proc/AVIC-CAASEC/Sources/x.rs"]),
        )
        .await
        .expect("close");
    assert!(!wt_dir.join("ecr-ECR-001").exists(), "worktree 应已移除");
    let branches = git(&repo, &["branch", "--list"]);
    assert!(
        !branches.contains("ecr/ECR-001"),
        "分支应已删除: {branches}"
    );

    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_dir_all(&wt_dir);
}

#[tokio::test]
async fn ecr_manifest_out_of_scope_rejected() {
    let repo = temp_repo("scope");
    let wt_dir = std::env::temp_dir().join(format!("vb-ecr-scope-{}", std::process::id()));
    let adapter = EcrBranchAdapter::new(&repo, &wt_dir, vec!["Pre-Proc/**".into()]);

    let err = adapter
        .on_transition(
            "ECR-002",
            EcrEvent::Submit,
            &manifest(&["Gateway/backend/src/main.rs"]),
        )
        .await
        .expect_err("超范围应拒绝");
    assert!(err.to_string().contains("超出 pathAllowlist"), "got: {err}");

    // 拒绝后无分支产生
    let branches = git(&repo, &["branch", "--list"]);
    assert!(!branches.contains("ecr/ECR-002"));

    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_dir_all(&wt_dir);
}

#[tokio::test]
async fn ecr_approve_without_submit_fails() {
    let repo = temp_repo("nosubmit");
    let wt_dir = std::env::temp_dir().join(format!("vb-ecr-nosubmit-{}", std::process::id()));
    let adapter = EcrBranchAdapter::new(&repo, &wt_dir, vec!["**".into()]);

    let err = adapter
        .on_transition("ECR-003", EcrEvent::Approve, &manifest(&["base.txt"]))
        .await
        .expect_err("未 submit 先 approve 应失败");
    assert!(err.to_string().contains("不存在"), "got: {err}");

    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_dir_all(&wt_dir);
}

#[tokio::test]
async fn ecr_empty_manifest_rejected() {
    let repo = temp_repo("nomanifest");
    let wt_dir = std::env::temp_dir().join(format!("vb-ecr-nomanifest-{}", std::process::id()));
    let adapter = EcrBranchAdapter::new(&repo, &wt_dir, vec!["**".into()]);

    let err = adapter
        .on_transition("ECR-004", EcrEvent::Submit, &[])
        .await
        .expect_err("空 manifest 应拒绝");
    assert!(err.to_string().contains("manifest 为空"), "got: {err}");

    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_dir_all(&wt_dir);
}
