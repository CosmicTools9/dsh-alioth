# Preflight 入口预检（唯一正本）

> 每个 Track 启动时 MUST 执行。引用方：`alioth-block`、`alioth-module`（各 SKILL.md 的 ⛔ Preflight 节均指向本文件，禁止复制本内容）。

进入 Track 0/1/2 前，必须先对当前最新原型执行管道预检，验证原型完整性、同步 mock 数据。

```bash
# Module 原型（Track 0 / Module 级）：
bash scripts/preflight-track.sh Pre-Proc/{ns}/Prototypes/Modules/{name}/m-v{N}.html

# Block 原型（Track 1/2 / Block 级）：
bash scripts/preflight-track.sh Pre-Proc/{ns}/Prototypes/Blocks/{id}/b-v{N}.html

# Track 0 自动检测（无需预检）：
bun scripts/prototype-tool.js check Pre-Proc/{ns}/Prototypes/Modules/{name}/

# Track 0/1 创建 llm-tsx/block.tsx 骨架 + build：
bun scripts/prototype-tool.js build Pre-Proc/{ns}/Prototypes/Blocks/{id}/llm-tsx/block.tsx
```

`preflight-track.sh` 执行 3 步：

1. **清理非原型数据** — 移除 artifacts
2. **原型验证**（`sync-prototype.sh --check-only`）— 确认原型可解析、无孤立引用
3. **提取 mock 数据** — 同步到前端代码目录

返回码 0 = 通过，1 = 失败（必须修复后继续）。
