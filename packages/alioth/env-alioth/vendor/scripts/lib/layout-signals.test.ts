/// <reference types="bun" />
import { describe, expect, test } from 'bun:test';
import {
  DESKTOP_VIEWPORT,
  dimensionsFrom,
  isBaselined,
  LAYOUT_PROBE,
  minDimensionScore,
  narrowViolations,
  parseBaseline,
  scoreFrom,
  type ViewportMetrics,
} from './layout-signals';

/** 夹具：三页 × 三档的**实测**信号（2026-09-19 标定，SVG 内部已排除） */
function page(vstack: [number, number, number], overlap: [number, number, number]): ViewportMetrics[] {
  const names = ['desktop', 'tablet', 'mobile'];
  const sizes: [number, number][] = [
    [1440, 900],
    [768, 1024],
    [375, 812],
  ];
  return names.map((viewport, i) => ({
    viewport,
    width: sizes[i][0],
    height: sizes[i][1],
    ready: true,
    signals: { vw: sizes[i][0], vstack: vstack[i], realOverlap: overlap[i], hOverflow: 0, clipped: 0 },
  }));
}

const ISSUES = page([0, 8, 10], [0, 0, 3]);
const TRANSPORT_WZ = page([3, 12, 12], [0, 12, 12]);
const GIT_REPO = page([0, 0, 0], [0, 0, 0]);

describe('narrowViolations（窄档劣化判据）', () => {
  test('干净页（阴性对照）零违规', () => {
    expect(narrowViolations(GIT_REPO)).toEqual([]);
  });

  test('issues：tablet 由竖排命中、mobile 由竖排+重叠命中', () => {
    const v = narrowViolations(ISSUES);
    expect(v.map((x) => x.viewport)).toEqual(['tablet', 'mobile']);
    expect(v[0].signals).toEqual(['vstack']);
    expect(v[1].signals).toEqual(['vstack', 'overlap']);
    expect(v[1].vstackDelta).toBe(10);
    expect(v[1].overlapDelta).toBe(3);
  });

  test('transport-wz：两窄档双信号命中（桌面档本身的 3 例竖排不构成违规）', () => {
    const v = narrowViolations(TRANSPORT_WZ);
    expect(v.map((x) => x.viewport)).toEqual(['tablet', 'mobile']);
    for (const item of v) expect(item.signals).toEqual(['vstack', 'overlap']);
    expect(v[0].vstackDelta).toBe(9);
  });

  test('桌面档自身的高计数不算违规（取 Δ 而非绝对值的语义）', () => {
    const desktopNoisy = page([9, 9, 9], [5, 5, 5]);
    expect(narrowViolations(desktopNoisy)).toEqual([]);
  });

  test('阈值边界：Δvstack=2 不违规、=3 违规；Δoverlap=0 不违规、=1 违规', () => {
    expect(narrowViolations(page([0, 2, 0], [0, 0, 0]))).toEqual([]);
    expect(narrowViolations(page([0, 3, 0], [0, 0, 0])).map((v) => v.viewport)).toEqual(['tablet']);
    expect(narrowViolations(page([0, 0, 0], [0, 1, 0])).map((v) => v.viewport)).toEqual(['tablet']);
  });

  test('无 desktop 基准时不下结论（信息不足）', () => {
    const noDesktop = [page([0, 1, 2], [0, 1, 2])[1]];
    expect(narrowViolations(noDesktop)).toEqual([]);
  });

  test('未就绪的档不参与判据', () => {
    const m = page([0, 9, 9], [0, 9, 9]);
    m[1].ready = false;
    m[2].ready = false;
    expect(narrowViolations(m)).toEqual([]);
  });
});

describe('scoreFrom（三信号 + 窄屏劣化扣项）', () => {
  test('零硬错误零违规 = 基准分（不再是无条件的 90 天花板）', () => {
    expect(scoreFrom({ hardErrors: 0, violations: 0 })).toBe(90);
  });

  test('每违规扣 5，封顶 4 档', () => {
    expect(scoreFrom({ hardErrors: 0, violations: 1 })).toBe(85);
    expect(scoreFrom({ hardErrors: 0, violations: 2 })).toBe(80);
    expect(scoreFrom({ hardErrors: 0, violations: 4 })).toBe(70);
    expect(scoreFrom({ hardErrors: 0, violations: 9 })).toBe(70);
  });

  test('三项叠加且有下限 60', () => {
    expect(scoreFrom({ hardErrors: 2, violations: 1 })).toBe(75);
    expect(scoreFrom({ hardErrors: 6, violations: 4 })).toBe(60);
    expect(scoreFrom({ hardErrors: 99, violations: 99 })).toBe(60);
  });

  test('实测两页将跌破 90（即门禁应为 FAIL），干净页保持 90', () => {
    expect(scoreFrom({ hardErrors: 0, violations: narrowViolations(ISSUES).length })).toBeLessThan(90);
    expect(scoreFrom({ hardErrors: 0, violations: narrowViolations(TRANSPORT_WZ).length })).toBe(80);
    expect(scoreFrom({ hardErrors: 0, violations: narrowViolations(GIT_REPO).length })).toBe(90);
  });
});

describe('dimensionsFrom（信号驱动，不编造）', () => {
  test('无信号的维度为 null（不再恒定 85）', () => {
    const dims = dimensionsFrom(GIT_REPO, []);
    const byName = Object.fromEntries(dims.map((d) => [d.name, d.score]));
    expect(byName.Color).toBeNull();
    expect(byName.Interactive).toBeNull();
    expect(byName.Content).toBeNull();
  });

  test('干净页 Responsive = 90；异常页随违规数下降', () => {
    const clean = Object.fromEntries(dimensionsFrom(GIT_REPO, []).map((d) => [d.name, d.score]));
    expect(clean.Responsive).toBe(90);
    const broken = Object.fromEntries(dimensionsFrom(ISSUES, narrowViolations(ISSUES)).map((d) => [d.name, d.score]));
    expect(broken.Responsive).toBe(70);
  });

  test('Typography/Layout 反映真实信号计数（有区分度）', () => {
    const clean = Object.fromEntries(dimensionsFrom(GIT_REPO, []).map((d) => [d.name, d.score]));
    const broken = Object.fromEntries(dimensionsFrom(TRANSPORT_WZ, narrowViolations(TRANSPORT_WZ)).map((d) => [d.name, d.score]));
    expect(clean.Typography).toBe(90);
    expect(broken.Typography).toBe(70);
    expect(clean.Layout).toBe(90);
    expect(broken.Layout).toBe(70);
  });

  test('全部未就绪时六维皆为 null', () => {
    const notReady = GIT_REPO.map((m) => ({ ...m, ready: false }));
    expect(dimensionsFrom(notReady, []).every((d) => d.score === null)).toBe(true);
  });

  test('minDimensionScore 跳过 null；全 null 时为 Infinity', () => {
    const dims = dimensionsFrom(ISSUES, narrowViolations(ISSUES));
    expect(minDimensionScore(dims)).toBe(70);
    expect(minDimensionScore(dimensionsFrom(GIT_REPO.map((m) => ({ ...m, ready: false })), []))).toBe(Infinity);
  });
});

describe('基线机制（存量过渡）', () => {
  const baseline = parseBaseline('# 注释\n\nPre-Proc/A/x/m-v2.html\nPre-Proc/B/y/m-v9.html#mobile\n');

  test('解析：忽略注释与空行', () => {
    expect(baseline.size).toBe(2);
    expect(baseline.has('Pre-Proc/A/x/m-v2.html')).toBe(true);
  });

  test('整体路径与精确到档的基线项均可命中', () => {
    const v = narrowViolations(ISSUES)[1];
    expect(isBaselined(baseline, 'Pre-Proc/A/x/m-v2.html', v)).toBe(true);
    expect(isBaselined(baseline, 'Pre-Proc/B/y/m-v9.html', { ...v, viewport: 'mobile' })).toBe(true);
    expect(isBaselined(baseline, 'Pre-Proc/B/y/m-v9.html', { ...v, viewport: 'tablet' })).toBe(false);
    expect(isBaselined(baseline, 'Pre-Proc/C/z/m-v1.html', v)).toBe(false);
  });
});

describe('探针表达式（结构性约束）', () => {
  test('桌面档常量与窄档集合互不重叠', () => {
    expect(['tablet', 'mobile'].includes(DESKTOP_VIEWPORT)).toBe(false);
  });

  test('重叠判定 MUST 先按裁剪祖先收敛 rect（可滚动条带屏外项不参与）', () => {
    // 判据：探针源码含裁剪收敛函数、按 overflow 判定、且重叠循环用收敛后的 rects
    expect(LAYOUT_PROBE).toContain('const clipRect =');
    expect(LAYOUT_PROBE).toContain("cs.overflowX !== 'visible'");
    expect(LAYOUT_PROBE).toContain('const rects = nodes.map(clipRect)');
    expect(LAYOUT_PROBE).toContain('const ra = rects[i], rb = rects[j]');
    // 反例守卫：不得回退为直接使用原始 getBoundingClientRect 参与重叠判定
    expect(LAYOUT_PROBE).not.toContain('const ra = a.getBoundingClientRect()');
  });
});
