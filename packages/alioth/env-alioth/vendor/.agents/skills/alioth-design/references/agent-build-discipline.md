# 构建纪律：能力清单优先 · 门禁修复契约 · 未覆盖登记 · 方案先行 · 蓝图复用

> 本文件是 **module / block / app / compose 四技能共享的构建纪律正文**（单点维护，各技能只留触发句 + 链接）。
> 适用者：OMP/人工链（读 SKILL.md）与 AppAgent 运行时链（读 `Meta/backend/app-agent/skill-adapters/*.yaml`）。

## 为什么有这五条

构建链路的失败集中在同一处：**AI 在生成时不知道可用面在哪（只能全量试探），失败后拿到的是原始终端输出（没有下一步），方案与落地混成一步（无法确认），改已有产物时靠逐行读代码（易漏），交付时无法区分「做完」与「做了一部分」**。
五条纪律分别封堵这五处，共同点是：把判断依据从「散文」换成「可被脚本与门禁共同消费的机器可读面」。

---

## 一、能力清单优先（生成前收窄选择面）

**清单是判据，不是描述。** 生成 Block / Module 的可复用面、Service 引用、枚举取值，MUST 来自清单命令的输出，MUST NOT 来自技能正文的表格或记忆。

```bash
# 模块作用域清单（可复用 Block + 可用 Service + 枚举面）
bun scripts/capability-catalog.ts {namespace} --module {module-id}

# Block 作用域清单（带出归属 module 与消费 module）
bun scripts/capability-catalog.ts {namespace} --block {block-id}

# 声明校验：引用 ⊆ 清单（结构化违规 + 非零退出）
bun scripts/capability-catalog.ts {namespace} --check \
  Pre-Proc/{namespace}/Sources/Apps/Modules/{module-id}/module.json
```

判据要点：

- `blocks[].id` / `services[].id` / `modules[].id` 是**唯一合法引用集**；清单里没有的 id 不要写进声明。
- `enums.*` 取自 `Pre-Proc/{namespace}/_schema/*.schema.json`——取值域以 schema 为准，不臆造新值。
- 命令**只读**：不写 `Sources/**`；清单权威形态是 stdout JSON。
- 收窄后再设计：取清单 → 写 Block Brief / blockAssembly → 用 `--check` 自检 → 才进入构建。

> 清单与门禁同源（同一 loader）：清单里没有的东西，`check-composition.ts` 也会判违规。
> 生成期用清单少犯错，提交期由门禁兜底——两者不会给出相反结论。

## 二、门禁失败 = 修复契约（不是终端输出）

门禁失败返回结构化契约，不是裸 stderr：

```
[rule:<ruleId>] class=<fixable|retryable|not-fixable> · <message> · 下一步：<suggestedAction> · 证据：<首段原文>
```

处理纪律：

| `class` | 含义 | 动作 |
| --- | --- | --- |
| `fixable` | 产出可改 | 按「下一步」改**源文件**后重跑该门禁；不得手改构建产物 |
| `retryable` | 环境性/超时 | 缩小该步产出范围或提高该 gate `timeout_sec` 后重试 |
| `not-fixable` | 不可修复 | 不消耗重试预算，直接改方案或 ASK |

要点：

- **先读 `下一步`，再读证据**——契约的设计目的就是给出下一步动作；跳过它直接重跑 = 浪费轮次。
- `ruleId` 稳定标识根因：同一 ruleId 反复出现 = 同一根因未解决，MUST 换手段（改方案 / 补前置产物），不得同构重试。
- 契约的 `证据` 只带首段，全文在 trace 中——需要时去 trace 取完整输出。

## 三、交付必须登记未覆盖项

交付说明 MUST 含「未覆盖」一节，逐条列 `未覆盖范围 + 手工动作`；全部覆盖时 MUST 显式写「无未覆盖项」。

```markdown
## 未覆盖
- `{module}/detail` 页面未接线（缺前端目录）：需先实现模块前端后，按 createModuleLayout 四件套接线
- `{block}` 的导入动作未含权限校验：需人工在 Gateway 侧确认角色绑定
```

禁止以省略表达部分完成——「已完成」与「做了一部分」必须在产物上机器可分。

## 四、方案先行：plan 步与 apply 步分工

「先出方案、确认后落地」在 AppAgent 链路上是**步骤相位**，不是措辞：

- `phase: plan` 的步骤只产出**方案产物**（其 `output_glob` 声明的文件），`write_file` / `patch_file`
  的写面被引擎收窄到该范围——**写产品配置（`module.json` / `block.json` / `*.tsx`）会被直接拒绝**；
- 后续 `apply` 步消费方案产物落地（`inputs` 强制引入，方案不是可选参考）。

纪律：

- 方案步越界写被拒时，读拒绝信息里的「下一步」——**把落地写移到 apply 步**，不要反复重试同一路径。
- 方案产物 MUST 是机器可校验的结构（如 `block-briefs.json`），不是散文；其引用（块 id、service id）
  MUST 落在能力清单内。
- 方案步不得声明为 Track 的末步（方案无消费者）；加载期即拒绝。

## 五、读现有原型再改：蓝图导出与接线漂移

改动已有 Module 前，先取它的**结构文档**，不要逐行读 JSX 猜：

```bash
# 导出结构（块接线面 / 导航组 / 模块 Tab / 布局信息）
bun scripts/prototype-blueprint.ts export Pre-Proc/{ns}/Prototypes/Modules/{m}/llm-tsx/module.tsx

# 判定「声明 ↔ 接线」漂移
bun scripts/prototype-blueprint.ts check Pre-Proc/{ns}/Prototypes/Modules/{m}/llm-tsx/module.tsx
```

- 结构来源是**源码**（`BLOCK_COMPS` / `NAV_GROUPS` / `MODULE_TABS` 等字面量），不是构建产物——
  产物是运行时 SPA 壳，静态 HTML 无结构可读。
- 规则码：`PB1` 接线未声明 / `PB2` 声明未接线（侧栏声明存在却恒空的成因） / `PB3` 导航与接线不一致。
- 输出含 `unsupported[]`（静态不可得项 + 手工动作）：**读到 unsupported 就去看源码对应处**，
  不要假设该面为空。
- 布局与 `embedded`/`Navigation` 契约面由既有门禁（`check-module-contract.mjs`、
  `prototype-tool.js preCheckModule`）判定——不要在此重复判定。

## 检查表（交付前逐项）

- [ ] 可复用面与 Service 引用取自清单命令输出（非记忆、非正文表格）
- [ ] `--check` 对本次涉及的 module.json / block.json 通过（零违规）
- [ ] 枚举取值来自 `_schema/*.schema.json`
- [ ] 每个门禁失败都读过契约的「下一步」；同 ruleId 未同构重试
- [ ] 交付说明含「未覆盖」一节（含「无未覆盖项」的情形）
- [ ] 未手改构建产物（产物一律由源重建）
- [ ] 方案步只写其声明的方案产物；落地写全部在 apply 步
- [ ] 改已有 Module 前跑过 `prototype-blueprint.ts check`，并处理 PB1–PB3（或记录为未覆盖）
