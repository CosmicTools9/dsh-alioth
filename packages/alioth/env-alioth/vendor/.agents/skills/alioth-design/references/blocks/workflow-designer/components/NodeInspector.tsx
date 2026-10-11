/**
 * NodeInspector — 节点属性检查器（右侧抽屉）
 * 对应前端实现 FlowInspector.tsx；内嵌 RuleBuilder / ApproverPicker
 * 节点是 operation，数据上下文为流程预绑定的 event/task——只读展示自动识别字段，无手工字段编制
 */
import type { FlowContext, FlowNode, ContextField, FlowEdge, Rule } from '../flow-types';
import { ApproverPicker } from './ApproverPicker';
import { RuleBuilder } from './RuleBuilder';

/** 条件序列化（对齐生产 rule-helpers：裸标识符；zuid 整数字符串裸输出保精度） */
function blockSerializeRule(rule: Rule): string {
  const intLiteral = /^-?(0|[1-9]\d*)$/;
  const fieldRef = rule.field;
  if (rule.operator === 'in' && Array.isArray(rule.value)) {
    const items = rule.value
      .map((v) => (intLiteral.test(String(v)) ? String(v) : JSON.stringify(v)))
      .join(', ');
    return `${fieldRef} in [${items}]`;
  }
  if (typeof rule.value === 'boolean' || typeof rule.value === 'number') {
    return `${fieldRef} ${rule.operator} ${rule.value}`;
  }
  if (typeof rule.value === 'string' && intLiteral.test(rule.value.trim())) {
    return `${fieldRef} ${rule.operator} ${rule.value.trim()}`;
  }
  return `${fieldRef} ${rule.operator} ${JSON.stringify(rule.value)}`;
}

const INPUT = 'w-full h-8 px-2 text-xs border border-border rounded-md bg-card';
const LABEL = 'text-xs font-medium block mb-1';

export function NodeInspector({ node, index, allFields, context, edges, onChange, onEdgesChange, onClose }: {
  node: FlowNode | null;
  index: number | null;
  allFields: ContextField[];
  context?: FlowContext;
  edges?: FlowEdge[];
  onChange: (i: number, patch: Partial<FlowNode>) => void;
  /** 边条件更新（条件节点逐出边编制——fix-flow-designer-runtime-chain D1） */
  onEdgesChange?: (edges: FlowEdge[]) => void;
  onClose: () => void;
}) {
  if (!node || index === null) return null;
  return (
    <div className="border-l border-border bg-card shrink-0 p-4" style={{ width: 260, overflowY: 'auto' }}>
      <div className="flex justify-between items-center mb-4">
        <span className="text-sm font-bold">节点属性</span>
        <button onClick={onClose} className="text-muted-foreground text-base border-none cursor-pointer">{'✕'}</button>
      </div>
      <div>
        <div>
          <label className={LABEL}>节点名称</label>
          <input value={node.label} onChange={(e) => onChange(index, { label: e.target.value })} className={INPUT} />
        </div>
        {node.type === 'approval' && (
          <>
            <div className="mt-3">
              <label className={LABEL}>审批人 / 岗位</label>
              <ApproverPicker
                value={node.role ? { kind: node.roleKind ?? 'role', id: node.role, label: node.role } : undefined}
                onChange={(ref) => onChange(index, { role: ref.id, roleKind: ref.kind })}
              />
            </div>
            <div className="mt-3">
              <label className={LABEL}>SLA (小时)</label>
              <input type="number" value={node.sla ?? 24} onChange={(e) => onChange(index, { sla: Number(e.target.value) })} className={INPUT} />
            </div>
            <div className="mt-3">
              <label className={LABEL}>会签方式</label>
              <select value={node.mode ?? 'or_sign'} onChange={(e) => onChange(index, { mode: e.target.value })} className={INPUT}>
                <option value="or_sign">任一通过</option>
                <option value="and_sign">全部通过</option>
                <option value="sequential">依次审批</option>
              </select>
            </div>
            <div className="mt-3">
              <label className={LABEL}>数据上下文</label>
              <div className="bg-muted rounded-md p-2 space-y-2">
                <div className="flex items-center gap-1 text-[10px] text-muted-foreground">
                  <span className="font-medium">绑定输入</span>
                  <span>event / task</span>
                </div>
                <div className="text-xs font-medium" style={{ color: 'hsl(var(--primary))' }}>
                  {context?.concept ?? '未绑定'}
                </div>
                <div className="flex flex-wrap gap-1">
                  {(context?.fields ?? []).map((f) => (
                    <span
                      key={f.name}
                      className="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 border border-border rounded-full bg-card"
                    >
                      {f.label}
                      <span className="text-muted-foreground">{f.data_type}</span>
                    </span>
                  ))}
                </div>
                <div className="text-[10px] text-muted-foreground">字段自动识别自绑定 event/task，不可手工增删</div>
              </div>
            </div>
          </>
        )}
        {node.type === 'condition' && (
          <div className="mt-3">
            <label className={LABEL}>条件规则（逐出边编制）</label>
            {(() => {
              const outs = (edges ?? []).filter((e) => e.from === node.id);
              if (outs.length === 0) {
                return <div className="text-[10px] text-muted-foreground">该节点无出边——先在画布连接分支</div>;
              }
              return (
                <div className="flex flex-col gap-3">
                  {outs.map((edge, ei) => (
                    <div key={`${edge.to}-${ei}`}>
                      <label className={`${LABEL} text-[10px]`}>
                        分支 {ei + 1}{edge.label ? `（${edge.label}）` : ''}
                      </label>
                      <RuleBuilder
                        fields={allFields}
                        value={edge.cond ?? ''}
                        onChange={(rule: Rule) => {
                          const next = (edges ?? []).map((e, j) => {
                            const orig = outs[ei]!;
                            if (e.from === orig.from && e.to === orig.to) {
                              return { ...e, cond: blockSerializeRule(rule) };
                            }
                            return e;
                          });
                          onEdgesChange?.(next);
                        }}
                      />
                    </div>
                  ))}
                  {node.expr && (
                    <div className="text-[10px] text-muted-foreground">legacy expr: {node.expr}</div>
                  )}
                </div>
              );
            })()}
          </div>
        )}
        {node.type === 'end' && (
          <div>
            <label className={LABEL}>结束状态</label>
            <select value={node.outcome ?? 'complete'} onChange={(e) => onChange(index, { outcome: e.target.value })} className={INPUT}>
              <option value="complete">审批完成</option>
              <option value="rejected">已拒绝</option>
              <option value="cancelled">已取消</option>
            </select>
          </div>
        )}
      </div>
    </div>
  );
}
