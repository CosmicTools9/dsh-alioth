/**
 * FlowGallery — 流程模板画廊（Block 默认屏）
 * 对应 @alioth/components 的 FlowGallery；迷你预览由 GalleryMiniPreview 渲染
 */
import { useState } from 'react';
import { MetricCard, Badge } from '../../_kit/ui';
import type { FlowTemplate } from '../flow-types';
import { TEMPLATES } from '../flow-types';
import { GalleryMiniPreview } from './GalleryMiniPreview';
import { CreateFlowDialog } from './CreateFlowDialog';
import { InitiateFlowModal } from './InitiateFlowModal';

export function FlowGallery({ onEdit }: { onEdit: (tpl: FlowTemplate) => void }) {
  // #create 锚点：视觉验证直接打开新建流程对话框
  const [creating, setCreating] = useState(
    () => typeof location !== 'undefined' && location.hash === '#create',
  );
  // #initiate 锚点：视觉验证直接打开发起流程对话框（首个可发起模板）
  const [initiating, setInitiating] = useState<FlowTemplate | null>(() => {
    if (typeof location === 'undefined' || location.hash !== '#initiate') return null;
    return TEMPLATES.find((t) => t.status === 'published' && !!t.context_leaf) ?? null;
  });
  const [initiated, setInitiated] = useState(false);

  const canInitiate = (tpl: FlowTemplate) => tpl.status === 'published' && !!tpl.context_leaf;

  const handleInitiated = () => {
    setInitiated(true);
    window.setTimeout(() => setInitiated(false), 2500);
  };

  return (
    <div className="p-6 max-w-full mx-auto">
      <div className="flex items-center justify-between mb-4">
        <div>
          <h1 className="text-xl font-bold">流程设计器</h1>
          <p className="text-sm text-muted-foreground mt-1">审批流程定义与图形化设计</p>
        </div>
        <button
          className="inline-flex items-center h-8 px-3 rounded-lg text-sm font-medium border border-transparent bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
          onClick={() => setCreating(true)}
        >
          新建流程
        </button>
      </div>
      {creating && <CreateFlowDialog onClose={() => setCreating(false)} />}
      {initiating && (
        <InitiateFlowModal
          flow={initiating}
          onClose={() => setInitiating(null)}
          onInitiated={handleInitiated}
        />
      )}
      <div className="flex gap-3 mb-4">
        <MetricCard label="流程模板" value={String(TEMPLATES.length)} />
        <MetricCard label="已发布" value={String(TEMPLATES.filter((t) => t.status === 'published').length)} />
        <MetricCard label="草稿" value={String(TEMPLATES.filter((t) => t.status === 'draft').length)} />
      </div>
      <div className="flex flex-wrap gap-4">
        {TEMPLATES.map((tpl) => (
          <div
            key={tpl.id}
            className="bg-card border border-border rounded-lg p-4"
            style={{ width: 300 }}
            onClick={() => onEdit(tpl)}
          >
            <div className="flex items-center gap-2 mb-2">
              <span style={{ width: 10, height: 10, borderRadius: '50%', background: tpl.color }} />
              <span className="font-semibold text-sm">{tpl.name}</span>
              <Badge text={tpl.status === 'published' ? '已发布' : '草稿'} variant={tpl.status === 'published' ? 'success' : 'default'} />
            </div>
            <p className="text-xs text-muted-foreground mb-2">{tpl.desc}</p>
            <GalleryMiniPreview tpl={tpl} />
            {canInitiate(tpl) && (
              <div className="flex justify-end mt-2 pt-2 border-t border-border">
                <button
                  className="inline-flex items-center h-7 px-3 rounded-md text-xs font-medium border border-transparent bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
                  onClick={(e) => {
                    e.stopPropagation();
                    setInitiating(tpl);
                  }}
                >
                  发起
                </button>
              </div>
            )}
          </div>
        ))}
      </div>
      {initiated && (
        <div
          className="fixed top-4 right-4 z-[60] px-4 py-2 rounded-md text-sm text-primary-foreground"
          style={{ background: 'hsl(var(--success))' }}
        >
          流程已发起
        </div>
      )}
    </div>
  );
}
