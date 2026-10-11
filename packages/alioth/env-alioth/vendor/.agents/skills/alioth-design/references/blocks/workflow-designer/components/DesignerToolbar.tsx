/**
 * DesignerToolbar — 设计器顶部工具栏 + 校验状态条
 * 对应前端实现 FlowToolbar.tsx
 */
import { Badge } from '../../_kit/ui';
import type { FlowTemplate } from '../flow-types';

const BTN = 'px-2 py-1 text-xs border border-border rounded-md cursor-pointer bg-transparent';
const BTN_PRIMARY = 'px-2 py-1 text-xs border-none rounded-md cursor-pointer bg-primary text-primary-foreground font-medium';

export function DesignerToolbar({
  tpl, nodeCount, avgSla, paletteOpen, propOpen,
  onBack, onTogglePalette, onToggleProps,
}: {
  tpl: FlowTemplate | null;
  nodeCount: number;
  avgSla: number;
  paletteOpen: boolean;
  propOpen: boolean;
  onBack: () => void;
  onTogglePalette: () => void;
  onToggleProps: () => void;
}) {
  return (
    <>
      <div className="flex items-center gap-2 px-4 py-2 border-b border-border bg-card shrink-0">
        <button onClick={onBack} className={BTN}>{'←'} 返回</button>
        <div className="w-px h-5 bg-border mx-1" />
        <input defaultValue={tpl?.name ?? ''} className="h-8 px-2 text-xs border border-border rounded-md" />
        {tpl && <Badge text={tpl.status === 'published' ? '已发布' : '草稿'} variant={tpl.status === 'published' ? 'success' : 'default'} />}
        <div className="flex-1" />
        <button onClick={onTogglePalette} className={'px-2 py-1 text-xs border border-border rounded-md cursor-pointer ' + (paletteOpen ? 'bg-primary/10' : 'bg-transparent')}>{'▦'} 工具箱</button>
        <button onClick={onToggleProps} className={'px-2 py-1 text-xs border border-border rounded-md cursor-pointer ' + (propOpen ? 'bg-primary/10' : 'bg-transparent')}>{'☰'} 属性</button>
        <div className="w-px h-5 bg-border mx-1" />
        <span className="text-xs text-muted-foreground">100%</span>
        <button className={BTN}>+</button>
        <button className={BTN}>-</button>
        <div className="w-px h-5 bg-border mx-1" />
        <button className={BTN}>{'↶'} 撤销</button>
        <button className={BTN}>{'↷'} 重做</button>
        <button className={BTN_PRIMARY}>{'✔'} 校验</button>
        <button className={BTN_PRIMARY}>{'▶'} 模拟</button>
      </div>
      <div className="flex items-center gap-3 px-4 py-1 text-xs border-b border-border shrink-0">
        <span>{'✓'} 校验通过</span>
        <span>{nodeCount} 节点</span>
        <span>SLA: 平均 {avgSla}h</span>
      </div>
    </>
  );
}
