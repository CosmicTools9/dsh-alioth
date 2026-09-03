# Alioth Common Library

跨模块共享的基础设施组件，用于 AliothStudio 企业数据应用平台。

## 功能

- **请求上下文管理** (`context`) - Gateway、SSO 和业务模块间的用户身份传递
- **审计字段支持** - Repository 层的审计字段处理

## 核心类型

```rust
use alioth_common::context::{RequestContext, RequestContextExt, AuditFields};

// Handler 中提取用户ID
let user_id = req.audit_user_id();

// Repository 中使用
let audit = AuditFields::from_request(&req);
```

## 用法示例

### Handler

```rust
use actix_web::HttpRequest;
use alioth_common::context::RequestContextExt;

async fn handler(req: HttpRequest) {
    let user_id = req.audit_user_id(); // Option<i64>
}
```

### Repository

```rust
use alioth_common::context::AuditFields;

pub async fn create(&self, data: Input, user_id: Option<i64>) -> Result<Entity, Error> {
    sqlx::query("INSERT INTO table (created_by_id, ...) VALUES ($1, ...)")
        .bind(user_id)
        .execute(&self.pool).await
}
```

## 依赖

```toml
[dependencies]
alioth-common = { path = "../../Framework/backend/common" }
```
