/**
 * ui.tsx — AliothStudio 原型共享 UI 原语(Tailwind 工具类,对齐前端实现)。
 *
 * 全部组件基于 tailwind-utilities.css 设计令牌,Tailwind 工具类替代 inline style。
 * .toggle 保留自定义类(开关组件,Tailwind 无直接等价,样式在 shell-css.ts)。
 */
import { useState, useEffect, useRef } from 'react';

/** 设计令牌快捷引用(与 tailwind-utilities.css :root 对齐)。 */
export const C = {
  border: 'hsl(var(--border))',
  card: 'hsl(var(--card))',
  fg: 'hsl(var(--foreground))',
  muted: 'hsl(var(--muted))',
  mutedFg: 'hsl(var(--muted-foreground))',
  primary: 'hsl(var(--primary))',
  primaryFg: 'hsl(var(--primary-foreground))',
  mutedBg: 'hsl(var(--muted) / 0.5)',
  accent: 'hsl(var(--accent))',
  destructive: 'hsl(var(--destructive))',
  success: 'hsl(var(--success))',
  sidebarBg: 'hsl(var(--sidebar-background))',
};

/** 模块辨识色（`MODULE_SPEC \u00a710.1.1`）：由模块原型入口设置 `window.__MODULE_ACCENT__`
 *  （HSL 三元组，如 `'34 84% 45%'`）：设置时注入主色对 `--primary`/`--ring`/`--primary-foreground`
 *  （对比度 ≥ 4.5:1，`MODULE_SPEC §10.1.1 §3.2`）；未设置时返回注册的品牌回落（缺省 `{}`——不回落
 *  品牌色，避免模块原型与 ns 品牌面耦合；ns 侧可经 `setAccentFallback` 显式注册，如 WZ 品牌蓝）。 */
/** HSL 明度 → sRGB（0..1 三元组）。 */
function hslRgb(l: number, h: number, s: number): [number, number, number] {
  const sn = s / 100;
  const ln = l / 100;
  const k = (n: number): number => (n + h / 30) % 12;
  const al = sn * Math.min(ln, 1 - ln);
  const f = (n: number): number =>
    ln - al * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
  return [f(0), f(8), f(4)];
}

/** HSL 三元组 → WCAG 2.1 相对亮度对比度（与模块 theme.css / gateway-shell 同算法）。 */
function hslContrastOf(l: number, h: number, s: number, other: [number, number, number]): number {
  const rgb = (ll: number): [number, number, number] => {
    const sn = s / 100;
    const ln = ll / 100;
    const k = (n: number): number => (n + h / 30) % 12;
    const al = sn * Math.min(ln, 1 - ln);
    const f = (n: number): number =>
      ln - al * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
    return [f(0), f(8), f(4)];
  };
  const lum = ([r, g, b]: [number, number, number]): number => {
    const f = (c: number): number => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
  };
  const [la, lb] = [lum(rgb(l)), lum(other)];
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return (hi + 0.05) / (lo + 0.05);
}

/** accent（`h s% l%`）→ `[h, s, l]`；非法形态返回 null。 */
function accentTriplet(accent: string): [number, number, number] | null {
  const m = /^(\d+(?:\.\d+)?)\s+(\d+(?:\.\d+)?)%\s+(\d+(?:\.\d+)?)%$/.exec(accent.trim());
  return m ? [parseFloat(m[1]!), parseFloat(m[2]!), parseFloat(m[3]!)] : null;
}

/**
 * accent（HSL 三元组）→ `--primary-foreground`（`MODULE_SPEC` §10.1.1 §3.2：主色对对比度 MUST ≥ 4.5:1）。
 * 深主色取白字；浅主色（白字 < 4.5:1）取同色相深档（向下取满足 ≥ 5.0:1 的最大明度）。
 * 算法与 gateway-shell `primaryForegroundFor` 同源单规则；色值唯一来源仍是 `MODULE_SPEC §10.1.1` 登记表。
 */
export function primaryForegroundFor(accent: string): string {
  const triplet = accentTriplet(accent);
  if (!triplet) return '0 0% 100%';
  const [h, s, l] = triplet;
  const white: [number, number, number] = [1, 1, 1];
  const base = hslContrastOf(l, h, s, white);
  if (base >= 4.5) return '0 0% 100%';
  for (let L = l - 1; L >= 0; L -= 1)
    if (hslContrastOf(l, h, s, hslRgb(L, h, s)) >= 5) return `${h} ${s}% ${L}%`;
  return '0 0% 0%';
}

/**
 * 可选品牌回落（默认 null = 不回落品牌色，避免模块原型与 ns 品牌面耦合）。
 * ns 侧若需品牌色（如 WZ：无模块上下文处回落品牌蓝），在自身 `_shared/ui.tsx` 导入期调用本函数注册；
 * 注册内容 SHOULD 含 `--primary-foreground`（主色对 MUST 成对注入）。
 */
let _accentFallback: Record<string, string> | null = null;
export function setAccentFallback(vars: Record<string, string> | null): void {
  _accentFallback = vars;
}

/** 模块辨识色：accent 设置时注入主色对（primary/ring/primary-foreground）；未设置时返回注册的品牌回落（缺省 `{}`）。 */
export function moduleAccentVars(): Record<string, string> {
  const accent = typeof window !== 'undefined'
    ? (window as Window & { __MODULE_ACCENT__?: unknown }).__MODULE_ACCENT__
    : undefined;
  if (typeof accent === 'string' && accent) {
    return { '--primary': accent, '--ring': accent, '--primary-foreground': primaryForegroundFor(accent) };
  }
  return _accentFallback ? { ..._accentFallback } : {};
}

export function Page({ children, style }: any) {
  return (
    <div className="max-w-full mx-auto p-6" style={{ ...moduleAccentVars(), ...style }}>
      {children}
    </div>
  );
}

export function PageHeader({ title, desc, action }: {
  title: string;
  desc?: string;
  action?: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between flex-wrap gap-2 mb-4">
      <div className="min-w-0">
        <h1 className="text-xl font-bold text-foreground m-0 whitespace-nowrap">{title}</h1>
        {desc ? <p className="text-sm text-muted-foreground mt-1 m-0">{desc}</p> : null}
      </div>
      {action ? <div className="flex gap-2 flex-wrap">{action}</div> : null}
    </div>
  );
}

export function Card({ title, children, style, footer }: any) {
  return (
    <div className="bg-card border border-border rounded-lg mb-4 overflow-hidden" style={style}>
      {title ? (
        <div className="px-4 py-3 border-b border-border font-semibold text-sm text-foreground">{title}</div>
      ) : null}
      <div className="p-4">{children}</div>
      {footer ? (
        <div className="px-4 py-3 border-t border-border flex justify-end gap-2">{footer}</div>
      ) : null}
    </div>
  );
}

export function SectionTitle({ children }: any) {
  return (
    <div className="text-xs font-bold uppercase tracking-wider text-muted-foreground my-2 mb-3">
      {children}
    </div>
  );
}

export function Chip({ children, active, onClick }: any) {
  return (
    <span
      onClick={onClick}
      className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs ${onClick ? 'cursor-pointer' : ''} border transition-colors ${
        active
          ? 'border-primary bg-primary/10 text-primary'
          : 'border-border bg-card text-foreground hover:border-primary'
      } whitespace-nowrap`}
    >
      {children}
    </span>
  );
}

export function Row({ label, children, style }: any) {
  return (
    <div className="flex items-center justify-between py-2.5 border-b border-border gap-3" style={style}>
      <span className="text-sm text-foreground">{label}</span>
      <div className="flex items-center gap-2">{children}</div>
    </div>
  );
}

export function Field({ label, hint, children }: any) {
  return (
    <div className="mb-4">
      <label className="block text-sm font-medium text-foreground mb-1">{label}</label>
      {children}
      {hint ? <div className="text-xs text-muted-foreground mt-1">{hint}</div> : null}
    </div>
  );
}

export const inputStyle: any = {
  width: '100%',
  padding: '8px 12px',
  border: '1px solid hsl(var(--border))',
  borderRadius: '0.5rem',
  fontSize: '13px',
  background: 'hsl(var(--card))',
  color: 'hsl(var(--foreground))',
  outline: 'none',
};

export function Toggle({ on, onChange }: any) {
  return (
    <button
      type="button"
      onClick={() => onChange && onChange(!on)}
      className={`toggle ${on ? 'on' : ''}`}
      aria-pressed={on}
    />
  );
}

export function Button({ children, variant, onClick, style }: any) {
  const variantClass =
    variant === 'primary'
      ? 'bg-primary text-primary-foreground hover:bg-primary/90'
      : variant === 'outline'
        ? 'border border-border bg-transparent text-foreground hover:bg-accent'
        : variant === 'ghost'
          ? 'bg-transparent text-muted-foreground hover:bg-accent hover:text-accent-foreground'
          : 'bg-accent text-foreground hover:bg-accent/80';
  return (
    <button
      type="button"
      onClick={onClick}
      className={`inline-flex items-center gap-1.5 h-8 px-3 rounded-lg text-sm font-medium cursor-pointer border border-transparent transition-colors ${variantClass}`}
      style={style}
    >
      {children}
    </button>
  );
}

/* ════════════════════════════════════════════════════════════════
 * === 9 个 block.tsx 实际使用的组件（由 upgrade-block-shell-to-gl-gateway-classes change 补齐）
 * ════════════════════════════════════════════════════════════════ */

export function Skeleton({ variant = 'text', count = 1, style }: any) {
  const items: any[] = [];
  for (let i = 0; i < count; i++) {
    const cls = variant === 'title' ? 'skel skel-title'
      : variant === 'button' ? 'skel skel-button'
      : variant === 'row' ? 'skel-row'
      : 'skel skel-text';
    items.push(<div key={i} className={cls} style={style} />);
  }
  return <>{items}</>;
}

export function Spinner({ size = 16 }: any) {
  return (
    <span className="spinner" style={{ width: size, height: size, display: 'inline-block' }}>
      <svg viewBox="0 0 24 24" fill="none">
        <circle className="spinner-track" cx="12" cy="12" r="10" strokeWidth="3" />
        <path className="spinner-fill" d="M12 2a10 10 0 0 1 10 10" strokeWidth="3" stroke="currentColor" strokeLinecap="round" />
      </svg>
    </span>
  );
}

export function ConfirmDialog({
  open, title, message, confirmText = '确认', cancelText = '取消',
  confirmVariant = 'primary', requireTypeToConfirm = false, expectedText = '',
  onConfirm, onCancel,
}: any) {
  const [inputVal, setInputVal] = useState('');
  useEffect(() => {
    if (open) setInputVal('');
  }, [open]);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && onCancel) onCancel();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [open, onCancel]);
  if (!open) return null;
  const canConfirm = !requireTypeToConfirm || inputVal === expectedText;
  const variantClass =
    confirmVariant === 'destructive'
      ? 'bg-destructive text-destructive-foreground hover:bg-destructive/90'
      : 'bg-primary text-primary-foreground hover:bg-primary/90';
  return (
    <>
      <div
        className="confirm-overlay"
        onClick={(e: any) => { if (e.target === e.currentTarget && onCancel) onCancel(); }}
        role="presentation"
      />
      <div className="confirm-dialog" role="dialog" aria-modal="true" aria-labelledby="confirm-title">
        <h3 id="confirm-title">{title}</h3>
        {message ? <p className="confirm-message">{message}</p> : null}
        {requireTypeToConfirm ? (
          <input
            className={`confirm-input${inputVal && inputVal !== expectedText ? ' invalid' : ''}`}
            type="text"
            placeholder={`请输入 ${expectedText} 以确认`}
            value={inputVal}
            onChange={(e: any) => setInputVal(e.target.value)}
            autoFocus
          />
        ) : null}
        <div className="confirm-footer">
          <button type="button" className="inline-flex items-center h-8 px-3 rounded-lg text-sm font-medium border border-border bg-card text-foreground hover:bg-accent transition-colors" onClick={onCancel}>{cancelText}</button>
          <button
            type="button"
            className={`inline-flex items-center h-8 px-3 rounded-lg text-sm font-medium border border-transparent transition-colors ${variantClass}${canConfirm ? '' : ' opacity-60 cursor-not-allowed'}`}
            onClick={canConfirm ? onConfirm : undefined}
            disabled={!canConfirm}
          >
            {confirmText}
          </button>
        </div>
      </div>
    </>
  );
}

export function Toast({ message, variant = 'info', onDismiss }: any) {
  const variantClass =
    variant === 'success' ? 'bg-success text-success-foreground'
    : variant === 'error' ? 'bg-destructive text-destructive-foreground'
    : 'bg-card text-card-foreground border border-border';
  return (
    <div
      role="status"
      className={`fixed bottom-6 left-1/2 -translate-x-1/2 px-4 py-2 rounded-lg shadow-md text-sm ${variantClass}`}
      onClick={onDismiss}
    >
      {message}
    </div>
  );
}

export function SearchInput({ value, onChange, placeholder = '搜索…', icon }: any) {
  return (
    <div className="relative">
      {icon ? <span className="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground">{icon}</span> : null}
      <input
        type="search"
        value={value || ''}
        onChange={onChange}
        placeholder={placeholder}
        className="w-full h-9 pl-9 pr-3 rounded-lg border border-border bg-muted/50 text-sm outline-none focus:bg-background focus:border-primary/30 transition-colors"
        style={inputStyle}
      />
    </div>
  );
}

type SelectChoice = { value: string; label: string };
type SelectItem = string | SelectChoice | { label: string; options: SelectChoice[] };

export function Select({ value, onChange, onBlur, options = [], placeholder, style }: {
  value?: string | number;
  onChange?: React.ChangeEventHandler<HTMLSelectElement>;
  onBlur?: React.FocusEventHandler<HTMLSelectElement>;
  options?: ReadonlyArray<SelectItem>;
  placeholder?: string;
  style?: React.CSSProperties;
}) {
  return (
    <select
      value={value || ''}
      onChange={onChange}
      onBlur={onBlur}
      style={{ ...inputStyle, width: '100%', ...style }}
    >
      {placeholder ? <option value="">{placeholder}</option> : null}
      {options.map((o, i) => {
        // 分组项：{ label, options: [...] } → <optgroup>（长字典下拉的分组标题，如关系视角 18 行）
        if (o !== null && typeof o === 'object' && 'options' in o) {
          return (
            <optgroup key={`g-${o.label ?? i}`} label={o.label}>
              {o.options.map((c) => (
                <option key={c.value} value={c.value}>
                  {c.label}
                </option>
              ))}
            </optgroup>
          );
        }
        const optionValue = typeof o === 'object' ? o.value : o;
        return (
          <option key={optionValue} value={optionValue}>
            {typeof o === 'object' ? o.label : o}
          </option>
        );
      })}
    </select>
  );
}

export function Drawer({ open, onClose, title, children, footer, width }: {
  open: boolean;
  onClose: () => void;
  title?: React.ReactNode;
  children?: React.ReactNode;
  footer?: React.ReactNode;
  width?: string | number;
}) {
  if (!open) return null;
  return (
    <>
      <div className="drawer-overlay" onClick={onClose} />
      <div className="drawer" role="dialog" aria-modal="true" style={width ? { width, maxWidth: '92vw' } : undefined}>
        {title ? (
          <div className="drawer-header">
            <h3>{title}</h3>
            <button type="button" className="btn-icon" aria-label="关闭" onClick={onClose}>✕</button>
          </div>
        ) : null}
        <div className="drawer-body">{children}</div>
        {footer ? <div className="drawer-footer">{footer}</div> : null}
      </div>
    </>
  );
}

export function Wizard({ steps = [], currentStep = 0, onStepChange, children }: {
  steps?: string[];
  currentStep?: number;
  onStepChange?: (index: number) => void;
  children?: React.ReactNode;
}) {
  return (
    <div>
      {/* 窄屏：步骤条横向滚动而非压缩标签（min-w 由内容决定；nowrap 保标签单行） */}
      <div className="wizard-steps" style={{ display: 'flex', overflowX: 'auto' }}>
        {steps.map((label: string, i: number) => (
          <span key={i} style={{ display: 'flex', alignItems: 'center', flexShrink: 0 }}>
            <span
              className={`wizard-step${currentStep === i ? ' active' : currentStep > i ? ' done' : ''}`}
              onClick={() => onStepChange && onStepChange(i)}
              style={{ cursor: onStepChange ? 'pointer' : 'default', whiteSpace: 'nowrap' }}
            >
              <span className="wizard-step-dot">{currentStep > i ? '✓' : i + 1}</span>
              <span>{label}</span>
            </span>
            {i < steps.length - 1 ? <span className={`wizard-connector${currentStep > i ? ' done' : ''}`} /> : null}
          </span>
        ))}
      </div>
      <div>{children}</div>
    </div>
  );
}

export function ProgressBar({ value = 0, max = 100, variant }: any) {
  const pct = Math.min(100, Math.max(0, Math.round((value / max) * 100)));
  const fillClass =
    variant === 'full' || pct >= 100 ? 'full'
    : variant === 'partial' || pct > 0 ? 'partial'
    : '';
  return (
    <div className="progress-bar" style={{ width: '100%' }}>
      <div className={`fill ${fillClass}`} style={{ width: pct + '%' }} />
    </div>
  );
}

/* ════════════════════════════════════════════════════════════════
 * === P1 共享工具补齐（对齐 gateway-shell.tsx）
 *    - ErrorBanner     (reference: 1250 行)
 *    - useLoadingState (reference: 1268 行)
 *    - useFocusTrap    (reference: 1281 行)
 *    - statusTag       (reference: 2520 行)
 * 约定：P3 业务功能补齐时按需接入这些工具
 * ════════════════════════════════════════════════════════════════ */

const ERROR_BANNER_ICONS: Record<string, string> = {
  error:   '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M12 2L1 21h22L12 2zm0 6l7.5 13h-15L12 8zm-1 4v4h2v-4h-2zm0 6v2h2v-2h-2z"/></svg>',
  warning: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M1 21h22L12 2 1 21zm12-3h-2v-2h2v2zm0-4h-2v-4h2v4z"/></svg>',
  info:    '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-6h2v6zm0-8h-2V7h2v2z"/></svg>',
  success: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41L9 16.17z"/></svg>',
};

export function ErrorBanner({
  variant = 'info', title, message, action, dismissable = false, onDismiss,
}: any) {
  const variantClass =
    variant === 'error'   ? 'bg-destructive/8 border-l-destructive text-destructive'
    : variant === 'success' ? 'bg-success/8 border-l-success text-success'
    : variant === 'warning' ? 'bg-warning/8 border-l-warning text-warning'
    : 'bg-info/8 border-l-info text-info';
  return (
    <div
      className={`flex items-start gap-3 p-3 rounded-md border-l-[3px] mb-4 ${variantClass}`}
      role="alert"
      aria-live="assertive"
    >
      <span
        className="w-5 h-5 shrink-0"
        dangerouslySetInnerHTML={{ __html: ERROR_BANNER_ICONS[variant] || ERROR_BANNER_ICONS.info }}
      />
      <div className="flex-1">
        <strong className="block font-semibold text-foreground">{title}</strong>
        {message ? <p className="mt-1 text-xs text-muted-foreground">{message}</p> : null}
      </div>
      {action ? (
        <button
          type="button"
          className="h-7 px-2.5 rounded-md text-xs font-medium border border-border bg-card text-foreground cursor-pointer hover:bg-accent transition-colors whitespace-nowrap shrink-0"
          onClick={action.onClick}
        >
          {action.label}
        </button>
      ) : null}
      {dismissable ? (
        <button
          type="button"
          className="w-6 h-6 rounded flex items-center justify-center text-muted-foreground hover:bg-accent transition-colors border-0 bg-transparent cursor-pointer ml-2 shrink-0"
          aria-label="关闭"
          onClick={onDismiss}
        >
          ✕
        </button>
      ) : null}
    </div>
  );
}

export function useLoadingState() {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  function run(promiseFactory: () => Promise<any>, options?: { onSuccess?: (r: any) => void; onError?: (e: any) => void }) {
    setLoading(true);
    setError(null);
    return Promise.resolve()
      .then(() => promiseFactory())
      .then((result) => {
        setLoading(false);
        if (options && options.onSuccess) options.onSuccess(result);
        return result;
      })
      .catch((err: any) => {
        setLoading(false);
        const msg = (err && err.message) || '操作失败';
        setError(msg);
        if (options && options.onError) options.onError(err);
      });
  }
  function reset() {
    setLoading(false);
    setError(null);
  }
  return { loading, error, run, reset };
}

export function useFocusTrap(open: boolean, containerRef: React.RefObject<HTMLElement>, onClose: () => void) {
  useEffect(() => {
    if (!open) return;
    const prevFocus = document.activeElement as HTMLElement | null;
    const el = containerRef.current;
    if (el) {
      const first = el.querySelector<HTMLElement>(
        'input:not([type="hidden"]), select, textarea, button:not([aria-label*="关闭"])'
      );
      if (first) first.focus();
    }
    function handler(e: KeyboardEvent) {
      if (e.key === 'Escape') { onClose(); return; }
      if (e.key !== 'Tab') return;
      const container = containerRef.current;
      if (!container) return;
      const focusable = container.querySelectorAll<HTMLElement>(
        'input, select, textarea, button:not([disabled])'
      );
      if (!focusable.length) return;
      const firstEl = focusable[0];
      const lastEl = focusable[focusable.length - 1];
      if (e.shiftKey && document.activeElement === firstEl) {
        e.preventDefault();
        lastEl.focus();
      } else if (!e.shiftKey && document.activeElement === lastEl) {
        e.preventDefault();
        firstEl.focus();
      }
    }
    document.addEventListener('keydown', handler);
    return () => {
      document.removeEventListener('keydown', handler);
      if (prevFocus && prevFocus.focus) prevFocus.focus();
    };
  }, [open, containerRef, onClose]);
}

const STATUS_TAG_COLORS: Record<string, { bg: string; fg: string; label: string }> = {
  healthy:  { bg: 'bg-success/8',   fg: 'text-success',   label: '正常' },
  valid:    { bg: 'bg-success/8',   fg: 'text-success',   label: '正常' },
  warning:  { bg: 'bg-warning/8',   fg: 'text-warning',   label: '告警' },
  expiring: { bg: 'bg-warning/8',   fg: 'text-warning',   label: '告警' },
  expired:  { bg: 'bg-destructive/8', fg: 'text-destructive', label: '已过期' },
  destructive: { bg: 'bg-destructive/8', fg: 'text-destructive', label: '错误' },
  unknown:  { bg: 'bg-info/8',      fg: 'text-info',      label: '未知' },
  info:     { bg: 'bg-info/8',      fg: 'text-info',      label: '信息' },
};

export function statusTag(status: string) {
  const m = STATUS_TAG_COLORS[status] || STATUS_TAG_COLORS.unknown;
  return <span className={`inline-block px-1.5 py-0.5 rounded text-[10px] font-medium ${m.bg} ${m.fg}`}>{m.label}</span>;
}
export function MetricCard({ label, value, meta }: { label: string; value: string; meta?: string }) {
  return (
    <div className="bg-card border border-border rounded-lg p-4 flex-1">
      <div className="text-xs text-muted-foreground mb-1">{label}</div>
      <div className="text-2xl font-bold text-foreground">{value}</div>
      {meta ? <div className="text-xs text-muted-foreground mt-1">{meta}</div> : null}
    </div>
  );
}

/* ── Badge ── */

export function Badge({ text, variant }: { text: string; variant: 'success' | 'warning' | 'danger' | 'default' }) {
  const colors: Record<string, string> = {
    success: 'bg-success/15 text-success',
    warning: 'bg-warning/15 text-warning',
    danger: 'bg-destructive/15 text-destructive',
    default: 'bg-muted text-muted-foreground',
  };
  return <span className={'inline-flex items-center px-2 py-0.5 rounded text-xs font-medium ' + (colors[variant] ?? colors.default)}>{text}</span>;
}



export function DataTable({ headers, rows, emptyText }: {
  headers: string[]; rows: Array<Array<React.ReactNode | string>>; emptyText?: string;
}) {
  if (rows.length === 0) {
    return (
      <div className="bg-card border border-border rounded-lg overflow-hidden">
        <table className="w-full border-collapse">
          <thead><tr>{headers.map((h) => <th key={h} className="px-3 py-2 text-left text-[10.5px] font-semibold text-muted-foreground uppercase tracking-wider bg-muted/35 border-b border-border">{h}</th>)}</tr></thead>
        </table>
        <div className="flex flex-col items-center py-10 text-muted-foreground">
          <p className="text-sm">{emptyText ?? '暂无数据'}</p>
        </div>
      </div>
    );
  }
  const rowsEl = rows.map((row, i) => (
    <tr key={i} className="border-b border-border last:border-b-0 hover:bg-muted/35">
      {row.map((cell, j) => <td key={j} className="px-3 py-2 text-sm text-foreground">{cell}</td>)}
    </tr>
  ));
  return (
    <div className="bg-card border border-border rounded-lg overflow-hidden">
      <table className="w-full border-collapse">
        <thead><tr>{headers.map((h) => <th key={h} className="px-3 py-2 text-left text-[10.5px] font-semibold text-muted-foreground uppercase tracking-wider bg-muted/35 border-b border-border">{h}</th>)}</tr></thead>
        <tbody>{rowsEl}</tbody>
      </table>
    </div>
  );
}

/* ── Spinner / Error ── */

export function FilterTabs({ items, active, onChange }: {
  items: Array<{ key: string; label: string }>; active: string; onChange: (key: string) => void;
}) {
  return (
    <div className="flex items-center gap-1 mb-3 flex-wrap">
      {items.map((item) => (
        <button key={item.key} onClick={() => onChange(item.key)}
          className={'px-3 py-1 rounded-lg text-xs font-medium cursor-pointer border transition-colors whitespace-nowrap ' + (
            active === item.key
              ? 'border-primary bg-primary/10 text-primary'
              : 'border-border bg-card text-muted-foreground hover:border-primary'
          )}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}

/* ── Data Table ── */

export function StatusDot({ status, label }: { status: string; label?: string }) {
  const colors: Record<string, { bg: string; icon: string }> = {
    pass: { bg: 'hsl(var(--success))', icon: '\u2713' },
    reject: { bg: 'hsl(var(--destructive))', icon: '\u2715' },
    pending: { bg: 'hsl(var(--muted-foreground))', icon: '\u25cb' },
    'in-review': { bg: 'hsl(var(--info))', icon: '\u27f3' },
  };
  const c = colors[status] ?? colors.pending;
  return (
    <span className="inline-flex items-center gap-1">
      <span className="inline-flex items-center justify-center w-2.5 h-2.5 rounded-full text-[8px] font-bold text-white" style={{ background: c.bg }}>{c.icon}</span>
      {label ? <span className="text-xs text-muted-foreground">{label}</span> : null}
    </span>
  );
}

export function UtilBar({ value }: { value: number }) {
  const pct = Math.min(100, value);
  const color = pct >= 100 ? 'hsl(var(--destructive))' : pct >= 85 ? 'hsl(var(--warning))' : 'hsl(var(--primary))';
  return (
    <div className="flex items-center gap-2">
      <div className="w-16 h-1.5 bg-muted rounded-full overflow-hidden">
        <div className="h-full rounded-full" style={{ width: pct + '%', background: color }} />
      </div>
      <span className="text-xs text-muted-foreground">{value}%</span>
    </div>
  );
}

/* ── Filters ── */
