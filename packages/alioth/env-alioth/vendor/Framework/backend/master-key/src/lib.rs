//! 主密钥 keyring 与版本化信封。
//!
//! 提供两件事：
//!
//! 1. **keyring**（[`Keyring`] / [`MasterKey`]）：当前钥（active） + 既往钥（previous）多钥装载，
//!    支持运行期惰性刷新——轮换无需停机、无需重启进程。
//! 2. **版本化信封**（[`envelope`]）：新密文带密钥标识 `enc:v2:<key_id>:<base64(nonce‖ct‖tag)>`，
//!    解密级联兼容三种旧形态。
//!
//! 设计正本：`openspec/changes/add-master-key-rotation/design.md`（D1–D6）。

pub mod envelope;
pub mod keyring;

pub use envelope::{decode_with, encode, shape, Shape};
pub use keyring::{
    EnvKeyLoader, FileKeyLoader, Key, KeyError, KeyLoader, Keyring, MasterKey, ProvidedKeyLoader,
    KEY_LEN,
};
