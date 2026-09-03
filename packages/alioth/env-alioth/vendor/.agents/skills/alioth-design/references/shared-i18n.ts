/* ═══════════════════════════════════════════════════════════════
 * shared-i18n.ts — AliothStudio 管道共享国际化模块
 *
 * 所有 scene 的 TSX 文件通过 import 引用此模块的 t() 函数。
 * 字典在此一次性定义，避免场景间重复浪费 bundle 体积。
 *
 * 使用方式（scene.tsx）：
 *   import { t } from '../../../../.agents/skills/alioth-design/references/shared-i18n';
 *   // 在渲染中使用 t('key') 即可
 *
 * 字典是扁平键值对（dot-separated keys），与 t() 的查找方式一致：
 *   t('system-settings.page.language.h2') → '语言与区域'
 *
 * 维护原则：
 * - 新增翻译：在对应的模块键下追加
 * - 键名规范：{namespace}.{domain}.{item}.{field}
 * - 值可以含 {placeholder} 模板变量
 * ============================================================ */

// ── 扁平翻译字典 ──
var DICT = {
  /* system-settings */
  'system-settings.branding.name': '系统设置',
  'system-settings.branding.sub': '通用基础配置',
  'system-settings.nav.group-dimension': '量纲基础',
  'system-settings.nav.group-infrastructure': '基础设施',
  'system-settings.nav.group-appearance': '外观与语言',
  'system-settings.scene.unit-system': '单位制',
  'system-settings.scene.exchange-rate': '汇率',
  'system-settings.scene.environment': '运行环境',
  'system-settings.scene.license-mgmt': '许可证',
  'system-settings.scene.theme': '主题',
  'system-settings.scene.language': '语言',

  /* common */
  'system-settings.common.new': '新建',
  'system-settings.common.edit': '编辑',
  'system-settings.common.delete': '删除',
  'system-settings.common.save': '保存',
  'system-settings.common.cancel': '取消',
  'system-settings.common.search': '搜索',
  'system-settings.common.loading': '加载中…',
  'system-settings.common.empty': '暂无数据',
  'system-settings.common.confirmDeleteTitle': '确认删除 {name}？',
  'system-settings.common.confirmDeleteMessage': '此操作不可撤销。',
  'system-settings.common.testConnection': '测试连接',
  'system-settings.common.verifyCredential': '验证凭证',
  'system-settings.common.back': '返回',
  'system-settings.common.retry': '重试',
  'system-settings.common.complete': '完成',
  'system-settings.common.connectionSuccess': '连接成功',
  'system-settings.common.testing': '验证中…',

  /* pages */
  'system-settings.page.unit-system.h2': '单位制管理',
  'system-settings.page.exchange-rate.h2': '汇率管理',
  'system-settings.page.environment.h2': '运行环境',
  'system-settings.page.license-mgmt.h2': '许可证管理',
  'system-settings.page.theme.h2': '主题设置',
  'system-settings.page.language.h2': '语言与区域',

  /* drawers */
  'system-settings.drawer.unit-system-create-title': '新建单位',
  'system-settings.drawer.exchange-rate-create-title': '新增汇率',
  'system-settings.drawer.environment-create-title': '新增运行环境',
  'system-settings.drawer.license-activate-title': '激活许可证',
  'system-settings.drawer.license-create-title': '新增许可证',
  'system-settings.drawer.language-install-title': '安装语言包',

  /* errors */
  'system-settings.error.generic': '操作失败，请重试',
  'system-settings.error.networkError': '网络连接异常',
  'system-settings.error.retry': '重试',
  'system-settings.error.dismiss': '关闭',
  'system-settings.error.connectionTimeout': '连接超时：无法到达目标主机',
  'system-settings.error.invalidCredential': '凭证无效：认证失败',

  /* loading */
  'system-settings.loading.spinner': '加载中…',
  'system-settings.loading.skeleton': '内容加载中',

  /* empty */
  'system-settings.empty.noData': '暂无数据',

  /* wizard */
  'system-settings.wizard.step1': '连接测试',
  'system-settings.wizard.step2': '凭证验证',
  'system-settings.wizard.step3': '完成',
  'system-settings.wizard.testConnection': '测试连接',
  'system-settings.wizard.verifyCredential': '验证凭证',
  'system-settings.wizard.back': '返回',
  'system-settings.wizard.retry': '重试',
  'system-settings.wizard.complete': '完成',
  'system-settings.wizard.connectionSuccess': '连接成功',
  'system-settings.wizard.testing': '验证中…',
};

/* ── 翻译函数 ── */
export function t(key, vars) {
  var val = DICT[key];
  if (val === undefined) return key;
  if (!vars) return val;
  return val.replace(/\{(\w+)\}/g, function(m, name) {
    return vars[name] !== undefined ? String(vars[name]) : m;
  });
}
