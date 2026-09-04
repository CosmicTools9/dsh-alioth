#!/bin/bash
# link-dsh-profiles.sh — 把本仓库的 @dsh-alioth/* 包链接进 DSH 的 profile fallback
# (~/.dsh/profiles/node_modules/@dsh-alioth/)。dsh loader 从 profile 目录解析插件，
# fallback 目录由 launcher 依赖闭包维护（不含本仓库包）；手动链接幂等（heal 不清除）。
# 换机器/重装 dsh 后需重新执行。用法: bash scripts/link-dsh-profiles.sh
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
FALLBACK="$HOME/.dsh/profiles/node_modules/@dsh-alioth"
mkdir -p "$FALLBACK"
for p in env-alioth tool-alioth tool-alioth-meta tool-alioth-workflow tool-alioth-orchestrator gen-alioth skill-alioth bundle-alioth auth-alioth auth-web-alioth landing-alioth billing-alioth billing-web-alioth feedback-alioth feedback-web-alioth tool-feedback-alioth app-picker; do
  ln -sfn "$REPO/packages/alioth/$p" "$FALLBACK/$p"
  echo "linked $p"
done
