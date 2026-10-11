/**
 * GalleryMiniPreview — 流程模板迷你预览（分支感知）
 * 对应 @alioth/components 的 GalleryMiniPreview。
 * 顺序链水平渲染；某节点有多条出边（并行/条件网关）时，分支目标垂直堆叠表达，
 * 避免把并行分支误导为串行步骤。
 */
import type { FlowTemplate, FlowNode } from '../flow-types';
import { NODE_TYPES } from '../flow-types';
import { sequentialEdges } from '../flow-geometry';

function nodeColor(type?: string): string {
  return NODE_TYPES.find((t) => t.type === type)?.color ?? 'hsl(var(--muted-foreground))';
}

function Chip({ label, color }: { label: string; color: string }) {
  return (
    <span
      className="px-1 py-1 rounded text-xs"
      style={{ background: `color-mix(in srgb, ${color} 10%, transparent)`, color }}
    >
      {label}
    </span>
  );
}

const Arrow = () => <span className="text-xs text-muted-foreground">{'→'}</span>;

export function GalleryMiniPreview({ tpl }: { tpl: FlowTemplate }) {
  const nodes = tpl.nodes as FlowNode[];
  const edges = tpl.edges ?? sequentialEdges(nodes);
  const outById: Record<string, string[]> = {};
  for (const e of edges) (outById[e.from] ??= []).push(e.to);
  const isBranchTarget = new Set(
    Object.values(outById).filter((t) => t.length > 1).flat(),
  );
  const byId = new Map(nodes.map((n) => [n.id, n]));

  // 主链：从头节点沿单一出边走；遇到分叉时分支组内联渲染后跳到汇聚点
  const rendered = new Set<string>();
  const first = nodes[0];
  if (!first) return null;

  const items: JSX.Element[] = [];
  let cur: FlowNode | undefined = first;
  let guard = 0;
  while (cur && guard++ < 20) {
    if (rendered.has(cur.id)) break;
    rendered.add(cur.id);
    const outs = (outById[cur.id] ?? []).filter((id) => byId.has(id));
    if (outs.length > 1) {
      // 分叉：当前节点 + 分支组（垂直堆叠）
      const branches = outs.map((id) => byId.get(id)!).slice(0, 4);
      branches.forEach((b) => rendered.add(b.id));
      items.push(
        <span key={cur.id} className="flex items-center gap-1">
          <Chip label={cur.label ?? ''} color={nodeColor(cur.type)} />
          <Arrow />
          <span className="flex flex-col gap-0.5 border-l-2 pl-1" style={{ borderColor: nodeColor(cur.type) }}>
            {branches.map((b) => (
              <Chip key={b.id} label={b.label ?? ''} color={nodeColor(b.type)} />
            ))}
          </span>
        </span>,
      );
      // 汇聚点：所有分支的共同后继
      const nextSet = branches.map((b) => (outById[b.id] ?? [])[0]).filter(Boolean);
      const converge = nextSet.every((id) => id === nextSet[0]) ? nextSet[0] : undefined;
      cur = converge ? byId.get(converge) : undefined;
      if (cur) items.push(<Arrow key={'arr-' + cur.id} />);
    } else {
      items.push(
        <span key={cur.id} className="flex items-center gap-1">
          <Chip label={cur.label ?? ''} color={nodeColor(cur.type)} />
          {outs.length === 1 && <Arrow />}
        </span>,
      );
      cur = outs.length === 1 ? byId.get(outs[0]) : undefined;
    }
    if (rendered.size >= 6) break;
  }

  const remaining = nodes.filter((n) => !rendered.has(n.id) && !isBranchTarget.has(n.id)).length;

  return (
    <div className="flex items-center gap-1 flex-wrap">
      {items}
      {remaining > 0 && <span className="text-xs text-muted-foreground">...</span>}
    </div>
  );
}
