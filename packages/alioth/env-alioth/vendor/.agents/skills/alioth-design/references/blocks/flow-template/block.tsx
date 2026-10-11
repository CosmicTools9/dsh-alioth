/**
 * flow-template — 流程模板 Block 原型（**实现单一源 = 平台共享面**；system-approve · 坐标 PQ）
 *
 * 本单元被 ≥2 个 namespace 引用（Alioth / AVIC-CAASEC / SE 的 system-approve 模块）⇒ 按 ns 隔离
 * 不变量（openspec/specs/ns-isolation :: shared-prototype-single-source）住平台共享面；各 ns 的
 * `Prototypes/Blocks/flow-template/llm-tsx/block.tsx` MUST 只保留声明式共享入口（re-export）。
 * 单元 MUST 保持 ns 无关（MUST NOT 内嵌任何 ns 名称/branding）。
 *
 * 设计源（同源生产实现，模块源不随本单元搬迁）：WZ `comprehensive-wz` 模块的 FlowTemplatePage——
 *   图库式模板资产总览：状态筛选（全部/已发布/草稿/已停用）+ 搜索 + 卡片（迷你节点链
 *      开始→审批→完成）+ 发起（仅「已发布且绑定上下文」可发起）+ 编辑 + 删除确认。
 *   职责边界：本 Block = 模板资产总览与管理入口；模板的画布设计由流程定义（flow-designer）承担。
 *
 * 原型增补（高保真校验面）：
 *   ① 新建流程模板（分类 → 流程输入 → 名称）产出草稿，与「设计器统一发布」链路衔接
 *   ② 发起前置校验可视化（status=active && contextLeaf 非空）+ 绑定上下文缺失告警
 *   ③ 数据态切换（正常/加载/空/错误）+ 模板状态机口径面板
 *
 * 契约：零 props · content-only · mock 经 ./mock.json 引入 · displayName = 'flow-template'
 */
import { useMemo, useState } from 'react';
import {
  Page,
  PageHeader,
  Card,
  MetricCard,
  Badge,
  Button,
  FilterTabs,
  Select,
  Spinner,
  ErrorBanner,
  inputStyle,
} from '../_kit/ui';
import { createPrototypeLifecycle } from '../_kit/lifecycle';
import MOCK from './mock.json';

interface FlowTemplate {
  id: string;
  name: string;
  code: string;
  status: string;
  category: string;
  branchConcept: string;
  contextConcept: string;
  contextLeaf: string | null;
  updatedAt: string;
  owner: string;
  nodes: string[];
}
interface MockShape {
  templates: FlowTemplate[];
}

const DATA: MockShape = JSON.parse(JSON.stringify(MOCK)) as MockShape;

type StatusKey = 'all' | 'active' | 'draft' | 'deprecated';
type DataState = 'ready' | 'loading' | 'empty' | 'error';

const STATUS_META: Record<string, { label: string; variant: 'success' | 'warning' | 'default' }> = {
  active: { label: '已发布', variant: 'success' },
  draft: { label: '草稿', variant: 'warning' },
  deprecated: { label: '已停用', variant: 'default' },
};

const CATEGORY_OPTIONS = ['审批', '评审', '适航', '执行', '变更'];

/** 发起前置条件（与 WZ FlowTemplatePage canInitiate 同源）：已发布且绑定上下文叶子 */
function canInitiate(t: FlowTemplate): boolean {
  return t.status === 'active' && !!t.contextLeaf;
}

export function Block() {
  const [rows, setRows] = useState<FlowTemplate[]>(DATA.templates);
  const [statusFilter, setStatusFilter] = useState<StatusKey>('all');
  const [query, setQuery] = useState('');
  const [state, setState] = useState<DataState>('ready');
  const [deleteTarget, setDeleteTarget] = useState<FlowTemplate | null>(null);
  const [initiateTarget, setInitiateTarget] = useState<FlowTemplate | null>(null);
  const [form, setForm] = useState<{ name: string; category: string; branchConcept: string; contextConcept: string } | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);

  const counts = useMemo(() => {
    const acc: Record<string, number> = { all: rows.length };
    for (const k of ['active', 'draft', 'deprecated']) acc[k] = rows.filter((r) => r.status === k).length;
    return acc;
  }, [rows]);

  const visible = useMemo(() => {
    if (state === 'empty') return [];
    const q = query.trim().toLowerCase();
    return rows.filter((r) => {
      if (statusFilter !== 'all' && r.status !== statusFilter) return false;
      if (!q) return true;
      return [r.name, r.code, r.category, r.branchConcept, r.contextConcept]
        .map((v) => String(v ?? '').toLowerCase())
        .join(' ')
        .includes(q);
    });
  }, [rows, statusFilter, query, state]);

  const activeCount = counts.active ?? 0;
  const initiable = rows.filter(canInitiate).length;
  const unbound = rows.filter((r) => !r.contextLeaf).length;

  const submitCreate = () => {
    if (!form) return;
    if (!form.name.trim() || !form.branchConcept.trim()) {
      setFormError('流程名称与流程输入为必填项');
      return;
    }
    const category = form.category;
    setRows((prev) => [
      {
        id: `ft-new-${prev.length + 1}`,
        name: form.name.trim(),
        code: `FT-${form.branchConcept.trim().toUpperCase()}`,
        status: 'draft',
        category,
        branchConcept: form.branchConcept.trim(),
        contextConcept: form.contextConcept.trim() || `${category}-${form.branchConcept.trim()}`,
        contextLeaf: null,
        updatedAt: '2026-09-22',
        owner: '原型录入',
        nodes: ['开始', '提交', '结束'],
      },
      ...prev,
    ]);
    setToast(`流程模板「${form.name.trim()}」已创建（草稿，待流程定义发布）`);
    setForm(null);
    setFormError(null);
    setStatusFilter('draft');
  };

  const confirmDelete = () => {
    if (!deleteTarget) return;
    setRows((prev) => prev.filter((r) => r.id !== deleteTarget.id));
    setToast(`已删除流程模板「${deleteTarget.name}」`);
    setDeleteTarget(null);
  };

  return (
    <Page>
      <style>{`
        .ftm-toolbar { display: flex; flex-wrap: nowrap; overflow-x: auto; align-items: center; gap: 12px; border: 1px solid hsl(var(--border)); border-radius: 8px; padding: 12px; background: hsl(var(--card)); margin-bottom: 16px; }
        .ftm-input { width: 288px; border-radius: 8px; border: 1px solid hsl(var(--input)); background: transparent; padding: 8px 12px; font-size: 13px; color: hsl(var(--foreground)); }
        .ftm-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 16px; }
        /* 窄屏：工具栏/指标行改横向滚动（禁堆叠）；卡片列宽由 auto-fit minmax(260px,1fr) 兜底 */
        @media (max-width: 1024px) {
          .ftm-metrics { overflow-x: auto; flex-wrap: nowrap; }
          .ftm-toolbar { flex-wrap: nowrap; overflow-x: auto; }
          .ftm-chain { flex-wrap: nowrap; }
          .ftm-meta { white-space: nowrap; }
        }
        .ftm-metrics { display: flex; flex-wrap: nowrap; gap: 12px; margin-bottom: 16px; }
        .ftm-card { border: 1px solid hsl(var(--border)); border-radius: 12px; background: hsl(var(--card)); padding: 16px; display: flex; flex-direction: column; gap: 12px; }
        .ftm-card-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 8px; }
        .ftm-name { font-weight: 600; font-size: 14px; }
        .ftm-code { font-family: var(--font-mono); font-size: 12px; color: hsl(var(--muted-foreground)); }
        .ftm-meta { font-size: 12px; color: hsl(var(--muted-foreground)); line-height: 1.6; }
        .ftm-chain { display: flex; align-items: center; flex-wrap: wrap; gap: 4px; }
        .ftm-node { font-size: 11px; padding: 4px 8px; border-radius: 4px; background: hsl(var(--primary) / 0.1); color: hsl(var(--primary)); }
        .ftm-node-start { background: hsl(var(--success) / 0.14); color: hsl(var(--success)); }
        .ftm-node-end { background: hsl(var(--muted)); color: hsl(var(--muted-foreground)); }
        .ftm-arrow { font-size: 11px; color: hsl(var(--muted-foreground)); }
        .ftm-foot { display: flex; align-items: center; justify-content: space-between; gap: 8px; border-top: 1px solid hsl(var(--border)); padding-top: 12px; }
        .ftm-link { background: none; border: none; padding: 0; font-size: 12px; font-weight: 500; cursor: pointer; color: hsl(var(--primary)); }
        .ftm-link:disabled { color: hsl(var(--muted-foreground)); cursor: not-allowed; opacity: 0.6; }
        .ftm-link-danger { color: hsl(var(--destructive)); }
        .ftm-empty { border: 1px dashed hsl(var(--border)); border-radius: 12px; padding: 48px; text-align: center; font-size: 14px; color: hsl(var(--muted-foreground)); }
        .ftm-overlay { position: fixed; inset: 0; z-index: 60; background: hsl(var(--foreground) / 0.4); display: flex; align-items: center; justify-content: center; padding: 16px; }
        .ftm-dialog { background: hsl(var(--card)); border: 1px solid hsl(var(--border)); border-radius: 12px; box-shadow: var(--shadow-lg); width: 480px; max-width: 94vw; padding: 20px; }
        .ftm-dialog-title { font-size: 15px; font-weight: 600; margin-bottom: 8px; }
        .ftm-dialog-body { font-size: 13px; color: hsl(var(--muted-foreground)); line-height: 1.6; }
        .ftm-dialog-foot { display: flex; justify-content: flex-end; gap: 8px; margin-top: 20px; }
        .ftm-field { margin-bottom: 12px; }
        .ftm-field-label { display: block; font-size: 12px; color: hsl(var(--muted-foreground)); margin-bottom: 4px; }
        .ftm-toast { position: fixed; bottom: 24px; left: 50%; transform: translateX(-50%); z-index: 70; background: hsl(var(--foreground)); color: hsl(var(--background)); font-size: 13px; padding: 8px 16px; border-radius: 8px; box-shadow: var(--shadow-md); }
        .ftm-kv { display: flex; align-items: flex-start; gap: 12px; padding: 8px 0; border-bottom: 1px dashed hsl(var(--border)); font-size: 12px; }
        .ftm-kv:last-child { border-bottom: none; }
        .ftm-kv-k { flex: 0 0 32%; color: hsl(var(--muted-foreground)); }
      `}</style>

      <PageHeader
        title="流程模板"
        desc="图库式模板资产总览：状态筛选/搜索/新建草稿/发布/发起入口——画布设计请进入流程定义"
        action={
          <>
            <Button variant="primary" onClick={() => { setForm({ name: '', category: '审批', branchConcept: '', contextConcept: '' }); setFormError(null); }}>
              新建流程
            </Button>
            {(['ready', 'loading', 'empty', 'error'] as DataState[]).map((s) => (
              <Button key={s} variant={state === s ? 'primary' : 'outline'} onClick={() => setState(s)}>
                {s === 'ready' ? '正常' : s === 'loading' ? '加载' : s === 'empty' ? '空' : '错误'}
              </Button>
            ))}
          </>
        }
      />

      {toast ? (
        <div className="ftm-toast" onClick={() => setToast(null)}>
          {toast}
        </div>
      ) : null}

      {state === 'error' ? (
        <ErrorBanner message="流程模板库加载失败（GET /service/commitment/flows 503）——请重试" onRetry={() => setState('ready')} />
      ) : state === 'loading' ? (
        <Card title="加载中">
          <div className="flex items-center gap-3 py-10 justify-center" style={{ color: 'hsl(var(--muted-foreground))' }}>
            <Spinner size={20} />
            <span className="text-sm">正在加载流程模板库…</span>
          </div>
        </Card>
      ) : (
        <>
          <div className="flex flex-wrap gap-3 mb-4">
            <MetricCard label="模板总数" value={String(counts.all ?? 0)} meta="草稿 + 已发布 + 已停用" />
            <MetricCard label="已发布" value={String(activeCount)} meta="可被业务发起" />
            <MetricCard label="可发起" value={String(initiable)} meta="已发布且绑定上下文" />
            <MetricCard label="未绑定上下文" value={String(unbound)} meta="须先绑定业务对象方可发布" />
          </div>

          <div className="ftm-toolbar">
            <input
              className="ftm-input"
              style={inputStyle}
              placeholder="搜索流程名 / 编码 / 分类 / 输入概念"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              data-testid="flow-template-search"
            />
            <FilterTabs
              items={[
                { key: 'all', label: `全部 ${counts.all ?? 0}` },
                { key: 'active', label: `已发布 ${counts.active ?? 0}` },
                { key: 'draft', label: `草稿 ${counts.draft ?? 0}` },
                { key: 'deprecated', label: `已停用 ${counts.deprecated ?? 0}` },
              ]}
              active={statusFilter}
              onChange={(k) => setStatusFilter(k as StatusKey)}
            />
            <Select
              value={form ? form.category : '全部'}
              onChange={(val) => {
                if (val !== '全部') setForm({ name: '', category: val, branchConcept: '', contextConcept: '' });
                else setForm(null);
              }}
              options={[{ value: '全部', label: '全部分类' }, ...CATEGORY_OPTIONS.map((c) => ({ value: c, label: `新建分类：${c}` }))]}
              style={{ width: 176 }}
            />
          </div>

          {visible.length === 0 ? (
            <div className="ftm-empty">该状态下暂无流程模板——调整筛选或新建流程草稿</div>
          ) : (
            <div className="ftm-grid">
              {visible.map((tpl) => {
                const meta = STATUS_META[tpl.status] ?? STATUS_META.draft;
                const ok = canInitiate(tpl);
                return (
                  <div key={tpl.id} className="ftm-card" data-testid="flow-template-card">
                    <div className="ftm-card-head">
                      <div style={{ minWidth: 0 }}>
                        <div className="ftm-name">{tpl.name}</div>
                        <div className="ftm-code">{tpl.code}</div>
                      </div>
                      <Badge text={meta.label} variant={meta.variant} />
                    </div>

                    <div className="ftm-meta">
                      分类：{tpl.category} · 输入：{tpl.branchConcept}
                      <br />
                      绑定：{tpl.contextLeaf ? `${tpl.contextConcept} › ${tpl.contextLeaf}` : '未绑定上下文'}
                    </div>

                    <div className="ftm-chain">
                      {tpl.nodes.slice(0, 5).map((n, i) => (
                        <span key={`${tpl.id}-${i}`} style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
                          {i > 0 ? <span className="ftm-arrow">→</span> : null}
                          <span className={i === 0 ? 'ftm-node ftm-node-start' : i === tpl.nodes.length - 1 ? 'ftm-node ftm-node-end' : 'ftm-node'}>
                            {n}
                          </span>
                        </span>
                      ))}
                      {tpl.nodes.length > 5 ? <span className="ftm-arrow">…</span> : null}
                    </div>

                    {!tpl.contextLeaf ? (
                      <div className="ftm-meta" style={{ color: 'hsl(var(--warning))' }}>
                        未绑定业务上下文：发布后仍不可发起，请在流程定义中绑定范畴叶子。
                      </div>
                    ) : null}

                    <div className="ftm-foot">
                      <span className="ftm-meta">更新 {tpl.updatedAt} · {tpl.owner}</span>
                      <span style={{ display: 'inline-flex', gap: 12 }}>
                        <button
                          type="button"
                          className="ftm-link"
                          disabled={!ok}
                          title={ok ? '发起该流程' : '仅「已发布且绑定上下文」的模板可发起'}
                          onClick={() => setInitiateTarget(tpl)}
                          data-testid="flow-template-initiate"
                        >
                          发起
                        </button>
                        <button type="button" className="ftm-link" onClick={() => setToast(`进入画布编辑「${tpl.name}」（原型演示）`)}>
                          编辑
                        </button>
                        <button
                          type="button"
                          className="ftm-link ftm-link-danger"
                          onClick={() => setDeleteTarget(tpl)}
                          data-testid="flow-template-delete"
                        >
                          删除
                        </button>
                      </span>
                    </div>
                  </div>
                );
              })}
            </div>
          )}

          <div style={{ marginTop: 16 }}>
            <Card title="模板状态机与发起条件">
              <div className="ftm-kv">
                <span className="ftm-kv-k">draft（草稿）</span>
                <span>新建产出即草稿；节点链与绑定上下文均在流程定义中编辑，草稿不可发起</span>
              </div>
              <div className="ftm-kv">
                <span className="ftm-kv-k">active（已发布）</span>
                <span>发布后可在模板卡片发起；发起前置校验：status = active 且 contextLeaf 非空</span>
              </div>
              <div className="ftm-kv">
                <span className="ftm-kv-k">deprecated（已停用）</span>
                <span>停用后保留历史实例与审批链，但不再允许新发起</span>
              </div>
              <div className="ftm-kv">
                <span className="ftm-kv-k">服务 / 坐标</span>
                <span className="ftm-code">commitment（system-approve）· 坐标 PQ</span>
              </div>
              <div className="ftm-kv">
                <span className="ftm-kv-k">职责边界</span>
                <span>本 Block = 模板资产总览与管理入口；单模板画布设计与发布属流程定义（flow-designer）</span>
              </div>
            </Card>
          </div>
        </>
      )}

      {deleteTarget ? (
        <div className="ftm-overlay" onClick={(e) => { if (e.target === e.currentTarget) setDeleteTarget(null); }}>
          <div className="ftm-dialog" role="dialog" aria-modal="true">
            <div className="ftm-dialog-title">删除流程模板</div>
            <div className="ftm-dialog-body">
              确认删除流程模板「{deleteTarget.name}」（{deleteTarget.code}）？已发起的实例保留，
              但该模板不可再用于新流程发起，操作不可恢复。
            </div>
            <div className="ftm-dialog-foot">
              <Button variant="outline" onClick={() => setDeleteTarget(null)}>
                取消
              </Button>
              <Button
                variant="primary"
                onClick={confirmDelete}
                style={{ background: 'hsl(var(--destructive))', color: 'hsl(var(--destructive-foreground))' }}
              >
                确认删除
              </Button>
            </div>
          </div>
        </div>
      ) : null}

      {initiateTarget ? (
        <div className="ftm-overlay" onClick={(e) => { if (e.target === e.currentTarget) setInitiateTarget(null); }}>
          <div className="ftm-dialog" role="dialog" aria-modal="true">
            <div className="ftm-dialog-title">发起流程：{initiateTarget.name}</div>
            <div className="ftm-dialog-body">
              绑定上下文：{initiateTarget.contextConcept} › {initiateTarget.contextLeaf}
              <br />
              首节点：{initiateTarget.nodes[0] ?? '开始'} · 审批节点数：{Math.max(0, initiateTarget.nodes.length - 2)}
              <br />
              发起后将在工作台生成待办，并进入审批链流转。
            </div>
            <div className="ftm-dialog-foot">
              <Button variant="outline" onClick={() => setInitiateTarget(null)}>
                取消
              </Button>
              <Button
                variant="primary"
                onClick={() => {
                  setToast(`已发起「${initiateTarget.name}」（实例已创建，进入待办）`);
                  setInitiateTarget(null);
                }}
              >
                确认发起
              </Button>
            </div>
          </div>
        </div>
      ) : null}

      {form ? (
        <div className="ftm-overlay" onClick={(e) => { if (e.target === e.currentTarget) { setForm(null); setFormError(null); } }}>
          <div className="ftm-dialog" role="dialog" aria-modal="true">
            <div className="ftm-dialog-title">新建流程模板（草稿）</div>
            <div className="ftm-field">
              <span className="ftm-field-label">分类</span>
              <Select
                value={form.category}
                onChange={(val) => setForm({ ...form, category: val })}
                options={CATEGORY_OPTIONS.map((c) => ({ value: c, label: c }))}
              />
            </div>
            <div className="ftm-field">
              <span className="ftm-field-label">流程输入（范畴）*</span>
              <input
                style={inputStyle}
                value={form.branchConcept}
                placeholder="如：立项申请"
                onChange={(e) => setForm({ ...form, branchConcept: e.target.value })}
              />
            </div>
            <div className="ftm-field">
              <span className="ftm-field-label">绑定上下文概念</span>
              <input
                style={inputStyle}
                value={form.contextConcept}
                placeholder="如：适航-审定计划（留空按分类-输入自动生成）"
                onChange={(e) => setForm({ ...form, contextConcept: e.target.value })}
              />
            </div>
            <div className="ftm-field">
              <span className="ftm-field-label">流程名称 *</span>
              <input
                style={inputStyle}
                value={form.name}
                placeholder="如：标准三级审批"
                onChange={(e) => setForm({ ...form, name: e.target.value })}
              />
            </div>
            {formError ? <div style={{ color: 'hsl(var(--destructive))', fontSize: 12 }}>{formError}</div> : null}
            <div className="ftm-dialog-foot">
              <Button variant="outline" onClick={() => { setForm(null); setFormError(null); }}>
                取消
              </Button>
              <Button variant="primary" onClick={submitCreate}>
                创建草稿
              </Button>
            </div>
          </div>
        </div>
      ) : null}
    </Page>
  );
}

Block.displayName = 'flow-template';
export const { bootstrap, mount, unmount } = createPrototypeLifecycle({ name: 'flow-template', App: Block });
export default Block;
