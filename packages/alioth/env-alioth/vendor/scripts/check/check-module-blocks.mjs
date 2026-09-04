#!/usr/bin/env bun
// check-module-blocks.mjs — 校验模块 blockAssembly 中每个 block id 都有对应骨架
//
// 用法: bun scripts/check/check-module-blocks.mjs <module.json 相对路径>
// 退出码: 0 = 全部 block 存在；1 = 缺失或结构非法
//
// 服务对象: AppAgent 技能执行 gate（alioth-module 1.4 步骤）——防止 LLM 跑偏
// 只写 module.json/module.tsx 而不创建 block 骨架时，1.5 构建必然失败。
// 结构化 JSON 解析（JSON.parse），不用正则。

import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const modulePath = process.argv[2];
if (!modulePath) {
  console.error('usage: bun check-module-blocks.mjs <module.json 路径>');
  process.exit(2);
}

let mod;
try {
  mod = JSON.parse(readFileSync(modulePath, 'utf8'));
} catch (e) {
  console.error(`无法解析 ${modulePath}: ${e.message}`);
  process.exit(1);
}

// 兼容两种 blockAssembly 格式：flat list [{id,title,brief}] 与 dict {blocks:[...]}
const blocks = Array.isArray(mod.blockAssembly)
  ? mod.blockAssembly
  : Array.isArray(mod.blockAssembly?.blocks)
    ? mod.blockAssembly.blocks
    : [];
if (blocks.length === 0) {
  console.error(`blockAssembly 为空或缺失（${modulePath}）`);
  process.exit(1);
}

// ns 权威 = 路径（Pre-Proc/{ns}/Sources/Modules/{name}/module.json），与
// check-namespace-frontend.ts 一致；字段可选，
// 存在则交叉校验（防复制/移动漂移）。路径段天然满足格式约束，无需再校验。
const seg = modulePath.split('/');
const preIdx = seg.indexOf('Pre-Proc');
const ns = preIdx >= 0 ? seg[preIdx + 1] : '';
if (!ns) {
  console.error(`无法从路径推导 namespace（${modulePath}）`);
  process.exit(1);
}
const fieldNs = mod.namespace ?? mod.ns ?? '';
if (fieldNs && fieldNs !== ns) {
  console.error(`module.json namespace 字段 '${fieldNs}' 与路径 '${ns}' 不一致（${modulePath}）`);
  process.exit(1);
}

const missing = [];
// block.json 必填字段（BLOCK_SCHEMA + check-version-alignment 对齐；namespace 路径权威、可选）
const REQUIRED_BLOCK_FIELDS = ['id', 'name', 'version', 'services'];
for (const b of blocks) {
  const id = b.id ?? b.blockId;
  if (!id) continue;
  const blockJson = join('Pre-Proc', ns, 'Sources', 'Blocks', id, 'block.json');
  const blockTsx = join('Pre-Proc', ns, 'Sources', 'Blocks', id, 'llm-tsx', 'block.tsx');
  if (!existsSync(blockJson) || !existsSync(blockTsx)) {
    missing.push(`${id}（缺 block.json 或 llm-tsx/block.tsx）`);
    continue;
  }
  let bj;
  try {
    bj = JSON.parse(readFileSync(blockJson, 'utf8'));
  } catch {
    missing.push(`${id}（block.json 非合法 JSON）`);
    continue;
  }
  for (const f of REQUIRED_BLOCK_FIELDS) {
    if (!(f in bj)) missing.push(`${id} block.json 缺必填字段 ${f}`);
  }
  // namespace 路径权威：字段可选，存在则必须与路径一致（防复制/移动漂移）
  const bjNs = bj.namespace ?? '';
  if (bjNs && bjNs !== ns) {
    missing.push(`${id} block.json namespace 字段 '${bjNs}' 与路径 '${ns}' 不一致`);
    continue;
  }
  if (bj && !Array.isArray(bj.services)) {
    missing.push(`${id} block.json services 须为数组`);
  }
}

if (missing.length > 0) {
  console.error(`block 骨架缺失: ${missing.join(', ')}`);
  process.exit(1);
}
console.log(`✓ ${blocks.length} 个 block 骨架齐全（${modulePath}）`);
process.exit(0);
