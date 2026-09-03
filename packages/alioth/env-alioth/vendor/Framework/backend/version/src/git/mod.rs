//! 可插拔版本后端抽象（Ports & Adapters）
//!
//! - `VersionBackend`：Port。git 仅是其中一种实现（`GitBackend`），另有降级 `MemoryBackend`。
//! - 额外能力（如 ECR 分支）经 `capabilities()` 显式声明，默认全部不启用（opt-in）。
//! - 内容哈希与 Git OID 严格区分：OID 仅作引用唯一性；内容完整性走 `verify`（同口径 SHA256）。

pub mod backend;
pub mod config;
pub mod detect;
pub mod ecr_adapter;
pub mod error;
pub mod ops;

pub use backend::{GitBackend, MemoryBackend};
pub use config::{EcrBranchConfig, VersionBackendConfig};
pub use detect::detect_backend;
pub use ecr_adapter::{ecr_branch_name, EcrBranchAdapter, EcrEvent};
pub use error::{BackendError, BackendResult};

use async_trait::async_trait;

/// 后端类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Git,
    Db,
    Memory,
    Remote,
}

/// 可选能力（默认全部不启用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// ECR 生命周期 → git branch（worktree 隔离 + allowlist 校验，opt-in）
    EcrBranch,
    /// 快照 manifest（ECR 影响文件范围声明）
    SnapshotManifest,
    /// 远端同步（autoPush）
    RemoteSync,
}

/// Git 对象格式（仓库 object format）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OidFormat {
    Sha1,
    Sha256,
}

/// rev 解析结果：OID + 对象格式。
///
/// OID 是对象标识（blob/commit OID 含 object header），**不是内容哈希**——
/// 内容完整性必须走 [`VersionBackend::verify`]。
#[derive(Debug, Clone)]
pub struct ResolvedRef {
    pub oid: String,
    pub oid_format: OidFormat,
}

/// 快照写入规格。
///
/// `content` 是**唯一**字节源：写入 git blob 的字节与计算
/// `VersionRecord.checksum` 的字节必须完全相同（SHA256(content) == checksum）。
#[derive(Debug, Clone)]
pub struct SnapshotSpec {
    /// 快照在仓库内的 owned path（如 `.alioth/versions/{model}/{semver}.json`）
    pub repo_path: String,
    /// 写入的确切字节
    pub content: Vec<u8>,
    /// 基提交 ref（缺省取 HEAD；空仓库为 root commit）
    pub parent: Option<String>,
    /// commit message
    pub message: String,
}

/// 快照写入结果
#[derive(Debug, Clone)]
pub struct SnapshotRef {
    /// 新提交 OID
    pub commit: String,
    /// 快照在仓库内的路径
    pub tree_path: String,
}

/// 打 tag 规格
#[derive(Debug, Clone)]
pub struct TagSpec {
    /// tag 名（如 `model/avic@1.0.0` / `release/VER-001`）
    pub tag: String,
    /// 目标 commit/ref
    pub target: String,
    /// annotated tag message（可含 majority.sprint.revision 等）
    pub message: Option<String>,
}

/// tag 结果
#[derive(Debug, Clone)]
pub struct TagInfo {
    pub tag: String,
    pub target: String,
}

/// 文件级差异统计（git diff --numstat 语义）
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

/// 提交摘要（git log --oneline 语义）
#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub oid: String,
    pub subject: String,
}

/// 可插拔版本后端（Port）
#[async_trait]
pub trait VersionBackend: Send + Sync + std::fmt::Debug {
    /// 后端类型
    fn kind(&self) -> BackendKind;

    /// downcast 支持（handler 需获取具体 adapter，如 EcrBranchAdapter）
    fn as_any(&self) -> &dyn std::any::Any;

    /// 可选能力声明（默认空——ECR 等一律 opt-in）
    fn capabilities(&self) -> Vec<Capability> {
        Vec::new()
    }

    /// 把内容快照写入后端（git 后端 = plumbing 提交，零 index/共享工作树接触）
    async fn create_snapshot(&self, spec: &SnapshotSpec) -> BackendResult<SnapshotRef>;

    /// 给目标打版本 tag（git 后端 = annotated tag）
    async fn tag_version(&self, spec: &TagSpec) -> BackendResult<TagInfo>;

    /// 两 rev 间路径级差异统计
    async fn diff(&self, a: &str, b: &str, path: Option<&str>) -> BackendResult<Vec<FileDiff>>;

    /// 路径提交历史（从 `rev` 起点回溯，缺省 HEAD；limit 条）
    async fn log(
        &self,
        rev: Option<&str>,
        path: Option<&str>,
        limit: usize,
    ) -> BackendResult<Vec<CommitInfo>>;

    /// rev 解析为 Git OID + 对象格式（仅引用唯一性）
    async fn resolve(&self, spec: &str) -> BackendResult<ResolvedRef>;

    /// 当前工作分支名（只读探测；detached HEAD 或无分支返回 None）
    async fn current_branch(&self) -> BackendResult<Option<String>>;

    /// 读取内容实例字节（`git show <ref>:<path>`；prod-file 下载/预览用）
    async fn read_content(&self, git_ref: &str, path: &str) -> BackendResult<Vec<u8>>;

    /// 推送 refs（tag/branch）到远端 git 服务（RemoteSync；未配置远端返回 Unsupported/错误）
    async fn push(&self, refs: &[&str]) -> BackendResult<()>;

    /// 强制推送 refs（tag 覆盖幂等——单链覆盖语义下 release tag 指向最新实例 commit）
    async fn push_force(&self, refs: &[&str]) -> BackendResult<()>;

    /// 内容完整性验证：`git show <ref>:<path>` 读 blob 内容，
    /// 计算与 `VersionRecord.checksum` 同口径的 SHA256 后比对。
    async fn verify(&self, git_ref: &str, path: &str, expected_sha256: &str)
        -> BackendResult<bool>;
}
