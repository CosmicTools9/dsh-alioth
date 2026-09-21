# Block Schema 规约

> **版本**: v0.1.0 | **状态**: active | **Alioth 模型**: v10.0.0+
> **适用范围**: `Pre-Proc/{namespace}/Sources/Apps/Blocks/{id}/block.json`（Apps/Open 镜像，未迁移 namespace 暂为扁平 `Sources/`）
>
> **权威源**: `Pre-Proc/{namespace}/_schema/block.schema.json`（三 NS 各有独立副本，结构相同）
> **依赖**: `ALIOTH_ONTOLOGY_SPEC.md` (coordinates), `MODULE_SPEC.md` (module.json 消费方)

---

## 1. 字段总表

以下字段规格来自三 NS（Alioth / AVIC-CAASEC / WZ）共有的 `_schema/block.schema.json`。所有三份副本的 `properties` 和 `required` 集合一致。

### 1.1 REQUIRED（block.schema.json 强制）

| 字段               | 类型       | 说明                                                                                                                                                  |
| ------------------ | ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`               | `string`   | Block 标识符，kebab-case。同 namespace 内唯一。                                                                                                       |
| `name`             | `string`   | 展示名称。                                                                                                                                            |
| `version`          | `string`   | Block 定义版本，semver。                                                                                                                              |
| `prototypeVersion` | `string`   | **条件必填**（原 REQUIRED 已放宽，2026-09-12 裁决）。规范格式 `"v{N}"`（如 `"v1"`, `"v11"`），指向 `b-v{N}.html`。**同 ns `Prototypes/` 下该块目录存在原型痕迹（`b-v*.html` 或 `llm-tsx/`）时 MUST 存在**（门禁 R8）；无任何原型痕迹时 MAY 省略。推导口径见 §4.1。遗留格式 `"b-v{N}"` 仍有 164 个块（新建 MUST 用规范 `"v{N}"`）。 |
| `block`            | `string`   | Block 业务编码。命名空间 per-NS（Alioth: `"FE"`/`"SCEN"`；AVIC-CAASEC: `"FE"`/`"RD"`；WZ: `{模块前缀}-{功能}` 如 `"OU-PMT"`、`"TR-TRACK"`）。         |
| `services`         | `string[]` | 依赖的 service ID 或 factor code 列表。解析方式依赖 namespace 约定。                                                                                  |
| `sharing`          | `object`   | 跨模块可见性声明。格式见 §2。                                                                                                                         |
| `coordinates`      | `object`   | 本体坐标 `{scene:{code,id}, factor:{code,id}, function:{code,id}}`。语义见 §3 与 `ALIOTH_ONTOLOGY_SPEC.md`；dk 静态绑定见 §1.4 与 `BACKEND_FRAMEWORK §7.3.3`。                                                         |
| `aliothVersion`    | `string`   | Alioth 元模型版本（semver），真相源 = `Meta/backend/alioth-gen/src/lib.rs` 的 `ALIOTH_MODEL_VERSION`。MUST 在全部 namespace 中一致存在，MUST NOT 填技能版本或 Studio 版本；修复入口 `bun scripts/ontology/auto-fix-versions.ts --fix`。 |

### 1.2 OPTIONAL（block.schema.json properties 中有定义但非 required）

| 字段             | 类型     | 说明                                                                                                                                                                       |
| ---------------- | -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `namespace`      | `string` | 所属 namespace。**路径权威**：以 `Pre-Proc/{namespace}/Sources/Blocks/{id}/` 目录名为唯一真相源，字段可选；存在则必须与路径一致（交叉校验，防复制/移动漂移），缺失不报错。 |
| `flows`          | `array`  | 工作流定义数组。AVIC-CAASEC(30/30)、Alioth(9/14) 使用。WZ 不使用此字段（通过 Service 层处理流程）。                                                                        |
| `workbenchPosts` | `array`  | 工作台 post 定义。同上覆盖率模式。                                                                                                                                         |

### 1.3 NS-EXTENDED（schema 未列出，但存在实例）

| 字段               | 实例 NS / 覆盖率                       | 说明                                                                                                     |
| ------------------ | -------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `coordinates`      | 64/70 blocks 跨三 NS                   | 本体坐标 `{scene:{code}, factor:{code}, function:{code}}`。语义见 `ALIOTH_ONTOLOGY_SPEC.md`。格式见 §3。 |
| `icon` / `navIcon` | AVIC-CAASEC(24/30)                     | 内联 SVG 字符串（`<svg width="18"...>`），用于导航图标。非 lucide-react icon 名称。                      |
| `ontology`         | AVIC-CAASEC(3/30) airworthiness blocks | 完整本体映射 `{entities: [{name, table, coordinates}]}`。                                                |
| `notes`            | AVIC-CAASEC(3/30)                      | 附加说明文本。                                                                                           |
| `status`           | AVIC-CAASEC(3/30)                      | Block 生命周期状态。                                                                                     |
| `factors`          | Alioth(1/14)                           | 绑定特定 Factor。                                                                                        |
| `blockType`        | WZ(1/26)                               | Block 类型声明（如 `"scene"`）。                                                                         |
| `description`      | AVIC-CAASEC(3/30), WZ(1/26)            | 长文本描述。                                                                                             |

### 1.4 与 schema 的差异说明

`coordinates` 已入 `block.schema.json`（properties + required，三 NS 副本一致）。语义：`BACKEND_FRAMEWORK §7.3.3` dk 静态绑定——坐标**不是**前端请求数据，Service 实现静态绑定固定三元组；block.json 的 `coordinates` 是本体映射证据（人类可读），不作为运行时传值源。新增 Block MUST 包含此字段（schema required）。

---

## 2. sharing Schema

```json
{
  "sharing": {
    "mode": "single",
    "ownerModule": "Alioth/language",
    "consumers": []
  }
}
```

| 字段          | 类型       | 必填 | 说明                                        |
| ------------- | ---------- | ---- | ------------------------------------------- |
| `mode`        | `string`   | 是   | 共享模式：`"single"`（默认，仅属自身模块）/ `"shared"`（跨模块复用）。 |
| `ownerModule` | `string`   | 否   | 归属模块，格式 `"{namespace}/{moduleId}"`；MUST 指向存在的模块（校验器 R6）。骨架期可省略——自指值必然悬空。 |
| `consumers`   | `string[]` | 否   | 消费方模块列表，格式同 `ownerModule`，每项 MUST 指向存在模块（校验器 R7）。 |

**可复用单元（`mode: "shared"`）**：`consumers` MUST ≥ 2 且逐项可达——这是「同一 block 被多模块复用」的唯一声明机制（`BLOCK_SCHEMA` 之外无 `reusable`/`template` 字段）。
**归属一致性（校验器 R9）**：`ownerModule` 存在时，其指向模块的 `module.json#blockAssembly.blocks[]` MUST 登记本块 `id`——未登记即 Gateway Navigator 不可达（`module-block-assembly` 能力的 `block-json-must-register-in-module-json`；漂移修复按实现面改写 `ownerModule`，见 `BLOCK_READY_UNWIRED_REGISTRY.md`）。
**组装检索面**：`GET /blocks`（Meta）返回 `services[]`/`factors[]`/`sharing{mode,ownerModule,consumers}`/`status`/`hasBackend`，AppAgent 组装时按能力与复用声明检索既有单元，无需逐文件读取（`blocks.rs::BlockInfo`）。

---

## 3. coordinates 规格

```json
{
  "coordinates": {
    "scene": { "code": "YA" },
    "factor": { "code": "FJA" },
    "function": { "code": "↓_EV" }
  }
}
```

- `coordinates.*.id` = 对应维度行（`isahl.zc_id_scene` / `zc_id_factor` / `zc_id_function`，继承 `zc_ad_object`）的主键 **`uid`**，预处理阶段由 `code` 解析；**不是 `zuid`**（`zuid` 仅属 `zc_id_lifecycle` / `isahl_auth` / `isahl_audit` 系）
- **选组方法**：先读三者的 **`notice`**（语义名，如 `ZB`→组织架构、`↓_EH`→人事劳动）完成语义对位，再用 **`code`** 标记三元组；语义源 = `Framework/seed/seed-dimensions.sql`（解析后 MUST 与仓内既有 `code → uid` 交叉校验）
- 三维 code 构成 Alioth 本体坐标体系
- `scene.code` / `factor.code` / `function.code` 均为字符串，**无全局注册表**，各 NS 内部命名
- function.code 的 6 象限前缀语义见 `ALIOTH_ONTOLOGY_SPEC.md` §4.3.1
- 跨 NS 引用时：coordinates 仅在相同 namespace 内有语义约束

---

## 4. 版本语义

两个版本字段职责不同，互不替代：

| 字段               | 含义                                                            | 更新时机                                                             |
| ------------------ | --------------------------------------------------------------- | -------------------------------------------------------------------- |
| `version`          | Block **元数据版本**，semver。标识 block.json 结构变更。        | 添加/删除字段、修改 services 依赖、修改 sharing 时递增               |
| `prototypeVersion` | Block **原型迭代版本**。规范格式 `"v{N}"`，指向 `b-v{N}.html`。 | 每次更新 `llm-tsx/block.tsx` 并执行 `prototype-tool.js build` 后递增 |

**校验**：`check-version-alignment.sh` 检验 `version` 的 semver 合法性。`prototypeVersion` 由 `check-block-json.ts` 的 **R8** 判：同 ns `Prototypes/` 下该块目录有 `b-v*.html` 或 `llm-tsx/` 时必填，否则可省略。

**历史兼容**：部分 Block 使用旧前缀 `"b-v{N}"`（164 个块；等于 `prototypeVersion="b-v1"` 对应 `b-v1.html`）。新建 Block MUST 使用规范 `"v{N}"`。

### 4.1 prototypeVersion 推导口径（缺省时）

| 原型痕迹 | 取值 | 依据 |
| --- | --- | --- |
| `Prototypes/{Blocks\|Apps/Blocks\|Open/Blocks}/<id>/b-v{N}.html` | `"v{N}"`（取最大 N） | 构建产物即版本 |
| 仅有 `llm-tsx/`（原型源，未构建） | `"v1"` | 仓内既有约定（仅含 `llm-tsx` 的块均声明 `v1`） |
| 皆无 | 不填（MAY 省略） | 无据可推，须 owner 提供 |

实现：`scripts/lib/preproc-artifacts.ts::derivePrototypeVersion`（工具与门禁共用，单一事实源）。

---

## 5. 与 block.schema.json 的关系

- `_schema/block.schema.json` 是 block.json 的 JSON Schema 权威源，定义 `properties` 和 `required`
- 本规约补充 schema 外的字段分类、使用约定和跨 NS 差异说明
- 三 NS 的 schema 文件结构相同，目前为独立副本。如需统一，需在跨 NS 重构时合并
- **骨架态容忍**：脚手架（`create_block_scaffold`）产出的块骨架允许 `coordinates: null` 与 `block: ""`
  （坐标由本体映射阶段回填、业务编码由精化阶段回填）；骨架经精化后 MUST 满足 §1.1 REQUIRED 全部约束

---

## 6. Block 前端结构（与 Module 同构）

Block 的**前端产物**分两种形态，**至少具备其一**（判定依据是产物本身，不引入额外开关字段）：

| 形态 | 判定依据 | 用途 |
| --- | --- | --- |
| **SPA 块** | `Sources/Apps/Blocks/{id}/frontend/src/single-spa.tsx` 存在 | 可作为组件被 Module 引用（`@alioth/{id}-frontend`），由 Gateway `auto-routes` 解析 |
| **原型块** | `Prototypes/Blocks/{id}/llm-tsx/block.tsx` 存在（`prototype-tool.js build` → `b-v{N}.html`） | 仅用于原型设计与能力表达，不挂载前端 |

- SPA 块目录与 Module 前端同构：`frontend/{package.json,vite.config.ts,vitest.config.ts,tsconfig.json,index.html}` + `src/{main.tsx,single-spa.tsx,App.tsx,theme.css,index.css,pages/,components/,stores/,exports/,locales/}`（`MODULE_SPEC.md` §11.1 的口径，仅规模不同）
- `package.json` 名称 MUST 为 `@alioth/{id}-frontend`，exports MUST 暴露 `./single-spa`（与 Module 前端一致，见 `MODULE_SPEC.md` §11.1）
- **两者皆无** = 该块尚未产出可挂载/可预览产物 → 校验器报 Warning（`GET /blocks/{id}/validate` 的 `frontend_structure` 检查），骨架期不阻断，交付期 MUST 消除
- 脚手架 MUST 创建上述结构（MUST NOT 只写 `block.json`）；不按本结构创建的块无法被 Gateway 挂载
