//! Alioth System Config Framework
//!
//! 系统配置公共组件，提供应用接入外部数据链路、信息服务的通用管理能力：
//! - LLM 提供商配置（OpenAI、Anthropic、Kimi、DeepSeek 等）
//! - 邮件服务配置（SMTP）
//! - 即时通讯配置（企业微信、钉钉、飞书）
//! - Webhook 配置
//! - 对象存储配置（S3、阿里云 OSS）
//! - 短信服务配置
//!
//! # 核心特性
//! - **与物理表解耦**：只提供 `SystemConfigRepository` trait，不绑定具体表名和 schema
//! - **Schema 驱动**：预定义各分类的字段 Schema，前端可基于 Schema 动态渲染表单
//! - **敏感数据加密**：credentials 中的敏感字段自动 AES-256-GCM 加密存储，前缀标记 `enc:`
//! - **安全返回**：对外 API 返回脱敏数据；运行期消费经 `common::config_resolution` 解析生效配置（含凭证解密，change fix-config-effective-resolution）
//! - **泛型 Service**：`SystemConfigService<R>` 接受任意实现了 `SystemConfigRepository` 的类型
//!
//! # 使用方式
//!
//! ```rust,ignore
//! use system_config::{crypto, configure_routes};
//! use system_config::{SystemConfigService, SystemConfigRepository};
//!
//! // 1. 初始化加密（应用启动时）
//! crypto::init_encryption(&std::env::var("SYSTEM_CONFIG_ENC_KEY").unwrap()).unwrap();
//!
//! // 2. 自行实现 SystemConfigRepository（映射到 Gateway / Meta 各自的物理表）
//! #[derive(Clone)]
//! struct MyRepo { pool: PgPool }
//! #[async_trait]
//! impl SystemConfigRepository for MyRepo {
//!     async fn insert(&self, req: &CreateSystemConfigRequest, actor_id: Option<i64>) -> Result<SystemConfig, sqlx::Error> { ... }
//!     async fn find_by_id(&self, id: i64) -> Result<Option<SystemConfig>, sqlx::Error> { ... }
//!     async fn list(&self, limit: i64, offset: i64) -> Result<Vec<SystemConfig>, sqlx::Error> { ... }
//!     async fn update(&self, id: i64, req: &UpdateSystemConfigRequest, actor_id: Option<i64>) -> Result<Option<SystemConfig>, sqlx::Error> { ... }
//!     async fn soft_delete(&self, id: i64) -> Result<u64, sqlx::Error> { ... }
//!     async fn find_by_code(&self, code: &str) -> Result<Option<SystemConfig>, sqlx::Error> { ... }
//! }
//!
//! // 3. 注册路由（pool 用于审计/探测取行；probes 注入各类别连通性探测实现；
//! //    二者均注册为 scope 内 app_data）
//! let repo = MyRepo { pool: pool.clone() };
//! let probes = system_config::probe::ProbeRegistry::new().with("email", MyEmailProbe);
//! app.configure(|cfg| configure_routes(repo, pool, probes, cfg));
//!
//! ```

pub mod crypto;
pub mod handlers;
pub mod models;
pub mod operation;
pub mod probe;
pub mod repository;
pub mod schema;
pub mod service;

pub use handlers::configure_routes;
pub use models::*;
pub use probe::{ProbeOutcome, ProbeRegistry, ProviderProbe};
pub use repository::SystemConfigRepository;
pub use service::{SystemConfigError, SystemConfigService};
