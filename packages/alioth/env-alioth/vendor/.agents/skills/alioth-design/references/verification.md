# Verification：输出验证流程

大部分 agent 环境（Claude Code / Codex / Cursor / Trae 等）没有内置的 `fork_verifier_agent`。我们用 **ego-browser** 控制用户真实浏览器，覆盖相同的验证场景。

## 验证清单

每次产出 HTML 后，按这个清单做一遍：

### 1. 浏览器渲染检查（必做）

最基础：**HTML 能不能打开**？在 macOS 上：

```bash
open -a "Google Chrome" "/path/to/your/design.html"
```

推荐用 ego-browser 截图验证：

```bash
# 1. 健康检查
ego-browser --version
# 2. 按 visual-verification-protocol.md §8 执行 navigate + screenshot
```

### 2. 控制台错误检查

HTML 文件里最常见的问题是 JS 报错导致白屏。运行：

```bash
bun .agents/skills/alioth-design/scripts/verify.ts path/to/design.html
```

该脚本会：

1. 校验文件结构
2. 输出 ego-browser 验证指令
3. 自动调用 `audit-html-spec.ts` 做规约合规审计

详见 `.agents/skills/alioth-design/scripts/verify.ts`。

### 3. 多视口检查

```bash
bun .agents/skills/alioth-design/scripts/verify.ts design.html --viewports 1920x1080,1440x900,768x1024,375x667
```

### 4. 交互检查

推荐使用 ego-browser 的 `click` / `fill` / `evaluate` 在真实浏览器中触发并验证。

## ego-browser 操作速查

- `ego-browser --version` — 健康检查
- `navigate` — 打开页面（支持 `file://` 和 `http://`）
- `screenshot` — 截图（viewport / 元素 / 全页）
- `evaluate` — 注入 JS，读取 DOM / computed style / console 日志
- `click` / `fill` — 元素交互
- `console` — 捕获控制台日志
- `close_tab` / `close_session` — 关闭标签页（必须）

详细用法见 `skill://ego-browser/SKILL.md` 和本目录的 `visual-verification-protocol.md`。

## 截图最佳实践

### 截完整页面

```bash
# 通过 evaluate 获取 document.body.scrollHeight 后调用 screenshot
# 或 ego-browser 支持 full_page 时直接传入参数
```

### 截 viewport

```bash
# navigate 后设置 viewport，再 screenshot
```

### 截特定元素

```bash
# screenshot 传入 selector（CSS 或 snapshot 返回的 @e ref）
```

### 高清截图

```bash
# evaluate 设置 devicePixelRatio = 2，再 screenshot
```

### 等动画结束再截

```bash
# navigate 后等待 2s，让 CSS transition/animation settle
```

## 把截图发给用户

### 本地截图直接打开

```bash
open screenshot.png
```

### 项目推荐验证命令

```bash
# 基础：校验 + 输出 ego-browser 指令
bun .agents/skills/alioth-design/scripts/verify.ts design.html

# 多 viewport
bun .agents/skills/alioth-design/scripts/verify.ts design.html --viewports 1920x1080,375x667

# 全量原型 ego-browser 验证
bun scripts/visual-verify.ts
```

## 验证出错时

### 页面白屏

控制台一定有错。先检查：

1. 产物是否经 `prototype-tool.js build` 构建（esbuild IIFE bundle，非手写 HTML；见 `react-setup.md`）
2. 是不是 `const styles = {...}` 命名冲突（bundle 共享作用域）
3. 入口组件有没有 `export default function`（缺默认导出时产物白屏）
4. build 输出有无 esbuild 报错（import 路径/符号冲突）

### 动画卡

- 用 Chrome DevTools Performance tab 录一段
- 找 layout thrashing（频繁的 reflow）
- 动效优先用 `transform` 和 `opacity`（GPU 加速）

### 字体不对

- 检查 `@font-face` 的 url 是否可访问
- 检查 fallback 字体
- 中文字体加载慢：先显示 fallback，加载完再切换

### 布局错位

- 检查 `box-sizing: border-box` 是否全局应用
- 检查 `* { margin: 0; padding: 0 }` reset
- Chrome DevTools 里打开 gridlines 看实际布局

## 验证=设计师的第二双眼

**永远要自己过一遍**。AI 写代码时经常出现：

- 看起来对但 interaction 有 bug
- 静态截图好但 scroll 时错位
- 宽屏好看但窄屏崩
- Dark mode 忘了测
- Tweaks 切换后某些组件没响应

**最后 1 分钟的验证可以省 1 小时的返工**。
