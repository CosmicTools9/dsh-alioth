/**
 * module-shell.tsx — AliothStudio Module 原型壳（ESM 渲染模块）。
 *
 * 取代原 module-shell.html。由 prototype-tool.js 在构建期通过 bun import 调用
 * renderModuleShell(opts) 生成完整 HTML 文档，写出 m-v{N}.html。
 *
 * preview HTML 对齐 gateway-shell.tsx v2.0.0 DOM 结构：
 *   root → TopBar + body[Navigation + main[accent-bar + content[inner[block-scroll + Footer]]] + WorkspaceDock]
 *
 * embedded 模式行为：当模块被 App 以 embedded=true 集成时，
 * Module 只渲染 Nav sidebar + Content-area 容器（含唯一 overflow-y-auto 滚动视口），
 * 不渲染 TopBar / Footer。TopBar 由全局壳统一提供。
 * standalone 模式（embedded=false/undefined）：渲染完整 Gateway Shell 作为验证基准。
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
import { MODULE_SHELL_CSS } from './shell-css';

export interface ModuleShellOpts {
  title: string;
  rootPath: string;
  bundleJs: string;
  bodyClass: string;
  /** esbuild IIFE 暴露的全局名,如 Module__system_settings */
  globalName: string;
  /** 逻辑名(传给 lifecycle props.name) */
  name: string;
  tokensCss: string;
  iconPool: string;
  /** 预览模式:仅渲染壳骨架(boot-skeleton + CSS),省略 vendor/bundle/mountScript */
  previewMode?: boolean;
}

export function renderModuleShell(opts: ModuleShellOpts): string {
  const {
    title,
    rootPath,
    bundleJs,
    bodyClass,
    globalName,
    name,
    tokensCss,
    iconPool,
    previewMode,
  } = opts;
  const driver = previewMode
    ? ''
    : `${REQUIRE_SHIM}
<script src="${bundleJs}"></script>
${mountScript({ kind: 'module', globalName, name, baseUrl: '/', notFoundTitle: '模块未加载', notFoundHint: 'bundle 未导出模块布局组件' })}`;
  return `<!DOCTYPE html>
<html lang="zh-CN" dir="ltr">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>${title}</title>

<link rel="stylesheet" href="${rootPath}.agents/skills/alioth-design/references/vendor/fonts/inter.css">
<link rel="stylesheet" href="${rootPath}.agents/skills/alioth-design/references/vendor/fonts/jetbrains-mono.css">
${tokensCss}
<style>${MODULE_SHELL_CSS}</style>
</head>
<body class="${bodyClass}">

 ${previewMode ? '' : vendorScripts(rootPath)}

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
        <a href="#" class="relative inline-flex items-center gap-1.5 px-3 py-1 text-[13px] font-medium no-underline transition-colors text-foreground bg-background border border-border border-b-transparent rounded-t-lg shadow-tab" title="${name}">
          <span class="whitespace-nowrap">${name}</span>
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
              <button class="inline-flex items-center gap-1.5 h-8 px-3 rounded-lg bg-primary text-primary-foreground text-sm font-medium border-none cursor-pointer">+ 新建单位</button>
              <button class="inline-flex items-center gap-1.5 h-8 px-3 rounded-lg border text-sm cursor-pointer bg-transparent">⟳ 导入标准</button>
              <span class="ml-auto text-sm text-muted-foreground">共 32 个单位</span>
            </div>
            <div class="bg-card border border-border rounded-lg overflow-hidden">
              <div class="flex items-center gap-3 p-4 border-b border-border">
                <span class="w-8 h-8 rounded-lg bg-primary text-primary-foreground flex items-center justify-center font-bold text-sm">SI</span>
                <div class="flex-1"><div class="font-semibold text-[15px]">SI单位制</div><div class="text-xs text-muted-foreground">国际单位制，7 个基本量纲 + 22 个导出量纲</div></div>
                <div class="flex gap-1.5 flex-wrap">
                  <span class="text-[11px] px-2 py-0.5 rounded-md bg-primary/10 text-primary">长度 基准</span>
                  <span class="text-[11px] px-2 py-0.5 rounded-md bg-primary/10 text-primary">质量 基准</span>
                  <span class="text-[11px] px-2 py-0.5 rounded-md bg-primary/10 text-primary">时间 基准</span>
                  <span class="text-[11px] px-2 py-0.5 rounded-md bg-primary/10 text-primary">电流 基准</span>
                </div>
              </div>
              <table class="w-full text-sm border-collapse">
                <thead><tr class="bg-secondary text-left text-muted-foreground text-xs">
                  <th class="p-2.5 font-medium">单位名称</th><th class="p-2.5 font-medium">符号</th><th class="p-2.5 font-medium">换算关系</th><th class="p-2.5 font-medium">说明</th><th class="p-2.5 font-medium text-right">操作</th>
                </tr></thead>
                <tbody>
                  <tr class="border-b border-border"><td class="p-2.5">皮米</td><td class="p-2.5 text-primary">pm</td><td class="p-2.5 font-mono">1e-12</td><td class="p-2.5 text-muted-foreground">原子尺度</td><td class="p-2.5 text-right">✎ 🗑</td></tr>
                  <tr class="border-b border-border"><td class="p-2.5">纳米</td><td class="p-2.5 text-primary">nm</td><td class="p-2.5 font-mono">1e-9</td><td class="p-2.5 text-muted-foreground">分子尺度</td><td class="p-2.5 text-right">✎ 🗑</td></tr>
                  <tr class="border-b border-border"><td class="p-2.5">微米</td><td class="p-2.5 text-primary">μm</td><td class="p-2.5 font-mono">0.000001</td><td class="p-2.5 text-muted-foreground">细胞尺度</td><td class="p-2.5 text-right">✎ 🗑</td></tr>
                  <tr class="border-b border-border"><td class="p-2.5">毫米</td><td class="p-2.5 text-primary">mm</td><td class="p-2.5 font-mono">0.001</td><td class="p-2.5 text-muted-foreground">日常精细</td><td class="p-2.5 text-right">✎ 🗑</td></tr>
                </tbody>
              </table>
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
