/**
 * type-guards.ts — scripts 包 canonical 运行时类型守卫（唯一正本，禁止在调用点重复定义）。
 */

/** 值是「非数组的 plain object」；字段仍为 unknown，需进一步 typeof/in 收窄后使用。 */
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
