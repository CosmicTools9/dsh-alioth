/**
 * preproc-layout.d.mts — `preproc-layout.mjs` 的 TypeScript 声明（类型面，非实现面）。
 *
 * 为什么需要：`preproc-layout.mjs` 是 Pre-Proc 布局解析的**唯一实现**（Sources/Apps 优先、
 * 回退扁平 Sources 的契约在此集中），全仓 18 个 TS 调用点经 `.mjs` 说明符导入；无声明文件时
 * 每个调用点报 TS7016（implicit any），令 `bunx tsc --noEmit` 对本模块的调用方恒红。
 * 本文件只补类型签名——实现仍在 `.mjs`（单一真相源不变）。
 */
export function gatewaySourcesDir(projectRoot: string, ns: string): string;
export function nsSourcesDir(nsDir: string): string;
export function gatewaySourcesKindDir(projectRoot: string, ns: string, kind: string): string;
export function nsSourcesKindDir(nsDir: string, kind: string): string;
