/**
 * app-shell.tsx — AliothStudio App 原型壳（ESM 渲染模块）。
 *
 * 取代原 app-shell.html。由 prototype-tool.js 在构建期通过 bun import 调用
 * renderAppShell(opts) 生成完整 HTML 文档，写出 a-v{N}.html。
 * 注：app 壳按原设计不含 Gateway bridge 段（与旧 app-shell.html 行为一致）。
 *
 * preview HTML 对齐 gateway-shell.tsx v2.0.0 DOM 结构：
 *   root → TopBar + body[Navigation + main[accent-bar + content[inner[block-scroll + Footer]]] + WorkspaceDock]
 *
 * App 层只负责 Module 级工具栏（ModuleTabs），集成在 Global TopBar 内。
 * TopBar 本身（品牌 + 搜索 + actions + 用户菜单）是全局层，不属于 App 或任何单体层。
 * App standalone HTML 包含完整 Gateway Shell 作为验证基准。
 *
 * 集成契约见 openspec/changes/spec-block-module-app-shell-contract/specs/block-module-app-integration/spec.md
 */
import {
  vendorScripts,
  REQUIRE_SHIM,
  BOOT_FADE_CSS,
  PREVIEW_ENHANCE_CSS,
  PREVIEW_FADE_SCRIPT,
  bootSkeletonHTML,
  mountScript,
  previewTopBarRight,
} from './shell-shared';
import { APP_SHELL_CSS } from './shell-css';

export interface AppShellOpts {
  title: string;
  rootPath: string;
  bundleJs: string;
  /** esbuild IIFE 暴露的全局名,如 App__ai_b3ac30776a3a725d */
  globalName: string;
  /** 逻辑名(传给 lifecycle props.name) */
  name: string;
  tokensCss: string;
  iconPool: string;
  /** 预览模式:仅渲染壳骨架(boot-skeleton + CSS),省略 vendor/bundle/mountScript */
  previewMode?: boolean;
}

export function renderAppShell(opts: AppShellOpts): string {
  const { title, rootPath, bundleJs, globalName, name, tokensCss, iconPool, previewMode } = opts;
  const driver = previewMode
    ? ''
    : `${vendorScripts(rootPath)}

${REQUIRE_SHIM}
<script src="${bundleJs}"></script>
${mountScript({ kind: 'app', globalName, name, baseUrl: '/', notFoundTitle: 'App 未加载', notFoundHint: 'bundle 未导出 AppLayout' })}`;
  return `<!DOCTYPE html>
<html lang="zh-CN" dir="ltr">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>${title}</title>

<link rel="stylesheet" href="${rootPath}.agents/skills/alioth-design/references/vendor/fonts/inter.css">
<link rel="stylesheet" href="${rootPath}.agents/skills/alioth-design/references/vendor/fonts/jetbrains-mono.css">
${tokensCss}
<style>${APP_SHELL_CSS}</style>
</head>
<body>

<div id="boot-skeleton" class="boot-skeleton">
  ${bootSkeletonHTML('full')}
</div>

<div id="root">${
    previewMode
      ? `<div class="flex h-screen flex-col overflow-hidden bg-background">
  <header class="h-14 border-b flex items-center justify-between px-6 bg-background shrink-0">
    <div class="flex items-center gap-2 min-w-0 relative h-full">
      <a href="#" class="flex items-center gap-2.5 text-foreground no-underline shrink-0">
        <span class="text-lg font-bold whitespace-nowrap">Cosmic-Tools</span>
      </a>
      <div class="flex items-center h-full gap-0.5 pl-1">
        <a href="#" class="relative inline-flex items-center gap-1.5 px-3 py-1 text-[13px] font-medium no-underline transition-colors text-foreground bg-background border border-border border-b-transparent rounded-t-lg shadow-tab" title="系统设置">
          <span class="whitespace-nowrap">系统设置</span>
          <span class="absolute bottom-0 left-1.5 right-1.5 h-[2px] bg-primary"></span>
        </a>
        <a href="#" class="relative inline-flex items-center gap-1.5 px-3 py-1 text-[13px] font-medium no-underline transition-colors text-muted-foreground hover:bg-accent rounded-t-lg" title="EmpAgent">
          <span class="whitespace-nowrap">EmpAgent</span>
        </a>
      </div>
    </div>
    ${previewTopBarRight({
      triggers: [
        { title: 'EmpAgent' },
        { title: '审批' },
        { title: '收件箱', badge: 1 },
        { title: '个人中心' },
        { title: '日程' },
      ],
    })}
  </header>
  <div class="flex flex-1 min-h-0 overflow-hidden">
    <aside class="flex flex-col border-r bg-secondary shrink-0 w-60">
      <nav class="flex flex-col gap-1 py-2 flex-1 overflow-y-auto">
        <div class="mb-4">
          <div class="px-4 pt-3.5 pb-1 mb-1 text-[10px] font-bold uppercase tracking-[0.06em] text-muted-foreground/55">量纲基础</div>
          <a href="#" class="flex items-center gap-2.5 px-4 py-2 mx-2 rounded-lg text-sm font-medium bg-primary/10 text-primary font-semibold">单位制</a>
          <a href="#" class="flex items-center gap-2.5 px-4 py-2 mx-2 rounded-lg text-sm font-medium text-muted-foreground hover:bg-accent">汇率</a>
        </div>
        <div class="mb-4">
          <div class="px-4 pt-3.5 pb-1 mb-1 text-[10px] font-bold uppercase tracking-[0.06em] text-muted-foreground/55">基础设施</div>
          <a href="#" class="flex items-center gap-2.5 px-4 py-2 mx-2 rounded-lg text-sm font-medium text-muted-foreground hover:bg-accent">环境配置</a>
          <a href="#" class="flex items-center gap-2.5 px-4 py-2 mx-2 rounded-lg text-sm font-medium text-muted-foreground hover:bg-accent">许可证管理</a>
        </div>
        <div class="mb-4">
          <div class="px-4 pt-3.5 pb-1 mb-1 text-[10px] font-bold uppercase tracking-[0.06em] text-muted-foreground/55">外观与语言</div>
          <a href="#" class="flex items-center gap-2.5 px-4 py-2 mx-2 rounded-lg text-sm font-medium text-muted-foreground hover:bg-accent">主题</a>
          <a href="#" class="flex items-center gap-2.5 px-4 py-2 mx-2 rounded-lg text-sm font-medium text-muted-foreground hover:bg-accent">语言</a>
        </div>
      </nav>
      <div class="border-t px-3 h-10 flex items-center gap-2">
        <button class="w-7 h-7 rounded-md flex items-center justify-center text-muted-foreground hover:bg-accent transition-colors cursor-pointer border-none bg-transparent" title="折叠侧栏">◀</button>
      </div>
    </aside>
    <div class="flex flex-col min-w-0 overflow-hidden flex-1">
      <div class="h-[3px] w-full bg-primary/15 shrink-0"></div>
      <main class="flex-1 w-full h-full bg-muted/30 overflow-hidden">
        <div class="flex flex-col h-full">
          <div class="flex-1 min-h-0 overflow-y-auto p-6">
            <div class="flex items-center gap-3 mb-5">
              <span class="text-lg font-bold">SI 单位制</span>
              <span class="text-sm text-muted-foreground">IMP 英制单位制</span>
              <span class="text-sm text-muted-foreground">CN 市制单位制</span>
            </div>
            <div class="bg-card border border-border rounded-lg p-5 mb-4 flex items-center gap-2.5">
              <span class="w-9 h-9 rounded-lg bg-primary text-primary-foreground flex items-center justify-center font-bold">SI</span>
              <div><div class="font-semibold text-[15px]">SI单位制</div><div class="text-xs text-muted-foreground">国际单位制，7 个基本量纲 + 22 个导出量纲，是全球科学、工程、贸易通用标准。</div></div>
            </div>
          </div>
          <footer class="hidden md:flex h-10 border-t items-center justify-between px-4 md:px-6 text-xs text-muted-foreground shrink-0 bg-card">
            <span class="overflow-hidden text-ellipsis whitespace-nowrap">© 2026 Cosmic-Tools</span>
            <span class="hidden sm:inline overflow-hidden text-ellipsis whitespace-nowrap">0.1.1</span>
          </footer>
        </div>
      </main>
    </div>
    <div class="w-80 h-full shrink-0 border-l border-border bg-card flex flex-col overflow-hidden">
      <div class="h-14 border-b border-border flex items-center justify-between px-4 shrink-0">
        <span class="text-sm font-semibold text-foreground">工作区</span>
        <button class="w-7 h-7 rounded-md flex items-center justify-center text-muted-foreground hover:bg-accent transition-colors cursor-pointer border-none bg-transparent" title="关闭">✕</button>
      </div>
      <div class="flex-1 overflow-y-auto p-3">
        <div class="text-sm text-muted-foreground p-3">工作区内容区</div>
      </div>
    </div>
  </div>
</div>`
      : ''
  }</div>

${previewMode ? '' : iconPool}

${driver}

${BOOT_FADE_CSS}
${previewMode ? PREVIEW_FADE_SCRIPT : ''}
${previewMode ? PREVIEW_ENHANCE_CSS : ''}
</body>
</html>`;
}
