#!/usr/bin/env node
/**
 * preproc-layout.mjs — Pre-Proc 目录布局解析（Sources 全量镜像迁移，2026-09-01）。
 *
 * 布局契约（add-wz-external-open-activity D6）：
 * - Gateway 代码单元：Pre-Proc/{ns}/Sources/Apps/{Modules,Blocks,Services,...}
 * - OpenActivity 代码单元：Pre-Proc/{ns}/Sources/Open/{Modules,...}
 * - 解析优先 Sources/Apps，回退扁平 Sources（未迁移 namespace 不断链）。
 */
import { existsSync } from 'node:fs';
import path from 'node:path';

/** 解析 namespace 的 Sources 根（Gateway 侧）：Sources/Apps 存在则用，否则回退 Sources。 */
export function gatewaySourcesDir(projectRoot, ns) {
  return nsSourcesDir(path.join(projectRoot, 'Pre-Proc', ns));
}

/**
 * 由 namespace 目录（`.../Pre-Proc/{ns}`）解析 Sources 根——与 `gatewaySourcesDir`
 * 同一回退契约，供已持有 ns 目录（而非仓库根 + ns 名）的调用方复用。
 */
export function nsSourcesDir(nsDir) {
  const sources = path.join(nsDir, 'Sources');
  const apps = path.join(sources, 'Apps');
  return existsSync(apps) ? apps : sources;
}

/** 解析某类单元目录（Modules / Blocks / Services 等）。 */
export function gatewaySourcesKindDir(projectRoot, ns, kind) {
  return path.join(gatewaySourcesDir(projectRoot, ns), kind);
}

/** 由 namespace 目录解析某类单元目录（同一回退契约）。 */
export function nsSourcesKindDir(nsDir, kind) {
  return path.join(nsSourcesDir(nsDir), kind);
}
