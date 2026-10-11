/**
 * FlowNodeCard — 流程节点卡片（画布内）
 * 对应前端实现 FlowDesigner.tsx 内节点渲染；含 4 侧锚点、网关徽标、操作按钮
 */
import type { FlowNode } from '../flow-types';
import { NODE_TYPES, NODE_ICON } from '../flow-types';
import { getSize } from '../flow-geometry';

export function FlowNodeCard({
  node, selected, onSelect, onMoveUp, onMoveDown, onDelete,
}: {
  node: FlowNode;
  selected: boolean;
  onSelect: () => void;
  onMoveUp?: () => void;
  onMoveDown?: () => void;
  onDelete?: () => void;
}) {
  const tc = NODE_TYPES.find((t) => t.type === node.type);
  const color = tc?.color ?? 'hsl(var(--muted-foreground))';
  const sz = getSize(node.type);
  const isTerminal = node.type === 'start' || node.type === 'end';
  const isGateway = node.type === 'condition' || node.type === 'parallel';
  const borderColor = selected ? 'hsl(var(--primary))' : color;

  let meta = '';
  if (node.type === 'approval') meta = (node.role ?? '未指定') + ' · SLA ' + (node.sla ?? 24) + 'h';
  else if (node.type === 'condition') meta = node.expr ?? '点击配置';

  const actions = [
    onMoveUp ? { l: '▲', a: onMoveUp } : null,
    onMoveDown ? { l: '▼', a: onMoveDown } : null,
    onDelete ? { l: '✕', a: onDelete, d: true } : null,
  ].filter(Boolean) as Array<{ l: string; a: () => void; d?: boolean }>;

  return (
    <div style={{ position: 'absolute', left: node.x, top: node.y, zIndex: 10 }}>
      <div
        style={{
          width: sz.w, minHeight: isTerminal ? sz.h : Math.max(sz.h, 60),
          border: '2px solid ' + borderColor,
          borderRadius: isTerminal ? 20 : 8,
          background: isTerminal
            ? (node.type === 'start' ? 'hsl(var(--bg-success))' : 'hsl(var(--bg-destructive))')
            : 'hsl(var(--card))',
          position: 'relative', cursor: 'pointer',
          boxShadow: selected
            ? '0 0 0 3px hsl(var(--primary) / 0.25), 0 2px 8px hsl(var(--foreground) / 0.1)'
            : '0 1px 3px hsl(var(--foreground) / 0.06)',
        }}
        onClick={onSelect}
      >
        {/* Gateway diamond badge */}
        {isGateway && (
          <div style={{ position: 'absolute', top: -10, left: '50%', marginLeft: -11, width: 22, height: 22, transform: 'rotate(45deg)', backgroundColor: color, display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 10 }}>
            <span style={{ transform: 'rotate(-45deg)', fontSize: 11, fontWeight: 700, color: 'hsl(var(--primary-foreground))' }}>
              {node.type === 'condition' ? '×' : '+'}
            </span>
          </div>
        )}

        {/* Node head */}
        <div className="flex items-center gap-2 px-2 pt-1">
          <span style={{ width: 18, height: 18, borderRadius: 3, background: color, color: 'hsl(var(--primary-foreground))', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', fontSize: 10, fontWeight: 700, flexShrink: 0 }}>
            {NODE_ICON[node.type] ?? '?'}
          </span>
          <div className="min-w-0 flex-1">
            <div style={{ fontSize: 10, color, fontWeight: 600 }}>{tc?.label}</div>
            <div style={{ fontSize: 11, fontWeight: 500, color: 'hsl(var(--foreground))', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{node.label}</div>
          </div>
        </div>

        {/* Meta line */}
        {meta && (
          <div className="px-2 pb-1" style={{ fontSize: 9.5, color: 'hsl(var(--muted-foreground))', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
            {meta}
          </div>
        )}

        {/* Action buttons */}
        {actions.length > 0 && (
          <div className="flex justify-end gap-1 px-1 pb-1">
            {actions.map((b) => (
              <button
                key={b.l}
                onClick={(e) => { e.stopPropagation(); b.a(); }}
                style={{
                  width: 16, height: 16, display: 'flex', alignItems: 'center', justifyContent: 'center',
                  border: '1px solid hsl(var(--border))', borderRadius: 2,
                  background: b.d ? 'hsl(var(--bg-destructive))' : 'transparent',
                  cursor: 'pointer', fontSize: 8,
                  color: b.d ? 'hsl(var(--destructive))' : 'hsl(var(--muted-foreground))',
                }}
              >
                {b.l}
              </button>
            ))}
          </div>
        )}

        {/* Port indicators (4 sides, uniform circle geometry) */}
        {['top', 'right', 'bottom', 'left'].map((s) => (
          <div
            key={s}
            style={{
              position: 'absolute', width: 10, height: 10, borderRadius: '50%',
              background: 'hsl(var(--card))', border: '2px solid ' + borderColor,
              zIndex: 5, cursor: 'crosshair',
              ...(s === 'top' ? { top: -5, left: '50%', marginLeft: -5 } : {}),
              ...(s === 'right' ? { right: -5, top: '50%', marginTop: -5 } : {}),
              ...(s === 'bottom' ? { bottom: -5, left: '50%', marginLeft: -5 } : {}),
              ...(s === 'left' ? { left: -5, top: '50%', marginTop: -5 } : {}),
            }}
          />
        ))}
      </div>
    </div>
  );
}
