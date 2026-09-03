//! 版本后端错误

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("git 命令执行失败: {0}")]
    GitExec(String),

    #[error("git 命令非零退出 ({code}): {stderr}")]
    GitExit { code: i32, stderr: String },

    #[error("后端不支持该操作: {0}")]
    Unsupported(&'static str),

    #[error("输入无效: {0}")]
    InvalidInput(String),

    #[error("对象不存在: {0}")]
    NotFound(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("配置错误: {0}")]
    Config(String),
}

pub type BackendResult<T> = Result<T, BackendError>;
