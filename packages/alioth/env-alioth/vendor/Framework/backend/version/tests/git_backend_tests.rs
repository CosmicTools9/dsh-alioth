//! GitBackend 集成测试（临时 git 仓库 fixture）
//!
//! 覆盖 T1.6：plumbing 快照 / dirty-worktree 隔离 / OID≠内容哈希 / 字节源一致 /
//! tag / diff / log / resolve / verify。

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use version::git::{
    detect_backend, GitBackend, MemoryBackend, SnapshotSpec, TagSpec, VersionBackend,
};

/// 创建临时 git 仓库并返回路径
fn temp_repo(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vb-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ok = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        let _ = std::fs::remove_dir_all(&dir);
        panic!("git 不可用，无法运行 GitBackend 集成测试");
    }
    // 初始 commit（空树亦可，但保持可追踪）
    let _ = git(&dir, &["commit", "--allow-empty", "-m", "init"]);
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

fn sha256_of(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

#[tokio::test]
async fn snapshot_and_verify_roundtrip() {
    let repo = temp_repo("roundtrip");
    let backend = GitBackend::new(&repo, "model/");

    let content = br#"{"name":"avic","version":"1.0.0"}"#.to_vec();
    let spec = SnapshotSpec {
        repo_path: ".alioth/versions/avic@1.0.0.json".into(),
        content: content.clone(),
        parent: None,
        message: "记录模型版本 1.0.0".into(),
    };
    let snap = backend.create_snapshot(&spec).await.expect("snapshot");
    assert_eq!(snap.tree_path, ".alioth/versions/avic@1.0.0.json");
    assert!(!snap.commit.is_empty());

    // 同一字节源：SHA256(content) == checksum → verify Ok(true)
    let checksum = sha256_of(&content);
    let ok = backend
        .verify(&snap.commit, &snap.tree_path, &checksum)
        .await
        .expect("verify");
    assert!(ok, "同字节源 verify 应为 Ok(true)");

    // 篡改内容 → Ok(false)
    let bogus = sha256_of(b"tampered");
    let not_ok = backend
        .verify(&snap.commit, &snap.tree_path, &bogus)
        .await
        .expect("verify bogus");
    assert!(!not_ok, "篡改内容 verify 应为 Ok(false)");

    let _ = std::fs::remove_dir_all(&repo);
}

#[tokio::test]
async fn snapshot_does_not_touch_shared_worktree() {
    let repo = temp_repo("dirty");
    let backend = GitBackend::new(&repo, "model/");

    // 制造共享工作树未提交改动：修改已跟踪文件 + untracked 新文件
    std::fs::write(repo.join("tracked.txt"), "v1").unwrap();
    let _ = git(&repo, &["add", "tracked.txt"]);
    let _ = git(&repo, &["commit", "-m", "add tracked"]);
    let _ = git(&repo, &["config", "user.name", "test"]);
    let _ = git(&repo, &["config", "user.email", "t@t"]);
    // 未提交改动：修改 tracked + 新增 untracked
    std::fs::write(repo.join("tracked.txt"), "v2-uncommitted").unwrap();
    std::fs::write(repo.join("untracked.txt"), "i-am-dirty").unwrap();

    let status_before = git(&repo, &["status", "--porcelain"]);

    // 快照（plumbing，不应触碰工作树/index）
    let content = br#"snapshot-bytes"#.to_vec();
    let snap = backend
        .create_snapshot(&SnapshotSpec {
            repo_path: ".alioth/snap.json".into(),
            content,
            parent: None,
            message: "plumbing snapshot".into(),
        })
        .await
        .expect("snapshot");

    // 快照 commit 仅含快照文件（git show --stat 单文件）
    let stat = git(&repo, &["show", "--stat", "--format=", &snap.commit]);
    assert!(
        !stat.contains("tracked.txt"),
        "快照 commit 不应含 tracked.txt: {stat}"
    );
    assert!(
        !stat.contains("untracked.txt"),
        "快照 commit 不应含 untracked.txt: {stat}"
    );

    // 原 dirty 改动保持 dirty、未被 stage/commit
    let status_after = git(&repo, &["status", "--porcelain"]);
    assert_eq!(
        status_before, status_after,
        "共享工作树状态必须前后一致（未提交改动未被卷入）"
    );

    let _ = std::fs::remove_dir_all(&repo);
}

#[tokio::test]
async fn oid_is_not_content_hash() {
    let repo = temp_repo("oid");
    let backend = GitBackend::new(&repo, "model/");

    let content = br#"same-bytes"#.to_vec();
    let snap = backend
        .create_snapshot(&SnapshotSpec {
            repo_path: "f.json".into(),
            content: content.clone(),
            parent: None,
            message: "oid vs hash".into(),
        })
        .await
        .expect("snapshot");

    // OID（commit）与 blob 内容裸 SHA256 不同源——此处直接验证 blob OID 与内容哈希不同
    let blob_oid = git(&repo, &["rev-parse", &format!("{}:f.json", snap.commit)]);
    let content_hash = sha256_of(&content);
    assert_ne!(
        blob_oid, content_hash,
        "blob OID（含 object header）不得等于内容裸 SHA256"
    );

    // 但 verify（git show 读内容 → 同口径 SHA256）必须通过
    assert!(backend
        .verify(&snap.commit, "f.json", &content_hash)
        .await
        .expect("verify"));

    let _ = std::fs::remove_dir_all(&repo);
}

#[tokio::test]
async fn tag_diff_log_resolve_work() {
    let repo = temp_repo("meta");
    let backend = GitBackend::new(&repo, "model/");

    let v1 = backend
        .create_snapshot(&SnapshotSpec {
            repo_path: "m.json".into(),
            content: br#"{"v":1}"#.to_vec(),
            parent: None,
            message: "v1".into(),
        })
        .await
        .expect("v1");
    backend
        .tag_version(&TagSpec {
            tag: "model/avic@1.0.0".into(),
            target: v1.commit.clone(),
            message: Some("1.0.0".into()),
        })
        .await
        .expect("tag");

    let v2 = backend
        .create_snapshot(&SnapshotSpec {
            repo_path: "m.json".into(),
            content: br#"{"v":2}"#.to_vec(),
            parent: Some(v1.commit.clone()),
            message: "v2".into(),
        })
        .await
        .expect("v2");

    // diff：v1→v2 对 m.json 有 1 增 1 删
    let diffs = backend
        .diff(&v1.commit, &v2.commit, None)
        .await
        .expect("diff");
    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].path, "m.json");
    assert_eq!(diffs[0].additions, 1);
    assert_eq!(diffs[0].deletions, 1);

    // log：从 v2 回溯 m.json 路径 2 条（孤儿快照需显式起点）
    let commits = backend
        .log(Some(&v2.commit), Some("m.json"), 10)
        .await
        .expect("log");
    assert_eq!(commits.len(), 2);

    // resolve：tag → commit OID；格式解析
    let resolved = backend.resolve("model/avic@1.0.0").await.expect("resolve");
    assert_eq!(resolved.oid, v1.commit);

    // verify via tag ref
    let checksum = sha256_of(br#"{"v":1}"#);
    assert!(backend
        .verify("model/avic@1.0.0", "m.json", &checksum)
        .await
        .expect("verify via tag"));

    let _ = std::fs::remove_dir_all(&repo);
}

#[tokio::test]
async fn remote_sync_push_publishes_tag_to_remote() {
    let repo = temp_repo("remote-src");
    let remote =
        std::env::temp_dir().join(format!("vb-remote-{}-{}", "remote", std::process::id()));
    let _ = std::fs::remove_dir_all(&remote);
    std::fs::create_dir_all(&remote).unwrap();
    // 远端 = 裸仓库（模拟远端 git 服务）
    let ok = Command::new("git")
        .args(["init", "--bare", "-q"])
        .current_dir(&remote)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "git 不可用");
    // 源仓库挂 remote
    let _ = git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );

    let backend = GitBackend::new(&repo, "model/").with_remote("origin".to_string(), true);
    assert!(backend
        .capabilities()
        .contains(&version::git::Capability::RemoteSync));

    let snap = backend
        .create_snapshot(&SnapshotSpec {
            repo_path: "files/VER-001/doc.json".into(),
            content: br#"{"v":1}"#.to_vec(),
            parent: None,
            message: "v1".into(),
        })
        .await
        .expect("snapshot");
    backend
        .tag_version(&TagSpec {
            tag: "release/VER-001".into(),
            target: snap.commit.clone(),
            message: Some("1.0.0".into()),
        })
        .await
        .expect("tag");

    // push tag + 分支引用（RemoteSync）
    backend
        .push(&["release/VER-001", "HEAD:refs/heads/main"])
        .await
        .expect("push 到远端应成功");

    // 远端可解析到该 tag 与 commit（annotated tag 需 ^{commit} 剥解）
    let remote_tag = git(
        &remote,
        &["show-ref", "--verify", "refs/tags/release/VER-001"],
    );
    assert!(!remote_tag.is_empty(), "远端应持有 release tag");
    let remote_commit = git(
        &remote,
        &["rev-parse", "refs/tags/release/VER-001^{commit}"],
    );
    assert_eq!(remote_commit, snap.commit, "远端 tag 应指向快照 commit");

    // push 失败（不存在的 ref）→ 真实错误
    let err = backend
        .push(&["refs/tags/nonexistent"])
        .await
        .expect_err("不存在 ref push 应失败");
    assert!(
        err.to_string().contains("失败")
            || err.to_string().contains("error")
            || err.to_string().contains("无法"),
        "got: {err}"
    );

    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_dir_all(&remote);
}

#[tokio::test]
async fn read_content_and_verify_reject_path_traversal() {
    let repo = temp_repo("pathguard");
    let backend = GitBackend::new(&repo, "model/");
    let snap = backend
        .create_snapshot(&SnapshotSpec {
            repo_path: "files/ok.txt".into(),
            content: b"safe".to_vec(),
            parent: None,
            message: "x".into(),
        })
        .await
        .expect("snapshot");

    // path 注入面：绝对路径 / `..` / `-` 开头 / 空 → 拒绝（不执行 git show）
    for bad in [
        "/etc/passwd",
        "../secret.txt",
        "a/../../secret.txt",
        "-flag",
        "",
    ] {
        let r = backend.read_content(&snap.commit, bad).await;
        assert!(r.is_err(), "read_content 应拒绝 path={bad:?}");
        let v = backend.verify(&snap.commit, bad, "x").await;
        assert!(v.is_err(), "verify 应拒绝 path={bad:?}");
    }
    // git_ref 注入面
    let r = backend.read_content("-flag", "files/ok.txt").await;
    assert!(r.is_err(), "read_content 应拒绝 git_ref=-flag");
    // 合法 path 正常
    let content = backend
        .read_content(&snap.commit, "files/ok.txt")
        .await
        .expect("合法 path");
    assert_eq!(content, b"safe");

    let _ = std::fs::remove_dir_all(&repo);
}

#[tokio::test]
async fn memory_backend_degrades_gracefully() {
    let backend = MemoryBackend;
    assert_eq!(backend.kind(), version::git::BackendKind::Memory);
    assert!(backend.capabilities().is_empty());
    let r = backend
        .create_snapshot(&SnapshotSpec {
            repo_path: "x.json".into(),
            content: vec![],
            parent: None,
            message: "x".into(),
        })
        .await;
    assert!(r.is_err(), "MemoryBackend 应返回 Unsupported 错误");
}

#[tokio::test]
async fn detect_prefers_git_when_available() {
    let repo = temp_repo("detect");
    // 显式指定 repo 探测（测试进程 cwd 未必是 git 仓库）
    let cfg = version::git::config::VersionBackendConfig {
        backend_type: Some(version::git::BackendKind::Git),
        repo: repo.clone(),
        ..Default::default()
    };
    let explicit = detect_backend(Some(cfg));
    assert_eq!(explicit.kind(), version::git::BackendKind::Git);
    let _ = std::fs::remove_dir_all(&repo);
}
