//! keyring：当前钥（active） + 既往钥（previous）多钥装载与惰性刷新。
//!
//! - **active** 源：文件 `.encryption_key`（既有消费面不变）或环境变量（如 `SYSTEM_CONFIG_ENC_KEY`）。
//! - **previous** 源：目录 `.encryption_keyring/previous/<key_id>.key` 或逗号分隔环境变量。
//!
//! 轮换时旧钥降级为 previous 仍在环内 ⇒ 旧密文全程可读 ⇒ 无需停机、无需停写。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use sha2::{Digest, Sha256};

/// 密钥原始字节数（AES-256）。
pub const KEY_LEN: usize = 32;

/// keyring 与信封错误（一律 fail-loud，MUST NOT 回退明文）。
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("密钥文件读取失败 {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("密钥非合法 base64: {0}")]
    Base64(String),
    #[error("密钥解码后必须为 {expected} 字节，实得 {actual}")]
    Length { expected: usize, actual: usize },
    #[error("环境变量 {0} 未设置")]
    MissingEnv(String),
    #[error("keyring 锁中毒（前次 panic 遗留）")]
    Poisoned,
    #[error("密文形态不合法: {0}")]
    Malformed(String),
    #[error("密文要求的密钥 {0} 不在 keyring 内")]
    UnknownKeyId(String),
    #[error("该形态不由 Rust 解密（shell CBC）: {0}")]
    UnsupportedForm(String),
    #[error("解密失败（已尝试 {0} 把钥）: {1}")]
    Decrypt(usize, String),
    #[error("密码学操作失败: {0}")]
    Cipher(String),
}

/// 单把密钥：`id` = `hex(sha256(raw))[..8]`。
#[derive(Clone)]
pub struct Key {
    id: String,
    raw: [u8; KEY_LEN],
}

impl Key {
    /// 由原始 32 字节构造（id 由内容派生）。
    pub fn from_raw(raw: [u8; KEY_LEN]) -> Self {
        let digest = Sha256::digest(raw);
        let id = hex::encode(&digest[..4]);
        Self { id, raw }
    }

    /// 由 base64 文本构造（`openssl rand -base64 32` 形态）。
    pub fn from_base64(b64: &str) -> Result<Self, KeyError> {
        let decoded = BASE64
            .decode(b64.trim())
            .map_err(|e| KeyError::Base64(e.to_string()))?;
        let raw: [u8; KEY_LEN] = decoded.try_into().map_err(|v: Vec<u8>| KeyError::Length {
            expected: KEY_LEN,
            actual: v.len(),
        })?;
        Ok(Self::from_raw(raw))
    }

    /// 密钥标识（8 hex 字符）。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 原始密钥字节（仅 crate 内密码学路径使用）。
    pub(crate) fn raw(&self) -> &[u8; KEY_LEN] {
        &self.raw
    }

    /// base64 文本形态（供 keyring 落盘/备份用）。
    pub fn to_base64(&self) -> String {
        BASE64.encode(self.raw)
    }
}

impl std::fmt::Debug for Key {
    /// 刻意不打印原始密钥字节（防日志泄漏）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Key")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// 密钥环：active + previous（按 id 去重，active 优先）。
#[derive(Clone)]
pub struct Keyring {
    active: Key,
    previous: Vec<Key>,
}

impl Keyring {
    pub fn new(active: Key, previous: Vec<Key>) -> Self {
        let previous = previous.into_iter().filter(|k| k.id != active.id).collect();
        Self { active, previous }
    }

    pub fn active(&self) -> &Key {
        &self.active
    }

    /// 按 id 选钥。
    pub fn find(&self, id: &str) -> Option<&Key> {
        if self.active.id == id {
            return Some(&self.active);
        }
        self.previous.iter().find(|k| k.id == id)
    }

    /// 遍历全部钥（active 优先）。
    pub fn iter(&self) -> impl Iterator<Item = &Key> {
        std::iter::once(&self.active).chain(self.previous.iter())
    }

    pub fn key_ids(&self) -> Vec<String> {
        self.iter().map(|k| k.id.clone()).collect()
    }

    pub fn previous_key_ids(&self) -> Vec<String> {
        self.previous.iter().map(|k| k.id.clone()).collect()
    }

    pub fn len(&self) -> usize {
        1 + self.previous.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }
}

impl std::fmt::Debug for Keyring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keyring")
            .field("active", &self.active.id)
            .field("previous", &self.previous_key_ids())
            .finish()
    }
}

/// 密钥装载源。
pub trait KeyLoader: Send + Sync {
    fn load(&self) -> Result<Keyring, KeyError>;
    fn describe(&self) -> String;
}

/// 文件装载源：active = `<root>/.encryption_key`；previous = `<root>/.encryption_keyring/previous/*.key`。
pub struct FileKeyLoader {
    active_path: PathBuf,
    previous_dir: PathBuf,
}

impl FileKeyLoader {
    pub fn new(root: &Path) -> Self {
        Self {
            active_path: root.join(".encryption_key"),
            previous_dir: root.join(".encryption_keyring/previous"),
        }
    }

    pub fn active_path(&self) -> &Path {
        &self.active_path
    }

    pub fn previous_dir(&self) -> &Path {
        &self.previous_dir
    }
}

impl KeyLoader for FileKeyLoader {
    fn load(&self) -> Result<Keyring, KeyError> {
        let active_b64 = fs::read_to_string(&self.active_path).map_err(|e| KeyError::Io {
            path: self.active_path.clone(),
            source: e,
        })?;
        let active = Key::from_base64(&active_b64)?;

        let previous = read_previous_dir(&self.previous_dir)?;
        Ok(Keyring::new(active, previous))
    }

    fn describe(&self) -> String {
        format!("file:{}", self.active_path.display())
    }
}

/// 环境变量装载源：active = `<active_var>`；previous = `<previous_var>`（逗号分隔，可缺省）。
pub struct EnvKeyLoader {
    active_var: String,
    previous_var: String,
}

impl EnvKeyLoader {
    pub fn new(active_var: &str, previous_var: &str) -> Self {
        Self {
            active_var: active_var.to_string(),
            previous_var: previous_var.to_string(),
        }
    }
}

impl KeyLoader for EnvKeyLoader {
    fn load(&self) -> Result<Keyring, KeyError> {
        let active_b64 = std::env::var(&self.active_var)
            .map_err(|_| KeyError::MissingEnv(self.active_var.clone()))?;
        let active = Key::from_base64(&active_b64)?;
        let previous = std::env::var(&self.previous_var)
            .map(|raw| {
                raw.split(',')
                    .filter(|s| !s.trim().is_empty())
                    .filter_map(|s| Key::from_base64(s).ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(Keyring::new(active, previous))
    }

    fn describe(&self) -> String {
        format!("env:{}", self.active_var)
    }
}

/// 由调用方直接提供 active 钥文本；previous 可来自环境变量与/或目录。
///
/// 供 active 源不是文件而是环境变量的消费方（如 `Framework/system-config` 的
/// `SYSTEM_CONFIG_ENC_KEY`）复用同一 keyring 语义。
pub struct ProvidedKeyLoader {
    active_b64: String,
    previous_env_var: Option<String>,
    previous_dir: Option<PathBuf>,
}

impl ProvidedKeyLoader {
    pub fn new(active_b64: impl Into<String>) -> Self {
        Self {
            active_b64: active_b64.into(),
            previous_env_var: None,
            previous_dir: None,
        }
    }

    pub fn with_previous_env(mut self, var: impl Into<String>) -> Self {
        self.previous_env_var = Some(var.into());
        self
    }

    pub fn with_previous_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.previous_dir = Some(dir.into());
        self
    }
}

impl KeyLoader for ProvidedKeyLoader {
    fn load(&self) -> Result<Keyring, KeyError> {
        let active = Key::from_base64(&self.active_b64)?;
        let mut previous = Vec::new();
        if let Some(var) = &self.previous_env_var {
            if let Ok(raw) = std::env::var(var) {
                previous.extend(
                    raw.split(',')
                        .filter(|s| !s.trim().is_empty())
                        .filter_map(|s| Key::from_base64(s).ok()),
                );
            }
        }
        if let Some(dir) = &self.previous_dir {
            previous.extend(read_previous_dir(dir)?);
        }
        Ok(Keyring::new(active, previous))
    }

    fn describe(&self) -> String {
        "provided".to_string()
    }
}

/// 读取 previous 钥目录（`.key` 后缀文件；目录不存在视为空）。
///
/// 单把既往钥损坏不阻断整体装载（记 warn）；轮换收敛由 `rotate-master-key verify` 兜底。
fn read_previous_dir(dir: &Path) -> Result<Vec<Key>, KeyError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| KeyError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("key"))
        .collect();
    paths.sort();
    let mut keys = Vec::new();
    for p in paths {
        match fs::read_to_string(&p)
            .map_err(|e| KeyError::Io {
                path: p.clone(),
                source: e,
            })
            .and_then(|s| Key::from_base64(&s))
        {
            Ok(k) => keys.push(k),
            Err(e) => log::warn!("keyring: 跳过不可解析的既往钥 {}: {e}", p.display()),
        }
    }
    Ok(keys)
}

/// 进程级主密钥句柄：持有 keyring，解密失败时惰性重载后重试一次。
///
/// 该惰性重载是「在线轮换」的核心：`rotate-master-key promote` 换掉 active 文件后，
/// 运行中的进程无需重启即可读到新钥（旧密文由 previous 钥继续可读）。
pub struct MasterKey {
    loader: Box<dyn KeyLoader>,
    ring: RwLock<Keyring>,
}

impl MasterKey {
    /// 装载并固化 keyring。
    pub fn load(loader: Box<dyn KeyLoader>) -> Result<Self, KeyError> {
        let ring = loader.load()?;
        Ok(Self {
            loader,
            ring: RwLock::new(ring),
        })
    }

    /// 当前 active 钥标识。
    pub fn active_key_id(&self) -> String {
        self.ring
            .read()
            .map(|r| r.active.id.clone())
            .unwrap_or_default()
    }

    /// 全环钥标识（active 优先）。
    pub fn key_ids(&self) -> Vec<String> {
        self.ring.read().map(|r| r.key_ids()).unwrap_or_default()
    }

    /// 强制重载 keyring。
    pub fn refresh(&self) -> Result<(), KeyError> {
        let fresh = self.loader.load()?;
        let mut ring = self.ring.write().map_err(|_| KeyError::Poisoned)?;
        *ring = fresh;
        Ok(())
    }

    /// 加密 → `enc:v2:<key_id>:<base64(nonce‖ct‖tag)>`。
    pub fn encrypt(&self, plaintext: &str) -> Result<String, KeyError> {
        let ring = self.ring.read().map_err(|_| KeyError::Poisoned)?;
        crate::envelope::encode(ring.active(), plaintext)
    }

    /// 解密（级联兼容旧形态）；缺钥/失败时重载 keyring 后重试一次。
    pub fn decrypt(&self, stored: &str) -> Result<String, KeyError> {
        let first = {
            let ring = self.ring.read().map_err(|_| KeyError::Poisoned)?;
            crate::envelope::decode_with(&ring, stored)
        };
        let retryable = matches!(
            &first,
            Err(KeyError::UnknownKeyId(_) | KeyError::Decrypt(..))
        );
        if !retryable {
            return first;
        }
        if self.refresh().is_err() {
            return first;
        }
        let ring = self.ring.read().map_err(|_| KeyError::Poisoned)?;
        crate::envelope::decode_with(&ring, stored)
    }

    pub fn describe(&self) -> String {
        self.loader.describe()
    }

    /// 当前环的只读快照（供统计/验证）。
    pub fn snapshot(&self) -> Result<Keyring, KeyError> {
        self.ring
            .read()
            .map(|r| r.clone())
            .map_err(|_| KeyError::Poisoned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "master-key-test-{}-{}-{}",
                std::process::id(),
                tag,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            fs::create_dir_all(&p).expect("temp dir");
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_key(dir: &Path, name: &str, raw: u8) {
        fs::write(dir.join(name), BASE64.encode([raw; KEY_LEN])).expect("write key");
    }

    #[test]
    fn key_id_is_sha256_prefix_and_stable() {
        let k = Key::from_raw([9u8; KEY_LEN]);
        assert_eq!(k.id().len(), 8, "key_id MUST 为 8 个 hex 字符");
        assert!(k.id().bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(
            Key::from_raw([9u8; KEY_LEN]).id(),
            k.id(),
            "同钥 MUST 同 id"
        );
        assert_ne!(Key::from_raw([8u8; KEY_LEN]).id(), k.id());
    }

    #[test]
    fn from_base64_rejects_wrong_length() {
        let err = Key::from_base64(&BASE64.encode([0u8; 16])).expect_err("16 字节 MUST 拒绝");
        assert!(matches!(
            err,
            KeyError::Length {
                expected: 32,
                actual: 16
            }
        ));
    }

    #[test]
    fn keyring_dedups_active_from_previous() {
        let k = Key::from_raw([1u8; KEY_LEN]);
        let ring = Keyring::new(k.clone(), vec![k.clone(), Key::from_raw([2u8; KEY_LEN])]);
        assert_eq!(ring.len(), 2, "同 id 的 previous 必须被去重");
        assert_eq!(ring.find(k.id()).map(Key::id), Some(k.id()));
    }

    #[test]
    fn file_loader_ignores_missing_previous_dir() {
        let dir = TempDir::new("no-prev");
        write_key(dir.path(), ".encryption_key", 5);
        let ring = FileKeyLoader::new(dir.path()).load().expect("load");
        assert_eq!(ring.len(), 1, "previous 目录缺失时只应有 active");
    }

    #[test]
    fn file_loader_reads_previous_keys() {
        let dir = TempDir::new("prev");
        write_key(dir.path(), ".encryption_key", 5);
        let prev = dir.path().join(".encryption_keyring/previous");
        fs::create_dir_all(&prev).expect("prev dir");
        write_key(&prev, "aaaa.key", 6);
        write_key(&prev, "bbbb.key", 7);
        fs::write(prev.join("ignored.txt"), "not-a-key").expect("noise");
        let ring = FileKeyLoader::new(dir.path()).load().expect("load");
        assert_eq!(ring.len(), 3, "active + 2 把 previous（非 .key 后缀忽略）");
    }

    #[test]
    fn missing_active_key_is_loud() {
        let dir = TempDir::new("missing");
        let err = FileKeyLoader::new(dir.path())
            .load()
            .expect_err("缺 active MUST 报错");
        assert!(matches!(err, KeyError::Io { .. }));
    }

    #[test]
    fn master_key_refresh_picks_up_promoted_key() {
        let dir = TempDir::new("promote");
        write_key(dir.path(), ".encryption_key", 1);
        let mk = MasterKey::load(Box::new(FileKeyLoader::new(dir.path()))).expect("load");
        let old_id = mk.active_key_id();

        // 模拟 promote：旧钥降级 previous，新钥成为 active
        let prev = dir.path().join(".encryption_keyring/previous");
        fs::create_dir_all(&prev).expect("prev dir");
        fs::write(
            prev.join(format!("{old_id}.key")),
            BASE64.encode([1u8; KEY_LEN]),
        )
        .expect("demote");
        fs::write(
            dir.path().join(".encryption_key"),
            BASE64.encode([2u8; KEY_LEN]),
        )
        .expect("promote");

        // 惰性重载：不重启进程即见新 active
        mk.refresh().expect("refresh");
        assert_ne!(mk.active_key_id(), old_id, "refresh MUST 见新 active");
        assert!(mk.key_ids().contains(&old_id), "旧钥 MUST 仍在环内");

        // 旧钥密文仍可解（这是「无窗口」的判据：previous 钥在环内）
        let legacy = crate::envelope::encode(&Key::from_raw([1u8; KEY_LEN]), "v1-old")
            .expect("encrypt with old key");
        assert_eq!(mk.decrypt(&legacy).expect("previous 钥可解"), "v1-old");
    }

    #[test]
    fn decrypt_reloads_and_retries_on_unknown_key() {
        let dir = TempDir::new("retry");
        write_key(dir.path(), ".encryption_key", 1);
        let mk = MasterKey::load(Box::new(FileKeyLoader::new(dir.path()))).expect("load");

        // 进程外发生的轮换：active 换成钥 2，钥 1 入 previous
        let prev = dir.path().join(".encryption_keyring/previous");
        fs::create_dir_all(&prev).expect("prev dir");
        fs::write(prev.join("old1.key"), BASE64.encode([1u8; KEY_LEN])).expect("prev");
        fs::write(
            dir.path().join(".encryption_key"),
            BASE64.encode([2u8; KEY_LEN]),
        )
        .expect("rotate");

        // 用新 active 钥加密的密文：首解缺钥 → 惰性重载 → 重试成功
        let enc =
            crate::envelope::encode(&Key::from_raw([2u8; KEY_LEN]), "post-rotate").expect("enc");
        assert_eq!(mk.decrypt(&enc).expect("重试 MUST 成功"), "post-rotate");
    }

    #[test]
    fn env_loader_reports_missing_var() {
        let loader = EnvKeyLoader::new("MASTER_KEY_TEST_ABSENT_VAR", "MASTER_KEY_TEST_ABSENT_PREV");
        let err = loader.load().expect_err("缺 env MUST 报错");
        assert!(matches!(err, KeyError::MissingEnv(v) if v == "MASTER_KEY_TEST_ABSENT_VAR"));
    }
}
