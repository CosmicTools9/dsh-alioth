/**
 * InitiateFlowModal — 发起流程对话框
 * 对齐生产 WfInitiateFlowModal：published + context_leaf 流程可发起；
 * 业务对象单选列表（notice 叶表），空态「该范畴暂无业务对象」。
 */
import { useState } from 'react';
import type { FlowTemplate } from '../flow-types';
import { CONTEXT_OBJECTS } from '../flow-types';

export function InitiateFlowModal({
  flow,
  onClose,
  onInitiated,
}: {
  flow: FlowTemplate;
  onClose: () => void;
  onInitiated?: () => void;
}) {
  const objects = flow.context_leaf ? CONTEXT_OBJECTS[flow.context_leaf] ?? [] : [];
  const [selectedId, setSelectedId] = useState<number | null>(null);

  const handleSubmit = () => {
    if (!flow.context_leaf || selectedId === null) return;
    onInitiated?.();
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={onClose}
    >
      <div
        className="bg-card border border-border rounded-lg shadow-lg w-full"
        style={{ maxWidth: 480 }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="px-5 pt-4 pb-2">
          <div className="text-base font-semibold">发起流程</div>
          <div className="text-xs text-muted-foreground mt-0.5">{flow.name}</div>
        </div>
        <div
          className="px-5 py-2"
          style={{ display: 'flex', flexDirection: 'column', gap: 8, maxHeight: '50vh', overflowY: 'auto' }}
        >
          {objects.length === 0 ? (
            <div
              className="text-xs text-muted-foreground py-6 text-center"
              style={{ fontSize: 13 }}
            >
              该范畴暂无业务对象
            </div>
          ) : (
            objects.map((obj) => (
              <label
                key={obj.id}
                className="flex items-center gap-2 p-2.5 rounded-md border border-border cursor-pointer bg-card hover:bg-accent"
              >
                <input
                  type="radio"
                  name={`initiate-${flow.id}`}
                  checked={selectedId === obj.id}
                  onChange={() => setSelectedId(obj.id)}
                />
                <span className="text-sm flex-1" style={{ fontSize: 13 }}>
                  {obj.notice}
                </span>
                {obj.submittedAt && (
                  <span className="text-xs text-muted-foreground" style={{ fontSize: 11 }}>
                    {obj.submittedAt}
                  </span>
                )}
              </label>
            ))
          )}
        </div>
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, padding: '8px 20px 16px' }}>
          <button
            className="inline-flex items-center h-8 px-3 rounded-lg text-sm font-medium border border-border bg-card text-foreground hover:bg-accent transition-colors"
            onClick={onClose}
          >
            取消
          </button>
          <button
            className="inline-flex items-center h-8 px-3 rounded-lg text-sm font-medium border border-transparent bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50"
            disabled={!flow.context_leaf || selectedId === null}
            onClick={handleSubmit}
          >
            确认发起
          </button>
        </div>
      </div>
    </div>
  );
}
