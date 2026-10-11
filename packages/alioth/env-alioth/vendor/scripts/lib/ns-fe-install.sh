#!/usr/bin/env bash
# ns-fe-install.sh — ns 前端**自持安装面**的 shell 自愈单一实现（dev 启动链 / 前端构建链共用）
#
# 契约（ADR D-046 修订 2026-10-05 / change move-ns-frontend-assembly-to-ns；活规约
# `openspec/specs/dev-env-dep-selfheal` 的 `ns-frontend-deps-selfheal-at-dev-start` /
# `-before-frontend-build` 两条 Requirement 的**唯一实现**）：
#   · 通道 = ns 仓自持安装面：生成器 `scripts/ns/ns-frontend-workspace.ts sync --write` 写 **ns 仓根**
#     `pnpm-workspace.yaml`（成员 = App 声明面 ∩ 磁盘前端），依赖由 **ns 仓根** `pnpm install` 供给
#     （平台包以 `link:` 指向平台源码；单例族 overrides 指向平台实体副本 ⇒ 两 store 单副本）。
#   · 「就位」判据 = ns 仓根 `node_modules/.pnpm` 在场 **且** `sync --check` 无漂移；否则自愈（重生成 + install）。
#   · 幂等：已就位时零安装动作；失败 MUST fail-loud（返回非零，调用方 exit 1，MUST NOT 静默沿用过期依赖）。
#   · 平台仓 `pnpm-workspace.yaml`/`pnpm-lock.yaml` MUST NOT 记录 ns 成员（反向依赖禁止；
#     判据 `platform-ns-dependency-direction`）——本函数只写 ns 仓目录。
#
# 用法（调用方需已 export PROJECT_ROOT；返回值即结论）：
#   source scripts/lib/ns-fe-install.sh
#   ns_fe_surface_ensure <NS> [<project_root>] || exit 1
ns_fe_surface_ensure() {
    local ns="${1:-}"
    local root="${2:-${PROJECT_ROOT:-}}"
    if [ -z "$ns" ] || [ -z "$root" ]; then
        echo "[ns-fe] 用法: ns_fe_surface_ensure <NS> <project_root>" >&2
        return 2
    fi
    local gen="${root}/scripts/ns/ns-frontend-workspace.ts"
    local mount="${root}/Pre-Proc/${ns}"
    [ -d "$mount" ] || return 0            # 未挂载（平台内置实体 ns 目录等）⇒ 不适用本通道
    if [ ! -f "$gen" ]; then
        echo "[ns-fe] ❌ 生成器缺失：${gen}（ns 自持安装面通道未接入平台面）" >&2
        return 1
    fi
    # 物理化：bash `cd` 默认逻辑路径 ⇒ MUST `-P`（BSD 无 readlink -f）
    local repo
    repo="$(cd -P "${mount}/../.." && pwd -P)" || {
        echo "[ns-fe] ❌ ns 仓根解析失败：${mount}" >&2
        return 1
    }
    local ready=1
    [ -d "${repo}/node_modules/.pnpm" ] || ready=0
    if ! bun "$gen" sync --check >/dev/null 2>&1; then
        ready=0
        if ! bun "$gen" sync --write >/dev/null 2>&1; then
            echo "[ns-fe] ❌ ns 前端安装面重生成失败：bun scripts/ns/ns-frontend-workspace.ts sync --write" >&2
            return 1
        fi
    fi
    if [ "$ready" = 1 ]; then
        return 0
    fi
    echo "[ns-fe] ns 前端安装面缺失/漂移（${repo}）— pnpm install ..." >&2
    if ! (cd "$repo" && pnpm install --no-frozen-lockfile); then
        echo "[ns-fe] ❌ ns 仓根 pnpm install 失败（${repo}）——修复后重试" >&2
        return 1
    fi
    echo "[ns-fe] ✓ ns 前端依赖就位（${repo}）" >&2
    return 0
}
