/**
 * lifecycle.ts — AliothStudio 原型轻量 single-spa 生命周期工厂。
 *
 * 契约对齐 Framework/frontend/api/src/micro-app.tsx 的 createMicroAppLifecycle,
 * 但去掉 react-query / jotai(原型是 Track 1 轻量设计原型,不依赖 Framework 包)。
 *
 * 这样原型 tsx 既导出 { bootstrap, mount, unmount } 可被 Gateway 风格的
 * MicroFrontendLoader 直接 import() 消费(双用途),又能被 prototype.html 的
 * shell 通过 window[globalName].mount({domElement, ...}) 调用(独立预览)。
 */

import { createElement } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import type { NavGroup } from '../../.agents/skills/alioth-design/references/gateway-shell';

export interface PrototypeProps {
  /** 挂载容器(由调用方提供,模拟 Gateway MicroFrontendLoader 的 containerRef) */
  domElement: HTMLElement;
  domElementId: string;
  /** 模块/Block/App 逻辑名 */
  name: string;
  /** 路由基址(Gateway 真实传 '/system-settings' 等,原型预览传 '/') */
  baseUrl?: string;
  /** API 基址 */
  apiBaseUrl?: string;
  /** 是否嵌入在 Gateway 内(Gateway 真实 true,原型预览 false) */
  embedded?: boolean;
  /** 导航回调(Gateway 传真实 navigate,原型预览传 mock) */
  navigate?: (path: string) => void;
  [key: string]: unknown;
}

export interface PrototypeLifecycle {
  bootstrap: () => Promise<void>;
  mount: (props: PrototypeProps) => Promise<void>;
  unmount: (props: PrototypeProps) => Promise<void>;
  getConfig: () => Promise<PrototypeConfig>;
}

export interface PrototypeConfig {
  /** 由 App 外壳统一渲染的模块导航分组 */
  navGroups?: NavGroup[];
}

export interface CreatePrototypeOptions {
  /** 模块/Block/App 标识名(用于 CSS class 和日志前缀) */
  name: string;
  /** 根 React 组件 */
  App: React.ComponentType<Record<string, unknown>>;
  /** 自定义应用包装(如 BrowserRouter),原型预览通常不需要 */
  renderApp?: (app: React.ReactElement, props: PrototypeProps) => React.ReactElement;
  /** App/Module 导航配置,由 App 外壳统一渲染 */
  config?: PrototypeConfig;
}

/**
 * 创建原型 single-spa 生命周期对象。
 *
 * mount 行为与 createMicroAppLifecycle 一致:
 * - 暴露 props 到 window.__ALIOTH_APP_PROPS__(供模块检测集成模式)
 * - 给容器加 mod-{name} class
 * - createRoot(domElement) 渲染 App(可选 renderApp 包装)
 */
export function createPrototypeLifecycle(
  opts: CreatePrototypeOptions,
): PrototypeLifecycle {
  const { name, App, renderApp, config = {} } = opts;
  let root: Root | null = null;

  return {
    async bootstrap() {
      console.log(`[${name}:lifecycle] bootstrap`);
    },

    async getConfig() {
      return config;
    },

    async mount(props: PrototypeProps) {
      if (typeof window !== 'undefined') {
        (window as unknown as { __ALIOTH_APP_PROPS__?: PrototypeProps }).__ALIOTH_APP_PROPS__ = props;
      }
      console.log(`[${name}:lifecycle] mount`, props);

      const container = props.domElement;
      if (!container) {
        throw new Error(`[${name}:lifecycle] No domElement provided`);
      }
      container.classList.add(`mod-${name}`);

      if (!root) {
        root = createRoot(container);
      }

      const appElement = createElement(App, props);
      const wrapped = renderApp ? renderApp(appElement, props) : appElement;
      root.render(wrapped);
    },

    async unmount() {
      console.log(`[${name}:lifecycle] unmount`);
      if (root) {
        await new Promise<void>((resolve) => {
          setTimeout(() => {
            if (root) {
              root.unmount();
              root = null;
            }
            resolve();
          }, 0);
        });
      }
    },
  };
}
