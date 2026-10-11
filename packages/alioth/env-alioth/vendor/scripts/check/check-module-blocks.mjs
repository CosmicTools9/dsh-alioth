#!/usr/bin/env bun
// check-module-blocks.mjs — 校验模块 blockAssembly 声明的每个 block 有对应设计腿骨架
//
// 用法: bun scripts/check/check-module-blocks.mjs <module.json 路径>
//       bun scripts/check/check-module-blocks.mjs --self-test
// 退出码: 0 = 全部 block 设计腿存在；1 = 缺失或结构非法；2 = 用法错误
//
// 判定面（retire-module-block-skeleton-convention 方案 A，2026-10-08 裁决）：
//   每个 module.json#blockAssembly.blocks[].id MUST 在 Prototypes 面存在
//   `Prototypes/Blocks/<id>/llm-tsx/block.tsx`——块骨架的设计腿真相源，也是
//   `prototype-tool.js build` 唯一读取的块源面。
//   Sources 腿（`Sources/Apps/Blocks/<id>/{block.json, llm-tsx/block.tsx}`）**不再判定**：
//   block.json 的存在性/形状由 check-block-json / check-composition 负责；旧约定要求人工维护
//   两份手写同源文件（Sources 腿实测 0 构建/运行时消费者），已随本裁决废止。
// 服务对象: AppAgent 技能执行 gate（alioth-module 1.4 步骤）——防止 LLM 跑偏
// （只写 module.json 而不落块设计腿时，1.5 构建必然失败）。
// 结构化 JSON 解析（JSON.parse），不用正则。

import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { tmpdir } from 'node:os';

const USAGE = 'usage: bun check-module-blocks.mjs <module.json 路径> | --self-test';

/** 判定核：module.json 路径 → { ok, message }（除读盘无副作用；自测直调）。 */
export function checkModuleBlocks(modulePath) {
  let mod;
  try {
    mod = JSON.parse(readFileSync(modulePath, 'utf8'));
  } catch (e) {
    return { ok: false, message: `无法解析 ${modulePath}: ${e.message}` };
  }

  // 兼容两种 blockAssembly 格式：flat list [{id,title,brief}] 与 dict {blocks:[...]}
  const blocks = Array.isArray(mod.blockAssembly)
    ? mod.blockAssembly
    : Array.isArray(mod.blockAssembly?.blocks)
      ? mod.blockAssembly.blocks
      : [];
  if (blocks.length === 0) {
    return { ok: false, message: `blockAssembly 为空或缺失（${modulePath}）` };
  }

  // ns 权威 = 路径（Pre-Proc/{ns}/Sources/...），与 check-namespace-frontend.ts 一致；
  // 字段可选，存在则交叉校验（防复制/移动漂移）。路径段天然满足格式约束，无需再校验。
  const seg = modulePath.split('/');
  const preIdx = seg.indexOf('Pre-Proc');
  const ns = preIdx >= 0 ? seg[preIdx + 1] : '';
  if (!ns) {
    return { ok: false, message: `无法从路径推导 namespace（${modulePath}）` };
  }
  const fieldNs = mod.namespace ?? mod.ns ?? '';
  if (fieldNs && fieldNs !== ns) {
    return { ok: false, message: `module.json namespace 字段 '${fieldNs}' 与路径 '${ns}' 不一致（${modulePath}）` };
  }

  // 设计腿根 = 同一 Pre-Proc 前缀下的 Prototypes/Blocks（相对/绝对 module.json 路径均可，不依赖 cwd）
  const designBlocksRoot = join(seg.slice(0, preIdx + 2).join('/'), 'Prototypes', 'Blocks');
  const missing = [];
  for (const b of blocks) {
    const id = b?.id ?? b?.blockId;
    if (!id) continue;
    if (!existsSync(join(designBlocksRoot, id, 'llm-tsx', 'block.tsx'))) {
      missing.push(`${id}（缺 Prototypes/Blocks/${id}/llm-tsx/block.tsx）`);
    }
  }
  if (missing.length > 0) {
    return { ok: false, message: `block 设计腿缺失: ${missing.join(', ')}` };
  }
  return { ok: true, message: `✓ ${blocks.length} 个 block 设计腿齐全（${modulePath}）` };
}

/** 判据自测（正例 + 负例；夹具在临时目录，不触仓库面）。 */
function selfTest() {
  const root = mkdtempSync(join(tmpdir(), 'module-blocks-selftest-'));
  const ns = 'ZZTEST';
  const preproc = join(root, 'Pre-Proc', ns);
  const put = (rel, content) => {
    const p = join(preproc, rel);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, content);
  };
  const moduleJson = (name, blockAssembly, extra = {}) =>
    put(
      join('Sources', 'Apps', 'Modules', name, 'module.json'),
      JSON.stringify({ namespace: ns, blockAssembly, ...extra }),
    );
  const designLeg = (id) =>
    put(join('Prototypes', 'Blocks', id, 'llm-tsx', 'block.tsx'), 'export default function Block() { return null; }\n');
  const sourcesLeg = (id) => {
    put(join('Sources', 'Apps', 'Blocks', id, 'block.json'), JSON.stringify({ id, name: id, version: '1.0.0', services: [] }));
    put(join('Sources', 'Apps', 'Blocks', id, 'llm-tsx', 'block.tsx'), 'export default function Block() { return null; }\n');
  };

  const cases = [];
  const run = (name, expectOk) => {
    const r = checkModuleBlocks(join(preproc, 'Sources', 'Apps', 'Modules', name, 'module.json'));
    cases.push({ name, expectOk, ok: r.ok, message: r.message });
  };

  // 正例：dict 形态，两块设计腿齐全
  moduleJson('ok-dict', { mode: 'multi-block', blocks: [{ id: 'mod-a' }, { id: 'mod-b' }] });
  designLeg('mod-a');
  designLeg('mod-b');
  run('ok-dict', true);

  // 正例：flat 数组形态（兼容既有）单块
  moduleJson('ok-flat', [{ id: 'mod-c' }]);
  designLeg('mod-c');
  run('ok-flat', true);

  // 负例（判定不得丢）：声明了块，Prototypes 面无 llm-tsx 且 Sources 面无 block.json ⇒ 仍报错
  moduleJson('neg-nowhere', { mode: 'multi-block', blocks: [{ id: 'ghost' }] });
  run('neg-nowhere', false);

  // 负例（判定面唯一化）：Sources 腿齐全也不能替代 Prototypes 设计腿
  moduleJson('neg-sources-only', { mode: 'multi-block', blocks: [{ id: 'src-only' }] });
  sourcesLeg('src-only');
  run('neg-sources-only', false);

  // 负例：blockAssembly 为空/缺失
  moduleJson('neg-empty', {});
  run('neg-empty', false);

  // 负例：namespace 字段与路径不一致
  moduleJson('neg-ns', { blocks: [{ id: 'mod-d' }] }, { namespace: 'OTHER' });
  designLeg('mod-d');
  run('neg-ns', false);

  rmSync(root, { recursive: true, force: true });

  let failed = 0;
  for (const c of cases) {
    const pass = c.ok === c.expectOk;
    if (!pass) failed += 1;
    console.log(`${pass ? '✔' : '✘'} ${c.name}${c.expectOk ? '' : '（负例 MUST rc=1）'}: rc=${c.ok ? 0 : 1} — ${c.message}`);
  }
  console.log(`\ncheck-module-blocks self-test: ${cases.length - failed}/${cases.length} passed`);
  return failed === 0 ? 0 : 1;
}

const argv = process.argv.slice(2);
if (argv.includes('--self-test')) {
  process.exit(selfTest());
}
const modulePath = argv[0];
if (!modulePath) {
  console.error(USAGE);
  process.exit(2);
}
const result = checkModuleBlocks(modulePath);
if (!result.ok) {
  console.error(result.message);
  process.exit(1);
}
console.log(result.message);
process.exit(0);
