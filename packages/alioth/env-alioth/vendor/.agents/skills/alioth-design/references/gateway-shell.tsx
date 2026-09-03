/**
 * gateway-shell.tsx — AliothStudio 原型共享 Gateway Shell 组件集(所有 namespace 单一事实源)。
 *
 * 位于 alioth-design 技能 references/ 下,供各 namespace 的 App/Module/Block 级产物壳共同复用;
 * 不再按 namespace 在 Pre-Proc/<ns>/Prototypes/_shared/ 下各自复制。
 *
 * 对齐当前 Gateway 前端生产组件的 Tailwind 视觉契约:
 * - Framework TopBar / MainNav / Footer / ScrollTabs
 * - Gateway TopBar / ModuleTabs / Navigation / ContentArea
 * 保持独立实现,不依赖 @alioth/components、react-router、Jotai。
 *
 * 组件: GatewayShell / TopBar / Navigation / MainNav / NavItem / Logo /
 * ModuleTabs / Breadcrumbs / SearchSlot / ActionGroup / UserMenu /
 * Footer / WorkspaceDock / MobileSheet
 */
import { useState, useEffect, useRef, useCallback, useMemo } from 'react';

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

// ── 子组件 ──

function Logo({
  icon: iconKey,
  brand,
  showAppName,
  pageTitle,
}: {
  icon: string;
  brand: string;
  showAppName?: string;
  pageTitle?: string;
}) {
  const displayText = showAppName || pageTitle || brand;
  return (
    <a
      href="#"
      className={cn(
        'flex items-center gap-2.5 transition-colors hover:opacity-80 overflow-hidden no-underline shrink-0',
        showAppName && 'w-60',
      )}
      title="返回 Gateway"
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

export function ModuleTabs({ tabs, onTabClick }: { tabs: ModuleTab[]; onTabClick?: (id: string) => void }) {
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

export function SearchSlot({ placeholder }: { placeholder?: string }) {
  const [expanded, setExpanded] = useState(false);
  const [value, setValue] = useState('');
  const resolvedPlaceholder = placeholder || '搜索应用、模块...';

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

export function ActionGroup({
  triggers,
  onTrigger,
}: {
  triggers: WorkspaceTrigger[];
  onTrigger?: (id: string) => void;
}) {
  const [isDark, setDark] = useState(
    typeof document !== 'undefined' && document.documentElement.classList.contains('dark'),
  );
  const isHome = typeof window !== 'undefined' && window.location.pathname === '/';
  const toggleTheme = useCallback(() => {
    const next = !isDark;
    document.documentElement.classList.toggle('dark', next);
    setDark(next);
  }, [isDark]);

  return (
    <div className="flex items-center gap-3">
      <a
        href="/"
        className={cn(
          'relative w-9 h-9 rounded-lg flex items-center justify-center transition-colors',
          isHome
            ? 'bg-primary/10 text-primary'
            : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
        )}
        title="工作台"
        aria-label="工作台"
      >
        <span className="w-4 h-4">{icon('layoutDashboard')}</span>
      </a>
      <button
        type="button"
        onClick={toggleTheme}
        className="hidden sm:flex w-9 h-9 rounded-lg items-center justify-center text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors cursor-pointer border-none bg-transparent"
        title={isDark ? '切换到浅色' : '切换到深色'}
      >
        <span className="w-4 h-4">{icon(isDark ? 'sun' : 'moon')}</span>
      </button>
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

export function UserMenu({ user, onLogout }: { user: User; onLogout?: () => void }) {
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
            onClick={() => setOpen(false)}
          >
            <span className="w-4 h-4">{icon('user')}</span>个人资料
          </a>
          <a
            href="#"
            className="flex items-center gap-2 px-3 py-2 text-sm text-foreground hover:bg-accent no-underline transition-colors"
            onClick={() => setOpen(false)}
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

export function TopBar({
  brand,
  brandIcon,
  moduleTabs,
  breadcrumbs,
  searchPlaceholder,
  triggers,
  user,
  onTrigger,
  onModuleTabChange,
  onMobileMenuToggle,
}: {
  brand: string;
  brandIcon: string;
  moduleTabs?: ModuleTab[];
  breadcrumbs?: Breadcrumb[];
  searchPlaceholder?: string;
  triggers?: WorkspaceTrigger[];
  user: User;
  onTrigger?: (id: string) => void;
  onModuleTabChange?: (id: string) => void;
  onMobileMenuToggle?: () => void;
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
        <Logo
          icon={brandIcon}
          brand={brand}
          showAppName={moduleTabs && moduleTabs.length > 0 ? brand : undefined}
        />
        {moduleTabs && moduleTabs.length > 0 && <ModuleTabs tabs={moduleTabs} onTabClick={onModuleTabChange} />}
        {breadcrumbs && breadcrumbs.length > 0 && <Breadcrumbs crumbs={breadcrumbs} />}
      </div>
      <div className="flex items-center gap-3">
        <SearchSlot placeholder={searchPlaceholder} />
        <ActionGroup triggers={triggers || []} onTrigger={onTrigger} />
        <UserMenu user={user} />
      </div>
    </header>
  );
}

export function Footer({
  brand,
  version,
  links,
}: {
  brand: string;
  version: string;
  links?: { label: string; href: string }[];
}) {
  const defaultLinks = links === undefined
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

export function WorkspaceDock({
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
  brandIcon,
  children,
}: {
  open: boolean;
  onClose: () => void;
  brand: string;
  brandIcon: string;
  children: React.ReactNode;
}) {
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 md:hidden">
      <div className="absolute inset-0 bg-black/20" onClick={onClose} />
      <div className="absolute top-0 left-0 h-full w-[85vw] bg-background flex flex-col shadow-xl">
        <div className="h-14 flex items-center px-6 border-b border-border shrink-0">
          <a href="#" className="flex items-center gap-2.5 no-underline" onClick={onClose}>
            <span className="w-6 h-6 text-primary">{icon(brandIcon, 24)}</span>
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

export function Navigation({
  groups,
  activeId,
  collapsed,
  onSelect,
  onToggle,
  mobileOpen,
  onMobileClose,
  brand,
  brandIcon,
}: {
  groups: NavGroup[];
  activeId: string;
  collapsed: boolean;
  onSelect: (id: string) => void;
  onToggle: () => void;
  mobileOpen?: boolean;
  onMobileClose?: () => void;
  brand?: string;
  brandIcon?: string;
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
      <MobileSheet
        open={!!mobileOpen}
        onClose={onMobileClose || (() => {})}
        brand={brand || 'Alioth'}
        brandIcon={brandIcon || 'gatewayLogo'}
      >
        <div className="flex flex-col h-full">
          <MainNav groups={groups} activeId={activeId} collapsed={false} onSelect={onSelect} />
          <SidebarFoot collapsed={false} onToggle={onMobileClose || (() => {})} />
        </div>
      </MobileSheet>
    </>
  );
}

export function SidebarFoot({ collapsed, onToggle }: { collapsed: boolean; onToggle: () => void }) {
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

// ── GatewayShell ──
export interface GatewayShellProps {
  brand: string;
  brandIcon: string;
  moduleTabs?: ModuleTab[];
  breadcrumbs?: Breadcrumb[];
  searchPlaceholder?: string;
  triggers?: WorkspaceTrigger[];
  user: User;
  onTrigger?: (id: string) => void;
  navGroups?: NavGroup[];
  activeId?: string;
  collapsed?: boolean;
  onSelect?: (id: string) => void;
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
}

export function GatewayShell(props: GatewayShellProps) {
  const {
    brand,
    brandIcon,
    moduleTabs,
    breadcrumbs,
    searchPlaceholder,
    triggers = [],
    user,
    onTrigger,
    navGroups,
    activeId,
    collapsed = false,
    onSelect,
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
  } = props;

  const [internalCollapsed, setInternalCollapsed] = useState(collapsed);
  const [mobileOpen, setMobileOpen] = useState(false);

  const effectiveCollapsed = onToggle ? collapsed : internalCollapsed;
  const handleToggle = onToggle || (() => setInternalCollapsed((p) => !p));

  const activeTrigger = activeWorkspace
    ? triggers.find((t) => t.id === activeWorkspace)
    : undefined;
  const dockTitle = workspaceTitle || activeTrigger?.title || '工作区';

  return (
    <div className={cn('flex h-screen flex-col overflow-hidden bg-background', rootClass)}>
      <TopBar
        brand={brand}
        brandIcon={brandIcon}
        moduleTabs={moduleTabs}
        breadcrumbs={breadcrumbs}
        searchPlaceholder={searchPlaceholder}
        triggers={triggers}
        user={user}
        onTrigger={onTrigger}
        onMobileMenuToggle={
          !hideNavigation && navGroups && navGroups.length > 0
            ? () => setMobileOpen(true)
            : undefined
        }
        onModuleTabChange={onModuleTabChange}
      />
      <div className="flex flex-1 min-h-0 overflow-hidden">
        {!hideNavigation && navGroups && navGroups.length > 0 && activeId != null && onSelect && (
          <Navigation
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
            brandIcon={brandIcon}
          />
        )}
        <div className="flex flex-col min-w-0 overflow-hidden flex-1">
          {showAccent && <div className="accent-bar h-[3px] w-full bg-primary/15 shrink-0" />}
          <main className="flex-1 w-full h-full bg-muted/30 overflow-hidden">
            <div className="flex flex-col h-full">
              <div className={cn('flex-1 min-h-0', !noContentScroll && 'overflow-y-auto')}>
                {children}
              </div>
              {!hideFooter && <Footer brand={footerBrand || brand} version={footerVersion || ''} links={footerLinks} />}
            </div>
          </main>
        </div>
        {!hideWorkspaceDock && (
          <WorkspaceDock
            active={!!activeWorkspace}
            title={dockTitle}
            onClose={onWorkspaceClose || (() => {})}
          >
            {workspaceChildren}
          </WorkspaceDock>
        )}
      </div>
    </div>
  );
}

export default GatewayShell;
