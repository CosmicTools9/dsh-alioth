/**
 * my-approvals — Block prototype (我的审批)
 * 3 tabs: pending (待审批) / history (已处理) / my-requests (我的申请)
 * Mirrors production WfNewRequestModal + ApprovalDetailModal capabilities
 */
import { useMemo, useState, type ChangeEvent, type MouseEvent } from 'react';
import { Page, MetricCard, Badge, Button } from '../_kit/ui';
import { createPrototypeLifecycle } from '../_kit/lifecycle';

interface ApprovalItem { id: number; title: string; applicant: string; flowType: string; date: string; status: string; priority?: string; sla?: number; opinion?: string; resultTime?: string; formData?: Record<string, unknown>; }

interface FormField { name: string; title: string; dataType: 'string' | 'number' | 'date' | 'boolean' | 'select' | 'text'; required: boolean; defaultValue?: string; options?: string[]; }
interface FormSchema { collection: string; fields: FormField[]; }
interface FlowTemplate { id: string; name: string; formSchema: FormSchema; }
interface ApproverRef { kind: 'role' | 'engineer'; id: string; label: string; }

const PENDING: ApprovalItem[] = [
  { id: 1, title: '系统架构方案调整审批', applicant: '张博文', flowType: '标准三级审批', date: '2026-01-05', status: 'pending', priority: 'urgent', sla: 85 },
  { id: 2, title: '雷达算法参数变更申请', applicant: '赵明宇', flowType: '条件分支审批', date: '2026-01-04', status: 'pending', priority: 'high', sla: 60 },
  { id: 3, title: '柴油机测试标准更新', applicant: '林婉婷', flowType: '会签审批', date: '2026-01-02', status: 'pending', priority: 'normal', sla: 30 },
  { id: 4, title: '动力包接口规范变更', applicant: '李冠杰', flowType: '变更审批', date: '2025-12-30', status: 'pending', priority: 'high', sla: 45 },
];

const HISTORY: ApprovalItem[] = [
  { id: 5, title: '加密终端外壳材料替换', applicant: '陈思睿', flowType: '标准三级审批', date: '2026-01-03', status: 'approved', opinion: '方案可行，批准执行', resultTime: '2026-01-04 14:30' },
  { id: 6, title: '供应商物料替代申请', applicant: '刘浩然', flowType: '快速审批', date: '2025-12-28', status: 'approved', opinion: '同意替换', resultTime: '2025-12-29 09:15' },
  { id: 7, title: '动力包接口规范变更', applicant: '李冠杰', flowType: '变更审批', date: '2025-12-30', status: 'rejected', opinion: '需要补充技术方案评估', resultTime: '2026-01-02 16:00' },
];

const MY_REQUESTS: ApprovalItem[] = [
  { id: 8, title: '研发环境升级申请', applicant: '管理员', flowType: '标准三级审批', date: '2026-01-06', status: 'pending' },
  { id: 9, title: '测试设备采购审批', applicant: '管理员', flowType: '快速审批', date: '2026-01-05', status: 'approved', resultTime: '2026-01-05 17:00' },
];

const FLOW_TEMPLATES: FlowTemplate[] = [
  { id: 'tpl-1', name: '标准三级审批', formSchema: { collection: 'approve-req', fields: [
    { name: 'reason', title: '申请原因', dataType: 'text', required: true },
    { name: 'amount', title: '金额', dataType: 'number', required: false },
  ]}},
  { id: 'tpl-2', name: '条件分支审批', formSchema: { collection: 'approve-req', fields: [
    { name: 'reason', title: '申请原因', dataType: 'text', required: true },
    { name: 'amount', title: '金额', dataType: 'number', required: false },
    { name: 'urgent', title: '是否加急', dataType: 'boolean', required: false },
  ]}},
  { id: 'tpl-3', name: '快速审批', formSchema: { collection: 'approve-req', fields: [
    { name: 'reason', title: '申请原因', dataType: 'text', required: true },
  ]}},
];

const MOCK_APPROVERS: ApproverRef[] = [
  { kind: 'role', id: 'manager', label: '部门经理' },
  { kind: 'role', id: 'director', label: '总监' },
  { kind: 'role', id: 'finance', label: '财务' },
  { kind: 'role', id: 'legal', label: '法务' },
  { kind: 'engineer', id: 'e1', label: '张博文' },
  { kind: 'engineer', id: 'e2', label: '赵明宇' },
  { kind: 'engineer', id: 'e3', label: '林婉婷' },
];

type TabKey = 'pending' | 'history' | 'my-requests';
const TABS: Array<{ key: TabKey; label: string }> = [
  { key: 'pending', label: '待审批' },
  { key: 'history', label: '已处理' },
  { key: 'my-requests', label: '我的申请' },
];

function StatusBadge({ status }: { status: string }) {
  const colors: Record<string, { text: string; color: string; bg: string }> = {
    pending: { text: '待审批', color: 'hsl(var(--warning))', bg: 'hsl(var(--bg-warning))' },
    approved: { text: '已通过', color: 'hsl(var(--success))', bg: 'hsl(var(--bg-success))' },
    rejected: { text: '已拒绝', color: 'hsl(var(--destructive))', bg: 'hsl(var(--bg-destructive))' },
  };
  const c = colors[status] ?? { text: status, color: '#666', bg: 'hsl(var(--muted))' };
  return <span style={{ fontSize: 11, padding: '1px 7px', borderRadius: 4, background: c.bg, color: c.color, fontWeight: 500 }}>{c.text}</span>;
}

function PriorityTag({ p }: { p?: string }) {
  if (!p || p === 'normal') return null;
  return <span style={{ fontSize: 9, padding: '1px 5px', borderRadius: 3, background: p === 'urgent' ? 'hsl(var(--bg-destructive))' : 'hsl(var(--bg-warning))', color: p === 'urgent' ? 'hsl(var(--destructive))' : 'hsl(var(--warning))', fontWeight: 600, marginRight: 4 }}>{p === 'urgent' ? '紧急' : '优先'}</span>;
}

function ApproverPickerProto({ value, onChange }: { value: ApproverRef | null; onChange: (v: ApproverRef | null) => void }) {
  const [tab, setTab] = useState<'role' | 'engineer'>('role');
  const [query, setQuery] = useState('');
  const options = MOCK_APPROVERS.filter((o: ApproverRef) => o.kind === tab && o.label.toLowerCase().includes(query.toLowerCase()));
  return (
    <div className="space-y-2">
      <div className="flex gap-1">
        <button onClick={() => setTab('role')} className={'px-2 py-1 text-xs border border-border rounded-md cursor-pointer ' + (tab === 'role' ? 'bg-primary/10 text-primary' : 'bg-card')}>岗位</button>
        <button onClick={() => setTab('engineer')} className={'px-2 py-1 text-xs border border-border rounded-md cursor-pointer ' + (tab === 'engineer' ? 'bg-primary/10 text-primary' : 'bg-card')}>工程师</button>
      </div>
      <input
        value={query}
        onChange={(e: ChangeEvent<HTMLInputElement>) => setQuery(e.target.value)}
        placeholder="搜索..."
        className="w-full h-8 px-2 text-xs border border-border rounded-md bg-card"
      />
      <div className="max-h-32 border border-border rounded-md" style={{ overflowY: 'auto' }}>
        {options.length === 0 ? (
          <div className="text-xs text-muted-foreground p-2 text-center">无结果</div>
        ) : (
          options.map((opt: ApproverRef) => (
            <button
              key={`${opt.kind}:${opt.id}`}
              onClick={() => onChange(opt)}
              className={'w-full text-left text-xs px-2 py-1.5 cursor-pointer border-b border-border last:border-b-0 ' + (value?.id === opt.id ? 'bg-primary/10' : 'bg-transparent')}
            >
              {opt.label}
            </button>
          ))
        )}
      </div>
      {value && (
        <div className="flex items-center justify-between text-xs border border-border rounded-md p-1.5">
          <span>{value.label}</span>
          <button onClick={() => onChange(null)} className="text-muted-foreground text-xs border-none bg-transparent cursor-pointer">清除</button>
        </div>
      )}
    </div>
  );
}

function NewRequestModal({ onClose, onSubmit }: { onClose: () => void; onSubmit: (item: ApprovalItem) => void }) {
  const [flowId, setFlowId] = useState<string>(FLOW_TEMPLATES[0].id);
  const [reason, setReason] = useState('');
  const [formData, setFormData] = useState<Record<string, string | number | boolean>>({});
  const selectedFlow = useMemo(() => FLOW_TEMPLATES.find((f: FlowTemplate) => f.id === flowId) ?? FLOW_TEMPLATES[0], [flowId]);
  const fields = selectedFlow.formSchema.fields;

  const updateField = (name: string, value: string | number | boolean) => {
    setFormData((prev: Record<string, string | number | boolean>) => ({ ...prev, [name]: value }));
  };

  const submit = () => {
    if (!reason.trim()) return;
    const missing = fields.filter((f: FormField) => f.required && (formData[f.name] === undefined || formData[f.name] === ''));
    if (missing.length) return;
    const payload = { reason, ...formData };
    onSubmit({
      id: Date.now(),
      title: selectedFlow.name + '申请',
      applicant: '管理员',
      flowType: selectedFlow.name,
      date: new Date().toISOString().slice(0, 10),
      status: 'pending',
      formData: payload,
    });
    onClose();
  };

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.4)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 50 }} onClick={(e: MouseEvent<HTMLDivElement>) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="bg-card rounded-lg p-6" style={{ maxWidth: 480, width: '90%', maxHeight: '80vh', overflowY: 'auto' }}>
        <div className="flex justify-between items-start mb-4">
          <h3 className="text-base font-bold">发起新申请</h3>
          <button onClick={onClose} className="text-muted-foreground text-base bg-none border-none cursor-pointer">{'\u2715'}</button>
        </div>
        <div className="flex flex-col gap-3 mb-4">
          <div>
            <label className="text-xs font-medium block mb-1">审批流程</label>
            <select value={flowId} onChange={(e: ChangeEvent<HTMLSelectElement>) => setFlowId(e.target.value)} className="w-full h-8 px-2 text-xs border border-border rounded-md bg-card">
              {FLOW_TEMPLATES.map((f: FlowTemplate) => <option key={f.id} value={f.id}>{f.name}</option>)}
            </select>
          </div>
          <div>
            <label className="text-xs font-medium block mb-1">申请原因 <span className="text-destructive">*</span></label>
            <textarea value={reason} onChange={(e: ChangeEvent<HTMLTextAreaElement>) => setReason(e.target.value)} className="w-full min-h-[80px] p-2 border border-border rounded-lg text-sm" placeholder="请输入申请原因" />
          </div>
          {fields.map((f: FormField) => (
            <div key={f.name}>
              <label className="text-xs font-medium block mb-1">{f.title}{f.required && <span className="text-destructive">*</span>}</label>
              {f.dataType === 'boolean' ? (
                <select value={String(formData[f.name] ?? false)} onChange={(e: ChangeEvent<HTMLSelectElement>) => updateField(f.name, e.target.value === 'true')} className="w-full h-8 px-2 text-xs border border-border rounded-md bg-card">
                  <option value="false">否</option>
                  <option value="true">是</option>
                </select>
              ) : f.dataType === 'number' ? (
                <input type="number" value={String(formData[f.name] ?? '')} onChange={(e: ChangeEvent<HTMLInputElement>) => updateField(f.name, e.target.value === '' ? '' : Number(e.target.value))} className="w-full h-8 px-2 text-xs border border-border rounded-md bg-card" />
              ) : (
                <input value={String(formData[f.name] ?? '')} onChange={(e: ChangeEvent<HTMLInputElement>) => updateField(f.name, e.target.value)} className="w-full h-8 px-2 text-xs border border-border rounded-md bg-card" />
              )}
            </div>
          ))}
        </div>
        <div className="flex gap-2 justify-end">
          <Button variant="ghost" onClick={onClose}>取消</Button>
          <Button variant="primary" onClick={submit}>提交</Button>
        </div>
      </div>
    </div>
  );
}

function ApprovalDetailModal({ item, onClose }: { item: ApprovalItem; onClose: () => void }) {
  const [action, setAction] = useState<null | 'transfer' | 'cc'>(null);
  const [transferTo, setTransferTo] = useState<ApproverRef | null>(null);
  const [ccTo, setCcTo] = useState<ApproverRef | null>(null);
  const [opinion, setOpinion] = useState('');

  const handleApprove = () => {
    if (!opinion.trim()) return;
    onClose();
  };
  const handleReject = () => {
    if (!opinion.trim()) return;
    onClose();
  };
  const handleTransfer = () => {
    if (!transferTo) return;
    setAction(null);
  };
  const handleCC = () => {
    if (!ccTo) return;
    setAction(null);
  };

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.4)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 50 }} onClick={(e: MouseEvent<HTMLDivElement>) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="bg-card rounded-lg p-6" style={{ maxWidth: 520, width: '90%', maxHeight: '85vh', overflowY: 'auto' }}>
        <div className="flex justify-between items-start mb-4">
          <h3 className="text-base font-bold">{item.title}</h3>
          <button onClick={onClose} className="text-muted-foreground text-base bg-none border-none cursor-pointer">{'\u2715'}</button>
        </div>
        <div className="flex flex-col gap-2 text-sm mb-4">
          <div className="flex items-center gap-2"><span className="text-muted-foreground">申请人:</span> {item.applicant}</div>
          <div className="flex items-center gap-2"><span className="text-muted-foreground">流程:</span> {item.flowType}</div>
          <div className="flex items-center gap-2"><span className="text-muted-foreground">提交:</span> {item.date}</div>
          {item.resultTime && <div className="flex items-center gap-2"><span className="text-muted-foreground">完成:</span> {item.resultTime}</div>}
          <div><StatusBadge status={item.status} /></div>
        </div>
        <div className="text-xs font-semibold text-muted-foreground mb-2">审批流程</div>
        <div className="flex items-center gap-1 mb-4 flex-wrap">
          {['提交申请', '部门经理', '总监', '完成'].map((n, i, arr) => (
            <span key={i} className="flex items-center gap-0.5">
              <span className="text-[10px] px-1.5 py-0.5 rounded" style={{ background: i === 1 ? 'hsl(var(--primary)/0.12)' : i < 2 ? 'hsl(var(--success)/0.12)' : 'hsl(var(--muted))', color: i === 1 ? 'hsl(var(--primary))' : i < 2 ? 'hsl(var(--success))' : 'hsl(var(--muted-foreground))', fontWeight: i <= 1 ? 600 : 400 }}>{n}</span>
              {i < arr.length - 1 && <span className="text-[10px] text-muted-foreground">{'\u2192'}</span>}
            </span>
          ))}
        </div>
        {item.formData && Object.keys(item.formData).length > 0 && (
          <>
            <div className="text-xs font-semibold text-muted-foreground mb-2">表单内容</div>
            <div className="flex flex-col gap-1 text-sm mb-4 p-2 rounded-md bg-muted">
              {Object.entries(item.formData).map(([k, v]: [string, unknown]) => (
                <div key={k} className="flex items-center gap-2"><span className="text-muted-foreground">{k}:</span> {String(v)}</div>
              ))}
            </div>
          </>
        )}
        <div className="text-xs font-semibold text-muted-foreground mb-2">审批意见</div>
        <textarea value={opinion} onChange={(e: ChangeEvent<HTMLTextAreaElement>) => setOpinion(e.target.value)} className="w-full min-h-[80px] p-2 border border-border rounded-lg text-sm mb-4" placeholder="请输入审批意见..." />
        {action === 'transfer' && (
          <div className="mb-4 p-3 border border-border rounded-md">
            <div className="text-xs font-semibold mb-2">转交给</div>
            <ApproverPickerProto value={transferTo} onChange={setTransferTo} />
            <div className="flex gap-2 justify-end mt-2">
              <Button variant="ghost" onClick={() => setAction(null)}>取消</Button>
              <Button variant="primary" onClick={handleTransfer}>确认转交</Button>
            </div>
          </div>
        )}
        {action === 'cc' && (
          <div className="mb-4 p-3 border border-border rounded-md">
            <div className="text-xs font-semibold mb-2">抄送给</div>
            <ApproverPickerProto value={ccTo} onChange={setCcTo} />
            <div className="flex gap-2 justify-end mt-2">
              <Button variant="ghost" onClick={() => setAction(null)}>取消</Button>
              <Button variant="primary" onClick={handleCC}>确认抄送</Button>
            </div>
          </div>
        )}
        <div className="flex gap-2 justify-end">
          <Button variant="ghost" onClick={() => setAction('transfer')}>转交</Button>
          <Button variant="ghost" onClick={() => setAction('cc')}>抄送</Button>
          <Button variant="primary" style={{ background: 'hsl(var(--destructive))' }} onClick={handleReject}>驳回</Button>
          <Button variant="primary" onClick={handleApprove}>通过</Button>
        </div>
      </div>
    </div>
  );
}

export default function Block() {
  const [tab, setTab] = useState<TabKey>('pending');
  const [filter, setFilter] = useState<'all' | 'urgent' | 'high' | 'normal'>('all');
  const [detail, setDetail] = useState<ApprovalItem | null>(null);
  const [newRequestOpen, setNewRequestOpen] = useState(false);
  const [myRequests, setMyRequests] = useState<ApprovalItem[]>(MY_REQUESTS);
  const [withdrawTarget, setWithdrawTarget] = useState<ApprovalItem | null>(null);

  const handleWithdrawConfirm = () => {
    if (!withdrawTarget) return;
    setMyRequests((prev) => prev.filter((i) => i.id !== withdrawTarget.id));
    setWithdrawTarget(null);
  };

  const filtered = tab === 'pending'
    ? PENDING.filter((item: ApprovalItem) => filter === 'all' || item.priority === filter)
    : tab === 'history' ? HISTORY : myRequests;

  return (
    <Page>
      <div className="flex items-center justify-between mb-4">
        <div><h1 className="text-xl font-bold text-foreground m-0">我的审批</h1><p className="text-sm text-muted-foreground mt-1 m-0">待您处理的审批事项</p></div>
        <Button variant="primary" onClick={() => setNewRequestOpen(true)}>新建申请</Button>
      </div>

      <div className="flex gap-3 mb-4">
        <MetricCard label="待审批" value={String(PENDING.length)} />
        <MetricCard label="已处理" value={String(HISTORY.length)} />
        <MetricCard label="我的申请" value={String(myRequests.length)} />
        <MetricCard label="总审批" value={String(PENDING.length + HISTORY.length + myRequests.length)} />
      </div>

      <div className="flex gap-1 mb-4 border-b border-border pb-0">
        {TABS.map((t: { key: TabKey; label: string }) => (
          <button key={t.key} onClick={() => { setTab(t.key); setFilter('all'); }}
            style={{ padding: '6px 16px', fontSize: 12, fontWeight: 500, cursor: 'pointer', border: 'none', borderBottom: tab === t.key ? '2px solid hsl(var(--primary))' : '2px solid transparent', background: 'transparent', color: tab === t.key ? 'hsl(var(--primary))' : 'hsl(var(--muted-foreground))' }}>
            {t.label}
          </button>
        ))}
      </div>

      {tab === 'pending' && (
        <div className="flex gap-1 mb-4">
          {[{ k: 'all', l: '全部' }, { k: 'urgent', l: '紧急' }, { k: 'high', l: '优先' }, { k: 'normal', l: '普通' }].map((f: { k: string; l: string }) => (
            <button key={f.k} onClick={() => setFilter(f.k as typeof filter)}
              className={'px-3 py-1 text-xs rounded-lg cursor-pointer border transition-colors ' + (filter === f.k ? 'border-primary bg-primary/10 text-primary' : 'border-border bg-card text-muted-foreground hover:border-primary')}>{f.l}
            </button>
          ))}
        </div>
      )}

      <div className="flex flex-col gap-3">
        {filtered.length === 0 ? (
          <div className="flex flex-col items-center py-10 text-muted-foreground"><p className="text-sm">暂无数据</p></div>
        ) : filtered.map((item: ApprovalItem) => (
          <div key={item.id} className="bg-card border border-border rounded-lg p-4 cursor-pointer" onClick={() => setDetail(item)}>
            <div className="flex items-center justify-between mb-1">
              <div className="flex items-center gap-2">
                <PriorityTag p={item.priority} />
                <span className="text-sm font-semibold text-foreground">{item.title}</span>
              </div>
              <StatusBadge status={item.status} />
            </div>
            <div className="flex items-center gap-3 text-xs text-muted-foreground mb-2">
              <span>申请人: {item.applicant}</span>
              <span>流程: {item.flowType}</span>
              <span>{item.date}</span>
              {item.resultTime && <span>完成: {item.resultTime}</span>}
            </div>
            {item.sla !== undefined && (
              <div className="h-1.5 bg-muted rounded-full overflow-hidden mb-2">
                <div className="h-full rounded-full" style={{ width: item.sla + '%', background: item.sla > 70 ? 'hsl(var(--success))' : item.sla > 40 ? 'hsl(var(--warning))' : 'hsl(var(--destructive))' }} />
              </div>
            )}
            {item.opinion && (
              <div className="text-xs p-2 rounded-md bg-muted border-l-4 mt-1" style={{ borderLeftColor: item.status === 'approved' ? 'hsl(var(--success))' : 'hsl(var(--destructive))' }}>
                <span className="font-semibold text-muted-foreground mr-1">审批意见:</span>{item.opinion}
              </div>
            )}
            {tab === 'pending' && (
              <div className="flex gap-2 justify-end mt-2">
                <button onClick={(e: MouseEvent<HTMLButtonElement>) => { e.stopPropagation(); }} className="px-2 py-1 text-xs border border-border rounded-md cursor-pointer bg-transparent text-muted-foreground">催办</button>
                <button onClick={(e: MouseEvent<HTMLButtonElement>) => { e.stopPropagation(); setDetail(item); }} style={{ fontSize: 11, color: 'hsl(var(--primary))', cursor: 'pointer', background: 'none', border: 'none' }}>查看详情</button>
              </div>
            )}
            {tab === 'my-requests' && item.status === 'pending' && (
              <div className="flex items-center justify-between mt-1">
                <span className="text-[10px] px-1.5 py-0.5 rounded bg-muted">当前节点: 部门经理审批</span>
                <button
                  onClick={(e: MouseEvent<HTMLButtonElement>) => {
                    e.stopPropagation();
                    setWithdrawTarget(item);
                  }}
                  className="px-2 py-1 text-xs border border-border rounded-md cursor-pointer bg-transparent"
                  style={{ color: 'hsl(var(--destructive))' }}
                >
                  撤回
                </button>
              </div>
            )}
          </div>
        ))}
      </div>

      {withdrawTarget && (
        <div
          style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.4)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 60 }}
          onClick={() => setWithdrawTarget(null)}
        >
          <div className="bg-card rounded-lg p-6" style={{ maxWidth: 420, width: '90%' }} onClick={(e: MouseEvent<HTMLDivElement>) => e.stopPropagation()}>
            <div className="flex items-start gap-3 mb-3">
              <span
                className="inline-flex items-center justify-center rounded-full"
                style={{ width: 32, height: 32, background: 'hsl(var(--bg-destructive))', color: 'hsl(var(--destructive))', flexShrink: 0 }}
              >
                {'\u26A0'}
              </span>
              <div className="flex-1">
                <div className="text-base font-bold">撤回</div>
                <div className="text-xs text-muted-foreground mt-0.5">{withdrawTarget.title}</div>
              </div>
              <button onClick={() => setWithdrawTarget(null)} className="text-muted-foreground text-base bg-none border-none cursor-pointer">{'\u2715'}</button>
            </div>
            <div className="text-sm text-foreground mb-4">确认撤回该申请？撤回后流程将终止。</div>
            <div className="flex gap-2 justify-end">
              <Button variant="ghost" onClick={() => setWithdrawTarget(null)}>取消</Button>
              <Button variant="primary" style={{ background: 'hsl(var(--destructive))' }} onClick={handleWithdrawConfirm}>确认</Button>
            </div>
          </div>
        </div>
      )}
    </Page>
  );
}
Block.displayName = 'my-approvals';
export const { bootstrap, mount, unmount } = createPrototypeLifecycle({ name: 'my-approvals', App: Block });
