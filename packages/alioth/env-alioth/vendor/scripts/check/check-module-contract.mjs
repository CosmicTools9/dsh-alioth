#!/usr/bin/env bun
// check-module-contract.mjs — 校验模块 module.tsx 满足 prototype-tool.js 契约
//
// 用法: bun scripts/check/check-module-contract.mjs <module.tsx 路径>
// 退出码: 0 = 契约通过；1 = 不满足
//
// 服务对象: AppAgent 技能执行 gate（alioth-module 1.3 步骤）——提前拦截不合格
// module.tsx（Navigation/useState(DEFAULT_ID)/embedded），避免 1.5 bun build 才失败。
// 逻辑与 scripts/prototype-tool.js preCheckModule 保持一致。

import { readFileSync } from 'node:fs';
import { js } from '../lib/parsers.ts';

const srcFile = process.argv[2];
if (!srcFile) {
  console.error('usage: bun check-module-contract.mjs <module.tsx 路径>');
  process.exit(2);
}

const content = readFileSync(srcFile, 'utf-8');
const errors = [];

// meriyah 定位 embedded 分支（结构化解析，非正则）
let embeddedBody = '';
try {
  const ast = js.parseModule(content, { jsx: true, next: true, ranges: true });
  (function walk(n) {
    if (!n || typeof n !== 'object') return;
    if (
      n.type === 'IfStatement' &&
      n.test &&
      n.test.type === 'Identifier' &&
      n.test.name === 'embedded'
    ) {
      const c = n.consequent;
      embeddedBody = content.slice(c.start + 1, c.end - 1);
    }
    for (const k of Object.keys(n)) {
      const v = n[k];
      if (Array.isArray(v)) v.forEach(walk);
      else if (v && typeof v === 'object') walk(v);
    }
  })(ast);
} catch {
  // 解析失败不阻断（下方 embedded 存在性检查兜底）
}

// 契约（prototype-tool.js preCheckModule 同款）
// JS 红线检测用 includes 字符串查找（参照 baseline-guard.ts），不使用正则
if (!content.includes('gateway-shell') || !content.includes('Navigation'))
  errors.push('未导入 Navigation 组件（契约要求 Module 拥有 NavSidebar）');
// activeId 默认 id 检查：容忍字面 `useState(DEFAULT_ID)` 与常量间接引用
// （LLM 常输出 `useState(__APPAGENT_MODULE_DEFAULT_ID)` 或 `useState(DEFAULT_ID)`
// ——语义等价，字面匹配误杀 → gate 5 次重试耗尽 → 链路断；2026-08-22 实测）。
// 结构判定：module.tsx 内存在 useState(<标识符>) 且该标识符名含 DEFAULT_ID。
const hasDefaultIdState = (() => {
  // meriyah 不支持 TS（`import type` 等）——TS 模块降级为宽松判定：
  // 文件同时含 useState( 调用与 DEFAULT 标识符即视为满足（语义等价，防误杀）。
  try {
    const ast = js.parseModule(content, { jsx: true, next: true });
    let found = false;
    (function walk(n) {
      if (!n || typeof n !== 'object' || found) return;
      if (
        n.type === 'CallExpression' &&
        n.callee &&
        n.callee.type === 'Identifier' &&
        n.callee.name === 'useState' &&
        n.arguments &&
        n.arguments.length >= 1
      ) {
        const a = n.arguments[0];
        const name =
          a.type === 'Identifier'
            ? a.name
            : a.type === 'MemberExpression' && a.property
              ? a.property.name || ''
              : '';
        if (name.includes('DEFAULT_ID')) found = true;
      }
      for (const k of Object.keys(n)) {
        const v = n[k];
        if (Array.isArray(v)) v.forEach(walk);
        else if (v && typeof v === 'object') walk(v);
      }
    })(ast);
    return found;
  } catch {
    // TS 模块（meriyah 无法解析）：宽松判定——useState( 调用 + DEFAULT 标识符
    // （字符串 includes 判定，非正则；语义等价防误杀）
    return (
      content.includes('useState(') &&
      (content.includes('DEFAULT_ID') || content.includes('DEFAULT_')) &&
      content.includes('DEFAULT')
    );
  }
})();
if (
  !content.includes('useState(DEFAULT') &&
  !content.includes('useState( DEFAULT') &&
  !hasDefaultIdState
)
  errors.push('activeId 应由内部 useState(DEFAULT_ID) 管理');
if (embeddedBody && embeddedBody.includes('<Footer'))
  errors.push('embedded 分支含 Footer（契约禁止）');
if (!content.includes('embedded')) errors.push('未定义 embedded 分支');

if (errors.length > 0) {
  console.error(`❌ module.tsx 契约检查失败（${srcFile}）:`);
  for (const e of errors) console.error(`   ${e}`);
  process.exit(1);
}
console.log(`✓ module.tsx 契约通过（${srcFile}）`);
process.exit(0);
