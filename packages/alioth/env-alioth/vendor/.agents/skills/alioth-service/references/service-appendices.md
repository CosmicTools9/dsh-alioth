# Service 工具、结构与 Pipeline 集成（参考）

## 工具

### check-service-dag.ts（跨 Service 依赖拓扑）

校验 `dtoDependencies` 引用存在性与依赖无环（`SERVICE_SPEC` §11.2）：

```bash
bun scripts/check/check-service-dag.ts Pre-Proc/{ns} --fail
```

输出：命名空间内 Service 清单、依赖边、悬空引用与循环路径。pre-commit 已接入同一脚本（`Service DAG` 门禁）。

### ontology-mapping gap（Rust CLI）

Block 业务语义与本体规约差异检测：

```bash
target/debug/ontology-mapping gap Pre-Proc/{ns}/Prototypes/Blocks/{block-id}/b-v{N}.html
```

输出：mock 数据推断的业务语义形状、列表页列定义、与本体规约的差异/冲突（JSON）。

### 跨技能工具

以下工具属于 `alioth-block`，`alioth-service` 工作流中作为辅助使用：

- `extract-block-from-module.ts`（`alioth-block`）：从 Module 原型拆解 Scene Capability

使用完整路径调用：

```bash
bun .agents/skills/alioth-block/scripts/extract-block-from-module.ts Pre-Proc/{ns}/Prototypes/Modules/{module}/v{N}.html
```

## 对齐分工

| 技能                       | 输入                                                                 | 输出                                                     |
| -------------------------- | -------------------------------------------------------------------- | -------------------------------------------------------- |
| `alioth-block`             | 业务需求                                                             | Block 原型 + 数据接口 JSON                               |
| `alioth-ontology`          | 原型数据接口 JSON                                                    | 实体分解 + 坐标三元组（scene/factor/function）+ 查询模板 |
| `alioth-service`（本技能） | `alioth-ontology` 输出（`Pre-Proc/{ns}/local/ontology-output.json`） | 业务语义 DTO + Service + Handler + TDD 测试 + Block 绑定 |

## Service 单元结构

```
Pre-Proc/{ns}/Sources/Apps/Services/{unit}/
├── service.json                  # id, domain, dtoDependencies
├── dto/                         # DTO crate — 业务语义 API 契约
│   ├── Cargo.toml
│   └── src/
│       ├── {entity}_ref.rs      # 跨 Service 引用结构体
│       └── {entity}.rs          # 业务语义 DTO（ProductSummary/Detail 等）
└── backend/                     # backend crate — 实现层
    ├── Cargo.toml
    └── src/
        ├── domain/{entity}.rs           # DB entity（内部）
        ├── repository/{entity}_repo.rs  # sqlx::query_as
        ├── services/{entity}_service.rs # DB→DTO 转换 + 业务逻辑
        └── handlers/{entity}_handler.rs # HTTP thin layer
    └── tests/
        └── {entity}_api_test.rs         # TDD 集成测试
```

## 关键约束

| 约束                      | 规则                                              |
| ------------------------- | ------------------------------------------------- |
| Handler 不接触 Repository | Handler 只调用 Service                            |
| Handler 不接触 DB entity  | 返回类型必须是 DTO，永不返回 DB entity            |
| Service 是唯一转换点      | DB entity → DTO 只在 Service 中发生               |
| DTO 是 API 契约           | 前端和 Block 只接触 DTO，永不接触 DB entity       |
| DB 列名不泄漏             | `notice`/`qk_price`/`sk_currency` 不出现在 DTO 中 |
| TDD 优先                  | 先写测试描述业务期望，再实现                      |
| 人工审定                  | ONTOLOGY_SPEC.md 修改需人工确认后进入实现         |
| 冲突必标识                | `⚠️ CONFLICT` 标记不可省略                        |

## Pipeline 集成

alioth-service 是原型→交付管道的终点执行者。被 `alioth-build.sh` 调度时遵循以下契约。

### pipeline.yml 映射

| pipeline stage | 本技能 Track/Phase | 说明                                    |
| -------------- | ------------------ | --------------------------------------- |
| `factor_dev`   | Phase 0→4          | DTO → Service → Handler → TDD 全链路    |
| `factor_dev`   | Track 2            | 跨 Factor 引用桥接                      |
| -              | Track 3            | 现有 Service 优化（不从 pipeline 触发） |

### 输入

| 来源                     | 数据                | 格式                                           |
| ------------------------ | ------------------- | ---------------------------------------------- |
| `ontology_mapping` stage | MappingOutput JSON  | `Pre-Proc/{ns}/local/ontology-output.json`     |
| pipeline state           | namespace           | JSON（state.json）                             |
| ONTOLOGY_SPEC.md         | 实体定义 + 字段映射 | Markdown（`docs/specs/{ns}/ONTOLOGY_SPEC.md`） |

### 输出

| 产物                     | 路径模板                                             | 用途                |
| ------------------------ | ---------------------------------------------------- | ------------------- |
| Service DTO crate        | `Pre-Proc/{ns}/Sources/Apps/Services/{unit}/dto/`         | API 契约            |
| Service backend crate    | `Pre-Proc/{ns}/Sources/Apps/Services/{unit}/backend/`     | 可部署 API          |
| service.json             | `Pre-Proc/{ns}/Sources/Apps/Services/{unit}/service.json` | 声明配置            |
| ONTOLOGY_SPEC.md（补充） | `docs/specs/{ns}/ONTOLOGY_SPEC.md`                   | 冲突记录 + DTO 规范 |

### 人类门禁

| Gate ID               | 类型      | 触发条件                     | 传递至 state                 |
| --------------------- | --------- | ---------------------------- | ---------------------------- |
| `dto_shape`           | ask       | Phase 0-3 DTO 分组方案设计   | `pending_asks[]`             |
| `conflict_resolution` | ask_multi | Phase 1-5 发现 CONFLICT 标记 | `pending_asks[]`（逐项裁决） |
