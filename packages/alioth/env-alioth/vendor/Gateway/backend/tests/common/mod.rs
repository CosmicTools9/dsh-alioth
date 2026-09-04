//! Gateway 集成测试公共辅助函数
//!
//! 使用统一的测试基础设施。
//! 核心原则：测试数据绝不残留。

// Gateway 不依赖模块特定的 setup_schema（由 TRUNCATE 统一清理）
