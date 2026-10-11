/**
 * ego-cli.ts — ego-browser CLI / API **代际适配**（单一实现，禁各处自拼 argv）
 *
 * 为什么存在：ego 有两条**互不等价**的线（2026-09-28 实测定型；正本 =
 * `.agents/skills/ego-browser/SKILL.md` §平台差异：Linux）：
 *
 *   · `nodejs` 代际（macOS app runtime / 社区 fork host）
 *       CLI：`ego-browser [--ego-server-name=<n>] nodejs`（脚本经 stdin）
 *       heredoc 预载：flat helpers `cliLog` / `js` / `gotoUrl` / `pageInfo` / `captureScreenshot` /
 *       `openOrReuseTab` / `snapshot` / `listTaskSpaces` / `useOrCreateTaskSpace` / `completeTaskSpace` …
 *   · `stdin` 代际（上游 ego-lite Linux polyfill；`EGO_LINUX=1`，安装于 `EGO_LITE_HOME`）
 *       CLI：只认 `--help` / `--doctor` / `--reload`；**任何其他 argv ⇒ USAGE + rc 2**
 *       heredoc 预载：**仅 facade** `{page, browser, taskSpaces, site, fetch, cdp, help}` + 全局 `ego`
 *       ⇒ flat helpers 全缺（必须由 `egoCompatPrelude()` 补齐）
 *
 * 消费方（MUST 经本模块）：`scripts/visual-verify.ts`、`scripts/ego-safe-eval.ts`、
 * `scripts/check/check-prototype-render.ts`、`scripts/check/check-e2e-app.mjs`、`scripts/e2e/ego-api.mjs`、
 * `scripts/ego/ego-space-watcher.ts`；shell 侧同判据实现 = `scripts/lib/ego-cli.sh`。
 *
 * 代际检测：`VV_EGO_CLI_STYLE=nodejs|stdin` 显式覆写 > `ego-browser --help` 文本含 `nodejs` ⇒ nodejs > 否则 stdin。
 */
import { spawnSync } from 'node:child_process';

export type EgoCliStyle = 'nodejs' | 'stdin';

let cachedStyle: EgoCliStyle | null = null;

/** 当前 CLI 代际（缓存；`VV_EGO_CLI_STYLE` 可覆写）。 */
export function egoCliStyle(egoBin = 'ego-browser'): EgoCliStyle {
  const override = (process.env.VV_EGO_CLI_STYLE ?? '').trim();
  if (override === 'nodejs' || override === 'stdin') return override;
  if (cachedStyle) return cachedStyle;
  let help = '';
  try {
    const r = spawnSync(egoBin, ['--help'], { encoding: 'utf-8', timeout: 15_000 });
    help = `${r.stdout ?? ''}${r.stderr ?? ''}`;
  } catch {
    help = '';
  }
  cachedStyle = help.includes('nodejs') ? 'nodejs' : 'stdin';
  return cachedStyle;
}

/**
 * ego 子进程 argv（`nodejs` 代际含子命令；`stdin` 代际为空数组）。
 *
 * `stdin` 代际**不支持** `--ego-server-name`（多实例隔离）：调用方传了也 MUST NOT 拼进 argv
 * （否则整条命令被 CLI 判为非法 ⇒ USAGE + rc 2），仅按需由调用方自行告警。
 */
export function egoSpawnArgs(serverName = '', egoBin = 'ego-browser'): string[] {
  if (egoCliStyle(egoBin) === 'stdin') return [];
  return [...(serverName ? [`--ego-server-name=${serverName}`] : []), 'nodejs'];
}

/**
 * facade → 传统 flat helper 的**运行期兼容前导**（浏览器侧纯 JS，供 heredoc 前置拼接）。
 *
 * 只在缺失处补（`typeof === 'undefined'` 守卫）⇒ macOS `nodejs` 代际行为**逐字节不变**；
 * 映射依据（逐项有源码证据，正本见 SKILL.md §平台差异：Linux / 各 `src/driver/*`）：
 *   cliLog→console.log（输出 sink）· js→page.evaluate · gotoUrl→page.goto（`wait:false` ⇒ `waitUntil:'commit'`）
 *   pageInfo→page.info（注意 `{dialog}` 形态）· captureScreenshot(path|opts)→page.screenshot
 *   openOrReuseTab→browser.openOrReuseTab · snapshot/snapshotRaw/elementCenter/drainEvents→page.* 同名
 *   listTaskSpaces/useOrCreateTaskSpace/switchTaskSpace/newTaskSpace/claimTaskSpace/completeTaskSpace/
 *   handOffTaskSpace/takeOverTaskSpace/waitForAgentControl→taskSpaces.*（`completeTaskSpace` 的 `{keep}` 必填 ⇒ 默认补 `keep:false`）
 */
export function egoCompatPrelude(): string {
  return `// ═══ ego legacy-helper compat（facade 代际：Linux 线；缺失才补）═══
if (typeof globalThis.cliLog === 'undefined') {
  const P = globalThis.page;
  const B = globalThis.browser;
  const T = globalThis.taskSpaces;
  globalThis.cliLog = (m) => { try { console.log(typeof m === 'string' ? m : JSON.stringify(m)); } catch { /* sink 失败不阻断 */ } };
  if (P) {
    if (typeof globalThis.js === 'undefined') globalThis.js = (expr, _arg) => P.evaluate(expr);
    if (typeof globalThis.gotoUrl === 'undefined') globalThis.gotoUrl = (url, opts = {}) => {
      const { wait, settle, timeout, waitUntil } = opts || {};
      return P.goto(url, { settle, timeout, waitUntil: waitUntil ?? (wait === false ? 'commit' : 'load') });
    };
    if (typeof globalThis.pageInfo === 'undefined') globalThis.pageInfo = () => P.info();
    if (typeof globalThis.captureScreenshot === 'undefined') globalThis.captureScreenshot = (a, b) => {
      const opts = typeof a === 'string' ? { ...(b || {}), path: a } : (a || {});
      return P.screenshot(opts);
    };
    if (typeof globalThis.snapshot === 'undefined') globalThis.snapshot = (o) => P.snapshot(o);
    if (typeof globalThis.snapshotRaw === 'undefined') globalThis.snapshotRaw = (o) => P.snapshotRaw(o);
    if (typeof globalThis.elementCenter === 'undefined') globalThis.elementCenter = (s) => P.elementCenter(s);
    if (typeof globalThis.drainEvents === 'undefined') globalThis.drainEvents = () => P.drainEvents();
  }
  if (B && typeof globalThis.openOrReuseTab === 'undefined') globalThis.openOrReuseTab = (url, opts) => B.openOrReuseTab(url, opts);
  if (T) {
    if (typeof globalThis.listTaskSpaces === 'undefined') globalThis.listTaskSpaces = () => T.list();
    if (typeof globalThis.useOrCreateTaskSpace === 'undefined') globalThis.useOrCreateTaskSpace = (n) => T.useOrCreate(n);
    if (typeof globalThis.switchTaskSpace === 'undefined') globalThis.switchTaskSpace = (x) => T.switch(x);
    if (typeof globalThis.newTaskSpace === 'undefined') globalThis.newTaskSpace = (n) => T.new(n);
    if (typeof globalThis.claimTaskSpace === 'undefined') globalThis.claimTaskSpace = (x) => T.claim(x);
    if (typeof globalThis.completeTaskSpace === 'undefined') globalThis.completeTaskSpace = (x, o) => T.complete(x, o ?? { keep: false });
    if (typeof globalThis.handOffTaskSpace === 'undefined') globalThis.handOffTaskSpace = (x) => T.handOff(x);
    if (typeof globalThis.takeOverTaskSpace === 'undefined') globalThis.takeOverTaskSpace = (x) => T.takeOver(x);
    if (typeof globalThis.waitForAgentControl === 'undefined') globalThis.waitForAgentControl = (x, o) => T.waitForAgentControl(x, o);
  }
}
// ═══ /compat ═══`;
}
