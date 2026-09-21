# Token-aware 样式规则

**MUST — 禁止硬编码非 Token 值。验证：`node scripts/check/extract-design-tokens.mjs <prototype.html>`**

| 类别     | 允许值（Token）                                                                                                                                      | 禁止（常见漂移）                                  |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------- |
| **颜色** | `hsl(var(--primary))`, `hsl(var(--muted))`, `hsl(var(--card))`, `hsl(var(--border))`, `hsl(var(--background))`, `hsl(var(--foreground))` 等 CSS 变量 | `#3B82F6`, `#F0F0F0`, `rgb(...)`, 任何 hex 硬编码 |
| **间距** | 4pt 网格: 4, 8, 12, 16, 20, 24, 32, 40, 48, 64, 80, 96 (px)                                                                                          | 6, 10, 14, 18, 22, 30, 36, 44, 50px 等            |
| **字号** | 12px (xs), 13px (sm/表格), 14px (正文), 16px (base), 18px (lg), 24px (2xl), 36px (4xl), 48px (5xl)                                                   | 15, 17, 20, 22, 28, 32px                          |
| **字重** | 400 (normal), 500 (medium), 600 (semibold), 700 (bold), 800 (extrabold)                                                                              | 300, 450, 550, 650 等                             |
| **行高** | 1.1, 1.2, 1.3, 1.4, 1.5, 1.6                                                                                                                         | 1.15, 1.35, 1.45, 1.7, 1.8                        |
| **圆角** | 4px (sm), 8px (md), 12px (lg), 16px (xl)                                                                                                             | 6, 10, 14, 20, 24px                               |
| **阴影** | `shadow-sm/md/lg/xl/2xl`（对应 `var(--shadow-*)`）                                                                                                   | 自定义 `box-shadow` 值                            |

## 跨侧命名映射（原型别名 ↔ 权威基础 token ↔ 前端 Tailwind 别名）

同一语义在三处有不同名字，**取值来源 MUST 唯一**（权威基础 token）。原型别名 MUST 以 `var()` 引用，MUST NOT 直写值复制
（判据：`bun scripts/check/check-design-token-parity.ts --fail`）。

| 权威基础 token（`DESIGN_TOKENS` §8） | 原型别名（`prototype-base.css`） | 前端 Tailwind `@theme` 别名（`theme-base.css`） |
| --- | --- | --- |
| `--background` | `--bg` | `--color-background` |
| `--foreground` | `--text` | `--color-foreground` |
| `--muted-foreground` | `--text2`、`--muted-fg` | `--color-muted-foreground` |
| `--card` | `--surface` | `--color-card` |
| `--sidebar` | `--sidebar-background` | （无，模块自定义） |
| `--categorical-1..8` | `--chart-1..5`（别名指向 1..5） | `--color-*` 由 `@theme` 派生 |

> 两侧别名写法不同属**机制差异**（原型直接别名；前端由 Tailwind v4 `@theme` 生成 `hsl(var(--x))`），不是漂移；
> **漂移的判据是「取值来源不唯一」**（同一语义两处各带独立取值）。
