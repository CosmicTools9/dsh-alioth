/**
 * ego-safe-preamble.ts — ego 运行时调用的有界（watchdog）前置块，**单源**。
 *
 * 为什么存在：裸调 `js()` / `cdp()` / `gotoUrl()` 在导航竞态下可**无限挂起并堵死共享运行时的
 * CDP 有序队列**，连带堵死同实例其他会话的调用（`.agents/skills/ego-browser/SKILL.md` 已记载该
 * 失败模式；2026-09-22 全仓视觉验证通道死锁即此机制）。故所有面向 ego 的脚本 MUST 经本前置块
 * 提供的 `safeCdp` / `safeJs` / `safeGoto` / `boundedStep` 调用。
 *
 * 消费方（单一实现，禁止复制）：
 *   - `scripts/ego-safe-eval.ts`（`--emit-preamble` 直接打印本块）
 *   - `scripts/visual-verify.ts`（capture / element / warmup / probe 脚本前置本块）
 *
 * 文本内出现的 `gotoUrl` / `js` / `cdp` / `cliLog` 均为 heredoc 运行时预载的 helper。
 */
export function egoSafePreamble(defaultJsTimeoutMs = 8000): string {
  return `// ═══ ego-safe preamble ═══
// withWatchdog: 有界等待原语——被包住的调用在 timeout 内未返回即失败，绝不无限挂起
async function withWatchdog(promise, timeoutMs, message) {
  const { promise: guard, reject } = Promise.withResolvers();
  const timer = setTimeout(() => reject(new Error(message)), timeoutMs);
  try {
    return await Promise.race([Promise.resolve(promise), guard]);
  } finally {
    clearTimeout(timer);
  }
}
// sleep: 前置块内共用的有界等待
async function sleepP(ms) {
  const { promise, resolve } = Promise.withResolvers();
  setTimeout(resolve, ms);
  return promise;
}
// safeCdp: 所有 CDP 调用统一 watchdog——单个卡死命令不堵死有序队列
async function safeCdp(method, params = {}, timeout = 8000) {
  return withWatchdog(cdp(method, params), timeout, 'cdp ' + method + ' watchdog timeout');
}
// safeJs: 包住 ego 的 js() helper，watchdog 兜底；失败不抛，返回带 __safeJsError 的对象
async function safeJs(expr, timeout = ${defaultJsTimeoutMs}) {
  try {
    return await withWatchdog(js(expr), timeout, 'js watchdog timeout');
  } catch (e) {
    return { __safeJsError: String(e) };
  }
}
// safeGoto: 包住 gotoUrl，watchdog 兜底；失败不抛，返回带 __safeGotoError 的对象
async function safeGoto(url, opts = { wait: true, settle: 300 }, timeout = 30000) {
  try {
    await withWatchdog(gotoUrl(url, opts), timeout, 'goto watchdog timeout');
    return { ok: true };
  } catch (e) {
    return { ok: false, __safeGotoError: String(e) };
  }
}
// boundedStep: 有界 step——失败信息含 label/超时/耗时，供调用方定位「哪一步挂」
async function boundedStep(label, fn, timeout = ${defaultJsTimeoutMs}) {
  const t0 = Date.now();
  try {
    const value = await withWatchdog(Promise.resolve().then(fn), timeout, 'step watchdog timeout');
    return { ok: true, label, value, elapsedMs: Date.now() - t0 };
  } catch (e) {
    return { ok: false, label, error: String(e), elapsedMs: Date.now() - t0 };
  }
}
// safePageInfo: 包住 pageInfo()，watchdog 兜底；失败不抛，返回带 __safePageInfoError 的对象
async function safePageInfo(timeout = 8000) {
  try {
    return await withWatchdog(pageInfo(), timeout, 'pageInfo watchdog timeout');
  } catch (e) {
    return { __safePageInfoError: String(e) };
  }
}
// safeShot: 包住 captureScreenshot(path)，watchdog 兜底；失败不抛，返回 null 并标记错误
async function safeShot(path, timeout = 30000) {
  try {
    return await withWatchdog(captureScreenshot(path), timeout, 'screenshot watchdog timeout');
  } catch (e) {
    return { __safeShotError: String(e) };
  }
}
// probeWithRetry: 布局探针有界重试；仍失败返回 null = **未测量**（调用方 MUST 按 degraded 处理）
async function probeWithRetry(expr, attempts = 3) {
  for (let i = 0; i < attempts; i++) {
    const v = await safeJs(expr);
    if (v && !v.__safeJsError) return v;
    if (i < attempts - 1) await sleepP(400);
  }
  return null;
}
// waitReady: 轮询 #root 渲染就绪（与 visual-verify.ts 同谓词），每次调用有界
async function waitReady(textCheck, { attempts = 240, interval = 250 } = {}) {
  for (let i = 0; i < attempts; i++) {
    const st = await safeJs(\`(() => { const r = document.getElementById('root'); const b = document.getElementById('boot-skeleton'); const bootShowing = !!(b && !(b.classList.contains('removed') || getComputedStyle(b).display === 'none')); return { ready: !!(r && r.children.length > 0 && !bootShowing), rootChildren: r ? r.children.length : 0, bootShowing, text: document.body ? document.body.innerText.slice(0, 1500) : '' } })()\`);
    if (st && st.__safeJsError) { await sleepP(interval); continue; }
    if (st && st.ready && (!textCheck || (st.text && st.text.includes(textCheck)))) {
      return { ready: true, value: st, attempts: i + 1 };
    }
    await sleepP(interval);
  }
  return { ready: false, value: null };
}
// ═══ /preamble ═══`;
}
