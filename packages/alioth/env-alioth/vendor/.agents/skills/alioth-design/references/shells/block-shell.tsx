/**
 * block-shell.tsx — AliothStudio Block 原型壳（ESM 渲染模块）。
 *
 * 取代原 block-shell.html。由 prototype-tool.js 在构建期通过 bun import 调用
 * renderBlockShell(opts) 生成完整 HTML 文档，写出 b-v{N}.html。
 *
 * Block 原型独立打开时展示完整 Gateway Shell（TopBar + Sidebar + main + Footer），
 * 用作设计师验证基准（b-v{N}.html 视觉可在此层独立确认）。
 * Block 组件本身只负责内容区（PageHeader / cards / forms），不拥有滚动视口。
 * 由 mountScript 将其挂载到 main 内容区内的 #root。
 *
 * ESM 集成（Module → Block）时，Module 导入 Block 的 content-only 组件，
 * 将其渲染在 Module 的 Content-area 滚动容器（flex-1 min-h-0 overflow-y-auto）内。
 * Block 的独立 HTML 壳仅作验证用，非集成介质。
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
import { BLOCK_SHELL_CSS } from './shell-css';

export interface BlockShellOpts {
  title: string;
  rootPath: string;
  bundleJs: string;
  blockId: string;
  /** esbuild IIFE 暴露的全局名,如 Block__block_environment */
  globalName: string;
  /** prototype-base.css 的 <style>...</style> 内容（设计令牌单一真相源；已弃用 tailwind-utilities.css） */
  tokensCss: string;
  /** icon-pool.js 的 <script>...</script> 内容 */
  iconPool: string;
  /** 预览模式:仅渲染壳骨架(boot-skeleton + CSS),省略 vendor/bundle/mountScript */
  previewMode?: boolean;
}

const blockPreviewPlaceholder = (title: string) => `
<div class="p-6">
  <div class="mb-4">
    <h1 class="text-lg font-bold text-foreground">${title}</h1>
    <p class="text-sm text-muted-foreground">Block 预览占位</p>
  </div>
  <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
    <div class="rounded-xl border border-border bg-card p-5">
      <div class="font-semibold text-foreground mb-2">${title}</div>
      <div class="text-sm text-muted-foreground">Block content placeholder</div>
    </div>
    <div class="rounded-xl border border-border bg-card p-5">
      <div class="font-semibold text-foreground mb-2">Card Title</div>
      <div class="text-sm text-muted-foreground">Block content placeholder</div>
    </div>
    <div class="rounded-xl border border-border bg-card p-5">
      <div class="font-semibold text-foreground mb-2">Card Title</div>
      <div class="text-sm text-muted-foreground">Block content placeholder</div>
    </div>
  </div>
</div>
`;

export function renderBlockShell(opts: BlockShellOpts): string {
  const { title, rootPath, bundleJs, blockId, globalName, tokensCss, iconPool, previewMode } = opts;
  const notFoundHint = 'window[' + JSON.stringify(globalName) + '] 未定义，请检查 bundle';
  const driver = previewMode
    ? ''
    : `${REQUIRE_SHIM}
    <script src="${bundleJs}"></script>
    ${mountScript({
      kind: 'block',
      globalName,
      name: blockId,
      baseUrl: '/',
      notFoundTitle: 'Block 未加载',
      notFoundHint,
    })}`;
  return `<!doctype html>
<html lang="zh-CN" dir="ltr">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>${title}</title>

    <link rel="stylesheet" href="${rootPath}.agents/skills/alioth-design/references/vendor/fonts/inter.css" />
    <link rel="stylesheet" href="${rootPath}.agents/skills/alioth-design/references/vendor/fonts/jetbrains-mono.css" />
    ${tokensCss}
    <style>${BLOCK_SHELL_CSS}</style>
  </head>
  <body>
    ${previewMode ? '' : vendorScripts(rootPath)}

    <div id="boot-skeleton" class="boot-skeleton">
      ${bootSkeletonHTML('full')}
    </div>

    <div class="flex h-screen flex-col overflow-hidden bg-background">
      <header class="h-14 border-b flex items-center justify-between px-6 bg-background shrink-0">
        <div class="flex items-center gap-2 min-w-0 relative h-full">
          <a href="#" class="flex items-center gap-2.5 text-foreground no-underline shrink-0">
            <span class="text-lg font-bold whitespace-nowrap">Cosmic-Tools</span>
          </a>
          <div class="flex items-center h-full gap-0.5 pl-1">
            <a href="#" class="relative inline-flex items-center gap-1.5 px-3 py-1 text-[13px] font-medium no-underline transition-colors text-foreground bg-background border border-border border-b-transparent rounded-t-lg shadow-tab" title="${title}">
              <span class="whitespace-nowrap">${title}</span>
              <span class="absolute bottom-0 left-1.5 right-1.5 h-[2px] bg-primary"></span>
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
            <div class="px-4 pt-3.5 pb-1 mb-1 text-[10px] font-bold uppercase tracking-[0.06em] text-muted-foreground/55">当前 Block</div>
            <a href="#" class="flex items-center gap-2.5 px-4 py-2 mx-2 rounded-lg text-sm font-medium bg-primary/10 text-primary font-semibold">${title}</a>
          </nav>
          <div class="border-t px-3 h-10 flex items-center gap-2">
            <button class="w-7 h-7 rounded-md flex items-center justify-center text-muted-foreground hover:bg-accent transition-colors cursor-pointer border-none bg-transparent" title="折叠侧栏">◀</button>
          </div>
        </aside>
        <div class="flex flex-col min-w-0 overflow-hidden flex-1">
          <div class="h-[3px] w-full bg-primary/15 shrink-0"></div>
          <main class="flex-1 w-full h-full bg-muted/30 overflow-hidden">
            <div class="flex flex-col h-full">
              <div class="flex-1 min-h-0 overflow-y-auto">
                <div id="root">${previewMode ? blockPreviewPlaceholder(title) : ''}</div>
              </div>
              <footer class="hidden md:flex h-10 border-t items-center justify-between px-4 md:px-6 text-xs text-muted-foreground shrink-0 bg-card">
                <span class="overflow-hidden text-ellipsis whitespace-nowrap">© 2026 Cosmic-Tools</span>
                <span class="hidden sm:inline overflow-hidden text-ellipsis whitespace-nowrap">0.1.1</span>
              </footer>
        </div>
      </div>
    </div>

    ${previewMode ? '' : iconPool}

    ${driver}

    ${BOOT_FADE_CSS}
    ${previewMode ? PREVIEW_FADE_SCRIPT : ''}
    ${previewMode ? PREVIEW_ENHANCE_CSS : ''}
  </body>
</html>`;
}
