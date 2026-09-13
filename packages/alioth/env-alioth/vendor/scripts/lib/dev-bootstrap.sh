#!/usr/bin/env bash
# dev-bootstrap.sh — dev 任务统一前置（幂等启动批注服务器 + ego watcher）
# 供 .mise.toml meta / dev:{ns} 等任务调用；非阻塞（失败不阻断 dev 主流程）
set -euo pipefail
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# pnpm ≥12 原生二进制占位修复（mise/aube 安装后 bin 按 node 脚本执行——Mach-O 无法直接跑）
bash "${PROJECT_ROOT}/scripts/setup/fix-pnpm-native.sh" || echo '[fix-pnpm-native] 失败——pnpm 若报 SyntaxError 需手动跑该脚本'
bash "${PROJECT_ROOT}/scripts/feedback/ensure-server.sh" || echo '[feedback] server 未启动——批注上报不可用（dev 继续）'
bash "${PROJECT_ROOT}/scripts/ego/ensure-watcher.sh" || echo '[ego-watcher] 未启动——task space 自动回收不可用（dev 继续）'
