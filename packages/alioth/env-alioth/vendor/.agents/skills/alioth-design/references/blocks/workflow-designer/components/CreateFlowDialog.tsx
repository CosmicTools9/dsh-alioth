/**
 * CreateFlowDialog — 新建流程对话框（范畴化设计）
 * 字段序：流程名称 → 流程范畴（一级，proc 叶表分支）→ 分类（二级）→
 * 流程输入（领域 → 范畴项 两级联动，按分支硬编码语义筛选）→ 描述。
 * 选项一律展示业务概念，物理表名仅作值。
 */
import { useEffect, useState } from 'react';
import {
  FLOW_BRANCHES,
  CONTEXT_DOMAINS,
  BRANCH_CONTEXT_DOMAINS,
  FLOW_CATEGORIES,
  type ContextDomainKey,
} from '../flow-types';

const ALL_DOMAINS: ContextDomainKey[] = ['task', 'event', 'approve'];

export function CreateFlowDialog({ onClose }: { onClose: () => void }) {
  const [name, setName] = useState('');
  const [branch, setBranch] = useState('zc_id_proc-approve');
  const [category, setCategory] = useState('procurement');
  const [domain, setDomain] = useState<ContextDomainKey>('approve');
  const [contextTable, setContextTable] = useState('');
  const [description, setDescription] = useState('');

  const allowedKeys = BRANCH_CONTEXT_DOMAINS[branch] ?? ALL_DOMAINS;
  const domains = CONTEXT_DOMAINS.filter((d) => allowedKeys.includes(d.key));
  const activeDomain = domains.find((d) => d.key === domain) ?? domains[0];
  const items = activeDomain?.items ?? [];

  // 级联复位：选项集变化后当前选择失效时回退到首项
  useEffect(() => {
    if (domains.length > 0 && !domains.some((d) => d.key === domain)) {
      setDomain(domains[0].key);
    }
  }, [domains, domain]);
  useEffect(() => {
    if (items.length > 0 && !items.some((i) => i.table === contextTable)) {
      setContextTable(items[0].table);
    }
  }, [items, contextTable]);

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
          <div className="text-base font-semibold">新建流程</div>
          <div className="text-xs text-muted-foreground mt-0.5">
            创建后进入可视化设计器编排节点
          </div>
        </div>
        <div className="px-5 py-2" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div>
            <div className="text-xs font-medium mb-1">流程名称</div>
            <input
              className="w-full h-8 px-2 text-sm border border-border rounded-md bg-card"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="请输入流程名称"
              autoFocus
            />
          </div>
          <div>
            <div className="text-xs font-medium mb-1">流程范畴</div>
            <select
              className="w-full h-8 px-2 text-sm border border-border rounded-md bg-card"
              value={branch}
              onChange={(e) => {
                setBranch(e.target.value);
                setContextTable('');
              }}
            >
              {FLOW_BRANCHES.map((b) => (
                <option key={b.table} value={b.table}>
                  {b.concept}
                </option>
              ))}
            </select>
          </div>
          <div>
            <div className="text-xs font-medium mb-1">分类</div>
            <select
              className="w-full h-8 px-2 text-sm border border-border rounded-md bg-card"
              value={category}
              onChange={(e) => setCategory(e.target.value)}
            >
              {Object.entries(FLOW_CATEGORIES).map(([k, v]) => (
                <option key={k} value={k}>
                  {v}
                </option>
              ))}
            </select>
          </div>
          <div>
            <div className="text-xs font-medium mb-1">流程输入</div>
            <div style={{ display: 'flex', gap: 8 }}>
              <select
                className="h-8 px-2 text-sm border border-border rounded-md bg-card"
                style={{ width: 140, flexShrink: 0 }}
                value={activeDomain?.key ?? domain}
                onChange={(e) => {
                  setDomain(e.target.value as ContextDomainKey);
                  setContextTable('');
                }}
              >
                {domains.map((d) => (
                  <option key={d.key} value={d.key}>
                    {d.label}
                  </option>
                ))}
              </select>
              <select
                className="h-8 px-2 text-sm border border-border rounded-md bg-card"
                style={{ flex: 1 }}
                value={contextTable}
                onChange={(e) => setContextTable(e.target.value)}
              >
                {items.map((i) => (
                  <option key={i.table} value={i.table}>
                    {i.concept}
                  </option>
                ))}
              </select>
            </div>
          </div>
          <div>
            <div className="text-xs font-medium mb-1">描述</div>
            <textarea
              className="w-full px-2 py-1.5 text-sm border border-border rounded-md bg-card"
              style={{ height: 72, resize: 'vertical' }}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="可选"
            />
          </div>
        </div>
        <div
          style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, padding: '8px 20px 16px' }}
        >
          <button
            className="inline-flex items-center h-8 px-3 rounded-lg text-sm font-medium border border-border bg-card text-foreground hover:bg-accent transition-colors"
            onClick={onClose}
          >
            取消
          </button>
          <button
            className="inline-flex items-center h-8 px-3 rounded-lg text-sm font-medium border border-transparent bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
            onClick={onClose}
          >
            创建并设计
          </button>
        </div>
      </div>
    </div>
  );
}
