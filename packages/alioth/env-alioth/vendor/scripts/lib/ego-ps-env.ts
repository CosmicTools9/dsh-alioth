/**
 * ego-ps-env.ts — `ps` 指纹采集/复核共用的 locale 环境（单一来源）
 *
 * 背景（2026-10-05 实测）：lease 记录（`scripts/ego/lease.ts`）与 lease 复核
 * （`scripts/ego/ego-space-watcher.ts` 的 `ownerAlive`）都用 `ps -o lstart=` 取 owner
 * 启动时刻并做字符串比对。两侧若各自继承不同 locale——记录侧在会话环境
 * （`LANG=LC_ALL=zh_CN.UTF-8`）得到 `一 10月/ 5 01:59:15 2026`，复核侧在 LaunchAgent
 * 环境（无 `LANG`）得到 `Mon Oct  5 01:59:15 2026`——比对必假，watcher 会把**活跃**
 * 空间判为「owner 已退出」并回收（该回收路径不受客户端保护限制）。
 *
 * 判据：两侧 MUST 用同一份 env 调用 `ps`。常量单源于此，任何一方漂移都会让
 * `ego-space-watcher.test.ts` 的「记录 → 复核」不变量用例失败。
 */
export const PS_ENV: NodeJS.ProcessEnv = {
  ...process.env,
  LC_ALL: "C",
  LANG: "C",
  LC_TIME: "C",
};
