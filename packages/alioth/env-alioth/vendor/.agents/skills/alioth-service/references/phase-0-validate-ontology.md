## Phase 0: 校验 ontology 映射 + 读取规约

### 0-0: 前置检查

在进入任何 Service API 生成之前，**必须先确认 alioth-ontology 已完成该 namespace 的本体提取**：

```bash
# 1. 确认 MappingOutput JSON 存在
ls Pre-Proc/{ns}/local/ontology-output.json 2>/dev/null

# 2. 确认文件覆盖了 namespace 下所有 Block（提取 prototype_semantics[] 的 block_id，缺时回退 source 原型文件路径）

# 3. 如果文件不存在 → 先执行 alioth-ontology，禁止跳过
```

**通过标准**：

- `Pre-Proc/{ns}/local/ontology-output.json` 存在，且条目形状符合 `ALIOTH_ONTOLOGY_SPEC` §十 唯一定义（`bun scripts/ts/check-ontology-contract.ts --ns {ns}`；形状不符即失败，不整条跳过）
- 覆盖闭合（按生产者形态二选一）：
  - **原型驱动**产物（含 `coverage.closure`，如 Alioth / SE）：`coverage.closure = "all_blocks_covered"`
  - **声明驱动**产物（无 `closure`，如 WZ / AVIC-CAASEC，由 `scripts/ontology/aggregate-mapping-output.ts` 聚合）：`coverage.gap = 0`——每个 `status: gap` 条目代表未声明坐标或表的实体，MUST 先补齐或在 change 内显式记录
- `prototype_semantics[]` 覆盖该 namespace 下所有 Block：按 `block_id`（声明驱动产物）或 `source`（原型驱动产物，取原型文件所在 Block 目录名）与 `Pre-Proc/{ns}/Sources/Apps/Blocks/` 清单比对

> 如果文件不存在 — **必须调用 alioth-ontology 完成本体提取**，禁止直接进入 DTO 设计。
> 无原型上下文、仅需声明驱动产物时，可用 `bun scripts/ontology/aggregate-mapping-output.ts --ns {ns}` 从 `service.json` / `block.json` 声明聚合（坐标未声明的实体以 `status: gap` 产出，门禁会显式点名）。

### 0-0.5: 过期检测（缺口 12 门禁）

确认 MappingOutput 存在后，**必须检查是否因原型迭代而过时**：

```bash
# 对每个 Block 执行过期检测
target/debug/ontology-mapping ontology-stale --ns {ns} --scene {scene}
```

**通过标准**：

- 脚本退出码 `0`（新鲜）→ 继续进入 Phase 0-1
- 退出码 `1`（过时）→ **必须先重新运行 alioth-ontology**，禁止跳过

> 如果在 `alioth-build` pipeline 中执行，此步骤由 `factor_dev` 阶段的 `preflight-track.sh` 自动处理（详见 `alioth-design` SKILL.md）。

### 0-1: 读取 MappingOutput JSON

从 `Pre-Proc/{ns}/local/ontology-output.json` 中提取：

| 提取内容                              | 来源                                                                                                          | 用途                                                       |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| 所有实体清单                          | `db_bindings[].model_entity`                                                                                  | 确定 Service 需要覆盖哪些实体（与 `service.json.ontology.entities[].name` 交叉校验） |
| 每个实体的字段映射（业务名 → 物理列） | `db_bindings[].fields[]`（`column` + `model_field`，`writability` 可选）                                       | DTO 字段设计                                               |
| 本体坐标（scene/factor/function）     | `prototype_semantics[].coordinates`（Block 级，dk 静态绑定输入，`BACKEND_FRAMEWORK` §7.3.3）；条目级证据见 `alignment_matrix[].layer2_coordinate_mapping` | service.json `ontology.entities[].coordinates` + block.json `coordinates` |
| 标量引用字段                          | `db_bindings[].fields[]` 中 `column` 前缀为 `qk_` / `sk_` 的字段（`DTO_DESIGN_SPEC` §6/§8）                    | Service 层标量解析（写入结构化值对象；读取经 `_refs` / `zc_id_scal-*`） |
| 跨 Service 引用                       | `alignment_matrix[].layer1_model_binding.entity` + `db_bindings[].table` → `service.json.ontology.entities[].relationships` | DTO 依赖声明（`dtoDependencies` / `dtoExposes`）           |
| 原型意图上下文                        | `prototype_semantics[].intent_context`（对象含 `page_title`/`user_action`，或字符串）                          | 业务语义推断                                               |
| 对齐证据与置信度                      | `alignment_matrix[]`（`layer1_model_binding.{evidence,provenance,confidence}`；声明驱动条目以可追溯 `provenance` 为准） | 审计追溯                                                   |

> 条目形状 MUST 符合 `ALIOTH_ONTOLOGY_SPEC` §十 唯一定义——**不得**再引用旧字段名（`model_entity` 顶层 / 扁平 `coordinates` / `scalar_table` 等；`scalar_table` 是 Rust 侧 `SensitiveColumn{scalar_table}` 的字段，不是本 JSON 的键）。

**此 JSON 是 alioth-service 的唯一且必需机器输入。** `docs/specs/{ns}/ONTOLOGY_SPEC.md` 是叙事式规约，仅供人类阅读，不作为代码生成输入。
`service.json.ontology.entities`（`SERVICE_SPEC` §3：`name` / `table` / `inherits` / `coordinates` / `field_mappings` / `relationships`）是 **Service 单元的声明正本**——本技能写出的声明 MUST 与映射输出一致，一致性由 `bash scripts/check/check-version-alignment.sh` 交叉校验。

### 0-2: 输出业务语义声明

```json
{
  "scene": "unit-catalog",
  "apiRequirements": [
    {
      "entity": "Unit",
      "shapes": {
        "summary": {
          "fields": ["id", "name", "symbol", "dimension", "category"],
          "resolvedScalars": []
        },
        "detail": {
          "fields": ["id", "name", "symbol", "dimension", "conversionChain", "prefixInfo"],
          "resolvedScalars": [],
          "semanticGroups": {
            "conversion": ["conversionChain", "prefixInfo"],
            "metadata": ["id", "name", "symbol", "dimension"]
          }
        }
      },
      "operations": ["list", "get", "create", "update", "delete"]
    }
  ]
}
```

### 0-2.1: 从 MappingOutput 提取业务语义（§十 单一定义）

从 `alignment_matrix[]` 和 `db_bindings[]` 提取：

1. `db_bindings[].model_entity` + `db_bindings[].fields[]`（`column` ↔ `model_field`）→ DTO 字段设计（业务名 → 物理列）；`alignment_matrix[].layer1_model_binding.field` 仅原型驱动产物存在，用于确认字段语义
2. `prototype_semantics[].coordinates` → service.json 的 `ontology.entities[].coordinates` + block.json 的 `coordinates`（Block 级坐标；`BACKEND_FRAMEWORK` §7.3.3）
3. `db_bindings[].fields[].writability` → 列可写性（缺省时以 `DTO_DESIGN_SPEC` §6 速查表为准）
4. `db_bindings[].fields[]` 中 `column` 前缀为 `qk_` / `sk_` 的字段 → Service 层标量解析（写入结构化值对象，读取经 `_refs`）
5. `prototype_semantics[].intent_context` → 业务语义分组推断（对象形态取 `page_title` / `user_action`；字符串形态直接作语义描述）
6. `prototype_semantics[].expressions`（原型驱动产物）→ 原型表达 → 业务语义 shape；声明驱动产物用 `services` + `block_name`

**示例**：`db_bindings[].fields[] { column: "notice", model_field: "name" }` → `UnitDetail.name: String`；`alignment_matrix[].layer1_model_binding { entity: "MeasurementUnit", field: "notice", evidence/provenance, confidence }` 佐证该绑定。

**业务语义分组建议**：从 `prototype_semantics[].intent_context`（`page_title` / `breadcrumb` 若为对象形态）推断，如 `["系统设置", "单位制"]` → group `system_settings`；无 intent_context 时按领域（`service.json.ontology.entities[].table` + `services`）分组。

### 0-3: 交互式 DTO 设计 Gate

基于 `ONTOLOGY_SPEC.md` 中列出的实体和字段，使用 `ask` 工具确认 DTO 形状设计：

```
问题：{entity} 的 detail shape 语义分组方案？
选项：
  A. {方案A} — {说明，如：pricing/inventory/lifecycle}
  B. {方案B} — {说明，如：core/extended/computed}
  C. 扁平结构 — 不分组（推荐用于简单实体）
```

**示例**：

```
问题：Product 实体的 detail shape 语义分组方案？
选项：
  A. { pricing, inventory, lifecycle } — 按业务领域分组（推荐）
  B. { basic, financial, logistics } — 按数据来源分组
  C. 扁平结构 — 不分组（字段 < 10 个时不推荐过度设计）
D. 其他方案（请在回复中说明）
```

> **批量标量检查（缺口 9）**: DTO 设计中若有 `qk_amount` / `qk_price` / `qk_qty` 等标量引用字段，Service 层**必须使用 `resolve_many(&[...])` 批量查询**而非逐条解析。
> 在 Phase 0-3 ASK 中包含此检查项：「标量字段数量超过 3 个时，是否已计划批量解析方案？」
