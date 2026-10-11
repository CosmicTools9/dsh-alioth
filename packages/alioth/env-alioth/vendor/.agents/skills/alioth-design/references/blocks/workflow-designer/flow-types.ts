/** 流程绑定 event/task 叶表自动识别的业务字段（context-fields 端点行） */
export interface ContextField {
  name: string;
  label: string;
  category: string;
  /** meta/DB 原生类型（text / int4 / numeric / bool / timestamptz …），经映射决定规则值输入形态 */
  data_type: string;
}

/** 流程级数据上下文：预绑定 event/task 叶表，业务字段由系统自动识别（节点是 operation，无手工增删字段） */
export interface FlowContext {
  /** 绑定 event/task 概念名（如「审批事件 · 付款申请」） */
  concept: string;
  fields: ContextField[];
}

export interface Rule {
  field: string;
  operator: '==' | '!=' | '>' | '<' | '>=' | '<=' | 'in' | 'contains';
  value: string | number | boolean | string[];
}

export interface ApproverRef {
  kind: 'role' | 'engineer';
  id: string;
  label: string;
}

export interface FlowNode {
  id: string;
  type: string;
  label: string;
  role?: string;
  roleKind?: 'role' | 'engineer';
  sla?: number;
  mode?: string;
  expr?: string;
  outcome?: string;
  x: number;
  y: number;
}

/** 显式边模型：from/to 为节点 id，label 为连线标签文字（附着在线上） */
export interface FlowEdge {
  from: string;
  to: string;
  /** 连线标签，渲染在 SVG 路径上 */
  label?: string;
  /** 边条件（expr DSL，运行时 select_targets 逐边求值——条件节点按出边编制） */
  cond?: string;
}

export interface FlowTemplate {
  id: string;
  name: string;
  desc: string;
  color: string;
  status: string;
  /** 流程叶表（业务对象表名）：published 流程绑定后可发起（如 notice） */
  context_leaf?: string;
  /** 流程级数据上下文：预绑定 event/task 叶表，业务字段自动识别 */
  context: FlowContext;
  nodes: Partial<FlowNode>[];
  edges?: FlowEdge[];
}

// ── Templates ──
export const TEMPLATES: FlowTemplate[] = [
  {
    id: 'tpl-1', name: '标准三级审批', desc: '提交→部门经理→总监→完成', color: 'hsl(var(--chart-2))', status: 'published',
    context_leaf: 'notice',
    context: {
      concept: '审批事件 · 付款申请',
      fields: [
        { name: 'reason', label: '申请原因', category: '业务', data_type: 'text' },
        { name: 'amount', label: '金额', category: '业务', data_type: 'numeric' },
      ],
    },
    nodes: [
      { id: 'n0', type: 'start', label: '提交申请', x: 60, y: 80 },
      { id: 'n1', type: 'approval', label: '部门经理审批', role: '部门经理', sla: 24, mode: 'or_sign', x: 300, y: 80 },
      { id: 'n2', type: 'approval', label: '总监审批', role: '总监', sla: 48, mode: 'or_sign', x: 540, y: 80 },
      { id: 'n3', type: 'end', label: '审批完成', x: 780, y: 80 },
    ],
  },
  {
    id: 'tpl-2', name: '条件分支审批', desc: '根据金额自动分流', color: 'hsl(var(--warning))', status: 'published',
    context_leaf: 'notice',
    context: {
      concept: '审批事件 · 付款申请',
      fields: [
        { name: 'reason', label: '申请原因', category: '业务', data_type: 'text' },
        { name: 'amount', label: '金额', category: '业务', data_type: 'numeric' },
      ],
    },
    nodes: [
      { id: 'n0', type: 'start', label: '提交申请', x: 60, y: 120 },
      { id: 'n1', type: 'condition', label: '金额≤5万?', x: 280, y: 120 },
      { id: 'n2', type: 'approval', label: '经理审批', role: '部门经理', sla: 24, mode: 'or_sign', x: 180, y: 260 },
      { id: 'n3', type: 'approval', label: '总监审批', role: '总监', sla: 48, mode: 'or_sign', x: 500, y: 120 },
      { id: 'n4', type: 'end', label: '审批完成', x: 780, y: 120 },
    ],
    edges: [
      { from: 'n0', to: 'n1' },
      { from: 'n1', to: 'n2', label: '≤5万', cond: 'amount <= 50000' },
      { from: 'n1', to: 'n3', label: '>5万' },
      { from: 'n2', to: 'n4' },
      { from: 'n3', to: 'n4' },
    ],
  },
  {
    id: 'tpl-3', name: '并行会签审批', desc: '多人并行审批', color: 'hsl(var(--success))', status: 'draft',
    context: {
      concept: '审批事件 · 采购申请',
      fields: [
        { name: 'amount', label: '采购金额', category: '业务', data_type: 'numeric' },
        { name: 'vendor', label: '供应商', category: '业务', data_type: 'text' },
        { name: 'urgent', label: '是否紧急', category: '业务', data_type: 'bool' },
      ],
    },
    nodes: [
      { id: 'n0', type: 'start', label: '提交申请', x: 60, y: 120 },
      { id: 'n1', type: 'parallel', label: '并行会签', x: 300, y: 120 },
      { id: 'n2', type: 'approval', label: '财务会签', role: '财务', sla: 24, mode: 'and_sign', x: 180, y: 260 },
      { id: 'n3', type: 'approval', label: '法务会签', role: '法务', sla: 24, mode: 'and_sign', x: 500, y: 260 },
      { id: 'n4', type: 'branch', label: '会签汇总', x: 340, y: 380 },
      { id: 'n5', type: 'end', label: '审批完成', x: 660, y: 380 },
    ],
    edges: [
      { from: 'n0', to: 'n1' },
      { from: 'n1', to: 'n2' },
      { from: 'n1', to: 'n3' },
      { from: 'n2', to: 'n4' },
      { from: 'n3', to: 'n4' },
      { from: 'n4', to: 'n5' },
    ],
  },
  {
    id: 'tpl-4', name: '逐级上报', desc: '按层级逐级上报', color: 'hsl(var(--chart-4))', status: 'published',
    context_leaf: 'notice',
    context: {
      concept: '审批事件 · 休假申请',
      fields: [
        { name: 'reason', label: '请假原因', category: '业务', data_type: 'text' },
        { name: 'days', label: '请假天数', category: '业务', data_type: 'int4' },
      ],
    },
    nodes: [
      { id: 'n0', type: 'start', label: '提交申请', x: 60, y: 120 },
      { id: 'n1', type: 'approval', label: '主管审批', role: '主管', sla: 12, mode: 'or_sign', x: 300, y: 120 },
      { id: 'n2', type: 'approval', label: '经理审批', role: '部门经理', sla: 24, mode: 'or_sign', x: 540, y: 120 },
      { id: 'n3', type: 'end', label: '审批完成', x: 780, y: 120 },
    ],
  },
];

// ── Node config ──
export const NODE_TYPES: Array<{ type: string; label: string; desc: string; color: string }> = [
  { type: 'start', label: '开始', desc: '流程起始节点', color: 'hsl(var(--success))' },
  { type: 'approval', label: '审批', desc: '人工审批节点', color: 'hsl(var(--chart-5))' },
  { type: 'condition', label: '条件', desc: '条件分支网关', color: 'hsl(var(--warning))' },
  { type: 'cc', label: '抄送', desc: '知会抄送节点', color: 'hsl(var(--chart-4))' },
  { type: 'parallel', label: '并行', desc: '并行网关', color: 'hsl(var(--chart-2))' },
  { type: 'branch', label: '汇聚', desc: '汇聚网关', color: 'hsl(var(--chart-1))' },
  { type: 'subflow', label: '子流程', desc: '嵌套子流程', color: 'hsl(var(--chart-3))' },
  { type: 'end', label: '结束', desc: '流程结束节点', color: 'hsl(var(--destructive))' },
];

export const NODE_ICON: Record<string, string> = {
  start: '▶', approval: '✅', condition: '◇', cc: '✉',
  parallel: '↔', branch: '↪', subflow: '▷▷', end: '■',
};

export const MOCK_APPROVERS: ApproverRef[] = [
  { kind: 'role', id: 'director', label: '总监' },
  { kind: 'role', id: 'dept-manager', label: '部门经理' },
  { kind: 'role', id: 'finance', label: '财务' },
  { kind: 'role', id: 'legal', label: '法务' },
  { kind: 'engineer', id: 'eng-001', label: '张三' },
  { kind: 'engineer', id: 'eng-002', label: '李四' },
  { kind: 'engineer', id: 'eng-003', label: '王五' },
];

// ── 流程范畴（新建流程对话框 mock，与 scope-options 端点结构对齐）──

export interface FlowBranchOption {
  table: string; // 物理表名（值用，不展示）
  concept: string; // 业务概念（展示）
}

/** 流程叶表分支（一级分类） */
export const FLOW_BRANCHES: FlowBranchOption[] = [
  { table: 'zc_id_proc-approve', concept: '流程-门禁审批' },
  { table: 'zc_id_proc-cicd', concept: '流程-持续集成' },
  { table: 'zc_id_proc-loading', concept: '流程-装载包装' },
  { table: 'zc_id_proc-make', concept: '流程-生产制造' },
  { table: 'zc_id_proc-project', concept: '流程-项目管理' },
  { table: 'zc_id_proc-purchase', concept: '流程-采购管理' },
  { table: 'zc_id_proc-service', concept: '流程-服务操作' },
];

export type ContextDomainKey = 'task' | 'event' | 'approve';

export interface ContextItem {
  table: string;
  concept: string;
}

export interface ContextDomain {
  key: ContextDomainKey;
  label: string;
  items: ContextItem[];
}

/** 流程输入范畴三域（任务 / 非审批事件 / 审批事件）→ 叶表项（业务概念） */
export const CONTEXT_DOMAINS: ContextDomain[] = [
  {
    key: 'task',
    label: '任务',
    items: [
      { table: 'zc_id_task-commission', concept: '任务-委托' },
      { table: 'zc_id_task-design', concept: '任务-设计' },
      { table: 'zc_id_task-develop', concept: '任务-研发' },
      { table: 'zc_id_task-fix', concept: '任务-修复' },
      { table: 'zc_id_task-storage', concept: '任务-储元' },
      { table: 'zc_id_task-testing', concept: '任务-测试' },
    ],
  },
  {
    key: 'event',
    label: '事件',
    items: [
      { table: 'zc_id_even-accident', concept: '事件-事故' },
      { table: 'zc_id_even-alert', concept: '事件-提示' },
      { table: 'zc_id_even-counting', concept: '事件-盘点' },
      { table: 'zc_id_even-issue', concept: '事件-问题' },
      { table: 'zc_id_even-log', concept: '事件-日志' },
      { table: 'zc_id_even-modify', concept: '事件-变更' },
      { table: 'zc_id_even-report', concept: '事件-汇报' },
      { table: 'zc_id_even-tracking', concept: '事件-追踪' },
    ],
  },
  {
    key: 'approve',
    label: '审批事件',
    items: [
      { table: 'zc_id_appr-authorization', concept: '审批-权限授予' },
      { table: 'zc_id_appr-bid-evaluation', concept: '审批-评标审定' },
      { table: 'zc_id_appr-code-review', concept: '审批-代码审查' },
      { table: 'zc_id_appr-damage', concept: '审批-损失申报' },
      { table: 'zc_id_appr-org-structure', concept: '审批-组织调整' },
      { table: 'zc_id_appr-payment', concept: '审批-付款申请' },
      { table: 'zc_id_appr-pricing', concept: '审批-价格调整' },
      { table: 'zc_id_appr-prj-initiation', concept: '审批-立项申请' },
      { table: 'zc_id_appr-prj_doc-push', concept: '审批-项目文档变更发布' },
      { table: 'zc_id_appr-prj_made-push', concept: '审批-项目内容变更发布' },
      { table: 'zc_id_appr-prj_request-push', concept: '审批-项目诉求变更发布' },
      { table: 'zc_id_appr-prj_sales-push', concept: '审批-项目需求变更发布' },
      { table: 'zc_id_appr-process', concept: '审批-处理流程' },
      { table: 'zc_id_appr-project-push', concept: '审批-项目基线变更发布' },
      { table: 'zc_id_appr-purchase', concept: '审批-采购申请' },
      { table: 'zc_id_appr-recruitment', concept: '审批-招聘申请' },
      { table: 'zc_id_appr-req-time_off', concept: '审批-休假申请' },
      { table: 'zc_id_appr-user_verify', concept: '审批-用户认证' },
    ],
  },
];

/** 流程范畴 → 可选流程输入领域（前端硬编码语义筛选，与生产实现对齐） */
export const BRANCH_CONTEXT_DOMAINS: Record<string, ContextDomainKey[]> = {
  'zc_id_proc-approve': ['approve'],
  'zc_id_proc-cicd': ['task'],
  'zc_id_proc-make': ['task'],
  'zc_id_proc-loading': ['task', 'event'],
  'zc_id_proc-purchase': ['approve', 'task'],
  'zc_id_proc-project': ['task', 'event', 'approve'],
  'zc_id_proc-service': ['event', 'task'],
};

/** 分类（二级，沿用现有类别） */
export const FLOW_CATEGORIES: Record<string, string> = {
  procurement: '采购供应链',
  finance: '财务费用',
  hr_admin: '人事行政',
  legal: '合同法务',
  project: '项目管理',
  it_service: 'IT 服务',
};

// ── 发起流程：业务对象（notice 叶表 mock，与 context-objects 端点结构对齐）──

export interface BusinessObject {
  id: string;
  /** notice 标题字段（展示名） */
  notice: string;
  /** 业务状态（pending=待处理） */
  status?: string;
  /** 提交时间 */
  submittedAt?: string;
}

/** context_leaf 表名 → 业务对象行（notice 叶表） */
export const CONTEXT_OBJECTS: Record<string, BusinessObject[]> = {
  notice: [
    { id: 101, notice: 'FY2026-018 原材料采购付款申请', status: 'pending', submittedAt: '2026-08-20' },
    { id: 102, notice: 'FY2026-019 设备维保付款申请', status: 'pending', submittedAt: '2026-08-22' },
    { id: 103, notice: 'FY2026-020 差旅报销付款申请', status: 'pending', submittedAt: '2026-08-24' },
    { id: 104, notice: 'FY2026-021 咨询服务付款申请', status: 'pending', submittedAt: '2026-08-25' },
  ],
};
