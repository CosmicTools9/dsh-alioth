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
 * Footer / WorkspaceDock / MobileSheet / NavPills / IconRail /
 * SecondaryNavColumn / NavDrawer / MenuModePicker / MenuModeSwatch
 *
 * 导航菜单模式（menuMode）: sidebar(单栏) / category(分类) / dual(双栏) /
 * top(顶部) / grouped(分组, 默认=现行为) / topDual(顶栏双行);
 * collapsedShowText 控制二级导航列折叠时是否保留文字。契约见
 * alioth-design/references/menu-modes.md。
 */
import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import * as React from "react";
import * as ReactDOM from "react-dom";

// 轻量 cn: 与生产代码的 tailwind-merge 行为等价,仅做过滤拼接
function cn(...inputs: Array<string | false | null | undefined>): string {
  return inputs.filter(Boolean).join(" ");
}

/** HSL 三元组 → WCAG 2.1 相对亮度对比度（原型侧与模块 theme.css 同算法）。 */
function hslContrastOf(
  l: number,
  h: number,
  s: number,
  other: [number, number, number],
): number {
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
    const f = (c: number): number =>
      c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
  };
  const [la, lb] = [lum(rgb(l)), lum(other)];
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return (hi + 0.05) / (lo + 0.05);
}

/** accent（`h s% l%`）→ `[h, s, l]`；非法形态返回 null。 */
function accentTriplet(accent: string): [number, number, number] | null {
  const m = /^(\d+(?:\.\d+)?)\s+(\d+(?:\.\d+)?)%\s+(\d+(?:\.\d+)?)%$/.exec(
    accent.trim(),
  );
  return m ? [parseFloat(m[1]!), parseFloat(m[2]!), parseFloat(m[3]!)] : null;
}

/** 模块主色面前景档（`MODULE_SPEC.md` §10.1.1 §3.2：`--primary` 与 `--primary-foreground`
 *  对比度 MUST ≥ 4.5:1）。深主色取 `0 0% 100%`；浅主色取**同色相深档**（向下取满足 ≥ 5.0:1 的最大明度）。
 *  与模块 `theme.css` 的取值同规则、参数为运行时 accent ⇒ 原型与实现不产生第二套色值。 */
function primaryForegroundFor(accent: string): string {
  const triplet = accentTriplet(accent);
  if (!triplet) return "0 0% 100%";
  const [h, s, l] = triplet;
  const white: [number, number, number] = [1, 1, 1];
  const base = hslContrastOf(l, h, s, white);
  if (base >= 4.5) return "0 0% 100%";
  for (let L = l - 1; L >= 0; L -= 1)
    if (hslContrastOf(l, h, s, hslRgb(L, h, s)) >= 5) return `${h} ${s}% ${L}%`;
  return "0 0% 0%";
}

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

/** 模块主色作**文字**用的档位（`MODULE_SPEC.md` §10.1.1 §3.2）：浅主色在白底上作文字对比度不足
 *  （1.55–4.41:1）⇒ 取同色相深档（向下取满足 ≥ 5.0:1 的最大明度）；深主色直接用 accent 本身。 */
function primaryTextFor(accent: string): string {
  const m = /^(\d+(?:\.\d+)?)\s+(\d+(?:\.\d+)?)%\s+(\d+(?:\.\d+)?)%$/.exec(
    accent.trim(),
  );
  if (!m) return accent;
  const [h, s, l] = [parseFloat(m[1]!), parseFloat(m[2]!), parseFloat(m[3]!)];
  const white = [1, 1, 1] as [number, number, number];
  if (hslContrastOf(l, h, s, white) >= 4.5) return accent;
  for (let L = l - 1; L >= 0; L -= 1)
    if (hslContrastOf(L, h, s, white) >= 5) return `${h} ${s}% ${L}%`;
  return `${h} ${s}% 0%`;
}

/** 模块辨识色注册（原型入口 `module.tsx` 调用）。
 *  单一来源仍是 `MODULE_SPEC.md` §10.1.1 登记表——字面值由各模块文件持有，此处只做登记：
 *  - `window.__MODULE_ACCENT__`：向后兼容的「当前模块」全局（standalone 渲染读它）；
 *  - `window.__MODULE_ACCENTS__[moduleId]`：按模块登记，供 App 原型按 tab 取各自 accent
 *    （否则多处 import 时 last-import-wins，App 原型只能显示一个色）。 */
export function registerModuleAccent(moduleId: string, accent: string): void {
  if (typeof window === "undefined") return;
  const host: Window & {
    __MODULE_ACCENT__?: string;
    __MODULE_ACCENTS__?: Record<string, string>;
  } = window;
  host.__MODULE_ACCENT__ = accent;
  host.__MODULE_ACCENTS__ = {
    ...(host.__MODULE_ACCENTS__ ?? {}),
    [moduleId]: accent,
  };
}

/** 指定模块（缺省 = 当前模块全局）的主色样式四件：`--primary` / `--ring` / 前景档 / 文字档。
 *  未注册或非 HSL 形态时返回 `{}`（沿用主题 CSS 默认）。 */
export function moduleAccentStyleFor(
  moduleId?: string,
): Record<string, string> {
  if (typeof window === "undefined") return {};
  const host: Window & {
    __MODULE_ACCENT__?: string;
    __MODULE_ACCENTS__?: Record<string, string>;
  } = window;
  const accent = moduleId
    ? host.__MODULE_ACCENTS__?.[moduleId]
    : host.__MODULE_ACCENT__;
  if (typeof accent !== "string" || !accent.trim()) return {};
  return {
    "--primary": accent,
    "--ring": accent,
    "--primary-foreground": primaryForegroundFor(accent),
    "--primary-text": primaryTextFor(accent),
  };
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
  /** 分组图标（category 分类列分组头 / dual 图标栏使用）；缺省回退该组首项 icon */
  icon?: string;
}

// ── 导航菜单模式 ──
export type MenuMode =
  "sidebar" | "category" | "dual" | "top" | "grouped" | "topDual";

export interface MenuModeDef {
  id: MenuMode;
  label: string;
  hint: string;
}

/** 六种导航菜单模式; 顺序 = 设计参考（设置页 3×2 网格）顺序 */
export const MENU_MODES: MenuModeDef[] = [
  { id: "sidebar", label: "单栏菜单", hint: "单列侧栏，扁平导航项" },
  {
    id: "category",
    label: "分类导航",
    hint: "分类列（单开手风琴：点击分组头切换）",
  },
  { id: "dual", label: "双栏菜单", hint: "图标栏 + 二级列" },
  { id: "top", label: "顶部菜单", hint: "导航项内联顶栏" },
  { id: "grouped", label: "分组菜单", hint: "侧栏带分组标题" },
  { id: "topDual", label: "顶栏双行", hint: "顶栏第二行承载导航" },
];

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
        // 品牌块 MUST 在窄屏可压缩：`shrink-0` 使其无法让位给右簇，实测 375px 顶栏
        // 左簇（品牌链接盒）与移动动作按钮几何互叠（Δoverlap=12）。改为 `min-w-0`
        // 保留 `w-60` 的桌面宽度对齐（与 240px 侧栏同宽），空间不足时按 flex 收缩；
        // 品牌标签自身是 `hidden sm:inline` + `truncate`，窄屏只剩图标即不再吃宽度。
        "flex items-center gap-2.5 transition-colors hover:opacity-80 overflow-hidden no-underline min-w-0",
        showAppName && "w-60",
      )}
      title="返回 Gateway"
    >
      <svg
        className="w-7 h-7 text-primary shrink-0"
        viewBox="0 0 32 32"
        fill="none"
      >
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
        <path
          d="M16 10V13"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
        />
        <circle cx="16" cy="7" r="1.5" fill="currentColor" />
      </svg>
      <span className="text-lg font-bold hidden sm:inline truncate">
        {displayText}
      </span>
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
  const [open, setOpen] = React.useState(false);
  if (!tabs || tabs.length === 0) return null;
  const active = tabs.find((t) => t.active) ?? tabs[0];
  return (
    <div
      className="relative flex items-center min-w-16"
      data-testid="scroll-tabs"
    >
      {/*
        <lg：单入口菜单 —— `docs/specs/NAV_MENU_BENCHMARK.md` 规定小屏「收为菜单/手风琴」，
        且「导航 MUST NOT 因空间不足被隐藏」(NN/g #1/#12)；MUST NOT 以横向滚动条承载
        （基准反例 D1：hide-scrollbar 时无任何可见线索）。与实现侧同款改法。
      */}
      {/* 根容器固定内容地板（2026-10-11 CT customer 375px 残余 8px 收口）：`<lg` 单入口
          button 的**固定内容**（icon16 + gap6 + chevron14 + gap6 + px-2.5×2 = 62px；label 走
          truncate 可归零，chevron 是末位 shrink-0）不可收缩 ⇒ 被压到 <62px 时 chevron 越盒溢入
          右簇（375×812 实测 [203,21,14,14]×[209,10,36,36] overlap 8×14）。前修 0a829ddc2a 的
          min-w-0/w-full 只约束盒宽、未给固定内容保底。地板 MUST 放**根容器**（左簇的 flex 项，
          min-w-0 → min-w-16）——放 `lg:hidden` 内层无效实证（m-v16 REVISE：root 仍可缩到 36px，
          内层地板只是把盒溢出 root，重叠转为 button×右簇更大面积）；根地板使挤压转移给可让位面
          （品牌块 min-w-0 窄屏仅剩图标 / 面包屑单行省略）吸收。≥lg 无影响（mall-erp App 壳
          1440 档 a-v193 实证：内联形态的 desktop overlap 与本地板无关，系嵌入模块换 0b92b9b1da
          后壳的既有暴露面）。 */}
      {/* <lg 单入口 MUST 能收缩到槽宽：容器 + button 补 `min-w-0`，button 再补
          `w-full max-w-full`。**仅容器 min-w-0 不够** —— button 自身 min-content
          = nowrap label + 固定图标/内距 = 118px，窄档仍溢入右簇 `shrink-0` 图标簇
          （768 档 Δoverlap=22px / 375 档 Δ=74px；同族先例见品牌块 L253-256）。
          本容器 `lg:hidden` ⇒ ≥lg 几何不受影响。 */}
      <div className="lg:hidden min-w-0">
        <button
          type="button"
          aria-haspopup="menu"
          aria-expanded={open}
          onClick={() => setOpen((v) => !v)}
          className="flex w-full max-w-full min-w-0 items-center gap-1.5 px-2.5 py-2 rounded-md text-sm font-medium text-foreground hover:bg-accent transition-colors"
        >
          {active.icon && (
            <span className="w-4 h-4 shrink-0">{icon(active.icon)}</span>
          )}
          <span className="truncate max-w-64">{active.label}</span>
          <span className="w-3.5 h-3.5 shrink-0">
            {icon("chevronDown", 14)}
          </span>
        </button>
        {open && (
          <div
            role="menu"
            className="absolute left-0 top-full z-30 mt-1 max-h-72 overflow-y-auto rounded-lg border border-border bg-popover py-1 shadow-md"
          >
            {tabs.map((t) => (
              <button
                key={t.id}
                type="button"
                role="menuitem"
                aria-current={t.active ? "page" : undefined}
                onClick={() => {
                  setOpen(false);
                  onTabClick && onTabClick(t.id);
                }}
                className={cn(
                  "flex w-full items-center gap-2 px-3 py-2 text-left text-sm whitespace-nowrap hover:bg-accent",
                  t.active
                    ? "bg-primary/10 text-primary font-semibold"
                    : "text-foreground",
                )}
              >
                {t.icon && (
                  <span className="w-4 h-4 shrink-0">{icon(t.icon)}</span>
                )}
                <span>{t.label}</span>
              </button>
            ))}
          </div>
        )}
      </div>

      {/* ≥lg：内联页签（既有视觉不变） */}
      <div
        role="tablist"
        aria-label="模块导航"
        className="hidden lg:flex items-center gap-0.5 min-w-0 "
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
                "relative flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium rounded-t-lg transition-all duration-150 whitespace-nowrap",
                isActive
                  ? "text-foreground bg-background shadow-tab border-x border-t border-border z-10 before:absolute before:bottom-[-1px] before:left-[6px] before:right-[6px] before:h-[2px] before:bg-primary before:rounded-t-[1px]"
                  : "text-muted-foreground hover:bg-accent/60 border-b border-transparent",
              )}
              title={t.label}
            >
              {t.icon && (
                <span className="w-4 h-4 shrink-0">{icon(t.icon)}</span>
              )}
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
    <nav className="hidden sm:flex items-center gap-1.5 text-sm min-w-0 whitespace-nowrap">
      {crumbs.map((c, i) => (
        <span key={i} className="flex items-center gap-1.5 min-w-0">
          {i > 0 && <span className="text-muted-foreground/50">/</span>}
          {i === crumbs.length - 1 ? (
            <span className="text-foreground font-medium truncate">
              {c.label}
            </span>
          ) : (
            <a
              href={c.href || "#"}
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
  const [value, setValue] = useState("");
  const resolvedPlaceholder = placeholder || "搜索应用、模块...";

  if (expanded) {
    return (
      <div className="absolute left-0 right-0 top-full bg-card border-b p-3 shadow-lg z-50 md:static md:w-72 md:bg-transparent md:border-0 md:p-0 md:shadow-none">
        <div className="relative w-full">
          {/* 桌面（md+）几何 MUST 与折叠态一致（`w-72 h-9`）——展开态只是结果下拉的载体，
              键入不得让搜索框跳动；`pr-9` 仅为窄屏关闭按钮留位（`md:pr-3`）。 */}
          <input
            type="search"
            autoFocus
            placeholder={resolvedPlaceholder}
            className="w-full h-10 pl-3 pr-9 rounded-lg border bg-muted/50 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/20 focus:border-primary/30 transition-colors md:h-9 md:pr-3"
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
          <button
            type="button"
            onClick={() => setExpanded(false)}
            className="absolute right-3 top-1/2 -translate-y-1/2 p-1 rounded hover:bg-accent md:hidden"
            aria-label="关闭搜索"
          >
            <span className="w-4 h-4">{icon("x", 16)}</span>
          </button>
        </div>
      </div>
    );
  }

  return (
    <>
      <div className="relative w-72 hidden lg:block">
        <input
          type="search"
          placeholder={resolvedPlaceholder}
          className="w-full h-9 pl-3 pr-3 rounded-lg border bg-muted/50 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/40 focus:bg-background transition-colors"
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />
      </div>
      {/* 宽度 288px 的桌面搜索框在 768–1023 会与品牌/面包屑/操作区挤压重叠（实测 tablet Δoverlap=12）
          ⇒ 桌面档抬到 lg（≥1024），768–1023 用紧凑按钮 */}
      <button
        type="button"
        onClick={() => setExpanded(true)}
        className="lg:hidden w-9 h-9 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors"
        aria-label="搜索"
        title="搜索"
      >
        <span className="w-4 h-4">{icon("search", 16)}</span>
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
    typeof document !== "undefined" &&
      document.documentElement.classList.contains("dark"),
  );
  const isHome =
    typeof window !== "undefined" && window.location.pathname === "/";
  const toggleTheme = useCallback(() => {
    const next = !isDark;
    document.documentElement.classList.toggle("dark", next);
    setDark(next);
  }, [isDark]);

  return (
    <div className="flex items-center gap-3">
      <a
        href="/"
        className={cn(
          "relative w-9 h-9 rounded-lg flex items-center justify-center transition-colors",
          isHome
            ? "bg-primary/10 text-primary"
            : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
        )}
        title="工作台"
        aria-label="工作台"
      >
        <span className="w-4 h-4">{icon("layoutDashboard")}</span>
      </a>
      <button
        type="button"
        onClick={toggleTheme}
        className="hidden sm:flex w-9 h-9 rounded-lg items-center justify-center text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors cursor-pointer border-none bg-transparent"
        title={isDark ? "切换到浅色" : "切换到深色"}
      >
        <span className="w-4 h-4">{icon(isDark ? "sun" : "moon")}</span>
      </button>
      {/* 工作区触发项（应用网格 / 待办 / 消息等）在 ≤md 隐藏：它们的 `w-9` 固定宽使右簇
          min-content 在窄屏超出视口（实测 375px：右簇 390px > 375 ⇒ 左簇被压为 0、菜单按钮与
          外溢右簇互叠，尾部被 header 的 overflow-hidden 裁掉）。移动端保留 搜索 / 主题 / 语言 / 用户，
          触发项经工作区 dock 可达。先例：SearchSlot 的桌面输入框 `hidden lg:block`。 */}
      {triggers.length > 0 && (
        <div className="hidden md:flex items-center gap-3">
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
      )}
    </div>
  );
}

export function UserMenu({
  user,
  onLogout,
}: {
  user?: User;
  onLogout?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const initial = (user?.name || "U").charAt(0).toUpperCase();

  useEffect(() => {
    if (!open) return undefined;
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node))
        setOpen(false);
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [open]);

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        onClick={() => setOpen((p) => !p)}
        className={cn(
          "flex items-center gap-1.5 cursor-pointer border-none bg-transparent p-1 rounded-lg transition-colors hover:bg-accent",
          open && "bg-accent",
        )}
        aria-label="用户菜单"
        aria-expanded={open}
      >
        <div className="w-7 h-7 rounded-md bg-primary/10 flex items-center justify-center shrink-0">
          <span className="text-xs font-bold text-primary">{initial}</span>
        </div>
        <span
          className={cn(
            "w-3 h-3 text-muted-foreground transition-transform",
            open && "rotate-180",
          )}
        >
          {icon("chevronDown", 12)}
        </span>
      </button>
      {open && (
        <div className="absolute right-0 top-full mt-1 w-56 rounded-lg border bg-card shadow-lg py-1 z-50 max-w-[calc(100vw-1rem)]">
          <div className="px-3 py-2 border-b border-border">
            <p className="text-sm font-medium truncate">
              {user?.name ?? "用户"}
            </p>
            <p className="text-xs text-muted-foreground truncate">
              {user?.email ?? ""}
            </p>
          </div>
          <a
            href="#"
            className="flex items-center gap-2 px-3 py-2 text-sm text-foreground hover:bg-accent no-underline transition-colors"
            onClick={() => setOpen(false)}
          >
            <span className="w-4 h-4">{icon("user")}</span>个人资料
          </a>
          <a
            href="#"
            className="flex items-center gap-2 px-3 py-2 text-sm text-foreground hover:bg-accent no-underline transition-colors"
            onClick={() => setOpen(false)}
          >
            <span className="w-4 h-4">{icon("settings")}</span>设置
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
              <span className="w-4 h-4">{icon("logOut")}</span>退出登录
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
  navInline,
}: {
  brand: string;
  brandIcon: string;
  moduleTabs?: ModuleTab[];
  breadcrumbs?: Breadcrumb[];
  searchPlaceholder?: string;
  triggers?: WorkspaceTrigger[];
  user?: User;
  onTrigger?: (id: string) => void;
  onModuleTabChange?: (id: string) => void;
  onMobileMenuToggle?: () => void;
  /** 顶栏内联导航（仅顶部菜单的导航项）；无则不改变现布局 */
  navInline?: React.ReactNode;
}) {
  return (
    <header className="h-14 border-b flex items-center justify-between gap-2 px-4 md:px-6 bg-background shrink-0 overflow-hidden">
      <div className={cn("flex items-center gap-2 min-w-0 flex-1 h-full")}>
        {onMobileMenuToggle && (
          <button
            type="button"
            onClick={onMobileMenuToggle}
            className="md:hidden p-2 rounded-lg hover:bg-accent transition-colors"
            aria-label="打开菜单"
          >
            <span className="w-5 h-5">{icon("menu", 20)}</span>
          </button>
        )}
        <Logo
          icon={brandIcon}
          brand={brand}
          showAppName={moduleTabs && moduleTabs.length > 0 ? brand : undefined}
        />
        {navInline ? (
          // 内联导航槽 MUST 只吃剩余空间（`flex-1` + `min-w-0`），溢出由导航自身承载
          // （`top` 走 Priority+「更多」菜单；MUST NOT 用横向滚动条承载内联溢出）
          <nav
            aria-label="主导航"
            data-nav-inline-slot=""
            className="flex items-center gap-0.5 min-w-0"
          >
            {navInline}
          </nav>
        ) : null}
        {moduleTabs && moduleTabs.length > 0 && (
          <ModuleTabs tabs={moduleTabs} onTabClick={onModuleTabChange} />
        )}
        {breadcrumbs && breadcrumbs.length > 0 && (
          // 面包屑 MUST 单行且可收缩省略（同生产 TopBar 约束；data-breadcrumbs = 驱动几何判据锚）
          <div className="min-w-0" data-breadcrumbs="">
            <Breadcrumbs crumbs={breadcrumbs} />
          </div>
        )}
      </div>
      <div className="flex items-center gap-3 shrink-0">
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
  links?: { label: string; href?: string }[];
}) {
  const defaultLinks =
    links === undefined
      ? [
          { label: "帮助", href: "#" },
          { label: "隐私", href: "#" },
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
        "hidden md:flex shrink-0 overflow-hidden transition-all duration-300 ease-in-out",
        active ? "w-80 border-l" : "w-0 border-l-0",
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
            <span className="w-4 h-4">{icon("panelRight", 16)}</span>
          </button>
        </div>
        <div className="flex-1 overflow-y-auto p-4">
          {children || (
            <div className="text-sm text-muted-foreground p-4">
              {title}内容区
            </div>
          )}
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
          <a
            href="#"
            className="flex items-center gap-2.5 no-underline"
            onClick={onClose}
          >
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
  showGroupTitles = true,
}: {
  groups: NavGroup[];
  activeId: string;
  collapsed: boolean;
  onSelect: (id: string) => void;
  /** 是否渲染分组标题（单栏菜单模式传 false；默认 true = 现行为） */
  showGroupTitles?: boolean;
}) {
  return (
    <nav className="flex h-full flex-col">
      <div className="flex-1 overflow-y-auto hide-scrollbar px-0 py-2">
        <div className="space-y-4">
          {groups.map((g, gi) => (
            <div key={g.label ?? `__group_${gi}`} className="space-y-1">
              {!collapsed && showGroupTitles && (
                <div className="px-4 py-3.5 pb-1" data-nav-group-title="">
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
      data-nav-item={item.id}
      onClick={() => onClick(item.id)}
      className={cn(
        "flex items-center rounded-md text-sm font-medium transition-colors",
        collapsed
          ? "justify-center h-9 w-9 mx-auto my-0.5 px-2"
          : "w-[calc(100%-1rem)] mx-2 gap-2.5 py-2 px-4",
        active
          ? "bg-primary/10 text-primary font-semibold"
          : "text-muted-foreground hover:bg-accent hover:text-foreground",
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
                "inline-flex items-center justify-center h-5 min-w-5 px-1 text-xs font-medium border-0 rounded-md shrink-0",
                active
                  ? "bg-primary/15 text-primary"
                  : "bg-muted text-muted-foreground",
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
  showGroupTitles = true,
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
  showGroupTitles?: boolean;
}) {
  return (
    <>
      <aside
        data-shell-sidebar=""
        className={cn(
          "hidden md:flex flex-col h-full bg-secondary border-r border-border transition-all duration-300 ease-in-out shrink-0",
          collapsed ? "w-16" : "w-60",
        )}
      >
        <MainNav
          groups={groups}
          activeId={activeId}
          collapsed={collapsed}
          onSelect={onSelect}
          showGroupTitles={showGroupTitles}
        />
        <SidebarFoot collapsed={collapsed} onToggle={onToggle} />
      </aside>
      <NavDrawer
        open={!!mobileOpen}
        onClose={onMobileClose || (() => {})}
        groups={groups}
        activeId={activeId}
        onSelect={onSelect}
        brand={brand || "Alioth"}
        brandIcon={brandIcon || "gatewayLogo"}
        showGroupTitles={showGroupTitles}
      />
    </>
  );
}

/** 移动端抽屉导航（所有菜单模式共用；移动端恒以列表呈现，含分组标题） */
export function NavDrawer({
  open,
  onClose,
  groups,
  activeId,
  onSelect,
  brand,
  brandIcon,
  showGroupTitles = true,
}: {
  open: boolean;
  onClose: () => void;
  groups: NavGroup[];
  activeId: string;
  onSelect: (id: string) => void;
  brand: string;
  brandIcon: string;
  showGroupTitles?: boolean;
}) {
  return (
    <MobileSheet
      open={open}
      onClose={onClose}
      brand={brand}
      brandIcon={brandIcon}
    >
      <div className="flex flex-col h-full">
        <MainNav
          groups={groups}
          activeId={activeId}
          collapsed={false}
          onSelect={onSelect}
          showGroupTitles={showGroupTitles}
        />
        <SidebarFoot collapsed={false} onToggle={onClose} />
      </div>
    </MobileSheet>
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
        title={collapsed ? "展开侧栏" : "折叠侧栏"}
      >
        <span className="w-3.5 h-3.5">
          {icon(collapsed ? "panelRight" : "panelLeft", 14)}
        </span>
      </button>
    </div>
  );
}

// ── 导航菜单模式子组件 ──

/**
 * 横向导航丸（顶部菜单 / 顶栏双行的导航项）
 *
 * `overflow=true` ⇒ **Priority+ 溢出折叠**（与生产 `NavPills.tsx` 同构；基准 `docs/specs/NAV_MENU_BENCHMARK.md`）：
 * 放得下的项内联、其余进「更多」菜单（点击触发），MUST NOT 以横向滚动条承载内联溢出。
 */
export function NavPills({
  items,
  activeId,
  onSelect,
  overflow = false,
}: {
  items: { id: string; label: string; icon: string }[];
  activeId: string;
  onSelect: (id: string) => void;
  overflow?: boolean;
}) {
  const GAP_PX = 2;
  const MORE_MIN_PX = 84;
  const [visibleCount, setVisibleCount] = React.useState<number | null>(null);
  const [open, setOpen] = React.useState(false);
  const rootRef = React.useRef<HTMLDivElement>(null);
  const measureRef = React.useRef<HTMLDivElement>(null);

  // 稳定内容键（同生产 NavPills）：避免每次渲染重跑度量而覆盖校验式递减
  const itemsKey = items.map((it) => it.id).join("|");
  const recompute = React.useCallback(() => {
    const host = rootRef.current;
    const meas = measureRef.current;
    if (!host || !meas) return;
    // 可用宽度以承载槽（父元素）为准（与生产 NavPills 同理由）
    const avail = host.parentElement?.clientWidth || host.clientWidth;
    if (avail <= 0) return;
    const pills = Array.from(
      meas.querySelectorAll<HTMLElement>("[data-pill-measure]"),
    );
    if (pills.length === 0) return;
    const moreW =
      meas
        .querySelector<HTMLElement>("[data-more-measure]")
        ?.getBoundingClientRect().width ?? MORE_MIN_PX;
    let used = 0;
    let fit = 0;
    for (let i = 0; i < pills.length; i += 1) {
      const reserve = pills.length - i - 1 > 0 ? GAP_PX + moreW : 0;
      const w = pills[i].getBoundingClientRect().width;
      if (used + w + reserve <= avail) {
        used += w + GAP_PX;
        fit += 1;
      } else break;
    }
    setVisibleCount(Math.min(pills.length, Math.max(1, fit)));
  }, []);

  React.useEffect(() => {
    if (!overflow) {
      setVisibleCount(null);
      return undefined;
    }
    recompute();
    document.fonts?.ready?.then(() => recompute()).catch(() => undefined);
    const host = rootRef.current;
    // 校验式收敛（同生产 NavPills）：逐帧检查承载槽是否真的不溢出，溢出则减一枚（最多 6 帧）
    let frames = 0;
    let raf = requestAnimationFrame(function verify() {
      const carrier = rootRef.current?.parentElement ?? rootRef.current;
      if (
        carrier &&
        carrier.scrollWidth > carrier.clientWidth + 1 &&
        frames < 6
      ) {
        frames += 1;
        setVisibleCount((n) => (n === null ? n : Math.max(1, n - 1)));
      }
      if (frames < 6) raf = requestAnimationFrame(verify);
    });
    if (!host || typeof ResizeObserver === "undefined")
      return () => cancelAnimationFrame(raf);
    const ro = new ResizeObserver(() => recompute());
    ro.observe(host);
    if (host.parentElement) ro.observe(host.parentElement);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
    };
  }, [overflow, recompute, itemsKey, activeId]);

  React.useEffect(() => {
    if (!open) return undefined;
    const onDoc = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const pillClass = (isActive: boolean) =>
    cn(
      "flex items-center gap-1.5 px-3 py-2 rounded-md text-sm whitespace-nowrap shrink-0 transition-colors cursor-pointer border-none bg-transparent",
      isActive
        ? "bg-primary/10 text-primary font-semibold"
        : "text-muted-foreground font-medium hover:bg-accent hover:text-foreground",
    );

  const collapsible = overflow && items.length > 1;
  const collapsed =
    collapsible && visibleCount !== null && visibleCount < items.length;
  const visible = collapsed ? items.slice(0, visibleCount as number) : items;
  const hidden = collapsed ? items.slice(visibleCount as number) : [];
  const hiddenActive = hidden.find((it) => it.id === activeId);

  const body = (
    <>
      {visible.map((it) => {
        const isActive = it.id === activeId;
        return (
          <button
            key={it.id}
            type="button"
            onClick={() => onSelect(it.id)}
            title={it.label}
            aria-current={isActive ? "page" : undefined}
            className={pillClass(isActive)}
          >
            <span className="h-4 w-4 shrink-0">{icon(it.icon)}</span>
            <span>{it.label}</span>
          </button>
        );
      })}
      {collapsed && (
        <div className="relative shrink-0">
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            aria-haspopup="menu"
            aria-expanded={open}
            aria-current={hiddenActive ? "page" : undefined}
            aria-label={hiddenActive ? `更多 · ${hiddenActive.label}` : "更多"}
            title={hiddenActive ? `更多 · ${hiddenActive.label}` : "更多"}
            className={pillClass(Boolean(hiddenActive))}
          >
            <span className="h-4 w-4 shrink-0">{icon("moreHorizontal")}</span>
            <span>更多</span>
          </button>
          {open && (
            <div
              role="menu"
              // min-width 用内联样式（同度量层理由：预生成 base CSS 不含 min-w-48）
              style={{ minWidth: 192 }}
              className="absolute right-0 top-full mt-1 z-30 rounded-md border border-border bg-popover py-1 shadow-md"
            >
              {hidden.map((it) => (
                <button
                  key={it.id}
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    onSelect(it.id);
                    setOpen(false);
                  }}
                  aria-current={it.id === activeId ? "page" : undefined}
                  className={cn(
                    "w-full flex items-center gap-2 px-3 py-1.5 text-sm text-left cursor-pointer border-none bg-transparent",
                    it.id === activeId
                      ? "bg-primary/10 text-primary font-semibold"
                      : "text-muted-foreground hover:bg-accent hover:text-foreground",
                  )}
                >
                  <span className="h-4 w-4 shrink-0">{icon(it.icon)}</span>
                  <span>{it.label}</span>
                </button>
              ))}
            </div>
          )}
        </div>
      )}
    </>
  );

  if (!collapsible) return body;
  return (
    <div ref={rootRef} className="flex items-center gap-0.5 min-w-0">
      {body}
      <PillsMeasureLayer
        measureRef={measureRef}
        items={items}
        pillClass={pillClass(false)}
        moreClass={pillClass(false)}
      />
    </div>
  );
}

/** 度量层（portal 到 body + fixed + 不可见，避免进入滚动区与地标树） */
function PillsMeasureLayer({
  measureRef,
  items,
  pillClass,
  moreClass,
}: {
  measureRef: React.RefObject<HTMLDivElement | null>;
  items: { id: string; label: string; icon: string }[];
  pillClass: string;
  moreClass: string;
}) {
  const layer = (
    <div
      ref={measureRef}
      aria-hidden="true"
      // 定位用内联样式：原型 base CSS 是预生成子集，任意值工具类不可靠（门禁 prototype-utility-coverage）
      style={{
        position: "fixed",
        top: -9999,
        left: 0,
        visibility: "hidden",
        pointerEvents: "none",
      }}
      className="flex items-center gap-0.5 whitespace-nowrap"
    >
      {items.map((it) => (
        <button
          key={it.id}
          type="button"
          tabIndex={-1}
          data-pill-measure=""
          className={pillClass}
        >
          <span className="h-4 w-4 shrink-0">{icon(it.icon)}</span>
          <span>{it.label}</span>
        </button>
      ))}
      <button
        type="button"
        tabIndex={-1}
        data-more-measure=""
        className={moreClass}
      >
        <span className="h-4 w-4 shrink-0">{icon("moreHorizontal")}</span>
        <span>更多</span>
      </button>
    </div>
  );
  if (typeof document === "undefined") return null;
  return ReactDOM.createPortal(layer, document.body);
}

/** 图标栏（双栏菜单第一列）：常驻；仅图标态 `w-16` / 展开态 `w-56`（图标+标签，同生产 IconRail） */
export function IconRail({
  groups,
  activeIndex,
  onSelectGroup,
  collapsed,
  onToggle,
  expanded = false,
}: {
  groups: NavGroup[];
  activeIndex: number;
  onSelectGroup: (index: number) => void;
  collapsed: boolean;
  onToggle: () => void;
  expanded?: boolean;
}) {
  return (
    <aside
      data-shell-sidebar=""
      aria-label="导航分类"
      data-rail-expanded={expanded ? "true" : "false"}
      className={cn(
        "hidden md:flex flex-col h-full bg-secondary border-r border-border shrink-0 transition-all duration-300 ease-in-out",
        expanded ? "w-56" : "w-16",
      )}
    >
      <div
        className={cn(
          "flex-1 overflow-y-auto hide-scrollbar py-2 flex flex-col gap-1",
          expanded ? "px-2 items-stretch" : "items-center",
        )}
      >
        {groups.map((g, i) => (
          <button
            key={g.label ?? `__rail_${i}`}
            type="button"
            onClick={() => onSelectGroup(i)}
            title={g.label}
            aria-current={i === activeIndex ? "true" : undefined}
            className={cn(
              "rounded-md flex items-center transition-colors cursor-pointer border-none bg-transparent",
              expanded ? "h-10 px-2.5 gap-2" : "h-10 w-10 justify-center",
              i === activeIndex
                ? "bg-primary/10 text-primary"
                : "text-muted-foreground hover:bg-accent hover:text-foreground",
            )}
          >
            <span className="h-4 w-4 shrink-0">
              {icon(g.icon || g.items[0]?.icon || "box")}
            </span>
            {expanded && (
              <span className="text-sm font-medium whitespace-nowrap">
                {g.label}
              </span>
            )}
          </button>
        ))}
      </div>
      <SidebarFoot collapsed={collapsed} onToggle={onToggle} />
    </aside>
  );
}

/**
 * 二级导航列：分类导航的分类列（variant='nav'）/ 双栏菜单的二级列（variant='panel'）。
 * 折叠宽度：w-16（仅图标）或 w-56（collapsedShowText 保留文字）；展开恒为 w-60。
 */
export function SecondaryNavColumn({
  group,
  activeId,
  collapsed,
  collapsedShowText,
  onSelect,
  onToggle,
  showFoot,
  variant,
  groups,
  activeIndex = 0,
  onSelectGroup,
}: {
  group: NavGroup;
  activeId: string;
  collapsed: boolean;
  collapsedShowText: boolean;
  onSelect: (id: string) => void;
  onToggle: () => void;
  showFoot: boolean;
  variant: "nav" | "panel";
  /** `category` 手风琴所需：全部分组 + 当前展开组 + 切换回调（`dual` 不传） */
  groups?: NavGroup[];
  activeIndex?: number;
  onSelectGroup?: (index: number) => void;
}) {
  const iconOnly = collapsed && !collapsedShowText;
  const accordion =
    variant === "nav" && groups && groups.length > 0 && onSelectGroup;
  return (
    <aside
      data-shell-sidebar=""
      aria-label={variant === "nav" ? "分类导航" : "二级导航"}
      data-nav-secondary-column=""
      className={cn(
        "hidden md:flex flex-col h-full border-r border-border shrink-0 transition-all duration-300 ease-in-out",
        collapsed ? (collapsedShowText ? "w-56" : "w-16") : "w-60",
        variant === "nav" ? "bg-secondary" : "bg-card",
      )}
    >
      {accordion ? (
        <div className="flex-1 overflow-y-auto py-1">
          {groups!.map((g, index) => {
            const open = index === activeIndex;
            return (
              <div key={g.label ?? `__category_${index}`}>
                <button
                  type="button"
                  onClick={() => onSelectGroup!(index)}
                  aria-expanded={open}
                  title={g.label}
                  data-nav-group-toggle=""
                  className={cn(
                    "w-full flex items-center gap-2 py-2 text-sm transition-colors cursor-pointer border-none bg-transparent",
                    iconOnly ? "justify-center px-0" : "px-3",
                    open
                      ? "text-foreground font-semibold"
                      : "text-muted-foreground hover:bg-accent hover:text-foreground",
                  )}
                >
                  <span className="h-4 w-4 shrink-0">
                    {icon(g.icon || g.items[0]?.icon || "box")}
                  </span>
                  {!iconOnly && (
                    <>
                      <span className="flex-1 text-left">{g.label}</span>
                      <span
                        className="h-3.5 w-3.5 shrink-0"
                        style={{ transform: open ? "none" : "rotate(-90deg)" }}
                      >
                        {icon("chevronDown", 14)}
                      </span>
                    </>
                  )}
                </button>
                {open && (
                  <MainNav
                    groups={[g]}
                    activeId={activeId}
                    collapsed={iconOnly}
                    onSelect={onSelect}
                  />
                )}
              </div>
            );
          })}
        </div>
      ) : (
        <MainNav
          groups={[group]}
          activeId={activeId}
          collapsed={collapsed && !collapsedShowText}
          onSelect={onSelect}
        />
      )}
      {showFoot && <SidebarFoot collapsed={collapsed} onToggle={onToggle} />}
    </aside>
  );
}

/**
 * 菜单模式缩略图：与该模式实际布局同构（深色块 = 导航面，浅色 = 内容面）。
 * 判定判据：category/top/topDual 有顶部横条；dual 有两条导航列；top/topDual 无左侧列。
 */
export function MenuModeSwatch({ mode }: { mode: MenuMode }) {
  const mark = "h-[2px] rounded-full bg-background";
  const soft = "h-[2px] rounded-full bg-border";
  const frame =
    "h-10 w-full rounded border border-border bg-muted/30 overflow-hidden flex";
  const navCol =
    "w-7 p-1 bg-foreground shrink-0 flex flex-col justify-center gap-1";
  const topBar = (
    <div className="h-3 w-full bg-foreground flex items-center gap-1 px-1 shrink-0">
      <span className={cn(mark, "w-5")} />
      <span className={cn(mark, "w-px")} />
      <span className={cn(mark, "w-px")} />
      <span className={cn(mark, "w-px")} />
    </div>
  );

  if (mode === "sidebar" || mode === "grouped") {
    return (
      <div className={frame}>
        <div className={navCol}>
          {mode === "grouped" && <span className={cn(mark, "w-3")} />}
          <span className={cn(mark, "w-4")} />
          <span className={cn(mark, "w-4")} />
          {mode === "grouped" && <span className={cn(mark, "w-3")} />}
          {mode === "sidebar" ? null : <span className={cn(mark, "w-4")} />}
        </div>
        <div className="flex-1" />
      </div>
    );
  }

  if (mode === "category") {
    // 分类列（单开手风琴）：无顶栏横条；列内首行为分组头（较宽标记）+ 当前展开组的项
    return (
      <div className={cn(frame, "flex-col")}>
        <div className="flex flex-1 min-h-0 w-full">
          <div className={navCol}>
            <span className={cn(mark, "w-4")} />
            <span className={cn(mark, "w-5")} />
            <span className={cn(mark, "w-2")} />
            <span className={cn(mark, "w-2")} />
          </div>
          <div className="flex-1" />
        </div>
      </div>
    );
  }

  if (mode === "dual") {
    return (
      <div className={frame}>
        <div className="w-5 p-1 bg-foreground shrink-0 flex flex-col items-center justify-center gap-1">
          <span className="w-3 h-3 rounded bg-background shrink-0" />
          <span className={cn(mark, "w-3")} />
          <span className={cn(mark, "w-3")} />
        </div>
        <div className="w-7 p-1 bg-card border-r border-border shrink-0 flex flex-col justify-center gap-1">
          <span className={cn(soft, "w-full")} />
          <span className={cn(soft, "w-full")} />
          <span className={cn(soft, "w-full")} />
          <span className={cn(soft, "w-full")} />
        </div>
        <div className="flex-1" />
      </div>
    );
  }

  if (mode === "top") {
    return (
      <div className={cn(frame, "flex-col")}>
        {topBar}
        <div className="flex-1" />
      </div>
    );
  }

  return (
    <div className={cn(frame, "flex-col")}>
      {topBar}
      <div className="h-3 w-full bg-muted/50 border-b border-border flex items-center gap-1 px-1 shrink-0">
        <span className={cn(soft, "w-4")} />
        <span className={cn(soft, "w-4")} />
        <span className={cn(soft, "w-4")} />
      </div>
      <div className="flex-1" />
    </div>
  );
}

export interface MenuModePickerProps {
  value: MenuMode;
  onChange: (mode: MenuMode) => void;
  collapsedShowText: boolean;
  onCollapsedShowTextChange: (next: boolean) => void;
  title?: string;
  hint?: string;
  modes?: MenuModeDef[];
}

/** 导航设置选择器：3 列模式卡片（单选）+ 「收起菜单时显示文字」开关 */
export function MenuModePicker({
  value,
  onChange,
  collapsedShowText,
  onCollapsedShowTextChange,
  title = "菜单模式",
  hint,
  modes = MENU_MODES,
}: MenuModePickerProps) {
  const COLS = 3;
  const rows: MenuModeDef[][] = [];
  for (let i = 0; i < modes.length; i += COLS)
    rows.push(modes.slice(i, i + COLS));
  return (
    <div className="rounded-lg border border-border bg-card p-3">
      <div className="mb-2">
        <div className="text-sm font-semibold text-foreground">{title}</div>
        {hint && (
          <div className="text-xs text-muted-foreground mt-1">{hint}</div>
        )}
      </div>
      <div
        role="radiogroup"
        aria-label={title}
        data-menu-mode-picker=""
        className="flex flex-col gap-2"
      >
        {rows.map((row, ri) => (
          <div key={ri} className="flex gap-2">
            {row.map((m) => {
              const selected = m.id === value;
              return (
                <button
                  key={m.id}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  aria-label={m.label}
                  title={m.hint}
                  onClick={() => onChange(m.id)}
                  className={cn(
                    "flex-1 min-w-0 rounded-lg p-1 transition-colors cursor-pointer border-none text-left",
                    selected
                      ? "bg-primary/15"
                      : "bg-transparent hover:bg-accent",
                  )}
                >
                  <div className="rounded-md border border-border bg-card p-2">
                    <MenuModeSwatch mode={m.id} />
                  </div>
                  <div
                    className={cn(
                      "text-xs text-center mt-1 truncate",
                      selected
                        ? "text-primary font-semibold"
                        : "text-muted-foreground",
                    )}
                  >
                    {m.label}
                  </div>
                </button>
              );
            })}
          </div>
        ))}
      </div>
      <div className="mt-2 border-t border-border">
        <div className="flex items-center justify-between py-2">
          <span className="text-sm text-foreground">收起菜单时显示文字</span>
          <button
            type="button"
            role="switch"
            aria-checked={collapsedShowText}
            aria-label="收起菜单时显示文字"
            onClick={() => onCollapsedShowTextChange(!collapsedShowText)}
            className={cn(
              "w-9 h-5 rounded-full flex items-center px-1 shrink-0 transition-colors cursor-pointer border-none",
              collapsedShowText ? "bg-primary justify-end" : "bg-border",
            )}
          >
            <span className="w-3.5 h-3.5 rounded-full bg-background shrink-0" />
          </button>
        </div>
      </div>
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
  user?: User;
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
  /** 导航菜单模式（默认 'grouped' = 改动前的分组侧栏；与 App/Module 模式正交） */
  menuMode?: MenuMode;
  /** 二级导航列折叠时是否保留文字（category 分类列 / dual 二级列）；主侧栏不受影响 */
  collapsedShowText?: boolean;
  /** App 原型用：按模块 id 解析该模块的辨识色（`registerModuleAccent` 登记）并注入主色四件，
   *  与生产「切 tab 整站换色」（Gateway 由 accentBarColor 注入 `--primary`）同语义。 */
  accentModuleId?: string;
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
    accentModuleId,
    menuMode = "grouped",
    collapsedShowText = false,
  } = props;

  const [internalCollapsed, setInternalCollapsed] = useState(collapsed);
  const [mobileOpen, setMobileOpen] = useState(false);
  const [pickedGroup, setPickedGroup] = useState<number | null>(null);

  const effectiveCollapsed = onToggle ? collapsed : internalCollapsed;
  const handleToggle = onToggle || (() => setInternalCollapsed((p) => !p));

  // 当前分类：由 activeId 反推所属分组；用户点击分类后以点击值为准，activeId 变化即复位
  useEffect(() => {
    setPickedGroup(null);
  }, [activeId]);

  const derivedGroupIndex = useMemo(() => {
    if (!navGroups || navGroups.length === 0) return 0;
    const idx = navGroups.findIndex((g) =>
      g.items.some((it) => it.id === activeId),
    );
    return idx >= 0 ? idx : 0;
  }, [navGroups, activeId]);
  const activeGroupIndex =
    pickedGroup != null && navGroups && pickedGroup < navGroups.length
      ? pickedGroup
      : derivedGroupIndex;

  const handleSelect = useCallback(
    (id: string) => {
      onSelect?.(id);
      setMobileOpen(false);
    },
    [onSelect],
  );
  const closeMobile = useCallback(() => setMobileOpen(false), []);

  const hasNav = !!(
    !hideNavigation &&
    navGroups &&
    navGroups.length > 0 &&
    activeId != null &&
    onSelect
  );
  const activeGroup = navGroups ? navGroups[activeGroupIndex] : undefined;

  const selectGroup = (index: number) => {
    setPickedGroup(index);
    const first = navGroups?.[index]?.items?.[0];
    if (first) handleSelect(first.id);
  };

  // category：顶栏 MUST NOT 渲染分类丸（切换改由分类列内分组头承担）；`top` 仍走顶栏内联丸
  const itemPills =
    hasNav && (menuMode === "top" || menuMode === "topDual") && navGroups
      ? navGroups.flatMap((g) =>
          g.items.map((it) => ({ id: it.id, label: it.label, icon: it.icon })),
        )
      : undefined;
  // topDual 的导航项只进第二行导航条（见 body 内 h-11 条），不进 TopBar 内联槽——避免双份
  const navInline =
    menuMode === "top" && itemPills ? (
      // top：Priority+ 溢出折叠（顶栏放不下的项进「更多」菜单）
      <NavPills
        items={itemPills}
        activeId={activeId || ""}
        overflow
        onSelect={handleSelect}
      />
    ) : undefined;
  const isColumnNav = menuMode === "sidebar" || menuMode === "grouped";
  const isTopNav = menuMode === "top" || menuMode === "topDual";
  const isSecondaryNav = menuMode === "category" || menuMode === "dual";

  const activeTrigger = activeWorkspace
    ? triggers.find((t) => t.id === activeWorkspace)
    : undefined;
  const dockTitle = workspaceTitle || activeTrigger?.title || "工作区";

  return (
    <div
      className={cn(
        "flex h-screen flex-col overflow-hidden bg-background",
        rootClass,
      )}
      style={moduleAccentStyleFor(accentModuleId)}
    >
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
        navInline={navInline}
      />
      {hasNav && menuMode === "topDual" && itemPills ? (
        <nav
          aria-label="主导航"
          className="h-11 border-b bg-background shrink-0 flex items-center gap-0.5 px-4 md:px-6 overflow-x-auto"
        >
          <NavPills
            items={itemPills}
            activeId={activeId || ""}
            onSelect={handleSelect}
          />
        </nav>
      ) : null}
      <div className="flex flex-1 min-h-0 overflow-hidden">
        {hasNav && isColumnNav && (
          <Navigation
            groups={navGroups!}
            activeId={activeId || ""}
            collapsed={effectiveCollapsed}
            showGroupTitles={menuMode === "grouped"}
            onSelect={handleSelect}
            onToggle={handleToggle}
            mobileOpen={mobileOpen}
            onMobileClose={closeMobile}
            brand={brand}
            brandIcon={brandIcon}
          />
        )}
        {hasNav && menuMode === "dual" && (
          <IconRail
            groups={navGroups!}
            activeIndex={activeGroupIndex}
            onSelectGroup={selectGroup}
            collapsed={effectiveCollapsed}
            onToggle={handleToggle}
            expanded={collapsedShowText}
          />
        )}
        {hasNav && isSecondaryNav && activeGroup && (
          <SecondaryNavColumn
            group={activeGroup}
            activeId={activeId || ""}
            groups={
              menuMode === "category" ? (navGroups ?? undefined) : undefined
            }
            activeIndex={activeGroupIndex}
            onSelectGroup={selectGroup}
            collapsed={effectiveCollapsed}
            collapsedShowText={collapsedShowText}
            onSelect={handleSelect}
            onToggle={handleToggle}
            showFoot={menuMode === "category"}
            variant={menuMode === "category" ? "nav" : "panel"}
          />
        )}
        {hasNav && isTopNav && (
          <NavDrawer
            open={mobileOpen}
            onClose={closeMobile}
            groups={navGroups!}
            activeId={activeId || ""}
            onSelect={handleSelect}
            brand={brand}
            brandIcon={brandIcon}
          />
        )}
        <div className="flex flex-col min-w-0 overflow-hidden flex-1">
          {showAccent && (
            <div className="accent-bar h-[3px] w-full bg-primary/15 shrink-0" />
          )}
          <main className="flex-1 w-full h-full bg-muted/30 overflow-hidden">
            <div className="flex flex-col h-full">
              <div
                className={cn(
                  "flex-1 min-h-0",
                  !noContentScroll && "overflow-y-auto",
                )}
              >
                {children}
              </div>
              {!hideFooter && (
                <Footer
                  brand={footerBrand || brand}
                  version={footerVersion || ""}
                  links={footerLinks}
                />
              )}
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
