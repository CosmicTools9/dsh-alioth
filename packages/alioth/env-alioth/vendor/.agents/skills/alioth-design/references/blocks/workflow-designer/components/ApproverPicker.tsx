/**
 * ApproverPicker — 审批人/岗位选择器（原型版）
 * 对应前端实现 ApproverPicker.tsx（已上抽 @alioth/components）
 */
import { useState } from 'react';
import type { ApproverRef } from '../flow-types';
import { MOCK_APPROVERS } from '../flow-types';

export function ApproverPicker({ value, onChange }: { value?: ApproverRef; onChange: (ref: ApproverRef) => void }) {
  const [tab, setTab] = useState<'role' | 'engineer'>(value?.kind ?? 'role');
  const [query, setQuery] = useState('');
  const options = MOCK_APPROVERS.filter((o) => o.kind === tab && o.label.toLowerCase().includes(query.toLowerCase()));
  return (
    <div className="space-y-2">
      <div className="flex gap-1">
        <button
          onClick={() => setTab('role')}
          className={`flex-1 text-xs px-2 py-1 rounded-md cursor-pointer border border-border ${tab === 'role' ? 'bg-primary/10' : 'bg-transparent'}`}
        >
          审批岗位
        </button>
        <button
          onClick={() => setTab('engineer')}
          className={`flex-1 text-xs px-2 py-1 rounded-md cursor-pointer border border-border ${tab === 'engineer' ? 'bg-primary/10' : 'bg-transparent'}`}
        >
          工程师
        </button>
      </div>
      <input
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="搜索..."
        className="w-full h-8 px-2 text-xs border border-border rounded-md bg-card"
      />
      <div className="max-h-32 border border-border rounded-md" style={{ overflowY: 'auto' }}>
        {options.length === 0 ? (
          <div className="text-xs text-muted-foreground p-2 text-center">无结果</div>
        ) : (
          options.map((opt) => (
            <button
              key={`${opt.kind}:${opt.id}`}
              onClick={() => onChange(opt)}
              className={`w-full text-left text-xs px-2 py-1.5 cursor-pointer border-b border-border last:border-b-0 ${value?.id === opt.id ? 'bg-primary/10' : 'bg-transparent'}`}
            >
              {opt.label}
            </button>
          ))
        )}
      </div>
    </div>
  );
}
