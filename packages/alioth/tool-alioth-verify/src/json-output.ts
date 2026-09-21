/**
 * 工具输出的 JSON 口——把 `verify-alioth` 的**只读库值**窄化到 harness 输出契约的
 * `{type: 'json'}` 推断类型。
 *
 * 为什么需要这一层：库以 `interface` + `readonly` 数组表达不可变结果（无索引签名），
 * 与 `JsonValue` 在类型上不同构，而在**运行时完全等价**（无损 JSON）。工具结果本身就是
 * 序列化边界，故此处只做一次性窄化，MUST NOT 深拷贝（避免无谓分配），也 MUST NOT 用
 * `any` 绕过类型检查。
 *
 * 目标类型由 `InferValue<{type:'json'}>` 派生（即 harness 输出 schema 里 `{type:'json'}`
 * 的推断值），因此本包不需要直接依赖 `@deepseek-ai/dsh-util-values`。
 * @module @dsh-alioth/tool-alioth-verify/json-output
 */

import type { InferValue } from '@deepseek-ai/dsh-tools'
import type { CapabilityValue } from '@dsh-alioth/verify-alioth'

/** harness 输出契约的 JSON 值类型（`{type:'json'}` 节点的推断值）。 */
export type JsonOutput = InferValue<{ type: 'json' }>

/**
 * 只读库值 → 输出 JSON 口：双断言是必要的（`T` 不约束到 JSON 形状，直接 `as` 会被判为
 * 无重叠），依据是「库值运行时即无损 JSON」这一契约。
 */
export function asJsonOutput<T>(value: T): JsonOutput {
  return value as unknown as JsonOutput
}

/**
 * 单组能力广告 → 输出 JSON 口：只窄化 `value`（`CapabilityValue` 的只读载荷），
 * `kind` / `reason` 原样透出，保证六组字段恒在场。
 */
export function capabilityGroupOutput<T>(
  group: CapabilityValue<T>,
): { kind: string; value?: JsonOutput; reason?: string } {
  return group.kind === 'value'
    ? { kind: group.kind, value: asJsonOutput(group.value) }
    : { kind: group.kind, reason: group.reason }
}
