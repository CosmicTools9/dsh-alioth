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
  const sources = path.join(projectRoot, 'Pre-Proc', ns, 'Sources');
  const apps = path.join(sources, 'Apps');
  return existsSync(apps) ? apps : sources;
}

/** 解析某类单元目录（Modules / Blocks / Services 等）。 */
export function gatewaySourcesKindDir(projectRoot, ns, kind) {
  return path.join(gatewaySourcesDir(projectRoot, ns), kind);
}
