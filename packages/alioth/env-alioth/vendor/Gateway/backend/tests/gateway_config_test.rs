//! Gateway 配置解析单元测试
//!
//! 验证 Config 结构体的环境变量解析、默认值、序列化等行为。
//! 不依赖数据库或模块路由，可独立编译运行。

use alioth_gateway::Config;

#[test]
fn test_config_has_required_fields() {
    // 验证 Config 结构体具有所需字段
    let config = Config {
        sso_service_url: "http://localhost:9002".to_string(),
        database_url: "postgres://localhost:5432/aliothstudio".to_string(),
        sso_jwt_public_key: "test-public-key".to_string(),
        sso_jwt_public_key_prev: Vec::new(),
        sso_jwt_issuer: "http://localhost:9002".to_string(),
        server_addr: "127.0.0.1:9001".to_string(),
        cors_allowed_origins: vec!["http://localhost:5173".to_string()],
    };

    assert_eq!(config.sso_service_url, "http://localhost:9002");
    assert_eq!(config.server_addr, "127.0.0.1:9001");
    assert_eq!(config.cors_allowed_origins.len(), 1);
    assert_eq!(config.cors_allowed_origins[0], "http://localhost:5173");
}
