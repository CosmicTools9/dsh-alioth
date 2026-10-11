/**
 * workflow-designer — Block 组装入口
 * 组件拆分对齐前端实现：FlowCanvas / FlowNodeCard / NodePalette / NodeInspector / DesignerToolbar
 * 数据模型：nodes + 显式 edges（模板无 edges 时顺序连接 fallback）
 */
import { useState, useMemo } from 'react';
import { createPrototypeLifecycle } from '../_kit/lifecycle';
import type { FlowContext, FlowEdge, FlowNode, FlowTemplate } from './flow-types';
import { sequentialEdges } from './flow-geometry';
import { FlowGallery } from './components/FlowGallery';
import { FlowCanvas } from './components/FlowCanvas';
import { NodePalette } from './components/NodePalette';
import { NodeInspector } from './components/NodeInspector';
import { DesignerToolbar } from './components/DesignerToolbar';

export default function Block() {
  const [screen, setScreen] = useState<'gallery' | 'canvas'>('gallery');
  const [editTpl, setEditTpl] = useState<FlowTemplate | null>(null);
  const [nodes, setNodes] = useState<FlowNode[]>([]);
  const [edges, setEdges] = useState<FlowEdge[]>([]);
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(true);
  const [propOpen, setPropOpen] = useState(true);

  // 字段来源：当前模板流程级 context（预绑定 event/task 自动识别），不随节点增删变化
  const context = editTpl?.context;
  const contextFields = useMemo(() => context?.fields ?? [], [context]);

  function enterCanvas(tpl: FlowTemplate) {
    const ns = tpl.nodes.map((n) => ({ ...n, type: n.type as string, x: n.x ?? 0, y: n.y ?? 0 })) as FlowNode[];
    setEditTpl(tpl);
    setNodes(ns);
    setEdges(tpl.edges ?? sequentialEdges(ns));
    setSelectedIdx(null);
    setScreen('canvas');
  }

  function handleAddNode(type: string) {
    const idx = nodes.length;
    const id = 'n' + Date.now();
    setNodes((prev) => [...prev, {
      id, type, label: '新节点', x: 80 + (idx % 4) * 200, y: 80 + Math.floor(idx / 4) * 120,
    } as FlowNode]);
    setEdges((prev) => (nodes.length > 0 ? [...prev, { from: nodes[nodes.length - 1].id, to: id }] : prev));
  }

  function updateNode(i: number, patch: Partial<FlowNode>) {
    setNodes((prev) => prev.map((n, idx) => (idx === i ? { ...n, ...patch } as FlowNode : n)));
  }

  /** 交换第 i 个节点与其前/后邻居在流程拓扑中的位置（数组顺序不变，仅重接边） */
  function swapInFlow(i: number, j: number) {
    if (j < 0 || j >= nodes.length) return;
    const idA = nodes[i].id, idB = nodes[j].id;
    setEdges((prev) =>
      prev.map((e) => ({
        from: e.from === idA ? idB : e.from === idB ? idA : e.from,
        to: e.to === idA ? idB : e.to === idB ? idA : e.to,
      })),
    );
    setSelectedIdx(j);
  }

  function deleteNode(i: number) {
    const del = nodes[i];
    setNodes((prev) => prev.filter((_, idx) => idx !== i));
    setEdges((prev) => {
      const incoming = prev.filter((e) => e.to === del.id);
      const outgoing = prev.filter((e) => e.from === del.id);
      const next = prev.filter((e) => e.from !== del.id && e.to !== del.id);
      for (const inc of incoming) {
        for (const out of outgoing) {
          next.push({ from: inc.from, to: out.to });
        }
      }
      return next;
    });
    setSelectedIdx(null);
  }

  if (screen === 'gallery') {
    return <FlowGallery onEdit={enterCanvas} />;
  }

  const avgSla = nodes.length > 0 ? Math.round(nodes.reduce((s, n) => s + (n.sla ?? 0), 0) / nodes.length) : 0;

  return (
    <div className="flex flex-col h-full w-full" style={{ background: 'hsl(var(--muted) / 0.3)' }}>
      <DesignerToolbar
        tpl={editTpl}
        nodeCount={nodes.length}
        avgSla={avgSla}
        paletteOpen={paletteOpen}
        propOpen={propOpen}
        onBack={() => setScreen('gallery')}
        onTogglePalette={() => setPaletteOpen((p) => !p)}
        onToggleProps={() => setPropOpen((p) => !p)}
      />
      <div className="flex flex-1 min-h-0 overflow-hidden">
        {paletteOpen && <NodePalette onAddNode={handleAddNode} />}
        <FlowCanvas
          nodes={nodes}
          edges={edges}
          selectedIdx={selectedIdx}
          onSelect={setSelectedIdx}
          onMoveUp={(i) => swapInFlow(i, i - 1)}
          onMoveDown={(i) => swapInFlow(i, i + 1)}
          onDelete={deleteNode}
        />
        {propOpen && (
          <NodeInspector
            node={selectedIdx !== null ? nodes[selectedIdx] : null}
            index={selectedIdx}
            allFields={contextFields}
            context={context}
            edges={edges}
            onChange={updateNode}
            onEdgesChange={setEdges}
            onClose={() => setPropOpen(false)}
          />
        )}
      </div>
    </div>
  );
}

Block.displayName = 'workflow-designer';
export const { bootstrap, mount, unmount } = createPrototypeLifecycle({ name: 'workflow-designer', App: Block });
