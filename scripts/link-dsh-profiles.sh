#!/bin/bash
# link-dsh-profiles.sh — 把 profile 解析面需要的包链接进 DSH 的 profile fallback
# (~/.dsh/profiles/node_modules/)。dsh loader 从 profile 目录解析每个 loader entry
# 的模块名，fallback 目录由 launcher 的安装依赖闭包维护，两类包不在其中：
#   1. 本仓库的 @dsh-alioth/*（launcher 闭包只走 harness 自己的包）
#   2. bundle 以 loader entry 名挂载、但没有任何 harness 包依赖它的 harness 包——
#      如 @deepseek-ai/dsh-page-feedback：harness 内无包依赖它，launcher 的
#      resolveModuleFallbackEntries 从 apps/cli 闭包出发永远到不了它，只有
#      bundle-alioth 的 patch 按名字挂载它（479f2d5 起）。
# launcher 的 heal 只写自己闭包内的条目、不清理手工链接，故本脚本幂等。
# 换机器 / 重装 dsh / 改动 bundle 组合后需重新执行。
# 用法: bash scripts/link-dsh-profiles.sh
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
FALLBACK="$HOME/.dsh/profiles/node_modules"
HARNESS="${DSH_HARNESS_ROOT:-$REPO/../deepseek-harness}"

mkdir -p "$FALLBACK/@dsh-alioth"
for p in env-alioth tool-alioth tool-alioth-meta tool-alioth-workflow tool-alioth-orchestrator gen-alioth skill-alioth bundle-alioth auth-alioth auth-web-alioth landing-alioth billing-alioth billing-web-alioth feedback-web-alioth tool-feedback-alioth app-picker; do
  ln -sfn "$REPO/packages/alioth/$p" "$FALLBACK/@dsh-alioth/$p"
  echo "linked @dsh-alioth/$p"
done

# harness 包（<包名>=<harness 仓库内相对路径>）。缺包即整个组合无法启动，直接失败。
HARNESS_PACKAGES=(
  'dsh-page-feedback=packages/feedback/page-feedback'
)
mkdir -p "$FALLBACK/@deepseek-ai"
for entry in "${HARNESS_PACKAGES[@]}"; do
  name="${entry%%=*}"
  target="$HARNESS/${entry#*=}"
  if [ ! -f "$target/package.json" ]; then
    echo "MISSING @deepseek-ai/$name: no package at $target (set DSH_HARNESS_ROOT)" >&2
    exit 1
  fi
  ln -sfn "$target" "$FALLBACK/@deepseek-ai/$name"
  echo "linked @deepseek-ai/$name"
done
