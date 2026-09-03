//! 后端实现：`GitBackend`（plumbing 快照）+ `MemoryBackend`（降级）

use crate::git::error::{BackendError, BackendResult};
use crate::git::ops::{git_available, object_format, run_git};
use crate::git::{
    BackendKind, Capability, CommitInfo, FileDiff, ResolvedRef, SnapshotRef, SnapshotSpec, TagInfo,
    TagSpec, VersionBackend,
};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

/// 空树 OID（git 内置常量）
const EMPTY_TREE_OID: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// git 后端：本地 git 仓库，plumbing 快照，零 index/共享工作树接触
#[derive(Debug, Clone)]
pub struct GitBackend {
    repo: PathBuf,
    /// 模型 tag 前缀（设计预留：RemoteSync 能力下 tag_version 自动加前缀；当前 tag 名由调用方显式给定）
    #[allow(dead_code)]
    tag_prefix: String,
    /// 远端 git 服务（配置后启用 RemoteSync capability；如 Gitea 等，Gateway 与远端互访）
    remote: Option<String>,
    /// push 后是否自动推送（tag/分支）
    auto_push: bool,
}

impl GitBackend {
    pub fn new(repo: impl Into<PathBuf>, tag_prefix: impl Into<String>) -> Self {
        Self {
            repo: repo.into(),
            tag_prefix: tag_prefix.into(),
            remote: None,
            auto_push: false,
        }
    }

    /// 配置远端 git 服务（RemoteSync：快照/标签 push 到远端，实例 URL 可被浏览器访问）
    pub fn with_remote(mut self, remote: impl Into<String>, auto_push: bool) -> Self {
        self.remote = Some(remote.into());
        self.auto_push = auto_push;
        self
    }

    pub fn repo(&self) -> &Path {
        &self.repo
    }

    pub fn remote(&self) -> Option<&str> {
        self.remote.as_deref()
    }

    /// 校验 git show 路径参数：拒绝空/绝对路径/`..`/以 `-` 开头（防越界与参数注入）
    fn validate_git_show_args(&self, git_ref: &str, path: &str) -> BackendResult<()> {
        if git_ref.is_empty() || git_ref.starts_with('-') {
            return Err(BackendError::InvalidInput(format!(
                "git_ref 禁止为空或 - 开头: {git_ref:?}"
            )));
        }
        if path.is_empty() || path.starts_with('-') {
            return Err(BackendError::InvalidInput(format!(
                "path 禁止为空或 - 开头: {path:?}"
            )));
        }
        // 复用快照路径规范化（拒绝绝对路径 / `..` 穿越）
        self.normalize_snapshot_path(path)?;
        Ok(())
    }

    /// 推送 refs（tag/branch）到远端 git 服务（GitBackend 固有：构造时 with_remote 配置）
    pub async fn push_refs(&self, refs: &[&str], force: bool) -> BackendResult<()> {
        let Some(remote) = &self.remote else {
            return Err(BackendError::InvalidInput(
                "未配置远端 git 服务（GitBackend::with_remote）".into(),
            ));
        };
        let mut args: Vec<String> = vec!["push".into()];
        if force {
            args.push("--force".into());
        }
        args.push(remote.clone());
        for r in refs {
            if r.is_empty() || r.starts_with('-') {
                return Err(BackendError::InvalidInput(format!(
                    "push ref 禁止为空或 - 开头: {r:?}"
                )));
            }
            args.push(r.to_string());
        }
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        run_git(&self.repo, &arg_refs, None).await?;
        Ok(())
    }

    /// 规范化快照路径：拒绝绝对路径 / `..` / 空段（防目录穿越）
    fn normalize_snapshot_path(&self, repo_path: &str) -> BackendResult<Vec<String>> {
        let p = Path::new(repo_path);
        if p.is_absolute() {
            return Err(BackendError::InvalidInput(format!(
                "快照路径必须为仓库内相对路径: {repo_path}"
            )));
        }
        let mut parts = Vec::new();
        for comp in p.components() {
            match comp {
                Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
                Component::CurDir => {}
                Component::ParentDir => {
                    return Err(BackendError::InvalidInput(format!(
                        "快照路径禁止 ..: {repo_path}"
                    )));
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(BackendError::InvalidInput(format!(
                        "快照路径必须为仓库内相对路径: {repo_path}"
                    )));
                }
            }
        }
        if parts.is_empty() {
            return Err(BackendError::InvalidInput("快照路径为空".into()));
        }
        Ok(parts)
    }

    /// 解析 parent：显式 spec.parent 优先，否则 HEAD；空仓库返回 None
    async fn resolve_parent(&self, spec: &SnapshotSpec) -> BackendResult<Option<String>> {
        if let Some(p) = &spec.parent {
            return Ok(Some(self.resolve_oid(p).await?));
        }
        match run_git(&self.repo, &["rev-parse", "--verify", "HEAD"], None).await {
            Ok(out) => Ok(Some(String::from_utf8_lossy(&out).trim().to_string())),
            Err(BackendError::GitExit { .. }) => Ok(None), // 空仓库
            Err(e) => Err(e),
        }
    }

    async fn resolve_oid(&self, spec: &str) -> BackendResult<String> {
        let out = run_git(
            &self.repo,
            &["rev-parse", "--verify", &format!("{spec}^{{commit}}")],
            None,
        )
        .await?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    /// 读 tree 条目：`mode\ttype\toid\tpath`（ls-tree 输出，`\t` 分隔——非正则解析）
    async fn ls_tree(
        &self,
        tree_oid: &str,
    ) -> BackendResult<Vec<(String, String, String, String)>> {
        let out = run_git(&self.repo, &["ls-tree", tree_oid], None).await?;
        let mut entries = Vec::new();
        for line in String::from_utf8_lossy(&out).lines() {
            if line.is_empty() {
                continue;
            }
            // 格式: <mode> SP <type> SP <oid> TAB <path>
            let mut it = line.splitn(2, ' ');
            let mode = it.next().unwrap_or("").to_string();
            let rest = it.next().unwrap_or("");
            let mut it = rest.splitn(2, ' ');
            let ty = it.next().unwrap_or("").to_string();
            let rest2 = it.next().unwrap_or("");
            let mut it = rest2.splitn(2, '\t');
            let oid = it.next().unwrap_or("").to_string();
            let path = it.next().unwrap_or("").to_string();
            entries.push((mode, ty, oid, path));
        }
        Ok(entries)
    }

    /// 在 tree 中插入/替换 path 的 blob：逐层 ls-tree + mktree（plumbing，不触碰 index）
    async fn insert_blob_into_tree(
        &self,
        tree_oid: &str,
        path_parts: &[String],
        blob_oid: &str,
    ) -> BackendResult<String> {
        // 递归 async fn 必须 Box::pin 避免无限大小的 future
        Box::pin(async move {
            let mut entries = self.ls_tree(tree_oid).await?;

            if path_parts.len() == 1 {
                // 叶子：替换同名条目或追加新条目
                let name = &path_parts[0];
                entries.retain(|(_, _, _, p)| p != name);
                entries.push((
                    "100644".into(),
                    "blob".into(),
                    blob_oid.to_string(),
                    name.clone(),
                ));
                return self.mktree(&entries).await;
            }

            // 目录：下探一层
            let dir = &path_parts[0];
            let existing = entries
                .iter()
                .find(|(_, ty, _, p)| ty == "tree" && p == dir)
                .map(|(_, _, oid, _)| oid.clone());

            let sub_tree_oid = match existing {
                Some(oid) => oid,
                None => EMPTY_TREE_OID.to_string(),
            };
            let new_sub = self
                .insert_blob_into_tree(&sub_tree_oid, &path_parts[1..], blob_oid)
                .await?;

            entries.retain(|(_, _, _, p)| p != dir);
            entries.push(("040000".into(), "tree".into(), new_sub, dir.clone()));
            self.mktree(&entries).await
        })
        .await
    }

    /// 构造 tree 对象（输入行 `<mode> <type> <oid>\t<path>`；git mktree 自排序）
    async fn mktree(&self, entries: &[(String, String, String, String)]) -> BackendResult<String> {
        let mut input = String::new();
        for (mode, ty, oid, path) in entries {
            input.push_str(&format!("{mode} {ty} {oid}\t{path}\n"));
        }
        let out = run_git(&self.repo, &["mktree"], Some(input.as_bytes())).await?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    /// plumbing 快照提交（§design 2.0.1）
    async fn snapshot_commit(
        &self,
        spec: &SnapshotSpec,
        blob_oid: &str,
        base_tree_oid: &str,
        parent: Option<&str>,
    ) -> BackendResult<String> {
        let parts = self.normalize_snapshot_path(&spec.repo_path)?;
        let new_tree = self
            .insert_blob_into_tree(base_tree_oid, &parts, blob_oid)
            .await?;

        let mut args: Vec<String> = vec!["commit-tree".into(), new_tree.clone()];
        if let Some(p) = parent {
            args.push("-p".into());
            args.push(p.to_string());
        }
        args.push("-m".into());
        args.push(spec.message.clone());
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let out = run_git(&self.repo, &arg_refs, None).await?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }
}

#[async_trait]
impl VersionBackend for GitBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Git
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn capabilities(&self) -> Vec<Capability> {
        let mut caps = Vec::new();
        if self.remote.is_some() {
            caps.push(Capability::RemoteSync);
        }
        caps
    }

    async fn create_snapshot(&self, spec: &SnapshotSpec) -> BackendResult<SnapshotRef> {
        // 1. 写入 blob（content 字节原样——与 checksum 同一字节源）
        let blob_out = run_git(
            &self.repo,
            &["hash-object", "-w", "--stdin"],
            Some(&spec.content),
        )
        .await?;
        let blob_oid = String::from_utf8_lossy(&blob_out).trim().to_string();

        // 2. parent（空仓库 = root commit）
        let parent = self.resolve_parent(spec).await?;

        // 3. base tree
        let base_tree = match &parent {
            Some(p) => {
                let out =
                    run_git(&self.repo, &["rev-parse", &format!("{p}^{{tree}}")], None).await?;
                String::from_utf8_lossy(&out).trim().to_string()
            }
            None => EMPTY_TREE_OID.to_string(),
        };

        // 4-5. commit-tree
        let commit = self
            .snapshot_commit(spec, &blob_oid, &base_tree, parent.as_deref())
            .await?;

        Ok(SnapshotRef {
            commit,
            tree_path: spec.repo_path.clone(),
        })
    }

    async fn tag_version(&self, spec: &TagSpec) -> BackendResult<TagInfo> {
        // 目标解析为 commit OID（安全：防止 tag 名注入）
        let target = self.resolve_oid(&spec.target).await?;
        match &spec.message {
            Some(msg) => {
                run_git(
                    &self.repo,
                    &["tag", "-a", &spec.tag, &target, "-m", msg],
                    None,
                )
                .await?;
            }
            None => {
                run_git(&self.repo, &["tag", &spec.tag, &target], None).await?;
            }
        }
        Ok(TagInfo {
            tag: spec.tag.clone(),
            target,
        })
    }

    async fn diff(&self, a: &str, b: &str, path: Option<&str>) -> BackendResult<Vec<FileDiff>> {
        let mut args: Vec<String> = vec!["diff".into(), "--numstat".into(), a.into(), b.into()];
        if let Some(p) = path {
            if p.starts_with('-') {
                return Err(BackendError::InvalidInput(format!(
                    "diff 路径禁止以 - 开头: {p}"
                )));
            }
            args.push("--".into());
            args.push(p.into());
        }
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let out = run_git(&self.repo, &arg_refs, None).await?;

        let mut diffs = Vec::new();
        // numstat 格式: <add>\t<del>\t<path>（\t 分隔；二进制行为 "-"）
        for line in String::from_utf8_lossy(&out).lines() {
            if line.is_empty() {
                continue;
            }
            let mut it = line.splitn(3, '\t');
            let add = it.next().unwrap_or("0");
            let del = it.next().unwrap_or("0");
            let path = it.next().unwrap_or("");
            diffs.push(FileDiff {
                path: path.to_string(),
                additions: add.parse().unwrap_or(0),
                deletions: del.parse().unwrap_or(0),
            });
        }
        Ok(diffs)
    }

    async fn log(
        &self,
        rev: Option<&str>,
        path: Option<&str>,
        limit: usize,
    ) -> BackendResult<Vec<CommitInfo>> {
        let limit = limit.clamp(1, 100);
        let mut args: Vec<String> = vec!["log".into(), "--oneline".into(), format!("-{limit}")];
        if let Some(r) = rev {
            if r.is_empty() || r.starts_with('-') {
                return Err(BackendError::InvalidInput(format!(
                    "log rev 禁止为空或 - 开头: {r:?}"
                )));
            }
            args.push(r.to_string());
        }
        if let Some(p) = path {
            if p.starts_with('-') {
                return Err(BackendError::InvalidInput(format!(
                    "log 路径禁止以 - 开头: {p}"
                )));
            }
            args.push("--".into());
            args.push(p.into());
        }
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let out = run_git(&self.repo, &arg_refs, None).await?;

        let mut commits = Vec::new();
        for line in String::from_utf8_lossy(&out).lines() {
            if line.is_empty() {
                continue;
            }
            // 格式: <oid> <subject...>（首个空格分隔）
            match line.split_once(' ') {
                Some((oid, subject)) => commits.push(CommitInfo {
                    oid: oid.to_string(),
                    subject: subject.to_string(),
                }),
                None => commits.push(CommitInfo {
                    oid: line.to_string(),
                    subject: String::new(),
                }),
            }
        }
        Ok(commits)
    }

    async fn resolve(&self, spec: &str) -> BackendResult<ResolvedRef> {
        if spec.is_empty() || spec.starts_with('-') {
            return Err(BackendError::InvalidInput(format!(
                "rev 禁止为空或 - 开头: {spec:?}"
            )));
        }
        // ^{commit} 剥离 annotated tag 对象 → 返回最终 commit OID
        let out = run_git(
            &self.repo,
            &["rev-parse", "--verify", &format!("{spec}^{{commit}}")],
            None,
        )
        .await?;
        let oid = String::from_utf8_lossy(&out).trim().to_string();
        let oid_format = object_format(&self.repo).await;
        Ok(ResolvedRef { oid, oid_format })
    }

    async fn current_branch(&self) -> BackendResult<Option<String>> {
        match run_git(
            &self.repo,
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
            None,
        )
        .await
        {
            Ok(out) => {
                let name = String::from_utf8_lossy(&out).trim().to_string();
                Ok(if name.is_empty() { None } else { Some(name) })
            }
            Err(BackendError::GitExit { .. }) => Ok(None), // detached HEAD / 空仓库
            Err(e) => Err(e),
        }
    }

    async fn read_content(&self, git_ref: &str, path: &str) -> BackendResult<Vec<u8>> {
        self.validate_git_show_args(git_ref, path)?;
        let show_spec = format!("{git_ref}:{path}");
        run_git(&self.repo, &["show", &show_spec], None).await
    }

    async fn push(&self, refs: &[&str]) -> BackendResult<()> {
        self.push_refs(refs, false).await
    }

    async fn push_force(&self, refs: &[&str]) -> BackendResult<()> {
        self.push_refs(refs, true).await
    }

    async fn verify(
        &self,
        git_ref: &str,
        path: &str,
        expected_sha256: &str,
    ) -> BackendResult<bool> {
        self.validate_git_show_args(git_ref, path)?;
        // `git show <ref>:<path>`：blob 内容原样（无 header），与写入时同字节源
        let show_spec = format!("{git_ref}:{path}");
        let out = run_git(&self.repo, &["show", &show_spec], None).await?;

        let mut hasher = Sha256::new();
        hasher.update(&out);
        let actual = hex::encode(hasher.finalize());

        Ok(actual.eq_ignore_ascii_case(expected_sha256.trim()))
    }
}

/// 降级后端：无 git / 非仓库时使用，保持现有行为
#[derive(Debug, Clone, Default)]
pub struct MemoryBackend;

#[async_trait]
impl VersionBackend for MemoryBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Memory
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn create_snapshot(&self, _spec: &SnapshotSpec) -> BackendResult<SnapshotRef> {
        Err(BackendError::Unsupported(
            "create_snapshot（MemoryBackend 降级）",
        ))
    }

    async fn tag_version(&self, _spec: &TagSpec) -> BackendResult<TagInfo> {
        Err(BackendError::Unsupported(
            "tag_version（MemoryBackend 降级）",
        ))
    }

    async fn diff(&self, _a: &str, _b: &str, _path: Option<&str>) -> BackendResult<Vec<FileDiff>> {
        Err(BackendError::Unsupported("diff（MemoryBackend 降级）"))
    }

    async fn log(
        &self,
        _rev: Option<&str>,
        _path: Option<&str>,
        _limit: usize,
    ) -> BackendResult<Vec<CommitInfo>> {
        Err(BackendError::Unsupported("log（MemoryBackend 降级）"))
    }

    async fn resolve(&self, _spec: &str) -> BackendResult<ResolvedRef> {
        Err(BackendError::Unsupported("resolve（MemoryBackend 降级）"))
    }

    async fn current_branch(&self) -> BackendResult<Option<String>> {
        Err(BackendError::Unsupported(
            "current_branch（MemoryBackend 降级）",
        ))
    }

    async fn read_content(&self, _git_ref: &str, _path: &str) -> BackendResult<Vec<u8>> {
        Err(BackendError::Unsupported(
            "read_content（MemoryBackend 降级）",
        ))
    }

    async fn push(&self, _refs: &[&str]) -> BackendResult<()> {
        Err(BackendError::Unsupported("push（MemoryBackend 降级）"))
    }

    async fn push_force(&self, _refs: &[&str]) -> BackendResult<()> {
        Err(BackendError::Unsupported(
            "push_force（MemoryBackend 降级）",
        ))
    }

    async fn verify(
        &self,
        _git_ref: &str,
        _path: &str,
        _expected_sha256: &str,
    ) -> BackendResult<bool> {
        Err(BackendError::Unsupported("verify（MemoryBackend 降级）"))
    }
}

/// 构造 GitBackend 前探测可用性（供 detect 使用）
pub fn git_backend_available(repo: &Path) -> bool {
    git_available() && crate::git::ops::is_git_repo(repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_rejects_traversal() {
        let b = GitBackend::new("/tmp/x", "");
        assert!(b.normalize_snapshot_path("../etc/passwd").is_err());
        assert!(b.normalize_snapshot_path("/abs/path").is_err());
        assert!(b.normalize_snapshot_path("a/../b").is_err());
        assert!(b.normalize_snapshot_path("").is_err());
        let ok = b
            .normalize_snapshot_path(".alioth/versions/m.json")
            .unwrap();
        assert_eq!(
            ok,
            vec![
                ".alioth".to_string(),
                "versions".to_string(),
                "m.json".to_string()
            ]
        );
    }
}
