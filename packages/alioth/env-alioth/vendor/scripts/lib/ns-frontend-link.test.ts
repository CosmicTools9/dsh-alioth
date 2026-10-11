/**
 * ns-frontend-link 夹具（change `unify-ns-frontend-link-resolution` · 架构候选 C4）：
 *   ① 平台包索引：`Framework/frontend/<dir>` → 包名（跳过无 package.json / 坏 JSON 的目录）
 *   ② 写规则：`linkSpecFor` 的相对深度（模块级 vs Block 级两种声明深度）
 *   ③ **往返不变量**（本 change 的核心）：`resolveLinkName(fromDir, linkSpecFor(fromDir, target))` === 目标包名
 *   ④ 容忍根：经链接写出的规格在声明目录未命中时，可由额外候选根解析
 *   ⑤ 未命中 ⇒ null（不猜）
 *
 * 隔离：全程 `mkdtemp` 假仓；不触碰真实检出。
 */
import { afterEach, describe, expect, test } from 'bun:test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, relative } from 'node:path';
import { linkSpecFor, platformPackageIndex, resolveLinkName } from './ns-frontend-link';

const roots: string[] = [];

function mkTree(files: Record<string, string>): string {
  const root = mkdtempSync(join(tmpdir(), 'ns-fe-link-'));
  roots.push(root);
  for (const [rel, body] of Object.entries(files)) {
    const abs = join(root, rel);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, body);
  }
  return root;
}

afterEach(() => {
  while (roots.length > 0) rmSync(roots.pop() as string, { recursive: true, force: true });
});

describe('ns-frontend-link', () => {
  test('① 平台包索引：name → Framework/frontend/<dir>；坏 JSON / 缺 manifest 跳过', () => {
    const root = mkTree({
      'Framework/frontend/api/package.json': '{"name":"@alioth/api"}\n',
      'Framework/frontend/components/package.json': '{"name":"@alioth/components"}\n',
      'Framework/frontend/broken/package.json': '{ not json',
      'Framework/frontend/nomanifest/readme.md': '# x\n',
    });
    const idx = platformPackageIndex(root);
    expect([...idx.keys()].sort()).toEqual(['@alioth/api', '@alioth/components']);
    expect(idx.get('@alioth/api')).toBe('Framework/frontend/api');
  });

  test('② 写规则：两种声明深度的相对规格（模块级 7 跳 / Block 级同构）', () => {
    const root = mkTree({
      'Framework/frontend/api/package.json': '{"name":"@alioth/api"}\n',
      'Pre-Proc/WZ/Sources/Apps/Modules/m/frontend/package.json': '{"name":"m"}\n',
      'Pre-Proc/WZ/Sources/Apps/Blocks/b/frontend/package.json': '{"name":"b"}\n',
    });
    const target = join(root, 'Framework/frontend/api');
    const modFrom = join(root, 'Pre-Proc/WZ/Sources/Apps/Modules/m/frontend');
    const blkFrom = join(root, 'Pre-Proc/WZ/Sources/Apps/Blocks/b/frontend');
    expect(linkSpecFor(modFrom, target)).toBe(`link:${relative(modFrom, target)}`);
    expect(linkSpecFor(blkFrom, target)).toBe(`link:${relative(blkFrom, target)}`);
    expect(linkSpecFor(modFrom, target).startsWith('link:')).toBe(true);
  });

  test('③ 往返不变量：写规则 ∘ 验规则 = 目标包名（同一事实两侧一致）', () => {
    const root = mkTree({
      'Framework/frontend/api/package.json': '{"name":"@alioth/api"}\n',
      'Pre-Proc/WZ/Sources/Apps/Modules/m/frontend/package.json': '{"name":"m"}\n',
    });
    const from = join(root, 'Pre-Proc/WZ/Sources/Apps/Modules/m/frontend');
    const target = join(root, 'Framework/frontend/api');
    const spec = linkSpecFor(from, target);
    expect(resolveLinkName(from, spec)).toBe('@alioth/api');
  });

  test('④ 容忍根：规格相对平台根写出时，可由 extraRoots 解析', () => {
    const root = mkTree({
      'Framework/frontend/api/package.json': '{"name":"@alioth/api"}\n',
      'elsewhere/package.json': '{"name":"x"}\n',
    });
    const from = join(root, 'elsewhere');
    // 规格以「平台根相对」形态写出（经链接的老形态）：声明目录未命中 ⇒ 容忍根命中
    const spec = 'link:Framework/frontend/api';
    expect(resolveLinkName(from, spec)).toBe(null);
    expect(resolveLinkName(from, spec, [root])).toBe('@alioth/api');
  });

  test('⑤ 未命中/非 link: 规格 ⇒ null（不猜）', () => {
    const root = mkTree({ 'x/package.json': '{"name":"x"}\n' });
    const from = join(root, 'x');
    expect(resolveLinkName(from, 'link:../nowhere')).toBe(null);
    expect(resolveLinkName(from, 'workspace:*')).toBe(null);
    expect(resolveLinkName(from, 'catalog:')).toBe(null);
  });
});
