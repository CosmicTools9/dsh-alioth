## Phase 2: TDD 模式补充开发 Service 接口

### 2-0: 脚手架生成（NEW — ontology-gen-bridge）

在执行 TDD 循环前，**先通过 ontology-gen-bridge 生成 Service 骨架代码**。

**适用性门禁**：若 `ontology-output.json` 中的实体包含 `qk_*` 字段，桥接会 hard-error。
此类实体需跳过脚手架，直接进入 TDD。

```bash
# 1. 确认 ontology-output.json 存在
test -f Pre-Proc/{ns}/local/ontology-output.json || { echo "MISSING: 先执行 alioth-ontology"; exit 1; }

# 2. 生成骨架（内部: MappingOutput → MetaModule → ModuleApiGenerator → 写盘）
#    --input 与 --database-url 为必填；--output / --name 可选（默认 ./backend 与 generated）
cargo run -p ontology-gen-bridge -- \
  --input Pre-Proc/{ns}/local/ontology-output.json \
  --database-url "$DATABASE_URL" \
  --output Pre-Proc/{ns}/Sources/Apps/Services/{service}/backend/ \
  --name {service}

# 3. 验证生成代码可编译
cargo check --manifest-path Pre-Proc/{ns}/Sources/Apps/Services/{service}/backend/Cargo.toml
```

> ⚠️ 生成的是**结构骨架**（entity struct, route 注册, handler 签名, error 类型）。
> 以下内容**不在生成范围内**，需在 TDD 循环中手工补全：
>
> - `AliothEntity` trait impl (含 `TABLE_NAME`) — 物理表名无法通过 IR-1 透传
> - `HasReferenceJoins` trait impl — 关联引用需手工配置
> - 标量字段 (`qk_*`) — 结构化值对象需手工实现
> - 业务逻辑、状态机转换、自定义验证
> - 系统列 (`id`, `domain_`, `dk_*`, `paths` 等) 不出现在 DTO 中

### 2-1: 先写测试

在写任何实现代码之前，先创建集成测试描述业务语义期望：

```rust
// Services/{ns}/catalog/backend/tests/product_api_test.rs

#[tokio::test]
async fn product_detail_returns_business_semantic_shape() {
    let pool = ::common::testing::connect_test_db().await;
    setup_test_schema(&pool).await;

    let svc = ProductService::new(pool.clone()).unwrap();

    // 1. 创建产品
    let created = svc.create_product(CreateProductRequest {
        name: "Test Product".into(),
        code: Some("TST-001".into()),
        price: Some("99.99".into()),
        currency_id: Some(1),
        ..Default::default()
    }, 0).await.unwrap();

    // 2. 断言 DTO 形状（不是 DB entity）
    assert_eq!(created.name, "Test Product");
    assert!(created.pricing.unit_price.is_some());
    assert_eq!(created.pricing.unit_price.as_ref().unwrap().currency, "CNY");
    assert!(!created.pricing.unit_price.as_ref().unwrap().amount.is_empty());

    // 3. 断言不包含 DB 列名
    // 编译期保证：ProductDetail 没有 notice/ck_category/qk_price 等字段
}

#[tokio::test]
async fn product_list_returns_summaries_not_db_entities() {
    let pool = test_utils::connect_test_db().await;
    test_utils::setup_test_schema(&pool).await;

    let svc = ProductService::new(pool.clone()).unwrap();
    let query = ListQuery::default();
    let page = svc.list_summaries(&query).await.unwrap();

    for item in &page.items {
        // Status should be a business enum, not a raw flag
        assert!(matches!(item.status, ProductStatus::Active | ProductStatus::Inactive | ProductStatus::Discontinued));
    }
}
```

### 2-2: 红→绿→重构循环

| 步骤        | 操作                            | 验证              |
| ----------- | ------------------------------- | ----------------- |
| 🔴 Red      | 写测试 → 编译失败（类型不存在） | `cargo test` fail |
| 🟢 Green    | 实现 DTO + Service 最小代码     | `cargo test` pass |
| 🔵 Refactor | 抽取业务语义分组、标量解析      | `cargo test` pass |

### 2-2.5: Mapping 验证骨架合并（缺口 7 门禁）

在红→绿循环开始之前，**必须将 alioth-ontology 产出的 `mapping-verify.rs` 骨架合并到 Service 的测试目录**：

```bash
# 1. 确认 mapping-verify.rs 骨架存在（由 Rust 测试脚手架维护，cargo test 自动生成/校验）
ls tests/{ns}/mapping_verify_layer1.rs tests/{ns}/mapping_verify_layer2.rs 2>/dev/null

# 2. 若不存在 → 运行 Rust 测试以生成/校验骨架
mise run test-rust

# 3. 合并到 Service 测试目录
cp tests/{ns}/mapping_verify_layer1.rs \
   Pre-Proc/{ns}/Sources/Apps/Services/{service-id}/backend/tests/mapping_verify.rs
```

**门禁规则**：

| 条件                                                            | 处理                                                                              |
| --------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `tests/{ns}/mapping_verify_layer*.rs` 存在                      | **必须**合并到 `tests/mapping_verify.rs`，并调整 Rust 类型引用让其编译            |
| `MappingOutput` JSON 缺 `db_bindings[]` 或 `alignment_matrix[]` | **必须先执行 alioth-ontology**（生成 `Pre-Proc/{ns}/local/ontology-output.json`） |
| 合并后编译                                                      | **必须 `cargo test` 通过**包含 `mapping_verify` 测试                              |
| `mapping_verify.rs` 已有但无任何字段变更                        | 跳过（已有验证）                                                                  |

**合并后的验收标准**：

```bash
cd Pre-Proc/{ns}/Sources/Apps/Services/{service-id}/backend
cargo test --test mapping_verify 2>&1 | grep -q "test result: ok"
# 通过 → 映射验证就绪
```

> ⚠️ 骨架中的类型引用（`{EntityName}Entity`、`{ServiceName}`、`mock_scalars`）需要根据实际的 Service DTO/Service 结构调整。
> 骨架提供的是断言合约（`assert_eq!(dto.{field}, "TEST_VALUE")`），类型名称必须由开发者填充实际 Rust 类型。

### 2-3: DTO 设计

DTO 是 Service 对外的业务语义契约——**前端和 Block 只看到 DTO，永不接触 DB entity**：

```rust
// dto/src/product.rs

/// 列表摘要
pub struct ProductSummary {
    pub id: String,             // business ID
    pub name: String,
    pub status: ProductStatus,  // enum
    pub unit_price: Option<Money>,
}

/// 详情
pub struct ProductDetail {
    pub id: String,
    pub name: String,
    pub pricing: PricingInfo,       // 语义分组
    pub inventory: InventoryInfo,   // 语义分组
    pub lifecycle: LifecycleInfo,   // 语义分组
}
```

### 2-4: Service 层（DB→DTO 转换）

Service 是唯一执行 DB entity → DTO 转换的地方：

```rust
impl ProductService {
    /// DB entity → business semantic DTO
    async fn to_detail(&self, scalar: &ScalarService, p: &ShopProduct) -> Result<ProductDetail, ApiError> {
        let currency = scalar.get_common(p.sk_currency).await?.map(|d| d.to_string());
        let price = scalar.get_price(p.qk_price).await?.map(|d| d.to_string());

        Ok(ProductDetail {
            id: p.code.clone().unwrap_or_else(|| p.id.to_string()),
            name: p.notice.clone().unwrap_or_default(),
            pricing: PricingInfo {
                currency: currency.clone(),
                unit_price: price.map(|a| Money {
                    amount: a,
                    currency: currency.unwrap_or("CNY".into()),
                }),
                price_input: None,
            },
            // ...
        })
    }
}
```

### 2-5: Handler 层

Handler 只做路由 + 调用 Service + 返回 DTO，**不直接接触 Repository 或 DB entity**：

```rust
pub async fn get(
    pool: web::Data<sqlx::PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, common::AliothError> {
    let svc = ProductService::new(pool.get_ref().clone())?;
    let product = svc.get_detail(path.into_inner()).await?
        .ok_or_else(|| common::AliothError::NotFound("product".into()))?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(product)))
}
```

---

### 2-6: dk 静态绑定（BACKEND_FRAMEWORK §7.3.3 2026-08-12 裁定）

写路径（INSERT）MUST 设置 `dk_scene/dk_factor/dk_function`。值**不是**前端请求数据，也不是运行时推导——**每个 API 内固定三元组**，来源 = `Pre-Proc/{ns}/local/ontology-output.json` 的 `prototype_semantics[].coordinates`（本体映射阶段产出，check-ontology-contract 门禁保证存在）。

生成模式（DkEntity，参考 transport-dispatch `repositories/mod.rs`）：

```rust
/// 实体 → 固定三元组 code（一实体一分支；语义校准只改此处）
pub(crate) enum DkEntity {
    ConsignmentTrade,   // 委托创建（对应 prototype_semantics[x].coordinates）
    FreightProduct,     // 产品管理创建
    // …
}
impl DkEntity {
    pub(crate) fn coords(&self) -> (&'static str, &'static str, &'static str) {
        match self {
            // 值取自 ontology-output.json coordinates（scene/factor/function code）
            DkEntity::ConsignmentTrade => ("GC", "FJA", "↓_BE"),
            DkEntity::FreightProduct => ("GC", "FJA", "↓_BE"),
        }
    }
}

// INSERT 前：按 code 解析维度行 ZUID（跨 dev/pre/prod 稳定；查不到 → NULL，列可空）
let (scene_id, factor_id, func_id) = resolve_ontology_coords(&mut tx, DkEntity::ConsignmentTrade).await?;
```

**公共基础设施（2026-08-12 合并）**：code→ZUID 解析统一走框架 crate `ontology-binding`（`resolve(pool, coords)` / `resolve_conn(conn, coords)` + `DkBinding` trait 一处实现）；实体映射保留各 Service（不同 namespace 同实体名坐标合法不同）。参考：`Framework/backend/ontology-binding/tests/ontology_resolve_test.rs`（单元 + DB 集成 `#[ignore]`）。

**规则**：
1. 每个写 API 的 dk 三元组从映射产物对应 `coordinates` 抄录（**静态**，编译期确定），禁前端传值/请求体字段/Header。
2. 同表不同 dk → 接口语义必然不同 → **必须拆 API**（不得在单 API 内按条件选 dk）。
3. 禁共享无参默认值函数（每个 API 显式传实体）；禁硬编码环境相关 ZUID。
4. 映射产物缺 `prototype_semantics[].coordinates` 三元组 → Phase 0 校验失败，先补映射再生成（`ALIOTH_ONTOLOGY_SPEC` §十）。
