/**
 * NodePalette — 节点工具箱（左侧面板）
 * 对应前端实现 FlowNodePalette.tsx
 */
import { NODE_TYPES, NODE_ICON } from '../flow-types';

export function NodePalette({ onAddNode }: { onAddNode: (type: string) => void }) {
  return (
    <div className="border-r border-border bg-card shrink-0 p-3" style={{ width: 180, overflowY: 'auto' }}>
      <p className="text-xs font-semibold mb-2 text-foreground">节点工具箱</p>
      {NODE_TYPES.filter((nt) => nt.type !== 'subflow').map((nt) => (
        <div
          key={nt.type}
          onClick={() => onAddNode(nt.type)}
          className="flex items-center gap-2 px-2 py-1 mb-1 border border-border rounded-md cursor-pointer bg-background"
        >
          <span style={{ width: 18, height: 18, borderRadius: 3, background: nt.color, display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 10, color: 'hsl(var(--primary-foreground))', fontWeight: 700, flexShrink: 0 }}>
            {NODE_ICON[nt.type] ?? '?'}
          </span>
          <div>
            <div className="text-xs font-medium">{nt.label}</div>
            <div className="text-xs text-muted-foreground">{nt.desc}</div>
          </div>
        </div>
      ))}
    </div>
  );
}
