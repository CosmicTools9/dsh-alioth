## Track 2: 跨 Service 引用桥接

当 `alioth-ontology` 的输出标注了实体间引用关系（`references: ["product"]`）时，需要建立跨 Service 的数据桥接。

**桥接步骤**：

1. 确认目标 Service 的 DTO crate 暴露了所需的 DTO 类型
2. 在调用方 Service 的 `Cargo.toml` 中添加 `factor-{id}-dto` 依赖
3. 在 `service.json` 的 `dtoDependencies` 中添加目标 Service 名
4. 在 Service 中使用目标 DTO 类型：

```rust
use factor_catalog_dto::ProductRef;

impl OrderService {
    pub async fn enrich_with_product(&self, order: &Order) -> Result<OrderDetail, ApiError> {
        let product_ref = factor_catalog_dto::resolve_product(&self.pool, order.fk_product).await?;
        Ok(OrderDetail {
            id: order.code.clone(),
            product_name: product_ref.map(|p| p.name),
            // ...
        })
    }
}
```

**禁止**：

- ❌ 调用其他 Service 的 backend crate
- ❌ 直接访问其他 Service 的 HTTP 端点
- ❌ 为其他 Service 的表自行构造 FK 值

---
