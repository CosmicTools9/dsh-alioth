/**
 * FlowCanvas — 画布与连线渲染（不含调色板/工具栏）
 * 对应前端实现 FlowDesigner.tsx 画布部分
 */
import { useMemo } from 'react';
import type { FlowNode, FlowEdge } from '../flow-types';
import { buildConnections, elbowPath } from '../flow-geometry';
import { FlowNodeCard } from './FlowNodeCard';

export function FlowCanvas({
  nodes, edges, selectedIdx, onSelect, onMoveUp, onMoveDown, onDelete,
}: {
  nodes: FlowNode[];
  edges: FlowEdge[];
  selectedIdx: number | null;
  onSelect: (i: number) => void;
  onMoveUp: (i: number) => void;
  onMoveDown: (i: number) => void;
  onDelete: (i: number) => void;
}) {
  const connections = useMemo(() => buildConnections(nodes, edges), [nodes, edges]);

  return (
    <div className="flex-1 relative overflow-hidden" style={{ background: 'hsl(var(--card))' }}>
      {nodes.length === 0 ? (
        <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
          <div className="text-2xl mb-2 opacity-30">{'▦'}</div>
          <p className="font-semibold text-sm">空白画布</p>
          <p className="text-xs">从左侧工具箱拖入节点开始设计</p>
        </div>
      ) : (
        <div style={{ position: 'absolute', inset: 0, overflow: 'auto' }}>
          <div style={{ position: 'relative', width: Math.max(1000, ...nodes.map((n) => n.x + 250)), height: Math.max(600, ...nodes.map((n) => n.y + 200)) }}>
            {/* SVG connectors */}
            <svg style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', pointerEvents: 'none', overflow: 'visible' }}>
              <defs>
                <marker id="wf-arrow" viewBox="0 0 8 8" refX="8" refY="4" markerWidth={5} markerHeight={5} orient="auto">
                  <path d="M 0 0 L 8 4 L 0 8 z" fill="hsl(var(--primary))" />
                </marker>
              </defs>
              {connections.map((c, i) => (
                <g key={'cg-' + i}>
                  <path
                    id={'edge-path-' + i}
                    d={elbowPath(c.sx, c.sy, c.ex, c.ey, c.fs, c.ts)}
                    fill="none"
                    stroke={c.fromIdx === selectedIdx ? 'hsl(var(--primary))' : 'hsl(var(--border))'}
                    strokeWidth={2} strokeLinejoin="round" strokeLinecap="round"
                    markerEnd="url(#wf-arrow)"
                  />
                  {c.label && (() => {
                    const pw = c.label.length * 9 + 20;
                    return (
                      <>
                        <rect
                          x={c.labelX - pw / 2} y={c.labelY - 8}
                          width={pw} height={16} rx={4} ry={4}
                          fill="hsl(var(--background))"
                          stroke="hsl(var(--border))"
                          strokeWidth={0.5}
                        />
                        <text
                          x={c.labelX} y={c.labelY}
                          textAnchor="middle" dominantBaseline="central"
                          fontSize={10} fill="hsl(var(--foreground))"
                          style={{ userSelect: 'none', pointerEvents: 'none' }}
                        >
                          {c.label}
                        </text>
                      </>
                    );
                  })()}
                </g>
              ))}
            </svg>

            {/* Node cards */}
            {nodes.map((n, i) => (
              <FlowNodeCard
                key={n.id}
                node={n}
                selected={selectedIdx === i}
                onSelect={() => onSelect(i)}
                onMoveUp={() => onMoveUp(i)}
                onMoveDown={() => onMoveDown(i)}
                onDelete={() => onDelete(i)}
              />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
