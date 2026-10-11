## Track 2: 跨 Service 引用桥接

当 `alioth-ontology` 的输出标注了实体间引用关系（`references: ["product"]`）时，需要建立跨 Service 的数据桥接。

**规约正本 = `docs/specs/SERVICE_SPEC.md` §11**：同一 namespace 内 Service 间 MUST 通过**函数调用**（共享 crate 或内部模块）协作、MUST NOT 走 HTTP；跨 namespace **禁止**直接调用，必须经 `Framework/backend/runtime-contract/` 的事件或编排层；跨 Service 引用的 DTO MUST 在 `dtoDependencies` 声明并在 `dtoExposes` 导出，调用关系 MUST 无环（门禁 `scripts/check/check-service-dag.ts`）。

**同一 namespace 内的桥接步骤**：

1. 在调用方 `backend/Cargo.toml` 添加被调方 service crate 的 path 依赖（例：`se-service-sap-execution = { path = "../sap-execution/backend" }`），或在共享 crate / 内部模块暴露函数。
2. 在调用方 `service.json` 的 `dtoDependencies` 声明被调 Service 的 `id`。
3. 在被调方 `service.json` 的 `dtoExposes.refs` 导出实际被消费的 DTO 名（**双向对齐**——只声明依赖不导出 DTO 会被 `check-service-dag.ts` 判悬空）。
4. 以函数调用形式消费（同进程、共享 PgPool），不构造 HTTP 调用。

```rust
// 同 ns 跨 Service 函数调用（同进程装配；被调方 crate 暴露的公开函数）
use se_service_sap_execution::aggregates::budget_usage;
let usage = budget_usage(&self.pool, year, &project_codes).await?;
```

**跨 namespace**：禁止直接调用（crate 依赖与 HTTP 均禁）；经 `Framework/backend/runtime-contract/` 的事件或编排层协调。

**禁止**：

- ❌ 跨 namespace 直调（crate 依赖或 HTTP）
- ❌ 为其他 Service 的表自行构造 FK 值
- ❌ 未在 `dtoDependencies` 声明即消费他 Service 的 DTO（门禁阻断）
- ⚠️ 复制他 Service 已实现的派生口径（如聚合/执行率）而非调用其函数——违反 `service-ownership-single-implementation`

---
