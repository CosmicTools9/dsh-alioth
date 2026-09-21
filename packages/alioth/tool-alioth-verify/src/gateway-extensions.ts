/**
 * Gateway 扩展声明形态的**运行时词汇表**——`alioth_verify extensions` 的 `allowedForms` 输入。
 * 常量派生自 vendored 真引擎加载器（MUST NOT 臆造形态名，形态名 = 文件族名）：
 *
 * - `packages/alioth/env-alioth/vendor/Framework/backend/runtime-engine/src/extension.rs:836-910`
 *   `ExtensionLoader::load_from_dir`：**只读固定文件名** `constraints.yaml` / `rules.yaml` /
 *   `statemachines.yaml` / `workflows.yaml` / `profiles.yaml`（逐文件 `exists()` 后读取）；前四个
 *   按顶层**数组**反序列化（`Vec<ConstraintExtension>` / `Vec<RuleExtension>` /
 *   `Vec<StateMachineExtension>` / `Vec<WorkflowDefinition>`），`profiles.yaml` 按
 *   `{ profiles: HashMap<String, AppModelConfig> }`（`ProfilesWrapper`）。
 * - `packages/alioth/env-alioth/vendor/Framework/backend/runtime-contract/src/extension.rs:53-77`
 *   `AppLogicExtension`：`constraints` / `business_rules` / `state_machines` / `workflows` 均为
 *   `Vec<T>`（`#[serde(default)]`），领域配置单为 `model_profiles: HashMap<String, AppModelConfig>`。
 * - **声明不被 loader 识别即致命**：`packages/alioth/env-alioth/vendor/Gateway/backend/src/main.rs:341-349`
 *   在 `load_from_dir` 返回 `Err` 时 `common::telemetry::error!` +
 *   `std::process::exit(1)`——Gateway 直接起不来（不是「忽略该文件」）。
 *
 * 因此「未覆盖」对三类输入 fail-closed（判定实现在 `verify-alioth/extension-verify.ts`）：
 * 1. `extensions/` 下存在加载器不认的**文件名**（永不加载）；
 * 2. 认可文件名但**顶层形状**不符（`Vec<T>` 期望数组 / `profiles.yaml` 期望对象包装）；
 * 3. 条目**缺反序列化必需键**：constraint/state-machine 需 `entity`，rule 需 `entity` + `name`。
 * 只要存在任一 uncovered，整体 `status='degraded'`——degraded ≠ passed。
 * @module @dsh-alioth/tool-alioth-verify/gateway-extensions
 */

/** 加载器认的五个形态（顺序 = `load_from_dir` 的读取顺序）。 */
export const GATEWAY_EXTENSION_FORMS: readonly string[] = [
  'constraints',
  'rules',
  'statemachines',
  'workflows',
  'profiles',
]

// 形态 ↔ 文件名 ↔ 顶层形状 ↔ 必需键的**唯一**表在 `@dsh-alioth/verify-alioth` 的
// `extension-verify.ts`（`FORM_SPECS`）——那里是判定实现所在地。本模块只持有
// `allowedForms` 的形态名清单，不复制第二份表。
