/**
 * i18n.ts — AliothStudio 原型共享 i18n 字典 + 工具（对齐 gateway-shell.tsx）。
 *
 * 来源：gateway-shell.tsx `I18N.dict['zh-CN']` —— system-settings 模块 i18n 字典，
 * 含 branding/nav/block/common/page/drawer/wizard 7 个命名空间。
 *
 * 用法：
 *   import { t, setLocale, toggleLocale } from '../_kit/i18n';
 *   <h1>{t('system-settings.page.unit-system.h2')}</h1>
 *
 * locale 切换（gateway-shell.tsx）：
 *   setLocale('en')  // 切换 I18N.dict[locale]
 *   toggleLocale()   // 在 zh-CN / en 间切换
 *
 * Note: P4 阶段交付。当前 (2026-07-11) 仅落地字典 + 工具函数；5 个 block + module.tsx 字面量替换
 *       在 tasks §4.4a 阶段完成。
 */

export interface I18nDict {
  'system-settings'?: {
    branding?: { name?: string; sub?: string };
    nav?: { 'group-dimension'?: string; 'group-infrastructure'?: string; 'group-appearance'?: string };
    block?: { 'unit-system'?: string; 'exchange-rate'?: string; environment?: string; 'license-mgmt'?: string; theme?: string; language?: string };
    common?: { new?: string; edit?: string; delete?: string; save?: string; cancel?: string; search?: string; loading?: string; empty?: string; confirmDeleteTitle?: string; confirmDeleteMessage?: string };
    page?: {
      'unit-system'?: { h2?: string };
      'exchange-rate'?: { h2?: string };
      environment?: { h2?: string };
      'license-mgmt'?: { h2?: string };
      theme?: { h2?: string };
      language?: { h2?: string };
    };
    drawer?: {
      'unit-system-create-title'?: string;
      'exchange-rate-create-title'?: string;
      'environment-create-title'?: string;
      'license-activate-title'?: string;
      'license-create-title'?: string;
      'language-install-title'?: string;
    };
    wizard?: {
      step1?: string;
      step2?: string;
      step3?: string;
      testConnection?: string;
      verifyCredential?: string;
      back?: string;
      retry?: string;
      complete?: string;
      testing?: string;
      connectionSuccess?: string;
    };
    empty?: { noData?: string };
  };
}

export const I18N: { locale: 'zh-CN' | 'en'; dict: Record<string, unknown> } = {
  locale: 'zh-CN',
  dict: {},
};

I18N.dict['zh-CN'] = {
  'system-settings': {
    branding: { name: '系统设置', sub: '通用基础配置' },
    nav: {
      'group-dimension': '量纲基础',
      'group-infrastructure': '基础设施',
      'group-appearance': '外观与语言',
    },
    block: {
      'unit-system': '单位制',
      'exchange-rate': '汇率',
      environment: '运行环境',
      'license-mgmt': '许可证',
      theme: '主题',
      language: '语言',
    },
    common: {
      new: '新建',
      edit: '编辑',
      delete: '删除',
      save: '保存',
      cancel: '取消',
      search: '搜索',
      loading: '加载中…',
      empty: '暂无数据',
      confirmDeleteTitle: '确认删除 {name}？',
      confirmDeleteMessage: '此操作不可撤销。',
    },
    page: {
      'unit-system': { h2: '单位制管理' },
      'exchange-rate': { h2: '汇率管理' },
      environment: { h2: '运行环境' },
      'license-mgmt': { h2: '许可证管理' },
      theme: { h2: '主题设置' },
      language: { h2: '语言与区域' },
    },
    drawer: {
      'unit-system-create-title': '新建单位',
      'exchange-rate-create-title': '新增汇率',
      'environment-create-title': '新建环境',
      'license-activate-title': '激活许可证',
      'license-create-title': '新建许可证',
      'language-install-title': '安装语言包',
    },
    wizard: {
      step1: '基本信息',
      step2: '测试连接',
      step3: '凭证验证',
      testConnection: '测试连接',
      verifyCredential: '验证凭证',
      back: '上一步',
      retry: '重试',
      complete: '完成',
      testing: '验证中…',
      connectionSuccess: '连接成功',
    },
    empty: { noData: '暂无数据' },
  },
};

// English fallback (used by setLocale('en') if 'en' dict not yet populated)
I18N.dict['en'] = {};

let _setLocaleBridge: ((loc: string) => void) | null = null;

/**
 * Set the active locale. If `I18N.dict[loc]` is empty, falls back to 'zh-CN'.
 */
export function setLocale(loc: string): void {
  if (I18N.locale === loc) return;
  const target = I18N.dict[loc];
  const targetFilled = target !== null && typeof target === 'object' && Object.keys(target).length > 0;
  if (targetFilled) {
    I18N.locale = loc as 'zh-CN' | 'en';
  } else if (I18N.dict['zh-CN']) {
    I18N.locale = 'zh-CN';
  }
  if (_setLocaleBridge) _setLocaleBridge(loc);
}

/**
 * Toggle between 'zh-CN' and 'en'.
 */
export function toggleLocale(): void {
  setLocale(I18N.locale === 'zh-CN' ? 'en' : 'zh-CN');
}

/**
 * Internal bridge used by the prototype shell to sync window-level locale state.
 * (Reference line 1059: `_setLocaleBridge(loc)`)
 */
export function _registerLocaleBridge(fn: (loc: string) => void): void {
  _setLocaleBridge = fn;
}

/**
 * Look up an i18n key like `'system-settings.page.unit-system.h2'`,
 * walking the dict for the active locale.
 * Falls back to the key itself if not found.
 *
 * Reference line 1050-1057 (function t).
 */
/** camelCase 段 → kebab（查表 miss 时的同义回落；ns 侧存在 `t('a.bCamelD')` 查 kebab 键的键形）。 */
function camelToKebab(s: string): string {
  return s.replace(/([a-z])([A-Z])/g, '$1-$2').toLowerCase();
}

/**
 * 字典节点行走（in-process 值：形态随 ns 字典而定，仅用 `typeof` / `in` 收窄判读）。
 * 段查表 miss 时回落 camelCase→kebab 同义键（ns 侧 `t('a.bCamelD')` 查 kebab 键的键形）。
 */
function lookupNode(dict: unknown, segments: readonly string[]): unknown {
  let cur: unknown = dict;
  for (const p of segments) {
    if (cur === null || typeof cur !== 'object') return undefined;
    const obj = cur as Record<string, unknown>; // 收窄后节点：编译期形态丢失，运行时即字典节点
    if (p in obj) {
      cur = obj[p];
      continue;
    }
    const kebab = camelToKebab(p);
    cur = kebab !== p && kebab in obj ? obj[kebab] : undefined;
  }
  return cur;
}

export function t(key: string, vars?: Record<string, string | number>): string {
  const val = lookupNode(I18N.dict[I18N.locale], key.split('.'));
  let s = typeof val === 'string' ? val : key;
  if (vars) {
    for (const k of Object.keys(vars)) {
      s = s.replace(new RegExp('\\{' + k + '\\}', 'g'), String(vars[k] ?? `{${k}}`));
    }
  }
  return s;
}
