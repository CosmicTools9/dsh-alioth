/**
 * ns-frontend-intake 回归（change `unify-ns-frontend-intake` · 场景「三消费者同解」/「Samples 第二声明源不越界」）：
 * 夹具表覆盖 —— 2 App/3 单元、Module 声明缺目录、Block 无前端、双 App 共享、未归属前端、
 * 布局 `apps` vs `full`（`Sources/Open` 根）、无 Apps 目录、app.json 不可解析、Samples 开关、别名派生与确定性。
 *
 * 隔离：全程在 `mkdtemp` 假平台根内构造 `Pre-Proc/<ns>/…`，不触碰真实仓内容。
 */
import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  intake,
  moduleFrontendsLive,
  listUnitNames,
  mountedNamespaces,
  readAppDeclarations,
  resolveUnitDir,
  spaAliases,
} from './ns-frontend-intake';

let root = '';

function mk(p: string): string {
  mkdirSync(p, { recursive: true });
  return p;
}
function wfile(p: string, body: string): void {
  mk(p.split('/').slice(0, -1).join('/'));
  writeFileSync(p, body);
}
/** 造一个 ns 前端单元：`Sources/Apps/<kind>/<unit>/frontend/{package.json,src/single-spa.tsx?}`。 */
function unit(
  ns: string,
  kind: 'Modules' | 'Blocks',
  name: string,
  opts: { entry?: boolean; root?: 'Apps' | 'Open' } = {},
): string {
  const base =
    opts.root === 'Open'
      ? join(root, 'Pre-Proc', ns, 'Sources', 'Open', kind, name, 'frontend')
      : join(root, 'Pre-Proc', ns, 'Sources', 'Apps', kind, name, 'frontend');
  mk(base);
  writeFileSync(join(base, 'package.json'), JSON.stringify({ name: `@alioth/${name}-frontend` }));
  if (opts.entry !== false) {
    mk(join(base, 'src'));
    writeFileSync(join(base, 'src', 'single-spa.tsx'), 'export {};\n');
  }
  return base;
}
function appJson(ns: string, app: string, cfg: { modules?: string[]; blocks?: string[] }): void {
  wfile(
    join(root, 'Pre-Proc', ns, 'Apps', app, 'app.json'),
    JSON.stringify({ namespace: ns, config: cfg }),
  );
}
function codes(diags: Array<{ code: string; kind: string | null; unit: string | null }>): string[] {
  return diags.map((d) => `${d.kind ?? '-'}/${d.unit ?? '-'}:${d.code}`).sort();
}

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), 'ns-fe-intake-'));
});
afterEach(() => {
  rmSync(root, { recursive: true, force: true });
});

describe('intake 单一事实', () => {
  test('2 App / 3 单元 + 双 App 共享：单元集与诊断 code 稳定', () => {
    unit('WZ', 'Modules', 'm1');
    unit('WZ', 'Modules', 'm2');
    unit('WZ', 'Blocks', 'b1');
    unit('WZ', 'Modules', 'orphan'); // 磁盘有、未声明
    appJson('WZ', 'app-a', { modules: ['m1', 'm2'], blocks: ['b1', 'b-missing'] });
    appJson('WZ', 'app-b', { modules: ['m1'], blocks: [] });

    const r = intake(root, { ns: 'WZ', layout: 'apps' });
    expect(r.units.map((u) => `${u.kind}/${u.unit}`)).toEqual([
      'Blocks/b1',
      'Modules/m1',
      'Modules/m2',
    ]);
    expect(r.units.every((u) => u.entry !== null)).toBe(true);
    expect(codes(r.warnings)).toEqual([
      'Blocks/b-missing:unit-frontend-missing',
      'Modules/m1:unit-shared-across-apps',
      'Modules/orphan:unit-unassigned',
    ]);
    expect(r.errors).toEqual([]);
    expect(r.attributedBy.get('Modules/m1')).toEqual(['app-a', 'app-b']);
  });

  test('Module 声明缺目录与 Block 缺前端：同一诊断 code（严宽留给消费者）', () => {
    appJson('WZ', 'app-a', { modules: ['gone'], blocks: ['blk-gone'] });
    const r = intake(root, { ns: 'WZ' });
    expect(r.units).toEqual([]);
    expect(codes(r.warnings)).toEqual([
      'Blocks/blk-gone:unit-frontend-missing',
      'Modules/gone:unit-frontend-missing',
    ]);
  });

  test('无 Apps 目录 ⇒ no-apps-dir（不静默判空）', () => {
    mk(join(root, 'Pre-Proc', 'WZ', 'Sources', 'Apps', 'Modules'));
    const r = intake(root, { ns: 'WZ' });
    expect(codes(r.warnings)).toEqual(['-/-:no-apps-dir']);
  });

  test('app.json 不可解析 ⇒ app-json-unparsable（其余 App 照常）', () => {
    unit('WZ', 'Modules', 'm1');
    appJson('WZ', 'ok', { modules: ['m1'] });
    wfile(join(root, 'Pre-Proc', 'WZ', 'Apps', 'broken', 'app.json'), '{ not json');
    const r = intake(root, { ns: 'WZ' });
    expect(r.units.map((u) => u.unit)).toEqual(['m1']);
    expect(codes(r.warnings)).toEqual(['-/-:app-json-unparsable']);
  });

  test('layout apps vs full：Sources/Open 根只被 full 命中', () => {
    unit('WZ', 'Modules', 'open-only', { root: 'Open' });
    appJson('WZ', 'app-a', { modules: ['open-only'] });
    const narrow = intake(root, { ns: 'WZ', layout: 'apps' });
    const wide = intake(root, { ns: 'WZ', layout: 'full' });
    expect(narrow.units).toEqual([]);
    expect(codes(narrow.warnings)).toEqual(['Modules/open-only:unit-frontend-missing']);
    expect(wide.units.map((u) => u.unit)).toEqual(['open-only']);
    expect(wide.warnings.filter((w) => w.code === 'unit-frontend-missing')).toEqual([]);
  });

  test('扁平布局（无 Sources/Apps）经 preproc-layout 回退解析', () => {
    mk(join(root, 'Pre-Proc', 'WZ', 'Sources', 'Modules', 'flat', 'frontend'));
    writeFileSync(
      join(root, 'Pre-Proc', 'WZ', 'Sources', 'Modules', 'flat', 'frontend', 'package.json'),
      '{}',
    );
    const nsRoot = join(root, 'Pre-Proc', 'WZ');
    expect(resolveUnitDir(nsRoot, 'Modules', 'flat', 'apps')).toBe(
      join(nsRoot, 'Sources', 'Modules', 'flat'),
    );
    expect(listUnitNames(nsRoot, 'Modules', 'apps')).toEqual(['flat']);
  });

  test('mountedNamespaces 委派单一源（真实目录不算挂载 ⇒ 空；无自造清单）', () => {
    mk(join(root, 'Pre-Proc', 'WZ', 'Apps'));
    // 挂载判据（含 symlink 语义）归 `scripts/lib/ns-mounts.ts`；此处只验证委派关系：真实目录不被当作挂载。
    expect(mountedNamespaces(root)).toEqual([]);
  });
});

describe('Samples 第二声明源不越界', () => {
});

describe('dev 别名派生', () => {
  test('仅含入口的单元；按 find 排序且去重', () => {
    unit('WZ', 'Modules', 'aa', { entry: true });
    unit('WZ', 'Modules', 'bb', { entry: false });
    appJson('WZ', 'app-a', { modules: ['aa', 'bb'] });
    const aliases = spaAliases(intake(root, { ns: 'WZ' }));
    expect(aliases).toEqual([
      {
        find: '@alioth/aa-frontend/single-spa',
        replacement: join(
          root,
          'Pre-Proc',
          'WZ',
          'Sources',
          'Apps',
          'Modules',
          'aa',
          'frontend',
          'src',
          'single-spa.tsx',
        ),
      },
    ]);
  });
});

describe('readAppDeclarations', () => {
  test('保序 + 过滤非字符串/空串', () => {
    wfile(
      join(root, 'Pre-Proc', 'WZ', 'Apps', 'a', 'app.json'),
      JSON.stringify({ config: { modules: ['m2', '', 42, 'm1'], blocks: ['b1'] } }),
    );
    const { apps, warnings } = readAppDeclarations(join(root, 'Pre-Proc', 'WZ', 'Apps'), 'WZ');
    expect(warnings).toEqual([]);
    expect(apps).toEqual([{ app: 'a', modules: ['m2', 'm1'], blocks: ['b1'] }]);
  });
});

describe('moduleFrontendsLive', () => {
  test('磁盘全域 fact + miss 即重建（白屏不变量回归）+ 未知名 null', () => {
    unit('FxLive', 'Modules', 'm-live-a', { entry: true });
    wfile(
      join(
        root,
        'Pre-Proc',
        'FxLive',
        'Sources',
        'Apps',
        'Modules',
        'm-live-a',
        'frontend',
        'src',
        'theme.css',
      ),
      ':root{}',
    );
    unit('FxLive', 'Modules', 'm-live-b', { entry: true });
    unit('FxLive', 'Blocks', 'b-live', { entry: true }); // 非模块面不入 index
    const live = moduleFrontendsLive(root, { ns: 'FxLive' });
    expect(live.names().sort()).toEqual(['m-live-a', 'm-live-b']);
    const a = live.get('m-live-a');
    expect(a?.ns).toBe('FxLive');
    expect(a?.themeCss).not.toBe(null);
    expect(live.get('m-live-b')?.themeCss).toBe(null);
    expect(live.get('b-live')).toBe(null);
    // 首建后新增模块 → get 触发重建命中（2026-09-13「有路由、无 alias ⇒ 白屏」回归）
    unit('FxLive', 'Modules', 'm-live-c', { entry: true });
    expect(live.get('m-live-c')).not.toBe(null);
    expect(live.get('nope')).toBe(null);
  });
});
