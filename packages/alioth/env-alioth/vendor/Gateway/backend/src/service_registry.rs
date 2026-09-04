//! Service 路由注册中心
//!
//! 全部由 build.rs 自动生成，包括每个 namespace 的 `register_{ns}_routes()` 函数
//! 以及按 NAMESPACE 环境变量分发的 `register_service_routes()` 顶级入口。
//!
//! 新增 namespace 只需：创建 Pre-Proc/{ns}/Sources/Services/{service}/service.json
//! 并在 Cargo.toml 中添加对应的 optional dep + feature gate。
//! 无需手动编辑本文件。

#[allow(clippy::single_component_path_imports)] // build.rs 生成：单组件路径 import（crate 名即路径）
mod auto_service_registry {
    include!(concat!(env!("OUT_DIR"), "/auto_service_registry.rs"));
}
pub use auto_service_registry::*;
