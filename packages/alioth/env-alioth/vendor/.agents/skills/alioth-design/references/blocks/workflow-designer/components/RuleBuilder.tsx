/**
 * RuleBuilder — 条件规则构建器（原型版）
 * 对应前端实现 RuleBuilder.tsx；表达式格式 `field op value`（规范序列化裸标识符，
 * 兼容历史 `{{field}} op value` 包裹形式）
 * 字段来源唯一：流程绑定 event/task 叶表自动识别的 ContextField（context-fields 端点），
 * data_type 为 meta/DB 原生类型，经映射决定值输入形态（number/boolean/date/string）。
 */
import { useMemo } from 'react';
import type { ContextField, Rule } from '../flow-types';

const OPERATORS: Rule['operator'][] = ['==', '!=', '>', '<', '>=', '<=', 'in', 'contains'];
const INPUT = 'w-full h-8 px-2 text-xs border border-border rounded-md bg-card';

/** meta/DB 原生类型 → 规则值输入形态（对齐前端 contextValueKind） */
function dataTypeKind(dataType?: string): 'string' | 'number' | 'boolean' | 'date' {
  const dt = (dataType ?? '').toLowerCase();
  if (
    ['bigint', 'integer', 'int', 'int2', 'int4', 'int8', 'smallint', 'numeric', 'decimal',
      'double precision', 'real', 'float', 'float4', 'float8', 'number'].includes(dt)
  ) {
    return 'number';
  }
  if (['boolean', 'bool'].includes(dt)) return 'boolean';
  if (dt.startsWith('timestamp') || ['date', 'timestamptz', 'datetime'].includes(dt)) {
    return 'date';
  }
  return 'string';
}

/** 规范序列化对字符串字面量走 JSON.stringify——回读去一层引号（转义交给 JSON.parse）。 */
function unquote(raw: string): string {
  if (raw.length < 2) return raw;
  const q = raw[0];
  if ((q !== '"' && q !== "'") || !raw.endsWith(q)) return raw;
  if (q === '"') {
    try {
      const v: unknown = JSON.parse(raw);
      if (typeof v === 'string') return v;
    } catch {
      /* 非合法 JSON：退回朴素去引号 */
    }
  }
  return raw.slice(1, -1);
}

export function RuleBuilder({ fields, value, onChange }: { fields: ContextField[]; value?: string; onChange: (rule: Rule) => void }) {
  const parsed = useMemo(() => {
    if (!value) return null;
    // 无 regex 结构化解析（NO_REGEX 规约）：分解规范序列化 `field op value`
    // （= blockSerializeRule 产物，裸标识符 DSL）；兼容历史 `{{field}} op value` 包裹形式。
    const OPERATORS = ['==', '!=', '>=', '<=', '>', '<', 'in', 'contains'] as const;
    const e = value.trim();
    if (!e) return null;
    let field: string;
    let rest: string;
    if (e.startsWith('{{')) {
      const close = e.indexOf('}}');
      if (close < 0) return null;
      field = e.slice(2, close);
      rest = e.slice(close + 2).trim();
    } else {
      const sep = e.search(/\s/);
      if (sep < 0) return null;
      field = e.slice(0, sep);
      rest = e.slice(sep).trim();
    }
    if (!field) return null;
    const operator = OPERATORS.find((o) => rest.startsWith(o));
    if (!operator) return null;
    const raw = rest.slice(operator.length).trim();
    if (raw === '') return null;
    const fieldMeta = fields.find((f) => f.name === field);
    const kind = fieldMeta ? dataTypeKind(fieldMeta.data_type) : 'string';
    let val: string | number | boolean | string[] = raw;
    if (operator === 'in') {
      val = raw.replace(/^\[|\]$/g, '').split(',').map((s) => unquote(s.trim())).filter(Boolean);
    } else if (kind === 'boolean') {
      val = raw === 'true';
    } else if (kind === 'number') {
      const n = Number(raw);
      val = Number.isNaN(n) ? 0 : n;
    } else {
      val = unquote(raw);
    }
    return { field, operator: operator as Rule['operator'], value: val };
  }, [value, fields]);

  const rule: Rule = parsed ?? { field: fields[0]?.name ?? '', operator: '==', value: '' };
  const selectedField = fields.find((f) => f.name === rule.field);

  const renderValueInput = () => {
    const kind = selectedField ? dataTypeKind(selectedField.data_type) : 'string';
    if (kind === 'boolean') {
      return (
        <select value={String(rule.value)} onChange={(e) => onChange({ ...rule, value: e.target.value === 'true' })} className={INPUT}>
          <option value="true">是</option>
          <option value="false">否</option>
        </select>
      );
    }
    if (rule.operator === 'in') {
      return (
        <input
          value={Array.isArray(rule.value) ? rule.value.join(', ') : String(rule.value)}
          onChange={(e) => onChange({ ...rule, value: e.target.value.split(',').map((s) => s.trim()).filter(Boolean) })}
          placeholder="逗号分隔多个值"
          className={INPUT}
        />
      );
    }
    if (kind === 'number') {
      return (
        <input
          type="number"
          value={String(rule.value ?? '')}
          onChange={(e) => onChange({ ...rule, value: e.target.value === '' ? '' : Number(e.target.value) })}
          className={INPUT}
        />
      );
    }
    if (kind === 'date') {
      return (
        <input
          type="date"
          value={String(rule.value ?? '')}
          onChange={(e) => onChange({ ...rule, value: e.target.value })}
          className={INPUT}
        />
      );
    }
    return (
      <input value={String(rule.value ?? '')} onChange={(e) => onChange({ ...rule, value: e.target.value })} className={INPUT} />
    );
  };

  if (fields.length === 0) {
    return (
      <div className="text-xs text-muted-foreground bg-muted rounded-md p-2 mt-2">
        暂无上下文字段：请先在流程输入中绑定 event/task，由系统自动识别业务字段
      </div>
    );
  }

  return (
    <div className="space-y-2 mt-2">
      <div>
        <label className="text-xs font-medium block mb-1">字段</label>
        <select value={rule.field} onChange={(e) => onChange({ ...rule, field: e.target.value, value: '' })} className={INPUT}>
          <option value="">选择字段</option>
          {fields.map((f) => (
            <option key={f.name} value={f.name}>{f.label || f.name}</option>
          ))}
        </select>
      </div>
      <div>
        <label className="text-xs font-medium block mb-1">比较方式</label>
        <select value={rule.operator} onChange={(e) => onChange({ ...rule, operator: e.target.value as Rule['operator'] })} className={INPUT}>
          {OPERATORS.map((op) => (
            <option key={op} value={op}>{op}</option>
          ))}
        </select>
      </div>
      <div>
        <label className="text-xs font-medium block mb-1">比较值</label>
        {renderValueInput()}
      </div>
      <div className="text-xs text-muted-foreground bg-muted rounded-md p-2">
        预览: {`{{${rule.field}}} ${rule.operator} ${JSON.stringify(rule.value)}`}
      </div>
    </div>
  );
}
