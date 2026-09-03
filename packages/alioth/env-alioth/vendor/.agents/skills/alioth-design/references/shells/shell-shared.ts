/**
 * shell-shared.ts — AliothStudio 原型共享壳的 ESM 片段构造器。
 *
 * 由 block/module/app-shell.tsx 复用，避免三壳重复编写相同的
 * vendor 脚本 / require shim / boot-skeleton fade / mount / Gateway bridge。
 *
 * 所有函数返回纯 HTML 字符串（构建期由 prototype-tool.js 通过 bun import 调用），
 * 不进入浏览器运行时，因此可在 TS 中直接用模板字符串拼装。
 */

const VENDOR_BASE = '.agents/skills/alioth-design/references/vendor/';

/** 本地 React 19 UMD vendor（禁止 CDN）。rootPath 为到 references/ 的相对路径。 */
export function vendorScripts(rootPath: string): string {
  const base = rootPath.replace(/\/$/, '') + '/' + VENDOR_BASE;
  return [
    `<script src="${base}react.umd.js"></script>`,
    `<script src="${base}react-dom.umd.js"></script>`,
    `<script src="${base}react-dom-client.umd.js"></script>`,
    `<script src="${base}react-bridge.js"></script>`,
  ].join('\n    ');
}

/**
 * esbuild IIFE 的 require shim。bundle 内 `__require('react')` / `__require('react/jsx-runtime')`
 * 解析到浏览器全局 UMD。
 *
 * 关键：--jsx=automatic 会生成 `import_jsx_runtime = __require("react/jsx-runtime")`，
 * 而浏览器 UMD 环境没有 react/jsx-runtime 模块。这里在 shim 里用 React.createElement 构造一个
 * jsx-runtime 等价物，使自动 JSX 运行时可在 UMD 环境下解析，否则 bundle 的 IIFE 会在 require 处
 * 抛 "Cannot require react/jsx-runtime" 而中止，导致 window.ModuleLayout / AppLayout 永不注册。
 */
export const REQUIRE_SHIM = `<script>
      window.ReactJsxRuntime = (function () {
        var R = window.React;
        function jsx(type, props, key) {
          var config = props;
          if (key !== undefined) {
            config = Object.assign({}, props);
            config.key = key;
          }
          return R.createElement(type, config);
        }
        return { jsx: jsx, jsxs: jsx, Fragment: R.Fragment };
      })();
      window.require = function (name) {
        if (name === 'react') return window.React;
        if (name === 'react-dom') return window.ReactDOM;
        if (name === 'react-dom/client') return window.ReactDOMClient || window.ReactDOM;
        if (name === 'react/jsx-runtime') return window.ReactJsxRuntime;
        throw new Error('Cannot require ' + name);
      };
    </script>`;

/** previewMode 增强样式:让 boot-skeleton 骨架块在白底上可见(默认 --muted 太浅近乎白屏)。
 * 仅 previewMode 注入,不影响真实原型的 boot-skeleton 视觉。结构对齐 reference 水平布局。 */
export const PREVIEW_ENHANCE_CSS = `<style>
      .boot-skeleton {
        background: hsl(var(--muted-foreground) / 0.08) !important;
      }
      .boot-skeleton-sidebar {
        background: hsl(var(--secondary)) !important;
        border-right: 1px solid hsl(var(--border)) !important;
      }
      .boot-skeleton-sidebar .boot-brand-text,
      .boot-skeleton-sidebar .boot-logo {
        background: hsl(var(--primary) / 0.25) !important;
        box-shadow: inset 0 0 0 1px hsl(var(--primary) / 0.35) !important;
      }
      .boot-skeleton-sidebar .boot-nav-item {
        background: hsl(var(--primary) / 0.18) !important;
      }
      .boot-skeleton-main {
        background: hsl(var(--card) / 0.5) !important;
      }
      .boot-skeleton-topbar {
        background: hsl(var(--background)) !important;
        border-bottom: 1px solid hsl(var(--border)) !important;
      }
      .boot-skeleton-topbar .boot-title,
      .boot-skeleton-topbar .boot-btn {
        background: hsl(var(--primary) / 0.25) !important;
        box-shadow: inset 0 0 0 1px hsl(var(--primary) / 0.35) !important;
      }
      .boot-skeleton-content .boot-page-title,
      .boot-skeleton-content .boot-page-sub,
      .boot-skeleton-content .boot-card {
        background: hsl(var(--primary) / 0.18) !important;
      }
      .boot-skeleton-loader {
        background: hsl(var(--card)) !important;
        padding: 8px 14px !important;
        border-radius: 999px !important;
        box-shadow: 0 4px 12px hsl(0 0% 0% / 0.1) !important;
      }
      .boot-preview-badge {
        position: fixed;
        top: 12px;
        right: 12px;
        z-index: 10000;
        background: hsl(var(--primary));
        color: hsl(var(--primary-foreground));
        padding: 4px 12px;
        border-radius: 999px;
        font-size: 12px;
        font-weight: 600;
        font-family: var(--font-sans, sans-serif);
        box-shadow: 0 2px 8px hsl(0 0% 0% / 0.2);
      }
    </style>
    <div class="boot-preview-badge">壳预览 · boot-skeleton</div>`;

/** boot-skeleton 布局 + 淡出 + 加载动画（block/module/app 三壳共用）。
 * 结构对齐 gateway-shell.tsx：水平 flex「sidebar | main[topbar + content]」。
 * - .boot-skeleton-topbar 对应 Gateway TopBar（品牌 + 全局操作）
 * - .boot-skeleton-sidebar 对应 Gateway Navigation（模块侧边栏导航）
 * - .boot-skeleton-content 对应 Gateway main 内容区
 * - .boot-skeleton-loader 为底部居中加载浮标（spinner + 文案），淡出后移除
 * block 现已自带独立 Gateway 壳（block-shell.tsx 静态 Tailwind 壳），故 block 用 'full' 变体；
 * 'content' 变体仍保留为仅渲染内容区骨架的兜底（无侧栏/顶栏/底栏）。
 */
/**
 * BOOT_FADE_CSS — boot-skeleton 骨架屏样式。
 * 结构对齐 gateway-shell.tsx：水平 flex(侧栏 | 主内容[顶栏 + 内容区])，
 * 即「sidebar + main[topbar + content]」布局，与 SKILL.md v2.0.0 变更日志一致。
 * 加载完成后由 mountScript / PREVIEW_FADE_SCRIPT 触发 .fade-out → .removed 淡出。
 */
export const BOOT_FADE_CSS = `<style>
      .boot-skeleton {
        display: flex;
        height: 100vh;
        width: 100vw;
        overflow: hidden;
        background: hsl(var(--background));
        position: fixed;
        inset: 0;
        z-index: 9999;
      }
      .boot-skeleton.fade-out {
        opacity: 0;
        transition: opacity 0.32s ease-out;
      }
      .boot-skeleton.removed { display: none; }
      /* ── Sidebar（左侧）──────── */
      .boot-skeleton-sidebar {
        width: var(--sidebar-width);
        height: 100%;
        background: hsl(var(--secondary));
        border-right: 1px solid hsl(var(--border));
        display: flex;
        flex-direction: column;
        padding: 16px;
      }
      .boot-skeleton-sidebar .boot-brand {
        display: flex;
        align-items: center;
        gap: 12px;
        margin-bottom: 24px;
      }
      .boot-skeleton-sidebar .boot-logo {
        width: 40px;
        height: 40px;
        border-radius: 12px;
        background: hsl(var(--muted));
      }
      .boot-skeleton-sidebar .boot-brand-text {
        width: 120px;
        height: 18px;
        border-radius: 4px;
        background: hsl(var(--muted));
      }
      .boot-skeleton-sidebar .boot-nav {
        display: flex;
        flex-direction: column;
        gap: 12px;
      }
      .boot-skeleton-sidebar .boot-nav-item {
        height: 36px;
        border-radius: 8px;
        background: hsl(var(--muted));
      }
      .boot-skeleton-sidebar .boot-nav-item:nth-child(1) { width: 100%; }
      .boot-skeleton-sidebar .boot-nav-item:nth-child(2) { width: 85%; }
      .boot-skeleton-sidebar .boot-nav-item:nth-child(3) { width: 70%; }
      /* ── Main（右侧：顶栏 + 内容）──────── */
      .boot-skeleton-main {
        flex: 1;
        display: flex;
        flex-direction: column;
      }
      .boot-skeleton-topbar {
        height: var(--topbar-height);
        border-bottom: 1px solid hsl(var(--border));
        background: hsl(var(--background));
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 0 24px;
      }
      .boot-skeleton-topbar .boot-title { width: 180px; height: 16px; border-radius: 4px; background: hsl(var(--muted)); }
      .boot-skeleton-topbar .boot-actions { display: flex; gap: 8px; }
      .boot-skeleton-topbar .boot-btn { width: 80px; height: 32px; border-radius: 6px; background: hsl(var(--muted)); }
      .boot-skeleton-content {
        flex: 1;
        padding: 24px;
      }
      .boot-skeleton-content .boot-page-head { margin-bottom: 24px; }
      .boot-skeleton-content .boot-page-title { width: 160px; height: 20px; border-radius: 4px; background: hsl(var(--muted)); margin-bottom: 8px; }
      .boot-skeleton-content .boot-page-sub { width: 240px; height: 14px; border-radius: 4px; background: hsl(var(--muted)); }
      .boot-skeleton-content .boot-cards {
        display: grid;
        grid-template-columns: repeat(3, 1fr);
        gap: 16px;
      }
      .boot-skeleton-content .boot-card { height: 120px; border-radius: 12px; background: hsl(var(--muted)); }
      /* ── Loader 浮标 ──────────────── */
      .boot-skeleton-loader {
        position: absolute;
        left: 50%;
        bottom: 40px;
        transform: translateX(-50%);
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 12px;
      }
      .boot-skeleton-loader .boot-spinner {
        width: 32px;
        height: 32px;
        border: 3px solid hsl(var(--border));
        border-top-color: hsl(var(--primary));
        border-radius: 50%;
        animation: boot-spin 1s linear infinite;
      }
      .boot-skeleton-loader .boot-loader-text {
        font-size: 13px;
        color: hsl(var(--muted-foreground));
      }
      @keyframes boot-spin { 0% { transform: rotate(0deg); } 100% { transform: rotate(360deg); } }
    </style>`;

/**
 * bootSkeletonHTML — 三壳共用的 boot-skeleton 骨架屏标记（内层结构，不含外层 id）。
 *
 * 结构对齐 gateway-shell.tsx：水平「sidebar | main[topbar + content]」。
 * 外层 `<div id="boot-skeleton" class="boot-skeleton">` 由各 shell（app/module/block-shell.tsx）
 * 包裹提供，本函数仅返回其内层子节点，避免重复 id。
 *
 * @param variant 'full'（module/app/block：侧栏 + 顶栏 + 内容区）| 'content'（仅内容区骨架兜底，block 内嵌于已有壳时可用）
 */
export function bootSkeletonHTML(variant: 'full' | 'content' = 'full'): string {
  if (variant === 'content') {
    // block 内嵌于已有 Gateway 壳，加载态只覆盖内容区（无侧栏/顶栏/底栏）
    return `<div class="boot-skeleton-main">
    <div class="boot-skeleton-content">
      <div class="boot-page-head">
        <div class="boot-page-title"></div>
        <div class="boot-page-sub"></div>
      </div>
      <div class="boot-cards">
        <div class="boot-card"></div>
        <div class="boot-card"></div>
        <div class="boot-card"></div>
      </div>
    </div>
  </div>
  <div class="boot-skeleton-loader"><div class="boot-spinner"></div><div class="boot-loader-text">正在加载...</div></div>`;
  }
  return `<div class="boot-skeleton-sidebar">
    <div class="boot-brand">
      <div class="boot-logo"></div>
      <div class="boot-brand-text"></div>
    </div>
    <div class="boot-nav">
      <div class="boot-nav-item"></div>
      <div class="boot-nav-item"></div>
      <div class="boot-nav-item"></div>
    </div>
  </div>
  <div class="boot-skeleton-main">
    <div class="boot-skeleton-topbar">
      <div class="boot-title"></div>
      <div class="boot-actions">
        <div class="boot-btn"></div>
        <div class="boot-btn"></div>
      </div>
    </div>
    <div class="boot-skeleton-content">
      <div class="boot-page-head">
        <div class="boot-page-title"></div>
        <div class="boot-page-sub"></div>
      </div>
      <div class="boot-cards">
        <div class="boot-card"></div>
        <div class="boot-card"></div>
        <div class="boot-card"></div>
      </div>
    </div>
  </div>
  <div class="boot-skeleton-loader"><div class="boot-spinner"></div><div class="boot-loader-text">正在加载...</div></div>`;
}

/** 预览模式专用：boot-skeleton 自动淡出脚本。
 * previewMode 下无 mountScript/MutationObserver，需独立注入此脚本让骨架屏在内容就绪后淡出。
 * 策略：检测 #root 有子节点后触发 fade-out（与 mountScript 的 fadeOut 逻辑一致）。 */
export const PREVIEW_FADE_SCRIPT = `<script>
    (function () {
      var sk = document.getElementById('boot-skeleton');
      var root = document.getElementById('root');
      if (!sk || !root) return;
      var check = function () {
        if (root.children.length > 0) {
          var faded = false;
          var doFade = function () {
            if (faded) return;
            faded = true;
            sk.classList.add('fade-out');
            setTimeout(function () { sk.classList.add('removed'); }, 320);
          };
          requestAnimationFrame(function () {
            requestAnimationFrame(doFade);
          });
          setTimeout(doFade, 100);
        } else {
          setTimeout(check, 100);
        }
      };
      if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', check);
      } else {
        check();
      }
    })();
  </script>`;

/** 通用挂载 IIFE:模拟 Gateway MicroFrontendLoader,通过 lifecycle 挂载目标组件。
 * kind 决定 notFound 文案;globalName 是 esbuild IIFE 暴露的全局名;
 * name 是逻辑名(传给 lifecycle props.name)。
 *
 * 优先走 single-spa lifecycle(bootstrap → mount({domElement, ...})),
 * 兼容纯 React 组件导出(mod.default / mod)直接 createRoot 渲染。
 *
 * script 标签带 data-auto-mount 结构性标记:audit-block-integration.ts §9.5 门禁
 * check 4 对带该标记的内联脚本豁免 root.render 检查(壳挂载语义,非用户代码)。
 * 任何非构建工具注入的内联脚本不得伪造此标记。 */
export function mountScript(opts: {
  kind: 'block' | 'module' | 'app';
  globalName: string;
  name: string;
  baseUrl?: string;
  notFoundTitle: string;
  notFoundHint: string;
}): string {
  const baseUrl = opts.baseUrl || '/';
  const notFound = `React.createElement('div', { className: 'empty-state' },
              React.createElement('h3', null, ${JSON.stringify(opts.notFoundTitle)}),
              React.createElement('p', null, ${JSON.stringify(opts.notFoundHint)})
            )`;
  return `<script data-auto-mount>
      (function () {
        var rootEl = document.getElementById('root');
        var root = ReactDOM.createRoot(rootEl);
        var sk = document.getElementById('boot-skeleton');
        var fadeOut = function () {
          if (sk) {
            var faded = false;
            var doFade = function () {
              if (faded) return;
              faded = true;
              sk.classList.add('fade-out');
              setTimeout(function () { sk.classList.add('removed'); }, 320);
            };
            requestAnimationFrame(function () {
              requestAnimationFrame(doFade);
            });
            setTimeout(doFade, 100);
          }
        };
        var renderFallback = function () {
          root.render(${notFound});
          fadeOut();
        };
        try {
          var mod = window[${JSON.stringify(opts.globalName)}];
          var mountProps = {
            domElement: rootEl,
            domElementId: 'root',
            name: ${JSON.stringify(opts.name)},
            baseUrl: ${JSON.stringify(baseUrl)},
            apiBaseUrl: '/api',
            embedded: false,
            navigate: function (path) { console.log('[mock navigate]', path); },
          };
          var lifecycle = mod && (typeof mod.mount === 'function') ? mod : null;
          var Comp = mod && (typeof mod.default === 'function' ? mod.default : (typeof mod === 'function' ? mod : null));
          if (lifecycle) {
            Promise.resolve()
              .then(function () { return lifecycle.bootstrap ? lifecycle.bootstrap() : undefined; })
              .then(function () { return lifecycle.mount(mountProps); })
              .then(fadeOut)
              .catch(function (e) {
                console.error('Mount failed:', e);
                document.title = 'Mount Error: ' + e.message;
                renderFallback();
              });
          } else if (Comp) {
            root.render(React.createElement(Comp));
            fadeOut();
          } else {
            renderFallback();
          }
        } catch (e) {
          console.error('Render failed:', e);
          document.title = 'Render Error: ' + e.message;
          renderFallback();
        }
      })();
    </script>`;
}

/**
 * 预览壳 TopBar 右侧统一模板：Search + ActionGroup + UserMenu。
 * 使用 Tailwind 工具类与 gateway-shell.tsx 运行时视觉对齐；图标为内联 SVG，
 * 不依赖 previewMode 下未加载的 icon-pool.js。
 */
export function previewTopBarRight(opts?: {
  triggers?: { title: string; badge?: number }[];
}): string {
  const triggers = opts?.triggers || [
    { title: 'EmpAgent' },
    { title: '收件箱', badge: 1 },
    { title: '个人中心' },
  ];
  const triggerButtons = triggers
    .map((t) => {
      const badge = t.badge
        ? `<span class="absolute -top-0.5 -right-0.5 min-w-4 h-4 rounded-full bg-destructive text-destructive-foreground text-[8px] leading-none flex items-center justify-center font-bold px-1 border-2 border-card">${t.badge}</span>`
        : '';
      return `<button type="button" class="relative w-9 h-9 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors cursor-pointer border-none bg-transparent" title="${t.title}">
        <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/></svg>
        ${badge}
      </button>`;
    })
    .join('\n      ');
  return `
    <div class="flex items-center gap-3">
      <div class="relative w-72 hidden md:block">
        <svg class="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground pointer-events-none" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.35-4.35"/></svg>
        <input type="search" placeholder="搜索…" class="pl-9 pr-3 w-full h-9 rounded-lg border bg-muted/50 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/20 focus:border-primary/40 focus:bg-background transition-colors" />
      </div>
      <div class="flex items-center gap-3">
        <button type="button" class="relative w-9 h-9 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors cursor-pointer border-none bg-transparent" title="工作台" aria-label="工作台">
          <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/></svg>
        </button>
        <button type="button" class="hidden sm:flex w-9 h-9 rounded-lg items-center justify-center text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors cursor-pointer border-none bg-transparent" title="切换深色模式">
          <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9z"/></svg>
        </button>
        ${triggerButtons}
      </div>
      <button type="button" class="flex items-center gap-1.5 cursor-pointer border-none bg-transparent p-1 rounded-lg transition-colors hover:bg-accent" aria-label="用户菜单">
        <div class="w-7 h-7 rounded-md bg-primary/10 flex items-center justify-center shrink-0">
          <span class="text-xs font-bold text-primary">D</span>
        </div>
        <svg class="w-3 h-3 text-muted-foreground" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>
      </button>
    </div>`;
}

/** Gateway bridge：仅在 block / module 壳注入（app 壳按原设计不含此段）。 */
export function gatewayBridge(opts: {
  kind: 'block' | 'module';
  blockId?: string;
  title?: string;
}): string {
  if (opts.kind === 'block') {
    const blockId = opts.blockId || '';
    return `<script>
      (function () {
        var isEmbedded = window.parent && window.parent !== window;

        window.Gateway = {
          ready: true,
          embedded: isEmbedded,
          blockId: ${JSON.stringify(blockId)},
          layout: {
            topbar: true,
            sidebar: false,
            branding: false,
          },
          getCurrentUser: function () {
            return { name: '开发者', email: 'dev@alioth.local', avatar: null };
          },
          navigateTo: function (path) {
            if (isEmbedded) {
              window.parent.postMessage({ type: 'gateway:navigate', path: path }, '*');
            } else {
              console.log('[Gateway] Navigate:', path);
            }
          },
          onBlockEvent: function (event, data) {
            if (isEmbedded) {
              window.parent.postMessage(
                { type: 'gateway:block-event', event: event, data: data },
                '*',
              );
            }
          },
          notifyReady: function () {
            if (isEmbedded) {
              window.parent.postMessage(
                { type: 'gateway:block-ready', blockId: ${JSON.stringify(blockId)} },
                '*',
              );
            }
          },
        };

        if (isEmbedded) {
          window.addEventListener('message', function (e) {
            if (e.data && e.data.type === 'gateway:navigate') {
              console.log('[Gateway] Received navigate:', e.data.path);
            }
          });
        }

        var notifyTimer = setTimeout(function () {
          window.Gateway.notifyReady();
        }, 1000);
      })();
    </script>`;
  }

  // module
  const title = opts.title || '';
  return `<script>
      (function () {
        var isEmbedded = window.parent && window.parent !== window;

        window.Gateway = {
          ready: true,
          embedded: isEmbedded,
          layout: {
            topbar: true,
            sidebar: true,
            branding: true,
          },
          getCurrentUser: function () {
            return { name: '开发者', email: 'dev@alioth.local', avatar: null };
          },
          navigateTo: function (path) {
            if (isEmbedded) {
              window.parent.postMessage({ type: 'gateway:navigate', path: path }, '*');
            } else {
              console.log('[Gateway] Navigate:', path);
            }
          },
          onBlockEvent: function (event, data) {
            if (isEmbedded) {
              window.parent.postMessage(
                { type: 'gateway:block-event', event: event, data: data },
                '*',
              );
            }
          },
          notifyReady: function () {
            if (isEmbedded) {
              window.parent.postMessage(
                { type: 'gateway:module-ready', moduleName: ${JSON.stringify(title)} },
                '*',
              );
            }
          },
        };

        if (isEmbedded) {
          window.addEventListener('message', function (e) {
            if (e.data && e.data.type === 'gateway:navigate') {
              console.log('[Gateway] Received navigate:', e.data.path);
            }
          });
        }

        setTimeout(function () {
          window.Gateway.notifyReady();
        }, 1000);
      })();
    </script>`;
}
