# alioth-system-config

AliothStudio 系统配置公共组件（Backend）。

## 职责

提供应用接入外部数据链路、信息服务的**通用配置管理抽象**：
- LLM 提供商配置（OpenAI、Anthropic、Kimi、DeepSeek 等）
- 邮件服务配置（SMTP）
- 即时通讯配置（企业微信、钉钉、飞书）
- Webhook 配置
- 对象存储配置（S3、阿里云 OSS）
- 短信服务配置

## 核心特性

- **与物理表解耦**：只定义 `SystemConfigRepository` trait，不绑定具体表名和 schema。Gateway / Meta 各自实现并映射到各自的物理表。
- **Schema 驱动**：预定义各分类的字段 Schema，前端可基于 Schema 动态渲染表单
- **敏感数据加密**：credentials 中的敏感字段自动 AES-256-GCM 加密存储，前缀标记 `enc:`
- **安全返回**：对外 API 返回脱敏数据（`SystemConfigSafeResponse`），内部服务通过 `get_full_config` 获取解密值
- **泛型 Service**：`SystemConfigService<R>` 接受任意实现了 `SystemConfigRepository` 的类型

## 使用方式

### 1. 初始化加密（应用启动时必做）

```rust
alioth_system_config::crypto::init_encryption(
    &std::env::var("SYSTEM_CONFIG_ENC_KEY").unwrap()
).unwrap();
```

### 2. 自行实现 Repository

```rust
use alioth_system_config::{SystemConfigRepository, SystemConfig, CreateSystemConfigRequest, UpdateSystemConfigRequest};
use async_trait::async_trait;
use sqlx::{PgPool, Error};

#[derive(Clone)]
struct MyRepo {
    pool: PgPool,
}

#[async_trait]
impl SystemConfigRepository for MyRepo {
    async fn insert(&self, req: &CreateSystemConfigRequest) -> Result<SystemConfig, Error> { ... }
    async fn find_by_id(&self, id: i64) -> Result<Option<SystemConfig>, Error> { ... }
    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<SystemConfig>, Error> { ... }
    async fn update(&self, id: i64, req: &UpdateSystemConfigRequest) -> Result<Option<SystemConfig>, Error> { ... }
    async fn soft_delete(&self, id: i64) -> Result<u64, Error> { ... }
    async fn find_by_code(&self, code: &str) -> Result<Option<SystemConfig>, Error> { ... }
}
```

### 3. 注册路由

```rust
use alioth_system_config::configure_routes;

let repo = MyRepo::new(pool);
app.configure(|cfg| configure_routes(repo, cfg));
```

路由前缀：`/system-config`


## 环境变量

| 变量名 | 说明 | 生成方式 |
|--------|------|----------|
| `SYSTEM_CONFIG_ENC_KEY` | AES-256 加密密钥（32 字节 Base64） | `alioth_system_config::crypto::generate_key()` |

## 重要约束

- **本 crate 不绑定具体物理表**。使用方（Gateway / Meta）需自行在各自 schema 下创建符合自身规范的表。
- `isahl` schema 下的表必须严格使用 Alioth 物理命名，并遵循 `zc_id_lifecycle` 继承规范。
- `isahl_meta` schema 下的表不受物理命名约束，但应使用 `BIGSERIAL` 主键。
