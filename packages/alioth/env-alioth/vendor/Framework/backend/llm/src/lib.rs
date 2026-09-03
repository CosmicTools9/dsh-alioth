//! LLM service framework.
//!
//! Each provider (DeepSeek / Kimi / MiniMax) has its own dedicated backend
//! implementation under `backends/`. The high-level [`LlmService`] façade
//! dispatches to the backend based on `LlmProvider`.

pub mod backends;
pub mod image;
pub mod service;
pub mod types;

pub use backends::{
    BackendError, CompletionRequest, CompletionResponse, ImageContent, LlmBackend, MessageContent,
    ToolCallResult,
};
pub use service::{LlmError, LlmService};
pub use types::{
    GenerationParams, LlmProvider, LlmResponse, LlmServiceConfig, ModelRole, ReasoningEffort,
    RoleModel, TokenUsage, ToolCall, ToolDefinition, DEFAULT_MAX_TOKENS,
};
