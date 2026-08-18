#!/usr/bin/env bun
/**
 * audit-css-framework.mjs — CSS 框架合规审计
 *
 * 检查原型 HTML 是否符合 CSS 新架构：
 * - 必须引用 prototype-base.css（兼容 legacy tailwind-utilities.css）
 * - 禁止引用 gate-layout.css / meta-layout.css / design-tokens.css
 * - 禁止内联 gl-* / al-* 类规则
 * - 禁止 @layer
 *
 * 用法: bun scripts/check/audit-css-framework.mjs <path/to/prototype.html>
 * 退出码: 0 = 合规, 1 = 有错误
 */

import { readFileSync } from 'fs';
import { spawnSync } from 'child_process';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const PARSER = join(__dirname, '..', 'parser-utils.mjs');
const BANNED_LINKS = ['gate-layout.css', 'meta-layout.css', 'design-tokens.css'];

function runParser(cmd, file, arg) {
  const args = [PARSER, cmd, file];
  if (arg) args.push(arg);
  const r = spawnSync('bun', args, { encoding: 'utf-8', timeout: 15000, maxBuffer: 1024 * 1024 });
  if (r.status === 0 && r.stdout?.trim()) {
    try { return JSON.parse(r.stdout); } catch (_) {}
  }
  return null;
}

function audit(filePath) {
  const html = readFileSync(filePath, 'utf-8');
  let errors = 0;
  let warnings = 0;
  const out = [];

  // ── 1. <link> 审计（字符串 indexOf，非正则） ──
  const PREFIX = '<link rel="stylesheet" href="';
  let pos = 0;
  const links = [];
  while ((pos = html.indexOf(PREFIX, pos)) >= 0) {
    const start = pos + PREFIX.length;
    const end = html.indexOf('"', start);
    if (end > start) links.push(html.slice(start, end));
    pos = end + 1;
  }

  const hasBaseLink = links.some(h => h.endsWith('prototype-base.css'));
  const hasTailwindLink = links.some(h => h.endsWith('tailwind-utilities.css'));
  // Base CSS may be inlined by the ESM build pipeline (prototype-tool.js build).
  // Accept either a link tag or an inline <style> block that contains the marker.
  const hasBaseInline = html.includes('prototype-base.css');
  const hasTailwindInline = !hasBaseInline && html.includes('tailwind-utilities.css');
  const hasBase = hasBaseLink || hasBaseInline;
  const hasTailwind = hasTailwindLink || hasTailwindInline;
  const banned = links.filter(h => BANNED_LINKS.some(b => h.endsWith(b)));

  if (!hasBase && !hasTailwind) {
    out.push('  ❌ 缺少 prototype-base.css 引用（应为 <link> 或内联 <style>；legacy tailwind-utilities.css 兼容）');
    errors++;
  }
  if ((hasBaseLink && hasBaseInline) || (hasTailwindLink && hasTailwindInline)) {
    out.push('  ⚠️ 同时存在 <link> 和内联框架 CSS；保留其一即可');
    warnings++;
  }
  for (const b of banned) {
    out.push(`  ❌ <link> 引用了已废弃的 ${b}（仅应引用 prototype-base.css）`);
    errors++;
  }

  // ── 2. CSS 选择器审计（通过 parser-utils，用 css-tree AST） ──
  const gl = runParser('find-css-selectors', filePath, '.gl-');
  const al = runParser('find-css-selectors', filePath, '.al-');

  const glRules = gl?.results || [];
  const alRules = al?.results || [];

  if (glRules.length > 0) {
    const s = glRules.slice(0, 3).map(r => r.selector).join(', ');
    out.push(`  ❌ 包含废弃的 gl-* 规则 (${glRules.length} 条): ${s}`);
    errors++;
  }
  if (alRules.length > 0) {
    const s = alRules.slice(0, 3).map(r => r.selector).join(', ');
    out.push(`  ❌ 包含废弃的 al-* 规则 (${alRules.length} 条): ${s}`);
    errors++;
  }

  // ── 3. <style> 块审计（通过 parser-utils） ──
  const blocks = runParser('extract-styles', filePath);
  if (Array.isArray(blocks)) {
    for (let i = 0; i < blocks.length; i++) {
      const block = blocks[i];
      if (block.includes('@layer')) {
        out.push(`  ❌ 内联 <style> #${i} 包含 @layer（原型不应使用 @layer）`);
        errors++;
      }
      // Brace counting — simple char counting, not structural parsing
      let opens = 0, closes = 0;
      for (const ch of block) {
        if (ch === '{') opens++;
        else if (ch === '}') closes++;
      }
      if (opens !== closes) {
        out.push(`  ❌ 内联 <style> #${i} CSS 括号不平衡: { ${opens} } ${closes}`);
        errors++;
      }
    }
  }

  // ── 4. :root 变量审计（通过 parser-utils） ──
  const rootVars = runParser('find-root-vars', filePath);
  const hasRoot = Array.isArray(rootVars) && rootVars.length > 0;
  if (!hasRoot && !hasBase) {
    out.push('  ⚠️  未找到 :root CSS 变量定义（可能完全依赖框架变量）');
    warnings++;
  }

  // ── Summary ──
  const framework = hasBase ? 'prototype-base.css'
    : hasTailwind ? 'tailwind-utilities.css (legacy)'
    : banned.length > 0 ? `已废弃: ${banned.join(', ')}`
    : '无';
  console.log(out.join('\n'));
  console.log(`\n结果: ${errors} 错误 / ${warnings} 警告`);
  console.log(`框架引用: ${framework}`);
  return errors > 0 ? 1 : 0;
}

const filePath = process.argv[2];
if (!filePath) {
  console.error('用法: bun scripts/check/audit-css-framework.mjs <path/to/prototype.html>');
  process.exit(1);
}
process.exit(audit(filePath));
