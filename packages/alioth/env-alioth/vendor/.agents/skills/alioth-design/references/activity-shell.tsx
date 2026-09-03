/**
 * activity-shell.tsx — OpenActivity 简化入口共享 Shell 组件集（WZ-TMS Gateway 简化前端入口）。
 *
 * 与 gateway-shell.tsx 同级、同契约：纯 Tailwind 工具类实现，无 @alioth/components、
 * react-router、Jotai 依赖，供 OpenActivity 的 App/Module/Block 级产物壳复用。
 *
 * 与 GatewayShell 的差异（简化入口契约）：
 * - 移除工作台逻辑：ActionGroup 无「工作台」href="/" 链接；无视角切换；登录后直达 App/index。
 * - 保留认证三页：ActivityAuthShell 承载 登录 / 注册 / 授权申请。
 * - WorkspaceDock 默认槽位缩减为：全文搜索 / 站内信 / 日历（DEFAULT_ACTIVITY_TRIGGERS），
 *   不含 AI 助理、审批、通讯录。
 * - TopBar 右侧保留：全文搜索（SearchSlot）、语言切换（LanguageSwitch）、
 *   明暗主题（ThemeToggle）、个人中心（UserMenu）。
 * - 沿用 Block→Module→App 集成模式：侧栏 Navigation + ModuleTabs 与 GatewayShell 一致。
 *
 * 组件: ActivityShell / ActivityAuthShell / ActivityTopBar / ActivityNavigation /
 * MainNav / NavItem / ActivityLogo / ModuleTabs / Breadcrumbs / SearchSlot /
 * LanguageSwitch / ThemeToggle / ActionGroup / UserMenu / ActivityFooter /
 * ActivityWorkspaceDock / MobileSheet
 */
import { useState, useEffect, useRef, useCallback } from 'react';

// 轻量 cn: 与生产代码的 tailwind-merge 行为等价,仅做过滤拼接
function cn(...inputs: Array<string | false | null | undefined>): string {
  return inputs.filter(Boolean).join(' ');
}

// 全局 window 类型,避免 any
// 注: prototype-tool.js 构建的 icon-pool.js 会在 window 上注册 SvgIcon + ICONS
declare global {
  interface Window {
    SvgIcon?: React.ComponentType<{ html: string; size?: number }>;
    ICONS?: Record<string, string>;
  }
}

const SvgIcon = window.SvgIcon;
const ICONS = window.ICONS || {};

function icon(key: string, size = 16) {
  return SvgIcon && ICONS[key] ? (
    <SvgIcon html={ICONS[key]} size={size} />
  ) : (
    <span style={{ fontSize: size - 2 }}>•</span>
  );
}

// ── 类型 ──
export interface NavItemDef {
  id: string;
  label: string;
  icon: string;
  href?: string;
  badge?: string | number;
  children?: NavItemDef[];
  section?: string;
}

export interface NavGroup {
  label: string;
  items: NavItemDef[];
}

export interface ModuleTab {
  id: string;
  label: string;
  icon?: string;
  active?: boolean;
}

export interface Breadcrumb {
  label: string;
  href?: string;
}

export interface WorkspaceTrigger {
  id: string;
  icon: string;
  title: string;
  pendingCount?: number;
  unreadCount?: number;
}

export interface User {
  name: string;
  email: string;
  role?: string;
}

export interface LocaleOption {
  code: string;
  label: string;
}

/**
 * OpenActivity WorkspaceDock 默认槽位：全文搜索 / 站内信 / 日历。
 * （Gateway 完整版的 ai / approval / contacts 槽位在简化入口中移除。）
 */
export const DEFAULT_ACTIVITY_TRIGGERS: WorkspaceTrigger[] = [
  { id: 'search', icon: 'search', title: '全文搜索' },
  { id: 'inbox', icon: 'mail', title: '站内信' },
  { id: 'calendar', icon: 'calendar', title: '日历' },
];

// ── 子组件 ──

export function ActivityLogo({
  brand,
  homeHref,
  homeTitle,
  showAppName,
  pageTitle,
}: {
  brand: string;
  homeHref?: string;
  homeTitle?: string;
  showAppName?: string;
  pageTitle?: string;
}) {
  const displayText = showAppName || pageTitle || brand;
  return (
    <a
      href={homeHref || '#'}
      className={cn(
        'flex items-center gap-2.5 transition-colors hover:opacity-80 overflow-hidden no-underline shrink-0',
        showAppName && 'w-60',
      )}
      title={homeTitle || '返回应用首页'}
    >
      <svg className="w-7 h-7 text-primary shrink-0" viewBox="0 0 32 32" fill="none">
        <path
          d="M4 28V14C4 8.477 8.477 4 14 4H18C23.523 4 28 8.477 28 14V28"
          stroke="currentColor"
          strokeWidth="2.2"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
        <path
          d="M12 28V20C12 17.791 13.791 16 16 16C18.209 16 20 17.791 20 20V28"
          stroke="currentColor"
          strokeWidth="2.2"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
        <path d="M16 10V13" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
        <circle cx="16" cy="7" r="1.5" fill="currentColor" />
      </svg>
      <span className="text-lg font-bold hidden sm:inline truncate">{displayText}</span>
    </a>
  );
}

export function ModuleTabs({
  tabs,
  onTabClick,
}: {
  tabs: ModuleTab[];
  onTabClick?: (id: string) => void;
}) {
  if (!tabs || tabs.length === 0) return null;
  return (
    <div className="relative flex items-center min-w-0 flex-1" data-testid="scroll-tabs">
      <div
        role="tablist"
        aria-label="模块导航"
        className="flex items-center gap-0.5 overflow-x-auto hide-scrollbar min-w-0 flex-1 "
      >
        {tabs.map((t) => {
          const isActive = !!t.active;
          return (
            <button
              key={t.id}
              onClick={() => onTabClick && onTabClick(t.id)}
              role="tab"
              aria-selected={isActive}
              data-tab-active={isActive}
              className={cn(
                'relative flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium rounded-t-lg transition-all duration-150 whitespace-nowrap',
                isActive
                  ? 'text-foreground bg-background shadow-tab border-x border-t border-border z-10 before:absolute before:bottom-[-1px] before:left-[6px] before:right-[6px] before:h-[2px] before:bg-primary before:rounded-t-[1px]'
                  : 'text-muted-foreground hover:bg-accent/60 border-b border-transparent',
              )}
              title={t.label}
            >
              {t.icon && <span className="w-4 h-4 shrink-0">{icon(t.icon)}</span>}
              <span>{t.label}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

export function Breadcrumbs({ crumbs }: { crumbs: Breadcrumb[] }) {
  if (!crumbs || crumbs.length === 0) return null;
  return (
    <nav className="hidden sm:flex items-center gap-1.5 text-sm">
      {crumbs.map((c, i) => (
        <span key={i} className="flex items-center gap-1.5">
          {i > 0 && <span className="text-muted-foreground/50">/</span>}
          {i === crumbs.length - 1 ? (
            <span className="text-foreground font-medium">{c.label}</span>
          ) : (
            <a
              href={c.href || '#'}
              className="text-muted-foreground hover:text-foreground no-underline"
            >
              {c.label}
            </a>
          )}
        </span>
      ))}
    </nav>
  );
}

/** 全文搜索（TopBar 内联槽位） */
export function SearchSlot({ placeholder }: { placeholder?: string }) {
  const [expanded, setExpanded] = useState(false);
  const [value, setValue] = useState('');
  const resolvedPlaceholder = placeholder || '全文搜索...';

  if (expanded) {
    return (
      <div className="absolute left-0 right-0 top-full bg-card border-b p-3 shadow-lg z-50 md:static md:bg-transparent md:border-0 md:p-0 md:shadow-none">
        <div className="relative w-full">
          <input
            type="search"
            autoFocus
            placeholder={resolvedPlaceholder}
            className="w-full h-10 pl-3 pr-9 rounded-lg border bg-muted/50 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/20 focus:border-primary/30 transition-colors"
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
          <button
            type="button"
            onClick={() => setExpanded(false)}
            className="absolute right-3 top-1/2 -translate-y-1/2 p-1 rounded hover:bg-accent md:hidden"
            aria-label="关闭搜索"
          >
            <span className="w-4 h-4">{icon('x', 16)}</span>
          </button>
        </div>
      </div>
    );
  }

  return (
    <>
      <div className="relative w-72 hidden md:block">
        <input
          type="search"
          placeholder={resolvedPlaceholder}
          className="w-full h-9 pl-3 pr-3 rounded-lg border bg-muted/50 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/20 focus:border-primary/40 focus:bg-background transition-colors"
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />
      </div>
      <button
        type="button"
        onClick={() => setExpanded(true)}
        className="md:hidden w-9 h-9 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors"
        aria-label="搜索"
        title="搜索"
      >
        <span className="w-4 h-4">{icon('search', 16)}</span>
      </button>
    </>
  );
}

/** 语言切换（OpenActivity 新增；GatewayShell 无此组件） */
export function LanguageSwitch({
  locales,
  value,
  onChange,
}: {
  locales?: LocaleOption[];
  value?: string;
  onChange?: (code: string) => void;
}) {
  const options =
    locales && locales.length > 0
      ? locales
      : [
          { code: 'zh-CN', label: '简体中文' },
          { code: 'en', label: 'English' },
        ];
  const [internal, setInternal] = useState(value || options[0].code);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const current = value || internal;

  useEffect(() => {
    if (!open) return undefined;
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handleClick);
    return () => document.removeEventListener('mousedown', handleClick);
  }, [open]);

  const select = (code: string) => {
    setInternal(code);
    setOpen(false);
    if (typeof document !== 'undefined') document.documentElement.lang = code;
    onChange?.(code);
  };

  const currentLabel = options.find((o) => o.code === current)?.label || current;

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        onClick={() => setOpen((p) => !p)}
        className={cn(
          'hidden sm:flex w-9 h-9 rounded-lg items-center justify-center text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors cursor-pointer border-none bg-transparent',
          open && 'bg-accent text-accent-foreground',
        )}
        title={`语言：${currentLabel}`}
        aria-label="语言切换"
        aria-expanded={open}
      >
        <span className="w-4 h-4">{icon('translate')}</span>
      </button>
      {open && (
        <div className="absolute right-0 top-full mt-1 w-36 rounded-lg border bg-card shadow-lg py-1 z-50 max-w-[calc(100vw-1rem)]">
          {options.map((o) => (
            <button
              key={o.code}
              type="button"
              onClick={() => select(o.code)}
              className={cn(
                'w-full flex items-center gap-2 px-3 py-2 text-sm text-left transition-colors cursor-pointer border-none bg-transparent',
                o.code === current
                  ? 'text-primary font-semibold bg-primary/10'
                  : 'text-foreground hover:bg-accent',
              )}
            >
              {o.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** 明暗主题切换 */
export function ThemeToggle() {
  const [isDark, setDark] = useState(
    typeof document !== 'undefined' && document.documentElement.classList.contains('dark'),
  );
  const toggleTheme = useCallback(() => {
    const next = !isDark;
    document.documentElement.classList.toggle('dark', next);
    setDark(next);
  }, [isDark]);

  return (
    <button
      type="button"
      onClick={toggleTheme}
      className="hidden sm:flex w-9 h-9 rounded-lg items-center justify-center text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors cursor-pointer border-none bg-transparent"
      title={isDark ? '切换到浅色' : '切换到深色'}
    >
      <span className="w-4 h-4">{icon(isDark ? 'sun' : 'moon')}</span>
    </button>
  );
}

/**
 * 右侧动作组：语言切换 + 明暗主题 + WorkspaceDock 触发器（全文搜索/站内信/日历）。
 * 与 Gateway 差异：无「工作台」链接（简化入口移除工作台逻辑）。
 */
export function ActionGroup({
  triggers,
  onTrigger,
  locales,
  locale,
  onLocaleChange,
}: {
  triggers: WorkspaceTrigger[];
  onTrigger?: (id: string) => void;
  locales?: LocaleOption[];
  locale?: string;
  onLocaleChange?: (code: string) => void;
}) {
  return (
    <div className="flex items-center gap-3">
      <LanguageSwitch locales={locales} value={locale} onChange={onLocaleChange} />
      <ThemeToggle />
      {triggers.map((t) => (
        <button
          key={t.id}
          type="button"
          onClick={() => onTrigger?.(t.id)}
          className="relative w-9 h-9 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors cursor-pointer border-none bg-transparent"
          title={t.title}
        >
          <span className="w-4 h-4">{icon(t.icon)}</span>
          {t.pendingCount || t.unreadCount ? (
            <span className="absolute -top-0.5 -right-0.5 min-w-4 h-4 rounded-full bg-destructive text-destructive-foreground text-[8px] leading-none flex items-center justify-center font-bold px-1 border-2 border-card">
              {t.unreadCount || t.pendingCount}
            </span>
          ) : null}
        </button>
      ))}
    </div>
  );
}

/** 个人中心 */
export function UserMenu({
  user,
  onLogout,
  onProfile,
  onSettings,
}: {
  user: User;
  onLogout?: () => void;
  /** 个人资料（缺省仅收起菜单） */
  onProfile?: () => void;
  /** 设置（缺省仅收起菜单） */
  onSettings?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const initial = (user.name || 'U').charAt(0).toUpperCase();

  useEffect(() => {
    if (!open) return undefined;
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handleClick);
    return () => document.removeEventListener('mousedown', handleClick);
  }, [open]);

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        onClick={() => setOpen((p) => !p)}
        className={cn(
          'flex items-center gap-1.5 cursor-pointer border-none bg-transparent p-1 rounded-lg transition-colors hover:bg-accent',
          open && 'bg-accent',
        )}
        aria-label="用户菜单"
        aria-expanded={open}
      >
        <div className="w-7 h-7 rounded-md bg-primary/10 flex items-center justify-center shrink-0">
          <span className="text-xs font-bold text-primary">{initial}</span>
        </div>
        <span
          className={cn('w-3 h-3 text-muted-foreground transition-transform', open && 'rotate-180')}
        >
          {icon('chevronDown', 12)}
        </span>
      </button>
      {open && (
        <div className="absolute right-0 top-full mt-1 w-56 rounded-lg border bg-card shadow-lg py-1 z-50 max-w-[calc(100vw-1rem)]">
          <div className="px-3 py-2 border-b border-border">
            <p className="text-sm font-medium truncate">{user.name}</p>
            <p className="text-xs text-muted-foreground truncate">{user.email}</p>
          </div>
          <a
            href="#"
            className="flex items-center gap-2 px-3 py-2 text-sm text-foreground hover:bg-accent no-underline transition-colors"
            onClick={(e) => {
              e.preventDefault();
              setOpen(false);
              onProfile?.();
            }}
          >
            <span className="w-4 h-4">{icon('user')}</span>个人资料
          </a>
          <a
            href="#"
            className="flex items-center gap-2 px-3 py-2 text-sm text-foreground hover:bg-accent no-underline transition-colors"
            onClick={(e) => {
              e.preventDefault();
              setOpen(false);
              onSettings?.();
            }}
          >
            <span className="w-4 h-4">{icon('settings')}</span>设置
          </a>
          <div className="border-t border-border mt-1 pt-1">
            <button
              type="button"
              className="w-full flex items-center gap-2 px-3 py-2 text-sm text-destructive hover:bg-destructive/10 transition-colors cursor-pointer border-none bg-transparent"
              onClick={() => {
                setOpen(false);
                onLogout?.();
              }}
            >
              <span className="w-4 h-4">{icon('logOut')}</span>退出登录
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

export function ActivityTopBar({
  brand,
  moduleTabs,
  breadcrumbs,
  searchPlaceholder,
  triggers,
  user,
  onTrigger,
  onLogout,
  onModuleTabChange,
  onMobileMenuToggle,
  locales,
  locale,
  onLocaleChange,
  onProfile,
  onSettings,
  homeHref,
}: {
  brand: string;
  moduleTabs?: ModuleTab[];
  breadcrumbs?: Breadcrumb[];
  searchPlaceholder?: string;
  triggers?: WorkspaceTrigger[];
  user: User;
  onTrigger?: (id: string) => void;
  onLogout?: () => void;
  onProfile?: () => void;
  onSettings?: () => void;
  onModuleTabChange?: (id: string) => void;
  onMobileMenuToggle?: () => void;
  locales?: LocaleOption[];
  locale?: string;
  onLocaleChange?: (code: string) => void;
  homeHref?: string;
}) {
  return (
    <header className="h-14 border-b flex items-center justify-between px-4 md:px-6 bg-background shrink-0">
      <div className="flex items-center gap-2 min-w-0 relative h-full">
        {onMobileMenuToggle && (
          <button
            type="button"
            onClick={onMobileMenuToggle}
            className="md:hidden p-2 rounded-lg hover:bg-accent transition-colors"
            aria-label="打开菜单"
          >
            <span className="w-5 h-5">{icon('menu', 20)}</span>
          </button>
        )}
        <ActivityLogo
          brand={brand}
          homeHref={homeHref}
          showAppName={moduleTabs && moduleTabs.length > 0 ? brand : undefined}
        />
        {moduleTabs && moduleTabs.length > 0 && (
          <ModuleTabs tabs={moduleTabs} onTabClick={onModuleTabChange} />
        )}
        {breadcrumbs && breadcrumbs.length > 0 && <Breadcrumbs crumbs={breadcrumbs} />}
      </div>
      <div className="flex items-center gap-3">
        <SearchSlot placeholder={searchPlaceholder} />
        <ActionGroup
          triggers={triggers || []}
          onTrigger={onTrigger}
          locales={locales}
          locale={locale}
          onLocaleChange={onLocaleChange}
        />
        <UserMenu user={user} onLogout={onLogout} onProfile={onProfile} onSettings={onSettings} />
      </div>
    </header>
  );
}

export function ActivityFooter({
  brand,
  version,
  links,
}: {
  brand: string;
  version: string;
  links?: { label: string; href?: string }[];
}) {
  const defaultLinks =
    links === undefined
      ? [
          { label: '帮助', href: '#' },
          { label: '隐私', href: '#' },
        ]
      : links;
  return (
    <footer className="hidden md:flex shrink-0 h-10 items-center justify-between border-t bg-card px-4 md:px-6 text-xs text-muted-foreground">
      <span className="truncate">© 2026 {brand}</span>
      <div className="flex items-center gap-4">
        <nav className="hidden md:flex items-center gap-4">
          {defaultLinks.map((l) => (
            <a
              key={l.label}
              href={l.href}
              className="hover:text-foreground transition-colors no-underline"
            >
              {l.label}
            </a>
          ))}
        </nav>
        <span className="hidden sm:inline truncate">{version}</span>
      </div>
    </footer>
  );
}

export function ActivityWorkspaceDock({
  active,
  title,
  onClose,
  children,
}: {
  active: boolean;
  title: string;
  onClose: () => void;
  children?: React.ReactNode;
}) {
  return (
    <div
      className={cn(
        'hidden md:flex shrink-0 overflow-hidden transition-all duration-300 ease-in-out',
        active ? 'w-80 border-l' : 'w-0 border-l-0',
      )}
    >
      <div className="w-80 h-full overflow-y-auto shrink-0 flex flex-col border-l border-border bg-card">
        <div className="h-14 border-b border-border flex items-center justify-between px-4 shrink-0">
          <span className="text-sm font-semibold text-foreground">{title}</span>
          <button
            type="button"
            onClick={onClose}
            className="w-7 h-7 rounded-md flex items-center justify-center text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors cursor-pointer border-none bg-transparent"
            title="关闭"
          >
            <span className="w-4 h-4">{icon('panelRight', 16)}</span>
          </button>
        </div>
        <div className="flex-1 overflow-y-auto p-4">
          {children || <div className="text-sm text-muted-foreground p-4">{title}内容区</div>}
        </div>
      </div>
    </div>
  );
}

function MobileSheet({
  open,
  onClose,
  brand,
  children,
}: {
  open: boolean;
  onClose: () => void;
  brand: string;
  children: React.ReactNode;
}) {
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 md:hidden">
      <div className="absolute inset-0 bg-black/20" onClick={onClose} />
      <div className="absolute top-0 left-0 h-full w-[85vw] bg-background flex flex-col shadow-xl">
        <div className="h-14 flex items-center px-6 border-b border-border shrink-0">
          <a href="#" className="flex items-center gap-2.5 no-underline" onClick={onClose}>
            <span className="w-6 h-6 text-primary">{icon('gatewayLogo', 24)}</span>
            <span className="text-lg font-bold">{brand}</span>
          </a>
        </div>
        <div className="flex-1 overflow-hidden">{children}</div>
      </div>
    </div>
  );
}

export function MainNav({
  groups,
  activeId,
  collapsed,
  onSelect,
}: {
  groups: NavGroup[];
  activeId: string;
  collapsed: boolean;
  onSelect: (id: string) => void;
}) {
  return (
    <nav className="flex h-full flex-col">
      <div className="flex-1 overflow-y-auto hide-scrollbar px-0 py-2">
        <div className="space-y-4">
          {groups.map((g, gi) => (
            <div key={g.label ?? `__group_${gi}`} className="space-y-1">
              {!collapsed && (
                <div className="px-4 py-3.5 pb-1">
                  <span className="text-[10px] font-bold uppercase tracking-[0.06em] text-muted-foreground/55">
                    {g.label}
                  </span>
                </div>
              )}
              {g.items.map((it) => (
                <NavItem
                  key={it.id}
                  item={it}
                  active={it.id === activeId}
                  collapsed={collapsed}
                  onClick={onSelect}
                />
              ))}
            </div>
          ))}
        </div>
      </div>
    </nav>
  );
}

export function NavItem({
  item,
  active,
  collapsed,
  onClick,
}: {
  item: NavItemDef;
  active: boolean;
  collapsed: boolean;
  onClick: (id: string) => void;
}) {
  const badge = item.badge;
  return (
    <button
      type="button"
      onClick={() => onClick(item.id)}
      className={cn(
        'flex items-center rounded-md text-sm font-medium transition-colors',
        collapsed
          ? 'justify-center h-9 w-9 mx-auto my-0.5 px-2'
          : 'w-[calc(100%-1rem)] mx-2 gap-2.5 py-2 px-4',
        active
          ? 'bg-primary/10 text-primary font-semibold'
          : 'text-muted-foreground hover:bg-accent hover:text-foreground',
      )}
      title={item.label}
    >
      <span className="h-4 w-4 shrink-0">{icon(item.icon)}</span>
      {!collapsed && (
        <>
          <span className="flex-1 truncate text-left">{item.label}</span>
          {badge !== undefined && (
            <span
              className={cn(
                'inline-flex items-center justify-center h-5 min-w-5 px-1 text-xs font-medium border-0 rounded-md shrink-0',
                active ? 'bg-primary/15 text-primary' : 'bg-muted text-muted-foreground',
              )}
            >
              {badge}
            </span>
          )}
        </>
      )}
    </button>
  );
}

export function ActivityNavigation({
  groups,
  activeId,
  collapsed,
  onSelect,
  onToggle,
  mobileOpen,
  onMobileClose,
  brand,
}: {
  groups: NavGroup[];
  activeId: string;
  collapsed: boolean;
  onSelect: (id: string) => void;
  onToggle: () => void;
  mobileOpen?: boolean;
  onMobileClose?: () => void;
  brand?: string;
}) {
  return (
    <>
      <aside
        className={cn(
          'hidden md:flex flex-col h-full bg-secondary border-r border-border transition-all duration-300 ease-in-out shrink-0',
          collapsed ? 'w-16' : 'w-60',
        )}
      >
        <MainNav groups={groups} activeId={activeId} collapsed={collapsed} onSelect={onSelect} />
        <SidebarFoot collapsed={collapsed} onToggle={onToggle} />
      </aside>
      <MobileSheet open={!!mobileOpen} onClose={onMobileClose || (() => {})} brand={brand || 'OpenActivity'}>
        <div className="flex flex-col h-full">
          <MainNav groups={groups} activeId={activeId} collapsed={false} onSelect={onSelect} />
          <SidebarFoot collapsed={false} onToggle={onMobileClose || (() => {})} />
        </div>
      </MobileSheet>
    </>
  );
}

export function SidebarFoot({
  collapsed,
  onToggle,
}: {
  collapsed: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="shrink-0 flex items-center border-t border-border px-3 h-10 gap-2">
      <button
        type="button"
        onClick={onToggle}
        className="w-7 h-7 rounded-md flex items-center justify-center text-muted-foreground hover:bg-accent hover:text-foreground transition-colors cursor-pointer border-none bg-transparent"
        title={collapsed ? '展开侧栏' : '折叠侧栏'}
      >
        <span className="w-3.5 h-3.5">{icon(collapsed ? 'panelRight' : 'panelLeft', 14)}</span>
      </button>
    </div>
  );
}

// ── ActivityShell（登录后 App 模式，直达 App/index，无工作台） ──
export interface ActivityShellProps {
  brand: string;
  moduleTabs?: ModuleTab[];
  breadcrumbs?: Breadcrumb[];
  searchPlaceholder?: string;
  /** WorkspaceDock 触发器；缺省为 DEFAULT_ACTIVITY_TRIGGERS（全文搜索/站内信/日历） */
  triggers?: WorkspaceTrigger[];
  user: User;
  onTrigger?: (id: string) => void;
  navGroups?: NavGroup[];
  activeId?: string;
  collapsed?: boolean;
  onSelect?: (id: string) => void;
  onLogout?: () => void;
  onToggle?: () => void;
  activeWorkspace?: string | null;
  onWorkspaceClose?: () => void;
  /** App 级：Tab 切换回调 */
  onModuleTabChange?: (id: string) => void;
  workspaceTitle?: string;
  workspaceChildren?: React.ReactNode;
  rootClass?: string;
  children: React.ReactNode;
  footerBrand?: string;
  footerVersion?: string;
  footerLinks?: { label: string; href?: string }[];
  showAccent?: boolean;
  hideNavigation?: boolean;
  /** App 级模式：隐藏 Footer */
  hideFooter?: boolean;
  /** App 级模式：children 不包 overflow-y-auto（由 Module 自己提供滚动视口） */
  noContentScroll?: boolean;
  /** 隐藏右侧 WorkspaceDock，用于无 dock 的实现模块 */
  hideWorkspaceDock?: boolean;
  /** Logo 链接（缺省 '#'）；简化入口登录后直达 App/index，Logo 应指向 App index */
  homeHref?: string;
  /** 语言切换 */
  locales?: LocaleOption[];
  locale?: string;
  onLocaleChange?: (code: string) => void;
  /** 个人资料/设置（个人中心菜单回调） */
  onProfile?: () => void;
  onSettings?: () => void;
}

export function ActivityShell(props: ActivityShellProps) {
  const {
    brand,
    moduleTabs,
    breadcrumbs,
    searchPlaceholder,
    triggers,
    user,
    onTrigger,
    navGroups,
    activeId,
    collapsed = false,
    onSelect,
    onLogout,
    onToggle,
    activeWorkspace,
    onModuleTabChange,
    onWorkspaceClose,
    workspaceTitle,
    workspaceChildren,
    rootClass,
    children,
    footerBrand,
    footerVersion,
    footerLinks,
    showAccent = false,
    hideNavigation = false,
    hideFooter = false,
    noContentScroll = false,
    hideWorkspaceDock = false,
    homeHref,
    locales,
    locale,
    onLocaleChange,
    onProfile,
    onSettings,
  } = props;

  const resolvedTriggers = triggers === undefined ? DEFAULT_ACTIVITY_TRIGGERS : triggers;
  const [internalCollapsed, setInternalCollapsed] = useState(collapsed);
  const [mobileOpen, setMobileOpen] = useState(false);

  const effectiveCollapsed = onToggle ? collapsed : internalCollapsed;
  const handleToggle = onToggle || (() => setInternalCollapsed((p) => !p));

  const activeTrigger = activeWorkspace
    ? resolvedTriggers.find((t) => t.id === activeWorkspace)
    : undefined;
  const dockTitle = workspaceTitle || activeTrigger?.title || '工作区';

  return (
    <div className={cn('flex h-screen flex-col overflow-hidden bg-background', rootClass)}>
      <ActivityTopBar
        brand={brand}
        moduleTabs={moduleTabs}
        breadcrumbs={breadcrumbs}
        searchPlaceholder={searchPlaceholder}
        triggers={resolvedTriggers}
        user={user}
        onTrigger={onTrigger}
        onLogout={onLogout}
        onMobileMenuToggle={
          !hideNavigation && navGroups && navGroups.length > 0
            ? () => setMobileOpen(true)
            : undefined
        }
        onModuleTabChange={onModuleTabChange}
        homeHref={homeHref}
        locales={locales}
        locale={locale}
        onLocaleChange={onLocaleChange}
        onProfile={onProfile}
        onSettings={onSettings}
      />
      <div className="flex flex-1 min-h-0 overflow-hidden">
        {!hideNavigation && navGroups && navGroups.length > 0 && activeId != null && onSelect && (
          <ActivityNavigation
            groups={navGroups}
            activeId={activeId || ''}
            collapsed={effectiveCollapsed}
            onSelect={(id) => {
              onSelect?.(id);
              setMobileOpen(false);
            }}
            onToggle={handleToggle}
            mobileOpen={mobileOpen}
            onMobileClose={() => setMobileOpen(false)}
            brand={brand}
          />
        )}
        <div className="flex flex-col min-w-0 overflow-hidden flex-1">
          {showAccent && <div className="accent-bar h-[3px] w-full bg-primary/15 shrink-0" />}
          <main className="flex-1 w-full h-full bg-muted/30 overflow-hidden">
            <div className="flex flex-col h-full">
              <div className={cn('flex-1 min-h-0', !noContentScroll && 'overflow-y-auto')}>
                {children}
              </div>
              {!hideFooter && (
                <ActivityFooter
                  brand={footerBrand || brand}
                  version={footerVersion || ''}
                  links={footerLinks}
                />
              )}
            </div>
          </main>
        </div>
        {!hideWorkspaceDock && (
          <ActivityWorkspaceDock
            active={!!activeWorkspace}
            title={dockTitle}
            onClose={onWorkspaceClose || (() => {})}
          >
            {workspaceChildren}
          </ActivityWorkspaceDock>
        )}
      </div>
    </div>
  );
}

// ── ActivityAuthShell（登录 / 注册 / 授权申请 三页共享的简化认证壳） ──
export interface ActivityAuthShellProps {
  brand: string;
  /** 认证页标题（如「登录」「注册」「授权申请」），展示在卡片上方 */
  title?: string;
  /** 认证页副标题/说明 */
  subtitle?: string;
  children: React.ReactNode;
  footerBrand?: string;
  footerLinks?: { label: string; href?: string }[];
  /** 语言切换（未登录态也保留） */
  locales?: LocaleOption[];
  locale?: string;
  onLocaleChange?: (code: string) => void;
}

export function ActivityAuthShell(props: ActivityAuthShellProps) {
  const {
    brand,
    title,
    subtitle,
    children,
    footerBrand,
    footerLinks,
    locales,
    locale,
    onLocaleChange,
  } = props;

  return (
    <div className="flex min-h-screen flex-col bg-muted/30">
      {/* 顶部：品牌 + 语言/主题（无搜索、无工作台、无用户菜单） */}
      <header className="h-14 border-b flex items-center justify-between px-4 md:px-6 bg-background shrink-0">
        <ActivityLogo brand={brand} homeHref="#" homeTitle={brand} />
        <div className="flex items-center gap-3">
          <LanguageSwitch locales={locales} value={locale} onChange={onLocaleChange} />
          <ThemeToggle />
        </div>
      </header>
      {/* 居中认证卡片 */}
      <main className="flex-1 flex items-center justify-center p-4">
        <div className="w-full max-w-md">
          {(title || subtitle) && (
            <div className="mb-6 text-center">
              {title && <h1 className="text-2xl font-bold text-foreground">{title}</h1>}
              {subtitle && <p className="mt-2 text-sm text-muted-foreground">{subtitle}</p>}
            </div>
          )}
          <div className="rounded-xl border bg-card p-6 shadow-sm">{children}</div>
        </div>
      </main>
      <ActivityFooter
        brand={footerBrand || brand}
        version=""
        links={footerLinks}
      />
    </div>
  );
}

export default ActivityShell;
