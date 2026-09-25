/**
 * layout-signals.ts — 视觉验证的机械布局信号与评分（纯函数，无副作用）
 *
 * 背景：`scripts/visual-verify.ts` 的评分原先只看 console 硬错误，布局零判据 ⇒
 * 窄屏明显破版的原型与干净原型拿到相同的 Responsive=90（假阳性）；维度分还用
 * 固定 `Math.min(score,85)` 编造 ⇒ 五份报告六维恒为 90/85/85/90/85/90。
 *
 * 本模块把"布局好坏"落到可机械判定的信号，并把 Responsive 的语义定为
 * **窄档相对桌面档的劣化**（Δ），而非绝对计数——实测依据（三页 × 三档）：
 *
 *   | 页面                      | vstack desktop/tablet/mobile | realOverlap 同序 |
 *   |---------------------------|------------------------------|------------------|
 *   | issues/m-v2（窄屏破版）    | 0 / 8 / 10                   | 0 / 0 / 3        |
 *   | transport-wz/m-v92（同）   | 3 / 12 / 12                  | 0 / 12 / 12      |
 *   | git-repo/m-v1（阴性对照）  | 0 / 0 / 0                    | 0 / 0 / 0        |
 *
 * 取 Δ 的原因：transport-wz 桌面档本身有 3 例竖排（密集表格的列宽取舍），绝对阈值会误伤。
 */

/** 单档视口的布局信号（页面内一次求值的结果） */
export interface ViewportSignals {
  /** 视口宽度（documentElement.clientWidth） */
  vw: number;
  /** 逐字竖排的叶子文本元素数 */
  vstack: number;
  /** 非 SVG 元素的重叠对数（**按裁剪祖先收敛后**可见部分的交面积 > 较小者 25%） */
  realOverlap: number;
  /** 文档级横向溢出像素（遥测：实测三页全 0，shell 隐藏溢出） */
  hOverflow: number;
  /** 文本裁切元素数（遥测） */
  clipped: number;
  /** 取证样本（≤5 条） */
  samples?: { vstack: string[]; overlap: string[] };
}

/** 一档视口的完整度量 */
export interface ViewportMetrics {
  viewport: string;
  width: number;
  height: number;
  ready: boolean;
  signals: ViewportSignals;
}

/** 命中窄屏劣化的视口（Δ 维度） */
export interface NarrowViolation {
  viewport: string;
  vstackDelta: number;
  overlapDelta: number;
  /** 该档触发硬判据的信号名 */
  signals: string[];
}

/** 维度分：有信号支撑给实数，无信号支撑给 null（"未测量"，不编造） */
export interface DimensionScore {
  name: string;
  score: number | null;
}

export const DESKTOP_VIEWPORT = 'desktop';
/** 窄档（相对 desktop 取 Δ 的档位） */
export const NARROW_VIEWPORTS = ['tablet', 'mobile'] as const;

/** 逐字竖排的窄屏劣化阈值（实测：干净页 Δ=0，异常页 Δ≥8 ⇒ 3 留 2.6 倍余量） */
export const VSTACK_DELTA_THRESHOLD = 3;
/** 真实重叠的窄屏劣化阈值（实测：干净页 Δ=0，异常页 Δ≥3） */
export const OVERLAP_DELTA_THRESHOLD = 1;

/** 评分参数（与 visual-verify.ts 的三信号模型对齐） */
export const SCORE_BASE = 90;
export const SCORE_MIN = 60;

/** 布局判据/探针版本。**探针语义变更（阈值、裁剪感知、字段集）时 MUST 递增**：
 * 报告记录的版本与该值不一致 ⇒ `check-visual-verify` 判 FAIL（提示重跑并重算基线），
 * 因为信号灵敏度变了，跨版本比较的数字没有可比性（历史教训：探针升级把存量退化读成「新增」、
 * 把重跑读成「修复」）。 */
export const LAYOUT_PROBE_VERSION = 1;
export const HARD_ERROR_DEDUCTION = 5;
export const HARD_ERROR_CAP = 6;
/** 每个命中窄屏劣化的视口扣分与封顶 */
export const NARROW_DEDUCTION = 5;
export const NARROW_CAP = 4;

/**
 * 页面内布局探针：一次求值返回 `ViewportSignals`。
 *
 * 约束（均有实测依据）：
 * - **排除 `<svg>` 及其后代**：桌面档 2 对重叠全是 `path×path` / `path×circle`（图标内部图元）。
 * - **裁剪感知**：重叠判定前先按 `overflow` 裁剪祖先（hidden/auto/scroll/clip）收敛 rect，
 *   屏外部分不计入 —— 否则可滚动条带（TopBar `ModuleTabs` 的 `overflow-x-auto`）内**屏外**项
 *   会与相邻区域几何相交成假阳（2026-09-22 实测：375px Δoverlap=12，实拍无可见压盖）。
 *   收敛只可能**减少**重叠对数（对未裁剪页面无影响）。
 * - **只读**：不修改 DOM（采集前由调用方 `Emulation.setScrollbarsHidden` 处理滚动条侵蚀）。
 * - **有界**：元素采样上限与命中上限，避免 O(n²) 失控（不进 ready 轮询循环）。
 */
export const LAYOUT_PROBE = `(() => {
  const de = document.documentElement;
  const vw = de.clientWidth;
  const vis = (el) => {
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && cs.visibility !== 'hidden' && cs.display !== 'none' && Number(cs.opacity) > 0.05;
  };
  const inSvg = (el) => {
    const tag = el.tagName ? el.tagName.toLowerCase() : '';
    if (tag === 'svg' || tag === 'path' || tag === 'circle' || tag === 'g' || tag === 'use') return true;
    return !!(el.closest && el.closest('svg'));
  };
  const label = (el) => {
    const cls = el.className && typeof el.className === 'string' ? '.' + el.className.trim().split(/\\s+/).slice(0, 2).join('.') : '';
    return (el.tagName.toLowerCase() + (el.id ? '#' + el.id : '') + cls).slice(0, 60);
  };

  const hOverflow = Math.max(0, de.scrollWidth - vw);

  // 裁剪感知：元素若被最近的 overflow 裁剪祖先（hidden/auto/scroll/clip）截断，屏外部分不参与
  // 重叠判定。缺此收敛时，可滚动条带（如 TopBar 的 ModuleTabs，overflow-x-auto）内**屏外**的
  // 项仍以完整 rect 参与判定，与相邻区域几何相交 ⇒ 假阳（实测 2026-09-22：375px 下标签按钮 ×
  // 右簇动作按钮 Δoverlap=12，而同档实拍无任何可见压盖/裁切）。
  const clipRect = (el) => {
    let r = el.getBoundingClientRect();
    const elCs = getComputedStyle(el);
    // 固定/绝对定位的元素可能逃出祖先裁剪（包含块在裁剪祖先之上）→ 不可用祖先 rect 收敛，
    // 否则会掩盖真实互叠（如浮层/下拉）。保守取原始 rect。
    if (elCs.position === 'fixed') return r;
    let positioned = elCs.position !== 'static';
    let p = el.parentElement;
    while (p) {
      const cs = getComputedStyle(p);
      const pPositioned = cs.position !== 'static';
      // 元素（或途中祖先）已建立定位包含块 → 其上方的裁剪祖先不再作用于本元素
      if (positioned && pPositioned) break;
      const cx = cs.overflowX !== 'visible';
      const cy = cs.overflowY !== 'visible';
      if (cx || cy) {
        const pr = p.getBoundingClientRect();
        const left = cx ? Math.max(r.left, pr.left) : r.left;
        const right = cx ? Math.min(r.right, pr.right) : r.right;
        const top = cy ? Math.max(r.top, pr.top) : r.top;
        const bottom = cy ? Math.min(r.bottom, pr.bottom) : r.bottom;
        r = { left, right, top, bottom, width: Math.max(0, right - left), height: Math.max(0, bottom - top) };
        if (r.width <= 0 || r.height <= 0) return r;
      }
      if (pPositioned) positioned = true;
      p = p.parentElement;
    }
    return r;
  };

  const sel = 'a,button,input,select,textarea,[role="button"],[data-testid],h1,h2,h3,td,th,header *,nav *';
  const nodes = Array.from(document.querySelectorAll(sel)).filter((el) => vis(el) && !inSvg(el)).slice(0, 300);
  const rects = nodes.map(clipRect);
  const overlapSamples = [];
  for (let i = 0; i < nodes.length; i++) {
    for (let j = i + 1; j < nodes.length; j++) {
      const a = nodes[i], b = nodes[j];
      if (a.contains(b) || b.contains(a)) continue;
      const ra = rects[i], rb = rects[j];
      // 被裁剪到不可见（宽/高 ≤1px）的盒不参与判定
      if (ra.width <= 1 || ra.height <= 1 || rb.width <= 1 || rb.height <= 1) continue;
      const ix = Math.min(ra.right, rb.right) - Math.max(ra.left, rb.left);
      const iy = Math.min(ra.bottom, rb.bottom) - Math.max(ra.top, rb.top);
      if (ix <= 1 || iy <= 1) continue;
      const small = Math.min(ra.width * ra.height, rb.width * rb.height);
      if (small <= 0 || (ix * iy) / small <= 0.25) continue;
      if (overlapSamples.length < 12) overlapSamples.push(label(a) + ' × ' + label(b));
      if (overlapSamples.length >= 12) break;
    }
    if (overlapSamples.length >= 12) break;
  }

  const textEls = Array.from(document.querySelectorAll('td,th,span,div,label,h1,h2,h3,a,li')).filter((el) => vis(el) && !inSvg(el)).slice(0, 600);
  const vstackSamples = [];
  let clipped = 0;
  for (const el of textEls) {
    if (el.children.length > 0) continue;
    const cs = getComputedStyle(el);
    const t = (el.textContent || '').trim().replace(/\\s+/g, '');
    if (t.length >= 2) {
      const fs = parseFloat(cs.fontSize) || 14;
      // 行数改用文本自身的行盒（Range.getClientRects）而非「盒高 ÷ 行高」：
      // 后者把「高 22px + line-height:1（=11px）」的胶囊徽章读成 lines=2 的假竖排
      // （实测 .badge 的 clientHeight/lineHeight 恒为 2，与是否换行无关）。
      const range = document.createRange();
      range.selectNodeContents(el);
      // 行数 = 文本行盒的**不同纵向位置**数。直接取 getClientRects().length 会把
      // 「同一行内的多个 inline 盒」读成多行：图标 + 文字的胶囊徽章、被拆成多盒的
      // 短文本（如 0%）都恒 ≥2 ⇒ 假竖排（2026-09-23 实测 .badge[0%] w=33 fs=11
      // 的 lines=2 与是否换行无关）。按 top 去重后只数真实文本行。
      const lines = new Set(Array.from(range.getClientRects()).map((r) => Math.round(r.top))).size;
      range.detach();
      if (lines >= 2 && el.clientWidth > 0 && el.clientWidth < 4 * fs && lines >= Math.min(t.length, 4)) {
        if (vstackSamples.length < 12) vstackSamples.push(label(el) + '[' + t.slice(0, 6) + ' w=' + Math.round(el.clientWidth) + ' fs=' + Math.round(fs) + ' lines=' + lines + ']');
      }
    }
    if (el.scrollWidth > el.clientWidth + 2 && (cs.overflow === 'hidden' || cs.overflowX === 'hidden' || cs.textOverflow === 'ellipsis')) clipped++;
  }

  return { vw, vstack: vstackSamples.length, realOverlap: overlapSamples.length, hOverflow, clipped, samples: { vstack: vstackSamples.slice(0, 5), overlap: overlapSamples.slice(0, 5) } };
})()`;

/** 命中硬判据的窄档（Δ 维度）；无 desktop 基准时返回空（信息不足不下结论） */
export function narrowViolations(metrics: ViewportMetrics[]): NarrowViolation[] {
  const desktop = metrics.find((m) => m.viewport === DESKTOP_VIEWPORT && m.ready);
  if (!desktop) return [];
  const out: NarrowViolation[] = [];
  for (const m of metrics) {
    if (!m.ready) continue;
    if (!(NARROW_VIEWPORTS as readonly string[]).includes(m.viewport)) continue;
    const vstackDelta = Math.max(0, m.signals.vstack - desktop.signals.vstack);
    const overlapDelta = Math.max(0, m.signals.realOverlap - desktop.signals.realOverlap);
    const signals: string[] = [];
    if (vstackDelta >= VSTACK_DELTA_THRESHOLD) signals.push('vstack');
    if (overlapDelta >= OVERLAP_DELTA_THRESHOLD) signals.push('overlap');
    if (signals.length > 0) out.push({ viewport: m.viewport, vstackDelta, overlapDelta, signals });
  }
  return out;
}

/** 总分：三信号模型 + 窄屏劣化扣项 */
export function scoreFrom(input: { hardErrors: number; violations: number }): number {
  const hard = HARD_ERROR_DEDUCTION * Math.min(input.hardErrors, HARD_ERROR_CAP);
  const narrow = NARROW_DEDUCTION * Math.min(input.violations, NARROW_CAP);
  return Math.max(SCORE_MIN, SCORE_BASE - hard - narrow);
}

/**
 * 六维分：仅保留有机械信号支撑的维度；无信号者置 `null`（"未测量"），
 * 不再用固定 `Math.min(score, 85)` 编造。
 */
export function dimensionsFrom(metrics: ViewportMetrics[], violations: NarrowViolation[]): DimensionScore[] {
  const ready = metrics.filter((m) => m.ready);
  const hasSignals = ready.length > 0;
  if (!hasSignals) {
    return ['Layout', 'Typography', 'Color', 'Responsive', 'Interactive', 'Content'].map((name) => ({ name, score: null }));
  }
  const maxOverlap = Math.max(...ready.map((m) => m.signals.realOverlap));
  const maxVstack = Math.max(...ready.map((m) => m.signals.vstack));
  const dim = (name: string, penalty: number): DimensionScore => ({
    name,
    score: Math.max(SCORE_MIN, SCORE_BASE - penalty),
  });
  return [
    dim('Layout', 5 * Math.min(maxOverlap, 4)),
    dim('Typography', 5 * Math.min(maxVstack, 4)),
    { name: 'Color', score: null },
    dim('Responsive', 10 * Math.min(violations.length, 4)),
    { name: 'Interactive', score: null },
    { name: 'Content', score: null },
  ];
}

/** 维度最低分：跳过 `null`（未测量）——与 check-visual-verify.ts 既有语义一致 */
export function minDimensionScore(dims: DimensionScore[]): number {
  const nums = dims.map((d) => d.score).filter((s): s is number => typeof s === 'number');
  return nums.length === 0 ? Infinity : Math.min(...nums);
}

/** 解析基线文件（每行：`<相对路径>` 或 `<相对路径>#<viewport>`；`#` 起为注释） */
export function parseBaseline(text: string): Set<string> {
  const out = new Set<string>();
  for (const raw of text.split('\n')) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    out.add(line);
  }
  return out;
}

/** 违规是否已在基线内（基线项可为整体路径，或 `路径#视口` 精确到档） */
export function isBaselined(baseline: Set<string>, reportPath: string, violation: NarrowViolation): boolean {
  return baseline.has(reportPath) || baseline.has(`${reportPath}#${violation.viewport}`);
}
