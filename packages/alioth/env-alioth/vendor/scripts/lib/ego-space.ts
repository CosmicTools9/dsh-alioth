/**
 * ego-space.ts — ego task space 创建语义的**唯一实现**
 *
 * 实测口径（2026-09-20，ego-browser 0.5.0.32 / chromium 152.0.7977.54）：
 *  1. v2 `taskSpace(name, { profileId })` 建出的空间，其 profile 的 cookie jar 与**用户自己的
 *     标签页、同 profile 的其他空间共享**（Default 空间可见 85 cookies / 含 localhost；
 *     `Profile 4「卓 张 (Edge)」` 523 cookies）⇒ 登录态复用走 profile，而非 CDP 注入 cookie。
 *  2. v1 `useOrCreateTaskSpace(name, { profileId })` **静默忽略 profileId**（空间落 Default）
 *     ⇒ 需要 profile 时必须走 v2 工厂。
 *  3. `taskSpace(name, { profileId })` 在空间**已存在**时抛
 *     `profileId only applies when creating a new task space` ⇒ 复用路径不得带 profileId。
 *  4. `taskSpace()` 拒绝未知选项（`visible` 报 `received unknown option`）⇒ v1 专有的
 *     `{ visible: false }` 只在无 profile 的 v1 路径保留（`egoCreateSpaceExpr` 的第三参）。
 *  5. `completeTaskSpace(task.id, { keep: false })` 对 v2 建出的空间仍返回 `{ done: true }`
 *     ⇒ 清理路径无需改写（而 v2 `task.finish({ keep: false })` 会抛错）。
 *
 * 用法（heredoc 生成侧）：
 *   const script = egoSpaceHeader(JSON.stringify(name), resolveEgoProfile(flag)) + body;
 *   // → `const task = await useOrCreateTaskSpace("<name>")\n`（无 profile，行为与既有完全一致）
 *   // → `const task = await taskSpaceWithProfile("<name>", "<profile>")\n`（含 prelude）
 */

/** 选择 ego profile 的环境变量（显式 `--profile` 优先）。 */
export const EGO_PROFILE_ENV = 'EGO_PROFILE_ID';

/** 解析 profile：显式参数 > `EGO_PROFILE_ID`；空白值视为未设置（继续回退下一来源）。 */
export function resolveEgoProfile(explicit?: string): string | undefined {
  const explicitValue = explicit?.trim();
  if (explicitValue) return explicitValue;
  const fromEnv = process.env[EGO_PROFILE_ENV]?.trim();
  return fromEnv ? fromEnv : undefined;
}

/**
 * 注入 heredoc 的创建 helper。依赖 ego runtime 的 `process`（heredoc 内可用）。
 * - 运行时无 v2 `taskSpace()` → 降级 v1 并在 stderr 告警（不静默忽略 profile）；
 * - 空间已存在 → 按名复用（profileId 只在创建时生效）。
 */
export const EGO_SPACE_PRELUDE = `async function taskSpaceWithProfile(name, profileId) {
  if (typeof taskSpace !== 'function') {
    process.stderr.write('ego-space: 运行时无 v2 taskSpace()；profileId=' + profileId + ' 被忽略\\n');
    return await useOrCreateTaskSpace(name);
  }
  try {
    return await taskSpace(name, { profileId });
  } catch (e) {
    if (/only applies when creating|already exists/i.test(String(e))) return await taskSpace(name);
    throw e;
  }
}
`;

/** 创建（或复用）task space 的 JS 表达式；prelude 见 {@link EGO_SPACE_PRELUDE}。
 * `v1OptionsExpr`：v1 flat helper 专有选项（如 `{ visible: false }`）——v2 `taskSpace()`
 * 拒绝未知选项，故**只在无 profile 的 v1 路径传入**（指定 profile 时该选项不生效）。 */
export function egoCreateSpaceExpr(
  nameExpr: string,
  profileId?: string,
  v1OptionsExpr?: string,
): string {
  const v1Args = v1OptionsExpr ? `, ${v1OptionsExpr}` : '';
  return profileId
    ? `taskSpaceWithProfile(${nameExpr}, ${JSON.stringify(profileId)})`
    : `useOrCreateTaskSpace(${nameExpr}${v1Args})`;
}

/** heredoc 头：`const task = await <expr>\n`（指定 profile 时前置 prelude）。 */
export function egoSpaceHeader(
  nameExpr: string,
  profileId?: string,
  v1OptionsExpr?: string,
): string {
  const expr = egoCreateSpaceExpr(nameExpr, profileId, v1OptionsExpr);
  return `${profileId ? EGO_SPACE_PRELUDE : ''}const task = await ${expr}\n`;
}

/** 列出可用 profile 的 heredoc JS（不需要 task space，故不经 egoSpaceHeader）。 */
export const EGO_PROFILES_SCRIPT = `(async () => {
  if (typeof profiles !== 'function') {
    cliLog('EGO_PROFILES:' + JSON.stringify({ v2: false, profiles: [] }));
    return;
  }
  const ps = await profiles();
  cliLog('EGO_PROFILES:' + JSON.stringify({
    v2: true,
    profiles: ps.map((p) => ({ id: p.id, name: p.name, isDefault: p.isDefault === true })),
  }));
})().catch((e) => cliLog('EGO_PROFILES:' + JSON.stringify({ error: String(e) })));
`;
