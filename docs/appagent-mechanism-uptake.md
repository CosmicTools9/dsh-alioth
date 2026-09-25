# AppAgent 机制吸收：输入前置、人工裁决沉降、进度交接、flow-plan

本轮把上游 `Meta/backend/app-agent` 的四个机制缺口按顺序补上（③ → ② → ① → ④）。
口径：**语义照搬上游，落点服从本仓架构**——上游的「写记录」面在本仓可能只是「派生投影」，
上游的「DB 表」在本仓可能是「部署自有文件账本」；每处偏离都在下面写明理由。

## ③ 步骤输入缺失 = fail-fast（上游 `dialog_tools/run_skill.rs::precheck_step_inputs`）

- 判定：`skill-alioth/src/step-inputs.ts`（`precheckStepInputs` / `resolveInputTemplate` / `stepInputExists`）。
- 时机：`tool-alioth-workflow` 在**构造步骤载荷之前**判定——缺输入即抛错，步骤既未启动也未推进
  （上游测试断言 `mock.calls()==0` 的等价形态）。
- 判据顺序与上游一致：本步 `output_glob` 命中 ⇒ 跳过；模板解析；文件存在性；`*` ⇒ 任一展开命中即算；
  不可用模式 ⇒ 判缺失（fail-closed）；`Pre-Proc/` 前缀按读取方同规剥除。
- 失败契约：`repair.ts` 新增 `FailureKind = 'step-input-missing'` ↔ 上游 `StepInputMissing`，
  `class: not-fixable`（缺的是上游产物，本步无法自修），错误文本走 `[rule:step-input-missing]` 信封。

## ② 人工映射裁决的沉降与召回（上游 `dialog_tools/record_verdict.rs`）

- 工具：`alioth_mapping_verdict`（`tool-alioth-verify/src/verdict-tool.ts`），动作 `record`（默认）/ `recall`。
- 语义逐条照搬：`keep_gap=true` → **错误**（「保持缺口」不沉降，防 LLM 把拒绝当成功继续）；
  `table` 不在平台目录（`isahl_meta.meta_collections` ∪ 编译期生命周期实体）→ 拒绝（防 stale）；
  目录不可判定（注册表不可达 / `meta_collections` 为空）→ **fail-closed 拒绝**。
  最后一条有个必守细节：**MUST NOT** 拿编译期常量列表当「目录可用」的证据，否则 DB 不可达时该判据恒真、
  fail-closed 分支永不生效。
- 落地差异（有据）：上游写它自己的 `isahl_meta.agent_memory`；本仓不往注册表 schema 加表
  （基线是 load-once 契约，增表会让「结构恒用包内基线」不再成立），改为部署自有的文件账本
  `<dataRoot>/mapping-verdicts/{namespace}.json`（追加语义与上游 memory 同规：新裁决在后，召回最新在前；
  tmp + rename 原子替换）。
- 召回面：`recall` 返回该命名空间最新在前的裁决，可按 `domain`/`table` 收窄。上游由 memory 子系统在装配
  上下文时召回；本仓先给出显式读取面，**注入进提示词尚未做**。

## ① 进度投影与交付交接（上游 `pipeline/progress.rs`，change `fix-appagent-pipeline-handoff`）

- 投影：`verify-alioth/src/pipeline-progress.ts`（`scanStageProgress` / `pipelineProgressJson`）。
  三态诚实：**通过** → `completed`；**盘上零声明产物** → `pending`（缺失 ≠ 完成，MUST NOT 谎报）；
  **有产物但 gate 不过** → 收进 `failures`（调用方须阻断，不许静默放行）。
- 单一判定源：产物合规性只由 `evaluateStageGate` 判（7 阶段实质判据的唯一实现）；
  「是否被尝试过」由**声明产物模式**（上游 `stage_config.yaml` 的 `output_patterns` 同源）在盘上判存在性。
  两者必须分开：`evaluateStageGate` 的 `artifacts` 只收**通过**的产物，缺失与形态损坏都返回 `[]`，
  拿它判 pending 会把「产物在但坏了」谎报成「尚未开始」。
- 与驱动 run 的语义差异（刻意）：orchestrator 的 `pipelineAdvance` 把「产物缺失」判 GATE-FAIL——
  它驱动的是一次跑到完成的 run，中途缺失就是失败；本投影是**读侧**（状态面板 / 交付 manifest），
  缺失只能如实说 pending。
- 交接产物：`pipeline_manifest.json`（`Pre-Proc/{namespace}/Apps/{app}/`，原子 tmp + rename），
  只在**全部 publish 前置通过**后写出，失败即不写（上游错误文本同样承诺「未写出 pipeline_manifest」）。
  段形状与上游逐键同形：`pipeline_progress`（含 `current_stage` / `stages` / `stage_source` / `projection`）、
  `artifact_manifest.entries`（相对路径 + `sha256:` + media_type + stage_id）、本仓增量 `open_human_gates`。
  `projection.non_projected_stages` 记 `null` + 原因：本仓不读 `.pipeline/pipeline.yml`（那是部署编排的 12 stage 口径），
  不假装求过差。
- 读取面：`alioth_verify` 新增 `action: 'progress'`（只读投影；`failures` **如实报告**而不 throw——
  只读面必须能描述一棵坏树，阻断归 publish 前置与 `pipelineAdvance`）。

## ④ flow-plan 产物（上游 `composer.rs::compose_from_flow_plan` + `state.rs::FlowPlan`）

- 判定：此前计划只作管线**参数**存在——消费者拿不到「这份 app 按什么计划生成」。现补齐产物面：
  `Pre-Proc/{namespace}/Apps/{app}/flow-plan.json`（上游同路径），由 `appCreation` 原子写出。
- wire 释义：`skill-alioth/src/flow-plan-wire.ts`（`flowPlanToWire` / `flowPlanFromWire`）——
  写出 snake_case（上游 serde 面），读入 snake/camel 两种名面都收，旧别名 `created_scenes`/`created_factors`
  先归一**再**判必需键（顺序反了会把一份合法的旧形态判成 null）；缺必需键 → `null`（fail-closed 不猜默认值）。
  `ontology_model_json` 是 JSON **字符串**：只转键名，不下钻内容。
- 计划字段不再装饰：`CreateArgs` 收扩展规划面（`semanticConcepts`/`computations`/`constraints`/`businessRules`/
  `appMeta`/`coreConstraints`），`buildPlan` 只在调用方真的给了才写，`alioth_app_create` 工具面同步开放。
- 非致命：app 树的权威写入是 `alioth_app_write`（写不动时它自己会响）；计划面跟着它落，失败**如实记进 evidence**
  但不改变本阶段失败语义——否则一个只读根会让管线在「app 已写好」之后因附带产物而红，失败归因错位。
- **尚未做**：像上游 `compose_app` 那样用计划里的 computations/constraints/businessRules **确定性组装**
  `extensions/{constraints,rules}.yaml` 等产物——本仓目前的对应物是 gen-alioth + `alioth_app_configure` 的声明式入口。

## 验证

- 单测：`step-inputs.spec.ts`、`mapping-verdict.spec.ts`、`pipeline-progress.spec.ts`、`flow-plan-wire.spec.ts`；
  端到端：`publish.spec.ts`（manifest 成功写出 + 前置失败不写）、`orchestrator.spec.ts`（flow-plan.json 落盘与键名）。
- 全套 `pnpm run test` 51 files / 696 passed | 1 skipped；composed 冒烟 26 工具注册、doctor core 绿（一次性库 + fixture 源）。
- 门禁：lint（`--deny-warnings`）、strip-only、typecheck、tree-assembly、versions、vendor、dicts 全绿。
