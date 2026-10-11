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
| `block`            | `string`   | Block 业务编码。命名空间 per-NS（Alioth: `"FE"`/`"SCEN"`；AVIC-CAASEC: `"FE"`/`"RD"`；WZ: `{模块前缀}-{功能}` 如 `"OU-PMT"`、`"TR-TRACK"`）。判据位置：`scripts/check/check-block-json.ts`（缺键 R2；存在但非非空字符串 R11；存量债入 `scripts/check/baselines/block-json-baseline.json`）。 |
| `services`         | `string[]` | 依赖的 service ID 或 factor code 列表。解析方式依赖 namespace 约定。                                                                                  |
| `sharing`          | `object`   | 跨模块可见性声明。格式见 §2。                                                                                                                         |
| `coordinates`      | `object`   | 本体坐标 `{scene:{code,id}, factor:{code,id}, function:{code,id}}`。语义见 §3 与 `ALIOTH_ONTOLOGY_SPEC.md`；dk 静态绑定见 §1.4 与 `BACKEND_FRAMEWORK §7.3.3`。                                                         |
| `aliothVersion`    | `string`   | Alioth 元模型版本（semver），真相源 = `Meta/backend/alioth-gen/src/lib.rs` 的 `ALIOTH_MODEL_VERSION`。MUST 在全部 namespace 中一致存在；修复入口 `bun scripts/ontology/auto-fix-versions.ts --fix`（门禁：`scripts/check/check-block-json.ts`）。 |

### 1.2 OPTIONAL（block.schema.json properties 中有定义但非 required）

| 字段             | 类型     | 说明                                                                                                                                                                       |
| ---------------- | -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `namespace`      | `string` | 所属 namespace。**路径权威**：以 `Pre-Proc/{namespace}/Sources/Apps/Blocks/{id}/` 目录名为唯一真相源，字段可选；存在则必须与路径一致（交叉校验，防复制/移动漂移），缺失不报错。 |
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

**设计腿完整性（校验器 R10，2026-09-22 立；判据收紧同日）**：`Sources/Apps/Blocks/{id}/block.json` 存在 ⇒
`Prototypes/Blocks/{id}` MUST 有 `b-v{N}.html` 且构建源 `llm-tsx/block.tsx` 非桩。**桩判据取或**：
① `<200B`；② **语义**——默认导出组件体仅 `return null`（空渲染声明壳，自测负例 D）。②为必需：
文档注释可把声明壳推到 200B 之上（实测 233–247B）而逃逸，产物仍是空壳（视觉验证
`ready_viewports=0`）。存量违例落 `scripts/check/baselines/block-json-baseline.json` 并 MUST 在
`BLOCK_READY_UNWIRED_REGISTRY.md` §3.1 有镜像条目；清偿 = 落地真实设计腿（alioth-block Track 1）
或用户裁决删除实体。

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
- 脚手架 MUST 创建上述结构；不按本结构创建的块无法被 Gateway 挂载（门禁：`scripts/check/check-block-json.ts`）

## 7. Block 粒度与视图轴

> 判据来源：Cosmic-Tools 四 app 合并（92 块 → 域/操作粒度）实测。细目见 `openspec/changes/define-block-granularity-and-view-axis/`；跨 `ownerModule` 的同域收敛判据见 `openspec/changes/align-block-declaration-and-domain-merge/`。

### 7.1 粒度判据

块 = **业务域 × 操作闭环**（用户在一个域里连续做完一件事的范围）。MUST NOT 按 CRUD 页 / 子视图 / 状态页一一建块。

**可机械判定的合并判据**：若 A、B 两块 `ownerModule` 相同、`services[]` 集合相同、且差异仅落在筛选维度（列表/新建/编辑/详情，或周/月/状态/阶段视图）⇒ 二者是同一域块的两个视图轴，**MUST 合并**，旧块 MUST 从 `Sources/Apps/Blocks/` 与 `Prototypes/Blocks/` 同步删除（禁多粒度并存）。

**跨 `ownerModule` 的子视图级块**（`ownerModule` 相同仅是可机械判定的**充分**条件，非必要条件）：同一业务对象生命周期内的子视图级块（派生列表 / 发布基线 / 受控明细等），若 `services[]` 集合相同、且 `coordinates` 三元组相同（或为该域的派生视图坐标），**MUST 收敛**为该域主块（主块取承载主列表 / 工作台者），MUST NOT 以 `ownerModule` 不同为由保留为独立块。收敛时并入块的 `flows` MUST 并入主块、`consumers` MUST 取全部登记模块的并集（含 `ownerModule` 自身）；成员→主块映射与坐标留证 MUST 写入该 change 的 `design.md`（零模型中心写入）。

**报告面（只读报告 / 看板）独立块判据**——上两支判据的边界：若某块的 `coordinates.function` 是模型中心签发的**独立功能码**（读径报告码，与主块功能码不同轴，如 `↓_GG`（操作执行）对 `↓_GN`（报告内容）），且该块承载**只读报告面**（KPI / 汇总 / 趋势类聚合读径，非同一数据集的切片），则二者 MUST NOT 视为「同一域块的两个视图轴」，该报告面 **MUST 独立成块**（可达性由同模块 `blockAssembly.blocks[]` 承载，MUST NOT 以页内 `data-section` 分区或 `?view=` 代替）。块 id 命名（如 `view-` 前缀）MUST NOT 参与判定。实测对照：Cosmic-Tools `org` 的 `block-manage-region`（`WE/FRA/↓_GG`）与 `block-view-operations-dashboard`（`WE/FRA/↓_GN`）为两个独立块（change `extract-operations-dashboard-block`）。

### 7.2 视图轴唯一合法表达

| 表达 | 合法性 |
| --- | --- |
| 块内筛选 pill（**≤5 项**）/ filter-bar 控件 | ✅ MUST |
| `?view=` 路由参数、子 tab 栏（块内视图切换） | ❌ MUST NOT |
| 跳转到另一个块 | ❌ MUST NOT |

理由：块是 Gateway Navigator 的**可挂载单元**；视图轴若表达为路由或跨块跳转，`navigation.defaultBlock` 语义失效、块边界失真。pill 行 > 5 项在 tablet/mobile 宽度下溢出（视觉验证的窄屏维度）⇒ 是视图轴失控信号，应再合并或下沉为 filter-bar select。

### 7.3 合并块的坐标与 service 继承

- `coordinates`（§3）MUST 从**主视图成员**（该域的主列表/工作台视图）**原样复制** —— 坐标是设计腿既定事实，合并不得改写布局意图。
- 成员间坐标冲突时 MUST 以主视图成员为准，其余成员坐标仅作留证。
- `services[]` / `flows[]` MUST 取成员**并集**（去重）；`serviceBindings` MUST 按目标块聚合 `services` 与 `dtoTypes` 并集。

### 7.4 声明面唯一真相源

块的**可达性**声明唯一真相源 = 实现模块的 `blockAssembly.blocks[]`（`MODULE_SPEC.md`）。`ownerModule` MUST 在自身 `blockAssembly.blocks[]` 登记其块（门禁 `scripts/check/check-block-json.ts` 的 R9）。

`app.json#config.blocks[]` 是 **App 归属同步面**（`_schema/app.schema.json` 一等字段）：

| 情形 | 合法性 |
| --- | --- |
| 块的 `sharing.ownerModule` 所属模块已列入该 app 的 `config.modules[]` | ✅ **MUST 列全** —— `scripts/ns/sync-app-block-ownership.ts` 按 `sharing.ownerModule` 机械回填缺失项（只追加、幂等，`--check` 有差异即 rc=1）；门禁 `check-app-frontend-isolation` 以该字段计算「App 隔离特征」（未声明块 ⇒ 隔离特征不可计算） |
| 声明**独立块**（无任何 module 登记它） | ✅ MAY —— schema 允许的正当用法 |
| 声明在同 ns 无对应 `block.json` 的 id | ❌ MUST NOT —— `check-composition` **C3**（`dangling-app-block`）拦截，`sync-app-block-ownership.ts --check` 同判 |

⚠️ 口径纠正：`config.blocks[]` **不是**「独立块专用清单」。把「已由 `config.modules[]` 覆盖」视为冗余声明与工具链**正面矛盾** —— 同一 id 既被模块登记、又出现在 App 归属面，是**预期形态**；归属面 MUST 与 `sharing.ownerModule` 逐块对齐，机器判据 = `sync-app-block-ownership.ts <ns> --check` rc=0。

**合并过渡期**：模块已改指新块、旧块目录尚在时，R9 会短暂出现 —— 属**预期中间态**，删除面完成即 MUST 归零。**顺序约束**：先 build 新块原型（消 R10/T3）→ 再删旧目录（消 R9）；反序会使两类红灯同现、无法区分。
