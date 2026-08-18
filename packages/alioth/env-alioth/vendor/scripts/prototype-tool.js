#!/usr/bin/env bun
import { readFileSync, writeFileSync, existsSync, mkdirSync, renameSync, statSync } from 'fs';
import { join, resolve, dirname, relative, sep } from 'path';
import { argv, exit } from 'process';
import { createRequire } from 'module';
import { globSync } from 'fs';
import { execSync } from 'child_process';
const _require = createRequire(import.meta.url);

/* ───────────────────────────────────────────
   Parser-based extraction helpers
   Replace regex-based structural parsing with proper AST/parser tools.
   Requires: css-tree, cheerio, acorn (installed in pnpm workspace)
   ─────────────────────────────────────────── */

function postAuditHtml(filePath) {
  // Post-hoc CSS framework audit + prototype-reference evaluator for generated prototype HTML
  try {
    var r = execSync('bun scripts/check/audit-css-framework.mjs ' + JSON.stringify(filePath), {
      encoding: 'utf-8',
      timeout: 15000,
    });
    if (r.indexOf('错误:') >= 0 && r.indexOf('错误: 0') < 0) {
      console.warn('\n⚠️ CSS 框架审计未通过。请修复后重新构建。');
    }
  } catch (e) {
    // Non-blocking: guard-pre-delivery.py catches at session end
  }
  try {
    var er = execSync(
      'bun scripts/eval/evaluate-prototype-reference.ts ' + JSON.stringify(filePath),
      {
        encoding: 'utf-8',
        timeout: 30000,
      },
    );
    console.log('\n' + er);
  } catch (e) {
    // Evaluator output already printed; keep build non-blocking here
    console.warn(
      '\n⚠️ 静态评分未通过。运行 `bun scripts/prototype-tool.js eval ' + filePath + '` 查看详情。',
    );
  }
}

var acorn = _require('acorn');

/** Strip import-style React destructuring assignments.
 *  Removes: var { x, y } = React; / const { x, y } = React;
 *  Also removes: var h = React.createElement; / const SvgIcon = window.SvgIcon;
 */
function stripReactImport(code) {
  var ast;
  try {
    ast = acorn.parse(code, { ecmaVersion: 'latest', sourceType: 'module' });
  } catch (e) {
    return code;
  }
  var removes = [];
  ast.body.forEach(function (node) {
    if (node.type === 'VariableDeclaration' && node.declarations && node.declarations[0]) {
      var d = node.declarations[0];
      if (d.init && d.init.type === 'Identifier' && d.init.name === 'React') removes.push(node);
      if (
        d.init &&
        d.init.type === 'MemberExpression' &&
        d.init.object &&
        d.init.object.name === 'React'
      )
        removes.push(node);
      if (
        d.init &&
        d.init.type === 'MemberExpression' &&
        d.init.object &&
        d.init.object.name === 'window'
      )
        removes.push(node);
    }
  });
  removes.sort(function (a, b) {
    return b.start - a.start;
  });
  removes.forEach(function (n) {
    code = code.substring(0, n.start) + code.substring(n.end);
  });
  return code;
}

/** Convert h( call expressions to React.createElement(. */
function convertHCalls(code) {
  var ast;
  try {
    ast = acorn.parse(code, { ecmaVersion: 'latest', sourceType: 'module' });
  } catch (e) {
    return code;
  }
  var targets = [];
  (function walk(node) {
    if (!node || typeof node !== 'object') return;
    if (Array.isArray(node)) {
      node.forEach(walk);
      return;
    }
    if (
      node.type === 'CallExpression' &&
      node.callee &&
      node.callee.type === 'Identifier' &&
      node.callee.name === 'h'
    )
      targets.push(node.callee);
    for (var k in node) if (k !== 'parent') walk(node[k]);
  })(ast);
  if (!targets.length) return code;
  targets.sort(function (a, b) {
    return b.start - a.start;
  });
  targets.forEach(function (n) {
    code = code.substring(0, n.start) + 'React.createElement' + code.substring(n.end);
  });
  return code;
}

/** Normalize ICONS.Xxx -> window.ICONS.xxx (lowercase first letter). */
function normalizeICONS(code) {
  var ast;
  try {
    ast = acorn.parse(code, { ecmaVersion: 'latest', sourceType: 'module' });
  } catch (e) {
    return code;
  }
  var targets = [];
  (function walk(node) {
    if (!node || typeof node !== 'object') return;
    if (Array.isArray(node)) {
      node.forEach(walk);
      return;
    }
    if (
      node.type === 'MemberExpression' &&
      node.object &&
      node.object.type === 'Identifier' &&
      node.object.name === 'ICONS' &&
      node.property &&
      node.property.type === 'Identifier'
    )
      targets.push(node);
    for (var k in node) if (k !== 'parent') walk(node[k]);
  })(ast);
  if (!targets.length) return code;
  targets.sort(function (a, b) {
    return b.property.start - a.property.start;
  });
  targets.forEach(function (n) {
    var name = code.substring(n.property.start, n.property.end);
    var lower = name.charAt(0).toLowerCase() + name.slice(1);
    code =
      code.substring(0, n.object.start) + 'window.ICONS.' + lower + code.substring(n.property.end);
  });
  return code;
}

/** Strip T_ICONS block: const T_ICONS = {...}; ... window.ICONS = T_ICONS; */
function stripTIconBlocks(code) {
  var ast;
  try {
    ast = acorn.parse(code, { ecmaVersion: 'latest', sourceType: 'module' });
  } catch (e) {
    return code;
  }
  var removes = [];
  for (var i = 0; i < ast.body.length; i++) {
    var node = ast.body[i];
    if (
      node.type === 'VariableDeclaration' &&
      node.declarations[0] &&
      node.declarations[0].id &&
      node.declarations[0].id.type === 'Identifier' &&
      node.declarations[0].id.name === 'T_ICONS'
    )
      removes.push(node);
    if (
      node.type === 'ExpressionStatement' &&
      node.expression.type === 'AssignmentExpression' &&
      node.expression.left.type === 'MemberExpression' &&
      node.expression.left.object.type === 'Identifier' &&
      node.expression.left.object.name === 'window' &&
      node.expression.left.property.type === 'Identifier' &&
      node.expression.left.property.name === 'ICONS'
    )
      removes.push(node);
  }
  removes.sort(function (a, b) {
    return b.start - a.start;
  });
  removes.forEach(function (n) {
    code = code.substring(0, n.start) + code.substring(n.end);
  });
  return code;
}

/** Resolve t('key') calls to actual string values at build time.
 *  Uses acorn to find t() call expressions, looks up keys in i18nDict,
 *  and replaces them with string literals.
 *  @param {string} code - JS/TS source code with t() calls
 *  @param {Object} i18nDict - dict with key→value mapping (e.g. {'key.name': '中文'})
 *  @returns {string} code with t() calls resolved
 */
function resolveI18nCalls(code, i18nDict) {
  if (!i18nDict || Object.keys(i18nDict).length === 0) return code;
  var ast;
  try {
    ast = acorn.parse(code, { ecmaVersion: 'latest', sourceType: 'module' });
  } catch (e) {
    return code;
  }
  var targets = [];
  (function walk(node) {
    if (!node || typeof node !== 'object') return;
    if (Array.isArray(node)) {
      node.forEach(walk);
      return;
    }
    if (
      node.type === 'CallExpression' &&
      node.callee &&
      node.callee.type === 'Identifier' &&
      node.callee.name === 't' &&
      node.arguments &&
      node.arguments.length >= 1 &&
      node.arguments[0].type === 'Literal' &&
      typeof node.arguments[0].value === 'string'
    ) {
      var key = node.arguments[0].value;
      if (i18nDict[key] !== undefined) {
        // Replace the entire CallExpression with the resolved string
        targets.push({ start: node.start, end: node.end, value: i18nDict[key] });
      }
    }
    for (var k in node) if (k !== 'parent') walk(node[k]);
  })(ast);
  if (!targets.length) return code;
  // Sort descending by start position
  targets.sort(function (a, b) {
    return b.start - a.start;
  });
  targets.forEach(function (t) {
    // Escape special characters for JS string literal
    var escaped = t.value
      .replace(/\\/g, '\\\\')
      .replace(/'/g, "\\'")
      .replace(/\n/g, '\\n')
      .replace(/\r/g, '\\r');
    code = code.substring(0, t.start) + "'" + escaped + "'" + code.substring(t.end);
  });
  return code;
}

/** Fix dangerouslySetInnerHTML references from window.ICONS.xxx to window.ICONS_SVG.xxx.
 *  The icon pool exposes raw SVG path data in window.ICONS (for SvgIcon component)
 *  and full SVG markup in window.ICONS_SVG (for dangerouslySetInnerHTML).
 *  Scene code using window.ICONS inside dangerouslySetInnerHTML needs the full SVG.
 */
function fixSVGRefs(code) {
  var ast;
  try {
    ast = acorn.parse(code, { ecmaVersion: 'latest', sourceType: 'module' });
  } catch (e) {
    return code;
  }
  var targets = [];
  (function walk(node) {
    if (!node || typeof node !== 'object') return;
    if (Array.isArray(node)) {
      node.forEach(walk);
      return;
    }
    // Look for Property nodes where key is 'dangerouslySetInnerHTML'
    if (
      node.type === 'Property' &&
      node.key &&
      node.key.type === 'Identifier' &&
      node.key.name === 'dangerouslySetInnerHTML' &&
      node.value &&
      node.value.type === 'ObjectExpression'
    ) {
      node.value.properties.forEach(function (prop) {
        if (
          prop.key &&
          prop.key.type === 'Identifier' &&
          prop.key.name === '__html' &&
          prop.value &&
          prop.value.type === 'MemberExpression' &&
          prop.value.object &&
          prop.value.object.type === 'MemberExpression' &&
          prop.value.object.object &&
          prop.value.object.object.type === 'Identifier' &&
          prop.value.object.object.name === 'window' &&
          prop.value.object.property &&
          prop.value.object.property.type === 'Identifier' &&
          prop.value.object.property.name === 'ICONS'
        ) {
          targets.push(prop.value.object.property);
        }
      });
    }
    for (var k in node) if (k !== 'parent') walk(node[k]);
  })(ast);
  if (!targets.length) return code;
  targets.sort(function (a, b) {
    return b.start - a.start;
  });
  targets.forEach(function (n) {
    code = code.substring(0, n.start) + 'ICONS_SVG' + code.substring(n.end);
  });
  return code;
}

const ROOT = resolve(import.meta.dirname, '..');
const UTILITIES_JSON = join(ROOT, 'Framework/frontend/components/utilities.json');
const REFERENCES_DIR = join(ROOT, '.agents/skills/alioth-design/references');
function rootPathFor(htmlPath) {
  return relative(dirname(htmlPath), ROOT).split(sep).join('/') + '/';
}
function showHelp() {
  console.log('prototype-tool.js - AliothStudio \u539f\u578b\u6784\u5efa CLI\n');
  console.log('  bun scripts/prototype-tool.js <command> [options]\n');
  console.log('build-utility-css [files...]   \u6ce8\u5165 utility CSS');
  console.log(
    'build <file-path>               \u6784\u5efa ESM \u6e90\u6587\u4ef6\u4e3a b-/m-/a-v{N}.html',
  );
  console.log(
    'eval <html-path>               \u8fd0\u884c prototype-reference \u9759\u6001\u8bc4\u5206\u5668',
  );
  console.log(
    'check [dir]                     \u9a8c\u8bc1 ESM \u539f\u578b\u53ef\u6784\u5efa\u3001\u65e0\u5b64\u7acb\u5f15\u7528',
  );
  console.log('list-utilities                \u5217\u51fa utilities.json \u4e2d\u6240\u6709\u7c7b');
  console.log(
    'generate-mocks <shell>         \u68c0\u67e5 llm-tsx/mock.json (\u5df2\u5e9f\u5f03\u4ece\u539f\u578b\u53cd\u5411\u540c\u6b65\u5230\u524d\u7aef)',
  );
  console.log(
    'sync-services <ns> <name>       \u540c\u6b65 block.json services[] \u5230 module.json serviceBindings',
  );
  console.log(
    'prepare-block-distribution <ns> <name>  Phase 2.5: scaffold blocks + output subagent plan',
  );
  console.log(
    'collect-block-results <ns> <name>       Phase 2.5: verify blocks + integrate into module',
  );
  console.log(
    'migrate-vendor-paths [dir]     \u5c06\u539f\u578b\u4e2d Meta/Framework \u8d44\u6e90\u8def\u5f84\u8fc1\u79fb\u5230 references/',
  );
  console.log(
    'scaffold <app|module|scene> <ns> <code>  \u521b\u5efa\u6700\u5c0f\u76ee\u5f55\u7ed3\u6784\u4e0e JSON/TSX \u9aa8\u67b6',
  );
  console.log(
    'render-shell <block|module|app> [ns] [id] [--out <path>]  \u6e32\u67d3\u7a7a\u58f3\u9aa8\u67b6\u4e3a *-shell.html\uff08\u8bbe\u8ba1\u5ba1\u67e5\u7528\uff09',
  );
}
function genCSS(u) {
  var lines = ['  /* UTILITY CSS */'];
  for (var g = 0; g < u.groups.length; g++) {
    var group = u.groups[g];
    for (var name in group) {
      if (name === '$pseudo') continue;
      var props = group[name];
      var pseudo = props.$pseudo || '';
      var d = Object.entries(props)
        .filter(function (e) {
          return e[0] !== '$pseudo';
        })
        .map(function (e) {
          return e[0] + ': ' + e[1] + ';';
        })
        .join(' ');
      lines.push('  .' + name + pseudo + ' { ' + d + ' }');
    }
  }
  return lines.join('\n');
}
function injectCSS(fp, css) {
  let html = readFileSync(fp, 'utf-8');
  const ss = html.indexOf('<style>');
  const se = html.indexOf('</style>', ss);
  if (ss < 0 || se < 0) return false;
  const content = html.substring(ss + 7, se);
  if (content.indexOf('UTILITY CSS') >= 0) return false;
  html = html.substring(0, se) + '\n' + css + '\n' + html.substring(se);
  writeFileSync(fp, html, 'utf-8');
  return true;
}

function cmdBuildUtilityCSS(args) {
  const u = JSON.parse(readFileSync(UTILITIES_JSON, 'utf-8'));
  const css = genCSS(u);
  let targets =
    args.length > 0
      ? args
          .map(function (f) {
            return resolve(ROOT, f);
          })
          .filter(function (f) {
            return existsSync(f);
          })
      : globSync(join(ROOT, 'Pre-Proc/Alioth/Prototypes', '**/*.html'));
  let ok = 0,
    skip = 0;
  for (const f of targets) {
    if (injectCSS(f, css)) {
      console.log('  OK: ' + f.substring(ROOT.length));
      ok++;
    } else skip++;
  }
  console.log('Updated ' + ok + ', skipped ' + skip);
}

function cmdListUtilities() {
  const u = JSON.parse(readFileSync(UTILITIES_JSON, 'utf-8'));
  for (const g of Object.values(u.groups)) {
    for (const name of Object.keys(g)) {
      if (name === '$pseudo') continue;
      console.log('  .' + name);
    }
  }
}

function resolveBlockToModule(ns, blockId) {
  // block.json 的 sharing.ownerModule 已记录了所属 Module
  var sj = join(ROOT, 'Pre-Proc', ns, 'Sources', 'Blocks', blockId, 'block.json');
  if (!existsSync(sj)) return null;
  try {
    var sc = JSON.parse(readFileSync(sj, 'utf-8'));
    var owner = sc.sharing && sc.sharing.ownerModule;
    if (owner && typeof owner === 'string') {
      var parts = owner.split('/');
      if (parts.length === 2) return { ns: parts[0], moduleName: parts[1] };
    }
  } catch (e) {}
  return null;
}

function cmdGenerateMocks(args) {
  // DEPRECATED: generate-mocks no longer reverse-extracts mock data from
  // prototype HTML to frontend source. Mock data for prototypes must live in
  // llm-tsx/mock.json and is consumed via import; frontend fallback/mock data
  // is maintained separately in frontend/src/hooks/ or equivalent.
  if (args.length < 1) {
    console.error('Usage: generate-mocks <module-or-block-shell.html>');
    exit(1);
  }
  const shellPath = resolve(ROOT, args[0]);
  if (!existsSync(shellPath)) {
    console.error('Not found: ' + shellPath);
    exit(1);
  }
  // Determine prototype type and root
  let protoRoot = null;
  const blockMatch = shellPath.match(/Pre-Proc\/([^/]+)\/Prototypes\/Blocks\/([^/]+)\//);
  const moduleMatch = shellPath.match(/Pre-Proc\/([^/]+)\/Prototypes\/Modules\/([^/]+)\//);
  if (blockMatch) {
    protoRoot = join(ROOT, 'Pre-Proc', blockMatch[1], 'Prototypes', 'Blocks', blockMatch[2]);
  } else if (moduleMatch) {
    protoRoot = join(ROOT, 'Pre-Proc', moduleMatch[1], 'Prototypes', 'Modules', moduleMatch[2]);
  } else {
    console.error('Cannot parse prototype root from: ' + shellPath);
    exit(1);
  }
  const mockJsonPath = join(protoRoot, 'llm-tsx', 'mock.json');
  if (existsSync(mockJsonPath)) {
    try {
      JSON.parse(readFileSync(mockJsonPath, 'utf-8'));
      console.log('✓ llm-tsx/mock.json is valid JSON');
    } catch (e) {
      console.error('✗ llm-tsx/mock.json is invalid JSON: ' + e.message);
      exit(1);
    }
  } else if (blockMatch) {
    console.log('△ llm-tsx/mock.json not found (create one if this block needs mock data)');
  } else {
    console.log('△ Module prototypes consume mock data from their Blocks');
  }
  console.log(
    'NOTE: generate-mocks is deprecated; mock data must not be reverse-synced to frontend.',
  );
}

function cmdSyncServices(args) {
  const ns = args[0];
  const name = args[1];
  if (!ns || !name) {
    console.error('Usage: sync-factors <namespace> <module>');
    exit(1);
  }
  const modPath = join(ROOT, 'Pre-Proc', ns, 'Sources', 'Modules', name, 'module.json');
  if (!existsSync(modPath)) {
    console.error('module.json not found: ' + modPath);
    exit(1);
  }
  const mod = JSON.parse(readFileSync(modPath, 'utf-8'));
  const sa = mod.blockAssembly || {};
  const scenes = sa.blocks || [];
  if (!scenes.length) {
    console.log('No scenes to check');
    return;
  }

  var bindings = {};
  for (const sc of scenes) {
    const sjPath = join(ROOT, 'Pre-Proc', ns, 'Sources', 'Blocks', sc.id, 'block.json');
    if (!existsSync(sjPath)) {
      console.log('  Skipped (no block.json): ' + sc.id);
      continue;
    }
    const sj = JSON.parse(readFileSync(sjPath, 'utf-8'));
    const factors = sj.services || [];
    for (const f of factors) {
      if (!bindings[f]) bindings[f] = { blockIds: [] };
      if (bindings[f].blockIds.indexOf(sc.id) < 0) bindings[f].blockIds.push(sc.id);
    }
  }

  const existing = sa.serviceBindings || {};
  var added = 0,
    removed = 0,
    changed = 0;
  for (const [f, v] of Object.entries(bindings)) {
    if (!existing[f]) {
      added++;
      changed++;
      continue;
    }
    var oldIds = existing[f].blockIds || [];
    var newIds = v.blockIds;
    var diff = newIds.filter(function (id) {
      return oldIds.indexOf(id) < 0;
    }).length;
    var gone = oldIds.filter(function (id) {
      return newIds.indexOf(id) < 0;
    }).length;
    if (diff > 0 || gone > 0) changed++;
  }
  for (const f of Object.keys(existing)) {
    if (!bindings[f]) {
      removed++;
      changed++;
    }
  }

  if (changed === 0) {
    console.log('All ' + scenes.length + " scenes' services are already synced.");
    return;
  }

  mod.blockAssembly.serviceBindings = bindings;
  var tmpPath = modPath + '.tmp';
  writeFileSync(tmpPath, JSON.stringify(mod, null, 2) + '\n', 'utf-8');
  renameSync(tmpPath, modPath);
  console.log(
    'Updated serviceBindings: ' +
      added +
      ' added, ' +
      removed +
      ' removed, ' +
      changed +
      ' changed',
  );
  for (const [f, v] of Object.entries(bindings)) {
    console.log('  ' + f + ': [' + v.blockIds.join(', ') + ']');
  }
}
var ICON_POOL_CONTENT =
  '<script>\n' +
  readFileSync(join(ROOT, '.agents/skills/alioth-design/references/icon-pool.js'), 'utf-8') +
  '\n</script>';
var PROTOTYPE_BASE_CSS_CONTENT =
  '<style>\n' +
  readFileSync(
    join(ROOT, '.agents/skills/alioth-design/references/prototype-base.css'),
    'utf-8',
  ) +
  '\n</style>';

// _shared/lifecycle.ts 模板:scaffold 时写入每个 namespace 的 Prototypes/_shared/
// 契约对齐 Framework/frontend/api/src/micro-app.tsx createMicroAppLifecycle,
// 但去掉 react-query/jotai(原型轻量定位)。
var SHARED_LIFECYCLE_CONTENT = [
  '/**',
  ' * lifecycle.ts — 原型轻量 single-spa 生命周期工厂(由 prototype-tool scaffold 生成)。',
  ' * 契约对齐 Framework createMicroAppLifecycle,但去掉 react-query/jotai。',
  ' */',
  "import { createElement } from 'react';",
  "import { createRoot, type Root } from 'react-dom/client';",
  '',
  'export interface PrototypeProps {',
  '  domElement: HTMLElement;',
  '  domElementId: string;',
  '  name: string;',
  '  baseUrl?: string;',
  '  apiBaseUrl?: string;',
  '  embedded?: boolean;',
  '  navigate?: (path: string) => void;',
  '  [key: string]: unknown;',
  '}',
  'export interface PrototypeLifecycle {',
  '  bootstrap: () => Promise<void>;',
  '  mount: (props: PrototypeProps) => Promise<void>;',
  '  unmount: (props: PrototypeProps) => Promise<void>;',
  '}',
  'export interface CreatePrototypeOptions {',
  '  name: string;',
  '  App: React.ComponentType<Record<string, unknown>>;',
  '  renderApp?: (app: React.ReactElement, props: PrototypeProps) => React.ReactElement;',
  '}',
  '',
  'export function createPrototypeLifecycle(opts: CreatePrototypeOptions): PrototypeLifecycle {',
  '  const { name, App, renderApp } = opts;',
  '  let root: Root | null = null;',
  '  return {',
  '    async bootstrap() { console.log(`[${name}:lifecycle] bootstrap`); },',
  '    async mount(props: PrototypeProps) {',
  '      if (typeof window !== "undefined") {',
  '        (window as unknown as { __ALIOTH_APP_PROPS__?: PrototypeProps }).__ALIOTH_APP_PROPS__ = props;',
  '      }',
  '      console.log(`[${name}:lifecycle] mount`, props);',
  '      const container = props.domElement;',
  '      if (!container) throw new Error(`[${name}:lifecycle] No domElement provided`);',
  '      container.classList.add(`mod-${name}`);',
  '      if (!root) root = createRoot(container);',
  '      const appElement = createElement(App);',
  '      const wrapped = renderApp ? renderApp(appElement, props) : appElement;',
  '      root.render(wrapped);',
  '    },',
  '    async unmount() {',
  '      console.log(`[${name}:lifecycle] unmount`);',
  '      if (root) {',
  '        await new Promise<void>((resolve) => {',
  '          setTimeout(() => { if (root) { root.unmount(); root = null; } resolve(); }, 0);',
  '        });',
  '      }',
  '    },',
  '  };',
  '}',
  '',
].join('\n');

/** 确保 namespace 的 Prototypes/_shared/lifecycle.ts 存在(scaffold 时调用) */
function ensureSharedLifecycle(ns) {
  var sharedDir = join(ROOT, 'Pre-Proc', ns, 'Prototypes', '_shared');
  var lifecyclePath = join(sharedDir, 'lifecycle.ts');
  if (!existsSync(lifecyclePath)) {
    mkdirSync(sharedDir, { recursive: true });
    writeFileSync(lifecyclePath, SHARED_LIFECYCLE_CONTENT, 'utf-8');
    console.log('    Created _shared/lifecycle.ts');
  }
  return lifecyclePath;
}

/** scaffold stub:含 lifecycle 导出(双用途) */
function writeTsx(p, componentName, ns, logicalName, params) {
  mkdirSync(dirname(p), { recursive: true });
  ensureSharedLifecycle(ns);
  var displayLine = componentName + ".displayName = '" + logicalName + "';";
  var content = [
    "import { createPrototypeLifecycle } from '../../../_shared/lifecycle';",
    '',
    'export default function ' + componentName + '(' + (params || '') + ') {',
    '  return null;',
    '}',
    displayLine,
    '',
    'export const { bootstrap, mount, unmount } = createPrototypeLifecycle({',
    "  name: '" + logicalName + "',",
    '  App: ' + componentName + ',',
    '});',
    '',
  ].join('\n');
  writeFileSync(p, content, 'utf-8');
}

function sanitizeGlobalName(id) {
  return id.replace(/-/g, '_');
}

function computeNextVersion(dir, prefix) {
  var files = globSync(join(dir, prefix + '-v*.html'));
  var max = 0;
  files.forEach(function (f) {
    var r = f.match(new RegExp(prefix + '-v(\\d+)\\.html'));
    if (r) {
      var n = parseInt(r[1], 10);
      if (n > max) max = n;
    }
  });
  return max + 1;
}

function findModulesByBlock(ns, blockId) {
  var found = [];
  var modsDir = join(ROOT, 'Pre-Proc', ns, 'Sources', 'Modules');
  var files = globSync(join(modsDir, '*', 'module.json'));
  files.forEach(function (fp) {
    var mod = JSON.parse(readFileSync(fp, 'utf-8'));
    if (
      mod.blocks &&
      mod.blocks.some(function (s) {
        return s.id === blockId;
      })
    ) {
      found.push({ name: mod.id, json: mod });
    }
  });
  return found;
}

function findAppsByModule(ns, moduleName) {
  var found = [];
  var appsDir = join(ROOT, 'Pre-Proc', ns, 'Apps');
  if (!existsSync(appsDir)) return found;
  var files = globSync(join(appsDir, '*', 'app.json'));
  files.forEach(function (fp) {
    var app = JSON.parse(readFileSync(fp, 'utf-8'));
    if (app.config && app.config.modules && app.config.modules.indexOf(moduleName) >= 0) {
      found.push({ code: app.code });
    }
  });
  return found;
}

// ═══ Pre-build 契约合规检查 ═══════════════════════════════
// 在 esbuild 前先验证源文件遵守三层 ESM 集成契约。
// 违规直接退出，不构建脏产物。

function preCheckBlock(srcFile, blockId) {
  var content = readFileSync(srcFile, 'utf-8');
  var errors = [];
  if (/overflow-y-auto/.test(content))
    errors.push('Block root 含 overflow-y-auto（滚动视口应由 Module 提供）');
  if (!/\.displayName/.test(content))
    errors.push("缺少 displayName（契约要求 BlockXxx.displayName = '" + blockId + "'）");
  if (/export default function \w+\([^)]+\)/.test(content))
    errors.push('默认 export 含 props（契约要求零 props）');
  return errors;
}

function preCheckModule(srcFile) {
  var content = readFileSync(srcFile, 'utf-8');
  var errors = [];
  if (!/from.*gateway-shell/.test(content) || !/Navigation/.test(content))
    errors.push('未导入 Navigation 组件（契约要求 Module 拥有 NavSidebar）');
  if (!/useState\(.*DEFAULT/.test(content) && !/useState\(DEFAULT/.test(content))
    errors.push('activeId 应由内部 useState(DEFAULT_ID) 管理');
  if (/if \(embedded\)/.test(content)) {
    var m = content.match(/if\s*\(embedded\)\s*\{([\s\S]*?)\n\s*\}/);
    if (m && /<Footer/.test(m[1])) errors.push('embedded 分支含 Footer（契约禁止）');
  } else {
    errors.push('未定义 embedded 分支');
  }
  return errors;
}

function preCheckApp(srcFile) {
  var content = readFileSync(srcFile, 'utf-8');
  var errors = [];
  if (/\bnavGroups=/.test(content))
    errors.push('GatewayShell 调用传了 navGroups（契约要求 App 不代管导航）');
  if (
    !/hideNavigation/.test(content) ||
    !/hideFooter/.test(content) ||
    !/noContentScroll/.test(content)
  )
    errors.push('缺少 TopBar-only 模式标志（hideNavigation/hideFooter/noContentScroll）');
  if (!/embedded=\{?true/.test(content)) errors.push('ModuleLayout 未以 embedded 模式渲染');
  return errors;
}

function runPreChecks(srcFile, fileType, id, errors) {
  if (errors.length === 0) return;
  console.error('\n  ❌ 契约检查失败 — ' + id);
  errors.forEach(function (e) {
    console.error('     ' + e);
  });
  console.error('  请修复后重新构建\n');
  process.exit(1);
}
// ════════════════════════════════════════════════════════════

async function buildBlock(blockDir, blockId, ns, ver) {
  var srcFile = join(blockDir, 'llm-tsx', 'block.tsx');
  if (!existsSync(srcFile)) {
    console.error('  No block.tsx in ' + blockDir);
    return;
  }
  runPreChecks(srcFile, 'block', blockId, preCheckBlock(srcFile, blockId));

  var globalName = 'Block__' + sanitizeGlobalName(blockId);
  var bundleFile = 'b-v' + ver + '.bundle.js';
  var bundlePath = join(blockDir, bundleFile);
  var htmlFile = 'b-v' + ver + '.html';
  var htmlPath = join(blockDir, htmlFile);

  var cmd =
    'esbuild ' +
    JSON.stringify(srcFile) +
    ' --bundle --format=iife --global-name=' +
    globalName +
    ' --jsx=automatic --external:react --external:react-dom' +
    ' --outfile=' +
    JSON.stringify(bundlePath);
  console.log('  esbuild: ' + cmd.substring(0, 120) + '...');
  execSync(cmd, { stdio: 'pipe' });
  console.log('  Bundle: ' + bundleFile);

  var shellPath = join(ROOT, '.agents/skills/alioth-design/references/shells/block-shell.tsx');
  var shellMod = await import(shellPath);
  var html = shellMod.renderBlockShell({
    title: blockId + ' \u00b7 ' + ns,
    rootPath: rootPathFor(htmlPath),
    bundleJs: bundleFile,
    blockId: blockId,
    globalName: globalName,
    name: blockId,
    tokensCss: PROTOTYPE_BASE_CSS_CONTENT,
    iconPool: ICON_POOL_CONTENT,
  });
  var gitSha = '';
  try {
    gitSha = execSync('git rev-parse --short HEAD', { encoding: 'utf-8' }).trim();
  } catch (e) {}
  var vc = '<!-- Block b-v' + ver + ' | Source: git-sha-' + gitSha + ' -->';
  html = vc + '\n' + html;
  // Final HTML emitted; postAuditHtml then runs CSS + prototype-reference evaluator on this file
  writeFileSync(htmlPath, html, 'utf-8');
  console.log('  HTML: ' + htmlFile);
  postAuditHtml(htmlPath);
  // Auto-sync prototype to Sources/ directory for Vite middleware serving
  try {
    var syncScript = join(ROOT, 'scripts', 'sync-prototype.sh');
    if (existsSync(syncScript)) {
      execSync('bash ' + JSON.stringify(syncScript) + ' ' + JSON.stringify(htmlPath), { stdio: 'pipe' });
      console.log('  ✓ Synced to Sources/');
    }
  } catch (e) {
    console.warn('  ⚠ Auto-sync failed: ' + (e.message || e));
  }
}

async function buildModule(moduleDir, moduleName, ns, sceneRefs) {
  var srcFile = join(moduleDir, 'llm-tsx', 'module.tsx');
  if (!existsSync(srcFile)) {
    console.error('  No module.tsx in ' + moduleDir);
    return;
  }
  runPreChecks(srcFile, 'module', moduleName, preCheckModule(srcFile));

  var globalName = 'Module__' + sanitizeGlobalName(moduleName);
  var ver = computeNextVersion(moduleDir, 'm');
  var bundleFile = 'm-v' + ver + '.bundle.js';
  var bundlePath = join(moduleDir, bundleFile);
  var htmlPath = join(moduleDir, 'm-v' + ver + '.html');

  var cmd =
    'esbuild ' +
    JSON.stringify(srcFile) +
    ' --bundle --format=iife --global-name=' +
    globalName +
    ' --jsx=automatic --external:react --external:react-dom' +
    ' --outfile=' +
    JSON.stringify(bundlePath);
  console.log('  esbuild: ' + cmd.substring(0, 120) + '...');
  execSync(cmd, { stdio: 'pipe' });

  var shellPath = join(ROOT, '.agents/skills/alioth-design/references/shells/module-shell.tsx');
  var shellMod = await import(shellPath);
  var html = shellMod.renderModuleShell({
    title: moduleName + ' \u00b7 ' + ns,
    rootPath: rootPathFor(htmlPath),
    bundleJs: bundleFile,
    bodyClass: moduleName,
    globalName: globalName,
    name: moduleName,
    tokensCss: PROTOTYPE_BASE_CSS_CONTENT,
    iconPool: ICON_POOL_CONTENT,
  });
  var gitSha = '';
  try {
    gitSha = execSync('git rev-parse --short HEAD', { encoding: 'utf-8' }).trim();
  } catch (e) {}
  var vc =
    '<!-- Module m-v' +
    ver +
    ' | scenes: ' +
    (sceneRefs || '?') +
    ' | Source: git-sha-' +
    gitSha +
    ' -->';
  html = vc + '\n' + html;
  // Final HTML emitted; postAuditHtml then runs CSS + prototype-reference evaluator on this file
  writeFileSync(htmlPath, html, 'utf-8');
  console.log('  Module: m-v' + ver + '.html (' + bundleFile + ')');
  postAuditHtml(htmlPath);
  // Auto-sync prototype to Sources/ directory for Vite middleware serving
  try {
    var syncScript = join(ROOT, 'scripts', 'sync-prototype.sh');
    if (existsSync(syncScript)) {
      execSync('bash ' + JSON.stringify(syncScript) + ' ' + JSON.stringify(htmlPath), { stdio: 'pipe' });
      console.log('  ✓ Synced to Sources/');
    }
  } catch (e) {
    console.warn('  ⚠ Auto-sync failed: ' + (e.message || e));
  }
}

async function buildApp(appDir, appCode, ns, moduleRefs) {
  var srcFile = join(appDir, 'llm-tsx', 'app.tsx');
  if (!existsSync(srcFile)) {
    console.error('  No app.tsx in ' + appDir);
    return;
  }
  runPreChecks(srcFile, 'app', appCode, preCheckApp(srcFile));
  var globalName = 'App__' + sanitizeGlobalName(appCode);
  var protoDir = join(ROOT, 'Pre-Proc', ns, 'Prototypes', 'Apps', appCode);
  mkdirSync(protoDir, { recursive: true });
  var ver = computeNextVersion(protoDir, 'a');
  var bundleFile = 'a-v' + ver + '.bundle.js';
  var bundlePath = join(protoDir, bundleFile);
  var htmlPath = join(protoDir, 'a-v' + ver + '.html');

  var cmd =
    'esbuild ' +
    JSON.stringify(srcFile) +
    ' --bundle --format=iife --global-name=' +
    globalName +
    ' --jsx=automatic --external:react --external:react-dom' +
    ' --outfile=' +
    JSON.stringify(bundlePath);
  console.log('  esbuild: ' + cmd.substring(0, 120) + '...');
  execSync(cmd, { stdio: 'pipe' });

  var shellPath = join(ROOT, '.agents/skills/alioth-design/references/shells/app-shell.tsx');
  var shellMod = await import(shellPath);
  var html = shellMod.renderAppShell({
    title: appCode + ' \u00b7 ' + ns,
    rootPath: rootPathFor(htmlPath),
    bundleJs: bundleFile,
    globalName: globalName,
    name: appCode,
    tokensCss: PROTOTYPE_BASE_CSS_CONTENT,
    iconPool: ICON_POOL_CONTENT,
  });
  var gitSha = '';
  try {
    gitSha = execSync('git rev-parse --short HEAD', { encoding: 'utf-8' }).trim();
  } catch (e) {}
  var vc =
    '<!-- App a-v' +
    ver +
    ' | modules: ' +
    (moduleRefs || '?') +
    ' | Source: git-sha-' +
    gitSha +
    ' -->';
  html = vc + '\n' + html;
  // Final HTML emitted; postAuditHtml then runs CSS + prototype-reference evaluator on this file
  writeFileSync(htmlPath, html, 'utf-8');
  console.log('  App: a-v' + ver + '.html (' + bundleFile + ')');
  postAuditHtml(htmlPath);

  // Auto-sync prototype to Sources/ directory for Vite middleware serving
  try {
    var syncScript = join(ROOT, 'scripts', 'sync-prototype.sh');
    if (existsSync(syncScript)) {
      execSync('bash ' + JSON.stringify(syncScript) + ' ' + JSON.stringify(htmlPath), { stdio: 'pipe' });
      console.log('  ✓ Synced to Sources/');
    }
  } catch (e) {
    console.warn('  ⚠ Auto-sync failed: ' + (e.message || e));
  }
}

function cmdPrepareBlockDistribution(args) {
  const ns = args[0];
  const name = args[1];
  if (!ns || !name) {
    console.error('Usage: prepare-block-distribution <namespace> <module> [--briefs <json>]');
    exit(1);
  }
  var briefsFile = null;
  if (args[2] === '--briefs' && args[3]) {
    briefsFile = resolve(ROOT, args[3]);
  }
  const modJsonPath = join(ROOT, 'Pre-Proc', ns, 'Sources', 'Modules', name, 'module.json');
  if (!existsSync(modJsonPath)) {
    console.error('module.json not found: ' + modJsonPath);
    exit(1);
  }
  const mod = JSON.parse(readFileSync(modJsonPath, 'utf-8'));
  var sceneAssembly = (mod.blockAssembly && mod.blockAssembly.blocks) || mod.blocks || [];
  if (sceneAssembly.length === 0) {
    console.error('No scenes in module.json');
    exit(1);
  }
  const modDir = join(ROOT, 'Pre-Proc', ns, 'Prototypes', 'Modules', name);
  const mode = 'esm';
  console.log('  Mode: ' + mode + ' (llm-tsx/module.tsx ESM pipeline)');
  var briefs = {};
  if (briefsFile) {
    briefs = JSON.parse(readFileSync(briefsFile, 'utf-8'));
    console.log('  Scene briefs loaded: ' + Object.keys(briefs).length);
  }
  const scenes = [];
  var needsSubagent = 0,
    needsScaffold = 0;
  for (const entry of sceneAssembly) {
    const sid = entry.id;
    const srcDir = join(ROOT, 'Pre-Proc', ns, 'Sources', 'Blocks', sid);
    const protoDir = join(ROOT, 'Pre-Proc', ns, 'Prototypes', 'Blocks', sid);
    const blockJsonPath = join(srcDir, 'block.json');
    const briefPath = join(srcDir, 'block-brief.json');
    const hasProto = existsSync(join(protoDir, 'llm-tsx', 'block.tsx'));
    const hasBrief = existsSync(briefPath);
    if (briefs[sid]) {
      mkdirSync(srcDir, { recursive: true });
      writeFileSync(briefPath + '.tmp', JSON.stringify(briefs[sid], null, 2) + '\n', 'utf-8');
      renameSync(briefPath + '.tmp', briefPath);
    }
    if (!existsSync(blockJsonPath) || !hasProto) {
      needsScaffold++;
      mkdirSync(srcDir, { recursive: true });
      mkdirSync(protoDir, { recursive: true });
      if (!existsSync(blockJsonPath)) {
        const scJson = {
          id: sid,
          name: entry.name || sid,
          namespace: ns,
          factors: [],
          flows: [{ id: sid + '-browse', name: entry.name || sid, steps: 1 }],
          workbenchPosts: [],
          version: '0.1.0',
          sharing: { mode: 'single', ownerModule: ns + '/' + name, consumers: [] },
        };
        writeFileSync(blockJsonPath + '.tmp', JSON.stringify(scJson, null, 2) + '\n', 'utf-8');
        renameSync(blockJsonPath + '.tmp', blockJsonPath);
        console.log('  ✓ block.json: ' + blockJsonPath);
      }
      if (!hasProto) {
        // Create llm-tsx/block.tsx stub for ESM pipeline
        mkdirSync(join(protoDir, 'llm-tsx'), { recursive: true });
        var stub =
          "import { useState, useEffect, useRef } from 'react';\n\n" +
          '/* Mock data */\nvar MOCK = [];\n\n' +
          '/* Main component */\n' +
          'export default function Block() {\n' +
          "  return React.createElement('div', { className: 'p-6' },\n" +
          "    React.createElement('h2', null, '" +
          sid +
          "'),\n" +
          '  );\n' +
          '}\n';
        writeFileSync(join(protoDir, 'llm-tsx', 'block.tsx'), stub, 'utf-8');
        console.log('  \u2713 llm-tsx/block.tsx: ' + join(protoDir, 'llm-tsx', 'block.tsx'));
      }
      scenes.push({ id: sid, name: entry.name || sid, status: 'scaffolded' });
    } else if (!hasBrief && Object.keys(briefs).length === 0) {
      needsSubagent++;
      scenes.push({ id: sid, name: entry.name || sid, status: 'needs-content' });
    } else {
      scenes.push({ id: sid, name: entry.name || sid, status: 'ready' });
    }
  }
  // Write distribution state
  const distPath = join(
    ROOT,
    'Pre-Proc',
    ns,
    'Sources',
    'Modules',
    name,
    '.distribution-state.json',
  );
  const state = {
    ns: ns,
    module: name,
    preparedAt: new Date().toISOString(),
    mode: mode,
    scenes: Object.fromEntries(scenes.map((s) => [s.id, { status: s.status, name: s.name }])),
    allReady: needsSubagent === 0 && needsScaffold === 0,
  };
  writeFileSync(distPath + '.tmp', JSON.stringify(state, null, 2) + '\n', 'utf-8');
  renameSync(distPath + '.tmp', distPath);
  // Output summary
  console.log('\n═══ Distribution Status ═══');
  console.log(
    '  Module: ' + ns + '/' + name + ' (' + sceneAssembly.length + ' scenes, mode: ' + mode + ')',
  );
  for (const s of scenes) {
    const icon = s.status === 'ready' ? '✅' : s.status === 'scaffolded' ? '🔧' : '⏳';
    console.log('  ' + icon + ' [' + s.status + '] ' + s.id);
  }
  var ready = scenes.filter((s) => s.status === 'ready').length;
  console.log(
    '  Ready: ' + ready + ' | Needs content: ' + needsSubagent + ' | Scaffolded: ' + needsScaffold,
  );
  if (needsSubagent > 0) {
    console.log('\n═══ Subagent Plan ═══');
    console.log('Blocks needing LLM content:');
    for (const s of scenes) {
      if (s.status === 'needs-content') {
        var bp = 'Pre-Proc/' + ns + '/Sources/Blocks/' + s.id + '/block-brief.json';
        var target = 'llm-tsx/block.tsx';
        console.log('  → ' + s.id + ' (' + s.name + ')');
        console.log('    Brief:   ' + bp + ' (write first, then re-run prepare)');
        console.log(
          '    Subagent: skill://alioth-block Track 1 → create ' +
            target +
            ' in ' +
            join('Pre-Proc', ns, 'Prototypes', 'Blocks', s.id),
        );
      }
    }
    console.log('\nAfter subagents complete, run:');
    console.log('  bun scripts/prototype-tool.js collect-block-results ' + ns + ' ' + name);
  }
}

async function cmdCollectBlockResults(args) {
  const ns = args[0],
    name = args[1];
  if (!ns || !name) {
    console.error('Usage: collect-block-results <namespace> <module>');
    exit(1);
  }
  const distPath = join(
    ROOT,
    'Pre-Proc',
    ns,
    'Sources',
    'Modules',
    name,
    '.distribution-state.json',
  );
  const modJsonPath = join(ROOT, 'Pre-Proc', ns, 'Sources', 'Modules', name, 'module.json');
  if (!existsSync(modJsonPath)) {
    console.error('module.json not found.');
    exit(1);
  }

  // ESM-only mode (traditional babel pipeline removed)
  var mode = 'esm';
  var state = {};
  if (existsSync(distPath)) {
    state = JSON.parse(readFileSync(distPath, 'utf-8'));
  }

  const mod = JSON.parse(readFileSync(modJsonPath, 'utf-8'));
  var sceneAssembly = (mod.blockAssembly && mod.blockAssembly.blocks) || mod.blocks || [];
  var failures = 0,
    success = 0;

  // ESM mode: validate llm-tsx/block.tsx for each scene, then build --all
  for (const entry of sceneAssembly) {
    const sid = entry.id;
    const tsxFile = join(ROOT, 'Pre-Proc', ns, 'Prototypes', 'Blocks', sid, 'llm-tsx', 'block.tsx');
    if (!existsSync(tsxFile)) {
      console.error('  \u274c No llm-tsx/block.tsx: ' + sid);
      failures++;
      continue;
    }
    const pc = readFileSync(tsxFile, 'utf-8');
    if (pc.indexOf('export default function') < 0 && pc.indexOf('export default ') < 0) {
      console.error('  \u274c No export default in: ' + sid + ' (' + tsxFile + ')');
      failures++;
      continue;
    }
    console.log('  \u2705 ' + sid + ' (llm-tsx/block.tsx)');
    success++;
  }
  console.log('\n\u2550\u2550\u2550 Result: ' + success + '/' + (success + failures) + ' valid');
  if (failures > 0) {
    console.error('  \u274c Fix ' + failures + ' scene(s) and re-run.');
    exit(1);
  }
  console.log('\nBuilding all scenes + module via esbuild...');
  await buildAll(join(ROOT, 'Pre-Proc', ns, 'Prototypes', 'Modules', name));
  console.log('\n\u2705 ESM distribution complete (m-v{N}.html built from tsx chain).');
  // Update state
  state.collectedAt = new Date().toISOString();
  state.allReady = true;
  state.mode = mode;
  writeFileSync(distPath + '.tmp', JSON.stringify(state, null, 2) + '\n', 'utf-8');
  renameSync(distPath + '.tmp', distPath);
}

async function cmdBuild(args) {
  if (args.length < 1) {
    console.error('Usage: build <file-path> [--all]');
    exit(1);
  }
  console.log('check [dir]                    验证 ESM 原型可构建、无孤立引用');

  // --all mode: build all scenes + module from module.json
  if (args[0] === '--all') {
    if (args.length < 2) {
      console.error('Usage: build --all <module-path>');
      exit(1);
    }
    return await buildAll(args[1]);
  }

  var srcPath = resolve(ROOT, args[0]);
  if (!existsSync(srcPath)) {
    console.error('Not found: ' + srcPath);
    exit(1);
  }

  var isScene = srcPath.indexOf('/Blocks/') >= 0 && srcPath.indexOf('/llm-tsx/') >= 0;
  var isModule = srcPath.indexOf('/Modules/') >= 0 && srcPath.indexOf('/llm-tsx/module.tsx') >= 0;
  var isApp = srcPath.indexOf('/Apps/') >= 0 && srcPath.indexOf('/llm-tsx/') >= 0;

  if (!isScene && !isModule && !isApp) {
    console.error('Cannot determine layer from path: ' + srcPath);
    exit(1);
  }

  var pp = srcPath.indexOf('/Pre-Proc/');
  if (pp < 0) {
    console.error('Path not under Pre-Proc/');
    exit(1);
  }
  var rest = srcPath.substring(pp + 10);
  var ns = rest.substring(0, rest.indexOf('/'));

  try {
    if (isScene) {
      var blockDir = dirname(dirname(srcPath));
      var blockId = blockDir.substring(blockDir.lastIndexOf('/') + 1);
      var ver = computeNextVersion(blockDir, 'b');
      console.log('Build Block: ' + blockId + ' (v' + ver + ')');
      await buildBlock(blockDir, blockId, ns, ver);
      var modules = findModulesByBlock(ns, blockId);
      for (const m of modules) {
        var mDir = join(ROOT, 'Pre-Proc', ns, 'Prototypes', 'Modules', m.name);
        if (existsSync(join(mDir, 'llm-tsx', 'module.tsx'))) {
          console.log('  Cascade to Module: ' + m.name);
          await buildModule(mDir, m.name, ns, blockId + '(b-v' + ver + ')');
          var apps = findAppsByModule(ns, m.name);
          for (const a of apps) {
            var appDir = join(ROOT, 'Pre-Proc', ns, 'Apps', a.code);
            if (existsSync(join(appDir, 'llm-tsx', 'app.tsx'))) {
              console.log('    Cascade to App: ' + a.code);
              await buildApp(appDir, a.code, ns, m.name + '(m-v?)');
            }
          }
        }
      }
    } else if (isModule) {
      var modDir = dirname(dirname(srcPath));
      var moduleName = modDir.substring(modDir.lastIndexOf('/') + 1);
      var modVer = computeNextVersion(modDir, 'm');
      console.log('Build Module: ' + moduleName + ' (m-v' + modVer + ')');
      // Resolve sceneRefs from module.json so the HTML comment lists real blocks
      var modJsonPath = join(ROOT, 'Pre-Proc', ns, 'Sources', 'Modules', moduleName, 'module.json');
      var modSceneRefs = '';
      try {
        var modJson = JSON.parse(readFileSync(modJsonPath, 'utf-8'));
        var modBlocks =
          (modJson.blockAssembly && modJson.blockAssembly.blocks) || modJson.blocks || [];
        modSceneRefs = modBlocks
          .map(function (b) {
            return b.id;
          })
          .join(',');
      } catch (e) {
        modSceneRefs = '';
      }
      await buildModule(modDir, moduleName, ns, modSceneRefs || '');
      var apps = findAppsByModule(ns, moduleName);
      for (const a of apps) {
        var appDir = join(ROOT, 'Pre-Proc', ns, 'Apps', a.code);
        if (existsSync(join(appDir, 'llm-tsx', 'app.tsx'))) {
          console.log('  Cascade to App: ' + a.code);
          await buildApp(appDir, a.code, ns, moduleName + '(m-v' + modVer + ')');
        }
      }
    } else if (isApp) {
      var appDir = dirname(dirname(srcPath));
      var appCode = appDir.substring(appDir.lastIndexOf('/') + 1);
      var appVer = computeNextVersion(appDir, 'a');
      console.log('Build App: ' + appCode + ' (a-v' + appVer + ')');
      await buildApp(appDir, appCode, ns, '');
    }
  } catch (e) {
    console.error('Build failed:', e.message);
    exit(1);
  }
}

async function cmdEval(args) {
  if (args.length < 1) {
    console.error('Usage: eval \u003chtml-path\u003e');
    exit(1);
  }
  var htmlPath = resolve(ROOT, args[0]);
  if (!existsSync(htmlPath)) {
    console.error('Not found: ' + htmlPath);
    exit(1);
  }
  console.log('Running prototype-reference evaluator on ' + htmlPath);
  try {
    execSync('bun scripts/eval/evaluate-prototype-reference.ts ' + JSON.stringify(htmlPath), {
      stdio: 'inherit',
      timeout: 30000,
    });
  } catch (e) {
    console.error('\u274c Evaluator failed');
    exit(e.status || 1);
  }
}

async function buildAll(modPath) {
  modPath = resolve(ROOT, modPath);
  var pp = modPath.indexOf('/Pre-Proc/');
  if (pp < 0) {
    console.error('Path not under Pre-Proc/');
    exit(1);
  }
  var rest = modPath.substring(pp + 10);
  var ns = rest.substring(0, rest.indexOf('/'));
  var modName = rest.match(/Modules\/([^/]+)/)?.[1];
  if (!modName) {
    console.error('Cannot extract module name');
    exit(1);
  }
  var modJsonPath = join(ROOT, 'Pre-Proc', ns, 'Sources', 'Modules', modName, 'module.json');
  if (!existsSync(modJsonPath)) {
    console.error('module.json not found');
    exit(1);
  }
  var modJson = JSON.parse(readFileSync(modJsonPath, 'utf-8'));
  var scenes = modJson.blocks || [];
  console.log('\\nBuilding all ' + scenes.length + ' scenes for ' + modName);
  for (const s of scenes) {
    var blockDir = join(ROOT, 'Pre-Proc', ns, 'Prototypes', 'Blocks', s.id);
    var blockFile = join(blockDir, 'llm-tsx', 'block.tsx');
    if (existsSync(blockFile)) {
      console.log('\\n  Block: ' + s.id);
      var ver = computeNextVersion(blockDir, 'b');
      await buildBlock(blockDir, s.id, ns, ver);
      var mods = findModulesByBlock(ns, s.id);
      for (const m of mods) {
        var md = join(ROOT, 'Pre-Proc', ns, 'Prototypes', 'Modules', m.name);
        if (existsSync(join(md, 'llm-tsx', 'module.tsx'))) {
          await buildModule(md, m.name, ns, s.id + '(b-v' + ver + ')');
        }
      }
    } else {
      console.log('  [CREATE] ' + s.id);
      var meta = {};
      mkdirSync(join(blockDir, 'llm-tsx'), { recursive: true });
      var stub =
        "import { useState, useEffect, useRef } from 'react';\n\n" +
        '/* Mock data */\n' +
        'var MOCK = [];\n\n' +
        '/* Main component */\n' +
        'export default function ' + s.id.replace(/-/g, '_') + '() {\n' +
        "  return React.createElement('div', { className: 'p-6' },\n" +
        "    React.createElement('h2', null, '" +
        s.id +
        "'),\n" +
        '  );\n' +
        '}\n';
      writeFileSync(blockFile, stub, 'utf-8');
    }
  }
  // Final module rebuild to have consistent version
  var modDir = join(ROOT, 'Pre-Proc', ns, 'Prototypes', 'Modules', modName);
  var modTsx = join(modDir, 'llm-tsx', 'module.tsx');
  if (existsSync(modTsx)) {
    var modVer = computeNextVersion(modDir, 'm');
    console.log('\\n  Module: ' + modName + ' (m-v' + modVer + ')');
    await buildModule(
      modDir,
      modName,
      ns,
      scenes
        .map(function (s) {
          return s.id;
        })
        .join(','),
    );
  }
}

var blockNameVariants = {
  'block-license-mgmt': 'LicensePage',
  'block-exchange-rate': 'ExchangeRatePage',
  'block-environment': 'EnvironmentPage',
  'block-theme': 'ThemePage',
  'block-language': 'LanguagePage',
  'block-unit-system': 'UnitSystemPage',
};

function extractBlockFromModule(ns, modName, blockId) {
  // Find the latest v{N}.html in the module prototype directory
  var modDir = join(ROOT, 'Pre-Proc', ns, 'Prototypes', 'Modules', modName);
  var protoFiles = globSync(join(modDir, 'v*.html')).sort();
  if (protoFiles.length === 0) {
    console.error('  No module prototype v{N}.html found');
    return null;
  }
  var protoPath = protoFiles[protoFiles.length - 1];
  var html = readFileSync(protoPath, 'utf-8');

  // Component function name from block ID: block-unit-system → UnitSystemPage
  var parts = blockId.split('-');
  var compName =
    parts
      .map(function (p, i) {
        return i === 0
          ? p.charAt(0).toUpperCase() + p.slice(1)
          : p.charAt(0).toUpperCase() + p.slice(1);
      })
      .join('') + 'Page';
  // Also try CamelCase after scene- prefix
  var baseName = blockId
    .replace(/^scene-/, '')
    .split('-')
    .map(function (p) {
      return p.charAt(0).toUpperCase() + p.slice(1);
    })
    .join('');
  var altName = baseName + 'Page';
  var baseOnly =
    blockId
      .split('-')
      .map(function (p) {
        return p.charAt(0).toUpperCase() + p.slice(1);
      })
      .join('') + 'Page';
  var shortName = blockNameVariants[blockId] || '';

  // Try both names
  var names = [compName, altName, baseOnly, shortName];
  var funcIdx = -1;
  for (var ni = 0; ni < names.length; ni++) {
    if (names[ni]) {
      funcIdx = html.indexOf('function ' + names[ni] + '(');
      if (funcIdx >= 0) {
        compName = names[ni];
        break;
      }
    }
  }
  if (funcIdx < 0) {
    console.error('  Component function not found in prototype: ' + names.join('/'));
    return null;
  }

  // Extract the function
  var funcStart = funcIdx;
  var brace = 0,
    i;
  for (i = funcStart; i < html.length; i++) {
    if (html[i] === '{') brace++;
    else if (html[i] === '}') {
      brace--;
      if (brace === 0) break;
    }
  }
  if (brace !== 0) {
    console.error('  Could not find end of function');
    return null;
  }
  var funcCode = html.substring(funcStart, i + 1);

  // Extract MOCK data (closest preceding var MOCK = {...})
  var mockIdx = html.lastIndexOf('var MOCK = {', funcStart);
  var mockCode = '';
  if (mockIdx >= 0) {
    var brace = 0,
      j;
    for (j = mockIdx; j < html.length; j++) {
      if (html[j] === '{') brace++;
      else if (html[j] === '}') {
        brace--;
        if (brace === 0) {
          mockCode = html.substring(mockIdx, j + 1);
          break;
        }
      }
    }
  }

  funcCode = convertHCalls(funcCode);
  funcCode = normalizeICONS(funcCode);
  funcCode = stripReactImport(funcCode);
  funcCode = fixSVGRefs(funcCode);
  // Extract I18N dict from module prototype
  var i18nData = '';
  var i18nStart = html.indexOf("I18N.dict['zh-CN']");
  if (i18nStart >= 0) {
    var brace = 0,
      j;
    for (j = i18nStart; j < html.length; j++) {
      if (html[j] === '{') brace++;
      else if (html[j] === '}') {
        brace--;
        if (brace === 0) {
          i18nData = html.substring(i18nStart, j + 1);
          break;
        }
      }
    }
  }

  // Flatten the nested I18N dict into flat key→value pairs for build-time resolution
  var flatDict = {};
  if (i18nData) {
    var eqIdx = i18nData.indexOf('=');
    if (eqIdx >= 0) {
      var jsonStr = i18nData.substring(eqIdx + 1).trim();
      // Remove trailing semicolon if present
      if (jsonStr.charAt(jsonStr.length - 1) === ';')
        jsonStr = jsonStr.substring(0, jsonStr.length - 1);
      try {
        var parsed = JSON.parse(jsonStr);
        (function flatten(obj, prefix) {
          for (var k in obj) {
            if (obj.hasOwnProperty(k)) {
              var fullKey = prefix ? prefix + '.' + k : k;
              if (typeof obj[k] === 'string') flatDict[fullKey] = obj[k];
              else if (typeof obj[k] === 'object') flatten(obj[k], fullKey);
            }
          }
        })(parsed, '');
      } catch (e) {
        // If JSON parse fails, fall back to runtime resolution
        console.log('  [warn] Could not parse i18n dict for build-time resolution');
      }
    }
  }

  // Resolve t() calls at build time
  funcCode = resolveI18nCalls(funcCode, flatDict);
  var helperCode = '';
  var imports = "import { useState, useEffect, useRef, Fragment } from 'react';\n\n";
  var i18nStr = i18nData
    ? 'if(typeof I18N==="undefined"){var I18N={};I18N.dict={\'zh-CN\':{}};}' +
      i18nData.replace(/^I18N\.dict/, 'I18N.dict') +
      "\n\nfunction t(key) { return I18N.dict['zh-CN'][key] || key; }\n\n"
    : 'function t(key) { return key; }\n\n';
  var mock = mockCode ? mockCode + '\n\n' : '';

  // Extract helper functions used by the scene component
  var helpers = '';
  var helperNames = [
    'useLoadingState',
    'useFocusTrap',
    'function ConfirmDialog',
    'function statusTag',
    'function fmtMult',
    'function Skeleton',
    'function ErrorBanner',
    'var DIM_COLORS = {',
  ];
  helperNames.forEach(function (h) {
    // Find the function in html
    var idx = html.indexOf(h);
    if (idx >= 0) {
      // Find the function start (backwards to find 'function')
      var fnStart = html.lastIndexOf('function ', idx);
      if (fnStart < 0) {
        fnStart = idx;
      }
      // Find the closing brace
      var brace = 0;
      var fnEnd = fnStart;
      var foundOpen = false;
      while (fnEnd < html.length) {
        if (html[fnEnd] === '{') {
          brace++;
          foundOpen = true;
        } else if (html[fnEnd] === '}') {
          brace--;
          if (foundOpen && brace === 0) {
            fnEnd++;
            break;
          }
        }
        fnEnd++;
      }
      var code = html.substring(fnStart, fnEnd);
      code = convertHCalls(code);
      code = normalizeICONS(code);
      code = fixSVGRefs(code);
      code = resolveI18nCalls(code, flatDict);
    }
  });

  var comp = funcCode
    .replace('function ' + compName + '(', 'export default function ' + compName + '(')
    .replace('function ' + altName + '(', 'export default function ' + altName + '(');

  return imports + i18nStr + helperCode + mock + comp;
}
async function cmdCheck(args) {
  if (args.length < 1) {
    console.error('Usage: check <module-path>');
    exit(1);
  }
  var modPath = resolve(ROOT, args[0]);
  var pp = modPath.indexOf('/Pre-Proc/');
  if (pp < 0) {
    console.error('Path not under Pre-Proc/');
    exit(1);
  }
  var rest = modPath.substring(pp + 10);
  var ns = rest.substring(0, rest.indexOf('/'));
  var modName = rest.match(/Modules\/([^/]+)/)?.[1];
  if (!modName) {
    console.error('Cannot extract module name');
    exit(1);
  }

  var modJsonPath = join(ROOT, 'Pre-Proc', ns, 'Sources', 'Modules', modName, 'module.json');
  if (!existsSync(modJsonPath)) {
    console.error('module.json not found: ' + modJsonPath);
    exit(1);
  }
  var modJson = JSON.parse(readFileSync(modJsonPath, 'utf-8'));
  var scenes = modJson.blocks || [];
  var assembly = modJson.blockAssembly || {};
  var blockMeta = {};
  (assembly.blocks || []).forEach(function (s) {
    blockMeta[s.id] = s;
  });

  var missing = [],
    present = [];
  scenes.forEach(function (s) {
    var blockFile = join(
      ROOT,
      'Pre-Proc',
      ns,
      'Prototypes',
      'Blocks',
      s.id,
      'llm-tsx',
      'block.tsx',
    );
    if (existsSync(blockFile)) {
      present.push(s.id);
    } else {
      missing.push(s.id);
    }
  });

  console.log('\\nModule: ' + modName + ' (' + ns + ')');
  console.log(
    '  Blocks: ' + scenes.length + ', Built: ' + present.length + ', Missing: ' + missing.length,
  );

  // Auto-create missing scenes
  if (missing.length > 0) {
    console.log('\\n  Creating ' + missing.length + ' missing scenes...');
    missing.forEach(function (id) {
      var blockDir = join(ROOT, 'Pre-Proc', ns, 'Prototypes', 'Blocks', id, 'llm-tsx');
      mkdirSync(blockDir, { recursive: true });
      var blockFile = join(blockDir, 'block.tsx');
      // Try to extract from module prototype first
      var extracted = extractBlockFromModule(ns, modName, id);
      if (extracted) {
        writeFileSync(blockFile, extracted, 'utf-8');
        console.log('    Extracted: ' + id + ' from ' + modName + ' prototype');
      } else {
        // Fallback: create stub
        var meta = blockMeta[id] || { name: id, icon: 'layout' };
        var stub =
          "import { useState, useEffect, useRef } from 'react';\n\n" +
          '/* Mock data */\n' +
          'var MOCK = [];\n\n' +
          '/* Main component */\n' +
          'export default function ' + id.replace(/-/g, '_') + '() {\n' +
          "  return React.createElement('div', { className: 'p-6' },\n" +
          "    React.createElement('h2', null, '" +
          (meta.name || id) +
          "'),\n" +
          '  );\n' +
          '}\n';
        writeFileSync(blockFile, stub, 'utf-8');
        console.log('    Created stub: ' + id + '');
      }
    });
  }

  // Build all scenes
  console.log('\\n  Building all ' + scenes.length + ' scenes...');
  for (const s of scenes) {
    var blockFile = join(
      ROOT,
      'Pre-Proc',
      ns,
      'Prototypes',
      'Blocks',
      s.id,
      'llm-tsx',
      'block.tsx',
    );
    if (existsSync(blockFile)) {
      var blockDir = join(ROOT, 'Pre-Proc', ns, 'Prototypes', 'Blocks', s.id);
      var ver = computeNextVersion(blockDir, 'b');
      console.log('    Block: ' + s.id + ' (b-v' + ver + ')');
      await buildBlock(blockDir, s.id, ns, ver);
    }
  }
  // Build module — auto-create module.tsx if missing
  var modDir = join(ROOT, 'Pre-Proc', ns, 'Prototypes', 'Modules', modName);
  var modTsx = join(modDir, 'llm-tsx', 'module.tsx');
  if (!existsSync(modTsx)) {
    mkdirSync(join(modDir, 'llm-tsx'), { recursive: true });
    var moduleLines = ["import { useState, useEffect, useRef } from 'react';", ''];
    var navGroups = {};
    var registryEntries = [];
    scenes.forEach(function (s) {
      var sid = s.id;
      var blockName = sid
        .replace('scene-', '')
        .split('-')
        .map(function (p) {
          return p.charAt(0).toUpperCase() + p.slice(1);
        })
        .join('');
      moduleLines.push(
        'import { default as ' +
          blockName +
          " } from '../../../Blocks/" +
          sid +
          "/llm-tsx/block';",
      );
      registryEntries.push("  '" + sid + "': " + blockName);
      var group = s.group || '其他';
      if (!navGroups[group]) navGroups[group] = [];
      var assembly = modJson.blockAssembly || {};
      var as =
        (assembly.blocks || []).filter(function (a) {
          return a.id === sid;
        })[0] || {};
      var navLabel = as.name || blockName;
      var navIcon = (as.icon || 'layout').charAt(0).toLowerCase() + (as.icon || 'layout').slice(1);
      navGroups[group].push(
        "{ id: '" + sid + "', label: '" + navLabel + "', icon: '" + navIcon + "' }",
      );
    });
    moduleLines.push('');
    moduleLines.push('var NAV_GROUPS = [');
    for (var g in navGroups) {
      moduleLines.push("  { label: '" + g + "', items: [" + navGroups[g].join(', ') + '] },');
    }
    moduleLines.push('];');
    moduleLines.push('');
    moduleLines.push('var BLOCK_REGISTRY = {');
    moduleLines = moduleLines.concat(
      registryEntries.map(function (e) {
        return e + ',';
      }),
    );
    moduleLines.push('};');
    moduleLines.push('');
    moduleLines.push('var ModuleLayout = function({ embedded }) {');
    moduleLines.push(
      "  return React.createElement('div', null, 'Module stub — implement Gateway Shell');",
    );
    moduleLines.push('}');
    moduleLines.push('export const { bootstrap, mount, unmount } = createPrototypeLifecycle({');
    moduleLines.push("  name: '" + modName + "',");
    moduleLines.push('  App: ModuleLayout,');
    moduleLines.push('});');
    ensureSharedLifecycle(ns);
    writeFileSync(modTsx, moduleLines.join('\n'), 'utf-8');
    console.log('    Created module.tsx stub');
  }
  var modVer = computeNextVersion(modDir, 'm');
  console.log('    Module: ' + modName + ' (m-v' + modVer + ')');
  await buildModule(
    modDir,
    modName,
    ns,
    scenes
      .map(function (s) {
        return s.id;
      })
      .join(','),
  );
  console.log('\\n  Done.');
}

function cmdScaffold(args) {
  var level = args[0];
  var ns = args[1];
  var code = args[2];

  if (!level || !ns || !code) {
    console.error(
      'Usage: scaffold <app|module|scene> <namespace> <code|name|id> [--modules m1,m2] [--scenes s1,s2]',
    );
    exit(1);
  }

  if (!/^[A-Z][a-zA-Z0-9-]*$/.test(ns)) {
    console.error('namespace 格式错误: 必须以大写字母开头，仅含字母数字横线');
    exit(1);
  }

  var extras = {};
  for (var i = 3; i < args.length; i += 2) {
    var key = args[i];
    var val = args[i + 1];
    if (key === '--modules' || key === '--scenes') {
      extras[key] = val
        ? val
            .split(',')
            .map(function (s) {
              return s.trim();
            })
            .filter(Boolean)
        : [];
    }
  }

  function writeJson(p, obj) {
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, JSON.stringify(obj, null, 2) + '\n', 'utf-8');
  }

  if (level === 'app') {
    var appJsonPath = join(ROOT, 'Pre-Proc', ns, 'Apps', code, 'app.json');
    var appTsxPath = join(ROOT, 'Pre-Proc', ns, 'Prototypes', 'Apps', code, 'llm-tsx', 'app.tsx');
    writeJson(appJsonPath, {
      namespace: ns,
      code: code,
      config: {
        modules: extras['--modules'] || [],
      },
    });
    writeTsx(appTsxPath, 'AppLayout', ns, code);
    console.log('Scaffolded App: ' + ns + '/' + code);
    console.log('  ' + appJsonPath);
    console.log('  ' + appTsxPath);
    return;
  }

  if (level === 'module') {
    var moduleJsonPath = join(ROOT, 'Pre-Proc', ns, 'Sources', 'Modules', code, 'module.json');
    var moduleTsxPath = join(
      ROOT,
      'Pre-Proc',
      ns,
      'Prototypes',
      'Modules',
      code,
      'llm-tsx',
      'module.tsx',
    );
    var scenes = (extras['--scenes'] || []).map(function (sid) {
      return { id: sid, name: sid };
    });
    writeJson(moduleJsonPath, {
      namespace: ns,
      name: code,
      version: '0.0.1',
      blockAssembly: {
        blocks: scenes,
      },
    });
    writeTsx(moduleTsxPath, 'ModuleLayout', ns, code, '{ embedded = false }');
    console.log('Scaffolded Module: ' + ns + '/' + code);
    console.log('  ' + moduleJsonPath);
    console.log('  ' + moduleTsxPath);
    return;
  }

  if (level === 'scene' || level === 'block') {
    var blockJsonPath = join(ROOT, 'Pre-Proc', ns, 'Sources', 'Blocks', code, 'block.json');
    var blockTsxPath = join(
      ROOT,
      'Pre-Proc',
      ns,
      'Prototypes',
      'Blocks',
      code,
      'llm-tsx',
      'block.tsx',
    );
    writeJson(blockJsonPath, {
      id: code,
      namespace: ns,
      name: code,
      version: '0.0.1',
    });
    writeTsx(blockTsxPath, 'Block', ns, code);
    console.log('Scaffolded Block: ' + ns + '/' + code);
    console.log('  ' + blockJsonPath);
    console.log('  ' + blockTsxPath);
    return;
  }

  console.error('Unknown level: ' + level + ' (expected app|module|scene)');
  exit(1);
}

async function cmdRenderShell(args) {
  // render-shell <block|module|app> [ns] [id] [--out <path>]
  // 渲染空壳骨架为 *-shell.html(仅 boot-skeleton + CSS,不含 bundle/mountScript)
  var kind = args[0];
  var ns = args[1] || 'Alioth';
  var id = args[2] || 'preview';
  var outIdx = args.indexOf('--out');
  var outPath = outIdx >= 0 && args[outIdx + 1] ? args[outIdx + 1] : null;
  if (!kind || !['block', 'module', 'app'].includes(kind)) {
    console.error('用法: render-shell <block|module|app> [ns] [id] [--out <path>]');
    console.error('  渲染空壳骨架为 *-shell.html(仅 boot-skeleton + CSS,供设计审查壳布局)');
    exit(1);
  }
  var shellName = kind + '-shell.tsx';
  var shellPath = join(ROOT, '.agents/skills/alioth-design/references/shells', shellName);
  var shellMod = await import(shellPath);
  var defaultOut = join(
    ROOT,
    '.agents/skills/alioth-design/references/shells',
    kind + '-shell.html',
  );
  var htmlPath = outPath ? resolve(outPath) : defaultOut;
  var rootPath = rootPathFor(htmlPath);
  var html;
  if (kind === 'block') {
    html = shellMod.renderBlockShell({
      title: id + ' shell preview \u00b7 ' + ns,
      rootPath: rootPath,
      bundleJs: 'placeholder.bundle.js',
      blockId: id,
      globalName: 'Block__' + sanitizeGlobalName(id),
      tokensCss: PROTOTYPE_BASE_CSS_CONTENT,
      iconPool: ICON_POOL_CONTENT,
      previewMode: true,
    });
  } else if (kind === 'module') {
    html = shellMod.renderModuleShell({
      title: id + ' shell preview \u00b7 ' + ns,
      rootPath: rootPath,
      bundleJs: 'placeholder.bundle.js',
      bodyClass: id,
      globalName: 'Module__' + sanitizeGlobalName(id),
      name: id,
      tokensCss: PROTOTYPE_BASE_CSS_CONTENT,
      iconPool: ICON_POOL_CONTENT,
      previewMode: true,
    });
  } else {
    html = shellMod.renderAppShell({
      title: id + ' shell preview \u00b7 ' + ns,
      rootPath: rootPath,
      bundleJs: 'placeholder.bundle.js',
      globalName: 'App__' + sanitizeGlobalName(id),
      name: id,
      tokensCss: PROTOTYPE_BASE_CSS_CONTENT,
      iconPool: ICON_POOL_CONTENT,
      previewMode: true,
    });
  }
  writeFileSync(htmlPath, html, 'utf-8');
  console.log('Rendered shell preview: ' + htmlPath);
}

function cmdMigrateVendorPaths(args) {
  var dryRun = args.indexOf('--dry') >= 0;
  var scanDir = args[0] && args[0] !== '--dry' ? join(ROOT, args[0]) : join(ROOT, 'Pre-Proc');
  var targets = globSync(join(scanDir, '**/*.html'));
  // gate-layout.css was deprecated 2026-07-07 (prototype-side). The regex below handles historical
  // migration but won't match anything new. Keep for idempotence.
  // No new prototypes should reference it. For new prototypes, use only tailwind-utilities.css.
  var oldSrcHref =
    /((?:\.\.\/)+)(Meta\/frontend\/public\/vendor\/|Framework\/frontend\/components\/src\/gate-layout\.css)/g;
  var oldComment = /Meta\/frontend\/public\/vendor\//g;
  var oldDesignTokens = /<link[^>]*href="design-tokens\.css"[^>]*>/g;
  var needsMigration =
    /Meta\/frontend\/public\/vendor|Framework\/frontend\/components\/src\/gate-layout\.css|href="design-tokens\.css"/;
  var fileCount = 0,
    matchCount = 0;
  for (var i = 0; i < targets.length; i++) {
    var f = targets[i];
    var html = readFileSync(f, 'utf-8');
    if (!needsMigration.test(html)) continue;
    oldSrcHref.lastIndex = 0;
    oldComment.lastIndex = 0;
    oldDesignTokens.lastIndex = 0;
    var prefix = relative(dirname(f), REFERENCES_DIR).split(sep).join('/') + '/';
    // design-tokens.css no longer exists (merged into tailwind-utilities.css 2026-07-07).
    // Migration: delete the <link> tag rather than rewrite its path.
    var next = html
      .replace(oldSrcHref, prefix + '$2')
      .replace(oldComment, '.agents/skills/alioth-design/references/vendor/')
      .replace(oldDesignTokens, '');
    if (next !== html) {
      var diffs = 0;
      oldSrcHref.lastIndex = 0;
      oldComment.lastIndex = 0;
      oldDesignTokens.lastIndex = 0;
      var m;
      while ((m = oldSrcHref.exec(html)) !== null) diffs++;
      oldComment.lastIndex = 0;
      while ((m = oldComment.exec(html)) !== null) diffs++;
      oldDesignTokens.lastIndex = 0;
      while ((m = oldDesignTokens.exec(html)) !== null) diffs++;
      fileCount++;
      matchCount += diffs;
      if (!dryRun) {
        var tmp = f + '.tmp';
        writeFileSync(tmp, next, 'utf-8');
        renameSync(tmp, f);
      }
    }
  }
  console.log(
    (dryRun ? '[DRY RUN] ' : '') +
      'Migrated ' +
      matchCount +
      ' paths across ' +
      fileCount +
      ' files',
  );
}

async function main() {
  const cmd = argv[2];
  if (!cmd || cmd === 'help' || cmd === '--help') {
    showHelp();
    return;
  }
  const map = {
    'build-utility-css': cmdBuildUtilityCSS,
    build: cmdBuild,
    eval: cmdEval,
    check: cmdCheck,
    'list-utilities': cmdListUtilities,
    'generate-mocks': cmdGenerateMocks,
    'sync-services': cmdSyncServices,
    'prepare-block-distribution': cmdPrepareBlockDistribution,
    'collect-block-results': cmdCollectBlockResults,
    'migrate-vendor-paths': cmdMigrateVendorPaths,
    scaffold: cmdScaffold,
    'render-shell': cmdRenderShell,
  };
  if (map[cmd]) await map[cmd](argv.slice(3));
  else {
    console.error('Unknown: ' + cmd);
    showHelp();
    exit(1);
  }
}
main().catch(function (e) {
  console.error(e);
  exit(1);
});
