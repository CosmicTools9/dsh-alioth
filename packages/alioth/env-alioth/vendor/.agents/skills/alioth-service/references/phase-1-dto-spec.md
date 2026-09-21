## Phase 1: DTO 规格与冲突标识

### 1-1: 读取 ONTOLOGY_SPEC.md

```bash
# 确认规约文件存在
ls docs/specs/{ns}/ONTOLOGY_SPEC.md
```

### 1-2: 差异分析（基于 MappingOutput JSON）

对比 `alioth-ontology` 的 MappingOutput JSON 与现有 ONTOLOGY_SPEC.md：

| 检查维度   | 来源                                                                                                     | 方法                                                                                       |
| ---------- | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| 实体覆盖   | `db_bindings[].model_entity`                                                                             | 是否全部已存在于 ONTOLOGY_SPEC.md 与 `service.json.ontology.entities[].name`                |
| 坐标一致性 | `prototype_semantics[].coordinates`（条目级 `alignment_matrix[].layer2_coordinate_mapping`）              | 三元组与 block.json `coordinates`、`service.json.ontology.entities[].coordinates` 一致      |
| 字段映射   | `db_bindings[].fields[]`（`column` ↔ `model_field`）                                                     | 业务名→column 映射与 ONTOLOGY_SPEC.md 一致                                                 |
| 标量引用   | `db_bindings[].fields[]` 中 `column` 前缀 `qk_` / `sk_` 的字段（`DTO_DESIGN_SPEC` §6/§8）                 | 在 DTO 设计中被标记为需解析                                                                 |
| 对齐置信度 | `alignment_matrix[].layer1_model_binding.confidence`（声明驱动产物无该字段，以可追溯 `provenance` 为准） | 低置信度字段需额外审查                                                                     |
| 语义分组   | `prototype_semantics[].intent_context`                                                                   | 推断 DTO 语义分组（对象形态取 `page_title` / `breadcrumb`；字符串形态直接使用）              |
| 覆盖闭合   | `coverage.closure`（原型驱动）或 `coverage.gap`（声明驱动）                                               | 前者须为 `"all_blocks_covered"`，后者须为 `0`，才能进入 Phase 2                              |

### 1-2.5: Service 命名空间校验

`service.json` **必须**包含 `namespace` 字段，格式 `^[A-Z][a-zA-Z0-9-]*$`。
当前保留值：`Alioth`、`Cosmic-Tools`。
跨层引用（如 `dtoDependencies` 引用的其他 Service）必须同 namespace。

### 1-2.6: service.json `consumers` 反向引用（缺口 10）

Service 被其他 Service 引用时，应在 `service.json` 中声明 `consumers` 字段，以便被引方变更时通知引用方：

```json
"consumers": [
  "{ns}/{service-id}",
  "Alioth/catalog"
]
```

**规则**：

- `dtoDependencies` 声明的是本 Service 依赖了谁（正向），`consumers` 声明的是谁依赖了本 Service（反向）
- consumers 在首次被引用时由引用者补充，非自动
- 被引 Service 变更语义字段时，应检查 consumers 列出的 Service 是否需要对应更新

### 1-3: 冲突标识规范

当发现冲突时，在 ONTOLOGY_SPEC.md 中以标准格式标识：

````markdown
> ⚠️ **CONFLICT v0.4.0** — 2026-06-17
>
> **冲突点**：Unit 实体的 Service 归属
>
> - **alioth-ontology 建议**: targetService = "catalog"（坐标三元组 scene/factor/function）
> - **现有 ONTOLOGY_SPEC.md**: 归属 measurement 模块
> - **影响**：若迁移到 catalog Service，measurement 模块的 Unit 引用需桥接
> - **建议**：保持 measurement 归属，在 catalog 中仅通过 DTO 引用
>
> **状态**：待人工审定

> ⚠️ **CONFLICT v0.4.0** — 2026-06-17
>
> **冲突点**：字段 `qk_price` 的 DTO 设计
>
> - **业务语义要求**: DTO 应返回 `pricing: { amount, currency }` 而非裸 `qk_price: i64`
> - **现有实现**: handler 直接返回 DB entity，`qk_price` 以标量 ID 形式暴露
> - **建议**：Service 层新增 `ProductService.to_detail()` 解析标量引用到业务语义
>
> **状态**：已按业务语义重构（见 §4）

### 1-4: ONTOLOGY_SPEC.md 补充模板

补充内容写为模型驱动叙事格式，紧跟现有章节结构：

```markdown
## §N. {章节标题 — 以模型推演为线索}

> **新增**: alioth-service v{版本} — {日期}

模型要求"{模型约束}"在此 namespace 中的体现：
```

{Service} 提供 {实体}，用于 {Block} 的 {业务场景}
└─ 字段：
├─ {字段名} — {业务说明}
└─ {字段名} — {业务说明}
└─ 关系: {概念级关系}

```

**DTO 端点**：

|端点|方法|说明|
|---|---|---|
|`/{resource}`|GET|列表|
|`/{resource}/:id`|GET|详情|
```

> 冲突标识格式不变（`⚠️ CONFLICT` 模板），但冲突正文使用业务字段名而非物理列名。
> 系统字段排除矩阵不再写入规约文档；列可写性速查表唯一正本为 `docs/specs/DTO_DESIGN_SPEC.md` §6。
> 这些契约信息已迁移至 MappingOutput JSON，下游工具从 `Pre-Proc/{ns}/local/ontology-output.json` 读取。

### 1-5: 交互式冲突裁决 Gate

<HARD-GATE>
每个 `⚠️ CONFLICT` 必须通过 `ask` 工具逐项裁决。禁止跳过任何冲突直接进入 Phase 2。
</HARD-GATE>

#### 裁决流程

对每个 CONFLICT，使用 `ask` 提出多选一问题：

```
问题：{冲突标题}。如何处理？
选项：
  A. {选项A — 含影响说明}
  B. {选项B — 含影响说明}
  C. 暂缓处理 — 标记为 TODO，先进入 Phase 2
```

**示例**：

```
问题：Unit 实体的 Service 归属冲突。alioth-ontology 建议 catalog Service，ONTOLOGY_SPEC.md 归属 measurement 模块。如何处理？
选项：
  A. 保留 measurement 归属 — 在 catalog 中仅通过 DTO 引用，不改动 measurement
  B. 迁移到 catalog Service — 更新 ONTOLOGY_SPEC.md 和 service.json，measurement 通过跨 Service 引用桥接（推荐）
  C. 拆分实体 — 核心定义留 measurement，展示/搜索功能放 catalog
  D. 暂缓处理 — 标记为 TODO，先完成其他实体的 API 开发
```

```
问题：qk_price 字段 DTO 设计冲突。业务语义要求返回 `pricing: { amount, currency }`，现有 handler 直接暴露标量 ID。如何处理？
选项：
  A. 新增 Service.to_detail() 解析标量引用 — 保持 handler 薄层（推荐）
  B. 在 handler 中直接解析 — 快速但违反分层约束
  C. 暂不解决 — 前端通过单独的 /scalars 端点自行解析
```

#### 裁决后 Gate

全部冲突裁决完成后，使用 `ask` 最终确认：

```
问题：ONTOLOGY_SPEC.md 修改是否可以进入 Phase 2（TDD 开发）？
选项：
  A. 批准 — 进入 TDD 开发
  B. 需调整（请在回复中说明具体修改）
```

用户确认后才进入 Phase 2。
---
````
