//! Alioth AI Agent 框架
//!
//! 为 Gateway EmpAgent 提供多 Agent 注册、路由与执行能力。
//!
//! 核心组件：
//! - `Agent` trait：所有专用 Agent 的实现接口
//! - `AgentRegistry`：运行时 Agent 元数据管理
//! - `AgentRouter`：基于规则 + LLM 的意图分类与 Agent 选择
//! - 五个内置专用 Agent + 一个通用回退 Agent

pub mod agents;
// pool 模块已删除（refactor-chat-ai-subject-identity-memory D9：AgentInstance.memory()
// 全仓零调用 ⇒ 只写不读的死路径；记忆读写只经 isahl_auth 双层 store）。
pub mod registry;
pub mod router;
pub mod tools;

pub use agents::{
    Agent, AgentCapability, AgentConfig, AgentContext, AgentResult, Attachment, ConfirmationLevel,
    DbAccessLevel, ExecutionTarget, Modality, RoutingInfo, RoutingWeights, TokenUsage, ToolChoice,
    ToolDefinition, ToolExecutor,
};
pub use registry::AgentRegistry;
pub use router::{
    AgentRouter, CompositeStrategy, FallbackStrategy, KeywordStrategy, LlmClassifierStrategy,
    RoutingContext, RoutingDecision, RoutingLevel, RoutingStrategy,
};
