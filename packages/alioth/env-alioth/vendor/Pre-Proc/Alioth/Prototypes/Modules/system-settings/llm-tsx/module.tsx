/**
 * system-settings module.tsx — 系统设置模块布局。
 * 使用 gateway-shell.tsx 的 GatewayShell（standalone）/ Navigation（embedded）。
 * 集成顺序: block → module(本文件) → app。
 */
import { useState } from 'react';
import BlockUnitSystem from '../../../Blocks/block-unit-system/llm-tsx/block';
import BlockExchangeRate from '../../../Blocks/block-exchange-rate/llm-tsx/block';
import BlockEnvironment from '../../../Blocks/block-environment/llm-tsx/block';
import LicensePage from '../../../Blocks/block-license-mgmt/llm-tsx/block';
import BlockTheme from '../../../Blocks/block-theme/llm-tsx/block';
import BlockLanguage from '../../../Blocks/block-language/llm-tsx/block';
import { createPrototypeLifecycle } from '../../../_shared/lifecycle';
import {
  GatewayShell,
  Navigation,
  WorkspaceDock,
  type NavGroup, type ModuleTab, type WorkspaceTrigger, type User,
} from '../../../../../../.agents/skills/alioth-design/references/gateway-shell';

const BLOCK_COMPS: Record<string, () => JSX.Element> = {
  'block-unit-system': BlockUnitSystem,
  'block-exchange-rate': BlockExchangeRate,
  'block-environment': BlockEnvironment,
  'block-license-mgmt': LicensePage,
  'block-theme': BlockTheme,
  'block-language': BlockLanguage,
};

const NAV_GROUPS: NavGroup[] = [
  { label: '量纲基础', items: [
    { id: 'block-unit-system', label: '单位制', icon: 'globe' },
    { id: 'block-exchange-rate', label: '汇率', icon: 'dollarSign' },
  ]},
  { label: '基础设施', items: [
    { id: 'block-environment', label: '环境配置', icon: 'server' },
    { id: 'block-license-mgmt', label: '许可证管理', icon: 'key' },
  ]},
  { label: '外观与语言', items: [
    { id: 'block-theme', label: '主题', icon: 'palette' },
    { id: 'block-language', label: '语言', icon: 'translate' },
  ]},
];

const ALL_ITEMS = NAV_GROUPS.flatMap((g) => g.items);
const DEFAULT_ID = ALL_ITEMS[0].id;

const MODULE_TABS: ModuleTab[] = [{ id: 'system-settings', label: '系统设置', active: true }];
const DEMO_USER: User = { name: '开发者', email: 'dev@alioth.local', role: 'admin' };
const WORKSPACE_TRIGGERS: WorkspaceTrigger[] = [
  { id: 'ai', icon: 'bot', title: 'AI 助手' },
  { id: 'approval', icon: 'clipboardCheck', title: '审批' },
  { id: 'inbox', icon: 'mail', title: '收件箱', unreadCount: 1 },
  { id: 'profile', icon: 'user', title: '个人中心' },
  { id: 'schedule', icon: 'calendar', title: '日程' },
];

function ModuleLayout({ embedded = false }: { embedded?: boolean }) {
  const [activeId, setActiveId] = useState(DEFAULT_ID);
  const [collapsed, setCollapsed] = useState(false);
  const [activeWorkspace, setActiveWorkspace] = useState<string | null>(null);

  const ActiveComp = BLOCK_COMPS[activeId] || null;
  const activeLabel = ALL_ITEMS.find((i) => i.id === activeId)?.label;

  const handleSelect = (id: string) => { setActiveId(id); setActiveWorkspace(null); };

  const blockContent = ActiveComp ? <ActiveComp /> : (
    <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground">
      <h3 className="text-base font-semibold text-foreground">场景未加载</h3>
      <p className="text-sm">{activeLabel}</p>
    </div>
  );

  // embedded 模式(App 内):Module 渲染 NavSidebar + Content-area 容器（含滚动视口）
  // TopBar 由 App 的 TopBar-only 模式统一提供
  if (embedded) {
    return (
      <div className="flex h-full w-full overflow-hidden">
        <Navigation
          groups={NAV_GROUPS}
          activeId={activeId}
          collapsed={collapsed}
          onSelect={handleSelect}
          onToggle={() => setCollapsed((c) => !c)}
          brand="Alioth Studio"
          brandIcon="gatewayLogo"
        />
        <div className="flex flex-col min-w-0 overflow-hidden flex-1">
          <main className="flex-1 w-full h-full bg-muted/30 overflow-hidden">
            <div className="flex flex-col h-full">
              <div className="flex-1 min-h-0 overflow-y-auto">{blockContent}</div>
            </div>
          </main>
        </div>
        {activeWorkspace && (
          <WorkspaceDock
            active
            title="工作区"
            onClose={() => setActiveWorkspace(null)}
          >
            <div className="p-3 rounded-lg border border-border bg-card mb-2">
              <div className="text-sm font-medium text-foreground mb-1">项目评审</div>
              <div className="text-xs text-muted-foreground">14:00 - 15:00</div>
            </div>
          </WorkspaceDock>
        )}
      </div>
    );
  }

  // 完整模式:使用 GatewayShell 组装,对齐 reference MainLayout
  return (
    <GatewayShell
      brand="Alioth Studio"
      brandIcon="gatewayLogo"
      moduleTabs={MODULE_TABS}
      searchPlaceholder="搜索设置项"
      triggers={WORKSPACE_TRIGGERS}
      user={DEMO_USER}
      onTrigger={setActiveWorkspace}
      navGroups={NAV_GROUPS}
      activeId={activeId}
      collapsed={collapsed}
      onSelect={handleSelect}
      onToggle={() => setCollapsed((c) => !c)}
      activeWorkspace={activeWorkspace}
      onWorkspaceClose={() => setActiveWorkspace(null)}
      workspaceChildren={
        activeWorkspace === 'schedule' ? (
          <div className="p-3 rounded-lg border border-border bg-card mb-2">
            <div className="text-sm font-medium text-foreground mb-1">项目评审</div>
            <div className="text-xs text-muted-foreground">14:00 - 15:00</div>
          </div>
        ) : undefined
      }
      rootClass="system-settings"
      footerBrand="Alioth Studio"
      footerVersion="v0.1.1"
    >
      {blockContent}
    </GatewayShell>
  );
}

window.ModuleLayout = ModuleLayout;
export default ModuleLayout;

export const { bootstrap, mount, unmount, getConfig } = createPrototypeLifecycle({
  name: 'system-settings',
  App: ModuleLayout,
});
