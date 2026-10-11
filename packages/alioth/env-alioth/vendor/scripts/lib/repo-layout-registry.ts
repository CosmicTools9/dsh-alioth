/**
 * repo-layout-registry.ts — `REPO_LAYOUT_SPEC.md` 登记表的**单一解析实现**
 *
 * 规约来源:
 *   - openspec/specs/repo-layout（条目登记面 = 正本表格；解析 MUST 走 mdast，禁按 `|` 拆行）
 *   - docs/specs/REPO_LAYOUT_SPEC.md §7（条目名提取 MUST 经 Markdown parser：
 *     `NO_REGEX_FOR_PARSING.md` 判据——Markdown 属结构化容器）
 *
 * 消费者（MUST 复用本模块，MUST NOT 各写一份表格解析）：
 *   · `scripts/check/check-repo-layout.ts` —— 顶层条目登记判定；
 *   · `scripts/check/check-runtime-face-not-tracked.ts` —— 运行态面声明面一致性 + 入库判据。
 */
import { markdown } from './parsers';

/** mdast 节点最小结构面（只声明消费方用到的字段；parser 由 `parsers.ts` 集中提供）。 */
export interface MdNode {
  type: string;
  value?: string;
  children?: MdNode[];
}

export interface RegistryRow {
  entry: string;
  /** 正本「跟踪态」原文 */
  tracking: string;
  /** 跟踪态=跟踪 → 磁盘缺失即违规；忽略/混合 → 允许缺席 */
  expectOnDisk: boolean;
}

function* walk(node: MdNode): Generator<MdNode> {
  yield node;
  for (const child of node.children ?? []) yield* walk(child);
}

/** 节点纯文本（text/inlineCode 叶子拼接；表格单元格与表头共用） */
function plainText(node: MdNode): string {
  if (node.type === 'text' || node.type === 'inlineCode') return node.value ?? '';
  return (node.children ?? []).map(plainText).join('');
}

/** 单元格 → 条目名：去空白与反引号（mdast 已剥壳，此处兼容手写文本形态），去尾部 `/` */
function cellEntry(cell: MdNode): string {
  return plainText(cell)
    .trim()
    .replace(/^`+|`+$/g, '')
    .trim()
    .replace(/\/+$/, '');
}

/**
 * 登记面抽取：遍历 AST 的全部 `table` 节点，按表头映射「条目」列。
 * 非登记表（无「条目」表头）跳过；登记表缺「跟踪态」列 → 报错（拒绝静默放行）。
 * 零条目 → 报错（正本腐化或解析器失效，同样不得静默放行）。
 */
export function registryFromSpec(specText: string): Map<string, RegistryRow> {
  const tree = markdown.fromMarkdown(specText) as unknown as MdNode;
  const rows = new Map<string, RegistryRow>();
  for (const node of walk(tree)) {
    if (node.type !== 'table') continue;
    const [header, ...body] = node.children ?? [];
    if (!header) continue;
    const heads = (header.children ?? []).map(cellEntry);
    const entryIdx = heads.indexOf('条目');
    if (entryIdx === -1) continue;
    const trackIdx = heads.indexOf('跟踪态');
    if (trackIdx === -1) {
      throw new Error(`登记表缺「跟踪态」列（表头：${heads.join(' / ')}）`);
    }
    for (const row of body) {
      const cells = row.children ?? [];
      const entry = cellEntry(cells[entryIdx] ?? { type: 'tableCell' });
      if (!entry) continue;
      const tracking = cellEntry(cells[trackIdx] ?? { type: 'tableCell' });
      rows.set(entry, { entry, tracking, expectOnDisk: tracking.startsWith('跟踪') });
    }
  }
  if (rows.size === 0) {
    throw new Error('未解析出任何登记条目（表格解析失败——拒绝静默放行）');
  }
  return rows;
}
