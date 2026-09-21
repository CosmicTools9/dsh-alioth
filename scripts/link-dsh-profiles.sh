#!/bin/bash
# link-dsh-profiles.sh — 把 profile 解析面需要的包链接进每个 DSH profile。
#
# 0.1.6 起 loader 以 profile 目录为基准解析每个 loader entry 的模块名（app-boot 把
# ctx.baseUrl 设为根配置所在目录），并用 Node 内部 cascaded loader 导入裸包名。
# 实测（见 PR/报告）：内部 loader 能解析指向包目录的**单层**符号链接，却解析不了
# 经 .dsh-module-fallback 中转的两层链（ERR_MODULE_NOT_FOUND）。因此这里直接把包
# 链进 <profile>/node_modules/<name>，让它们成为 profile 的本地依赖。
# 上一版把包链到 profiles 根 node_modules，0.1.6 的 loader 不再从那里解析，
# 组合会在启动时报告每个 entry "failed to import"。
#
# 两类包不在 launcher 闭包里，必须由本脚本补：
#   1. 本仓库的 @dsh-alioth/*
#   2. bundle 以 loader entry 名挂载、但无任何 harness 包依赖的 harness 包——
#      如 @deepseek-ai/dsh-page-feedback：harness 内无包依赖它，launcher 的
#      resolveModuleFallbackEntries 从 apps/cli 闭包出发永远到不了它，只有
#      bundle-alioth 的 patch 按名字挂载它（479f2d5 起）。
# launcher 的 heal 只写自己闭包内的条目、不清理手工链接，故本脚本幂等。
# 换机器 / 重装 dsh / 改动 bundle 组合 / 升级 harness 后需重新执行。
# 用法: bash scripts/link-dsh-profiles.sh
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
PROFILES="${DSH_HOME:-$HOME/.dsh}/profiles"
HARNESS="${DSH_HARNESS_ROOT:-$REPO/../deepseek-harness}"

if [ ! -d "$PROFILES" ]; then
  echo "MISSING profile root $PROFILES: run dsh once so it creates the profiles" >&2
  exit 1
fi

# <scope>/<name>=<target dir>，先收集完整集合，缺包即整个组合无法启动，直接失败。
TARGETS=()
for p in env-alioth tool-alioth tool-alioth-meta tool-alioth-workflow tool-alioth-orchestrator tool-alioth-verify gen-alioth skill-alioth verify-alioth guard-alioth bundle-alioth auth-alioth auth-web-alioth landing-alioth billing-alioth billing-web-alioth feedback-web-alioth tool-feedback-alioth app-picker; do
  TARGETS+=("@dsh-alioth/$p=$REPO/packages/alioth/$p")
done
# harness 包：bundle 以 loader entry 名挂载、但没有任何 harness 包依赖它（见文件头）。
PAGE_FEEDBACK="$HARNESS/packages/feedback/page-feedback"
if [ ! -f "$PAGE_FEEDBACK/package.json" ]; then
  echo "MISSING @deepseek-ai/dsh-page-feedback: no package at $PAGE_FEEDBACK (set DSH_HARNESS_ROOT)" >&2
  exit 1
fi
TARGETS+=("@deepseek-ai/dsh-page-feedback=$PAGE_FEEDBACK")

for profile in "$PROFILES"/*/; do
  [ -d "$profile" ] || continue
  name="$(basename "$profile")"
  # profiles/node_modules is the pre-0.1.6 shared fallback, not a profile.
  [ "$name" = node_modules ] && continue
  modules="$profile/node_modules"
  mkdir -p "$modules"
  for entry in "${TARGETS[@]}"; do
    spec="${entry%%=*}"
    target="${entry#*=}"
    scope="$(dirname "$spec")"
    mkdir -p "$modules/$scope"
    ln -sfn "$target" "$modules/$spec"
  done
  echo "linked $name: ${#TARGETS[@]} package(s)"
done
