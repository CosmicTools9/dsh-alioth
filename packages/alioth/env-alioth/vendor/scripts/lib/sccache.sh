#!/usr/bin/env bash
# sccache.sh — 编译缓存（sccache）接线的**唯一实现**与声明文案的唯一产生点
#
# 规约: openspec/specs/compilation/spec.md
#        · Requirement `build-cache-wired-at-every-entrypoint`（本文件是「唯一实现」）
#        · Requirement `build-cache-claims-match-wiring`（状态声明必须与事实一致）
#   门禁: scripts/check/check-sccache-wiring.ts（编译入口 MUST 经此接线或登记豁免）
#
# 用法:
#   source scripts/lib/sccache.sh
#   sccache_enable_if_available "<label>"    # 幂等（同进程只报告一次）
#   sccache_stats                            # 打印 `sccache --show-stats`
#   bash scripts/lib/sccache.sh [stats]      # CLI 面：无参=自检；stats=统计
#
# 判定顺序与语义（四条分支）:
#   ① ALIOTH_SCCACHE=0|off     → 显式停用（报告；MUST NOT 改动 RUSTC_WRAPPER）
#   ② RUSTC_WRAPPER 已由环境设 → 尊重既有选择（不覆盖他人 wrapper）
#   ③ 探测到 sccache 可执行     → 注入 RUSTC_WRAPPER + 缓存面环境
#   ④ 未安装                  → 报告未接线与启用方式，返回 0（fail-soft：缓存降级 ≠ 构建降级）
#
# 为什么 SCCACHE_BASEDIR = 仓库根：本仓编译发生在多个互不相同的绝对 cwd（`Gateway/backend`、
# `Meta/backend`、`Deploy/{ns}`、`/tmp/alioth-*`）；sccache 默认把绝对路径纳入 hash ⇒ 同一份源码
# 在不同 target 目录下无法命中。设置 BASEDIR 后 base 内路径归一为相对路径，使**跨 target 目录 /
# 跨 namespace / 跨 worktree** 的同一编译单元命中同一缓存项（这是本接线的核心收益）。
#
# MUST NOT 在本文件之外硬编码「sccache: 已启用」类声明——文案只能由本文件产生（门禁审计此点）。

# 幂等哨兵：同进程内只报告一次（多个 lib/入口互相 source 时不重复刷屏）
_sccache_reported="${_sccache_reported:-0}"

# _sccache_emit <文案> —— 状态行统一出口（stderr，避免污染命令替换的 stdout）
_sccache_emit() {
    printf '  sccache: %s\n' "$1" >&2
}

# sccache_enable_if_available [label] —— 启用二级编译缓存（幂等、fail-soft）
sccache_enable_if_available() {
    local label="${1:-}" suffix=""
    [ -n "${label}" ] && suffix="（${label}）"
    [ "${_sccache_reported}" = "1" ] && return 0

    case "${ALIOTH_SCCACHE:-}" in
        0|off|false|no)
            _sccache_reported=1
            _sccache_emit "已停用（ALIOTH_SCCACHE=${ALIOTH_SCCACHE}）${suffix}"
            return 0
            ;;
    esac

    if [ -n "${RUSTC_WRAPPER:-}" ]; then
        _sccache_reported=1
        if command -v "${RUSTC_WRAPPER}" >/dev/null 2>&1; then
            _sccache_emit "已启用（RUSTC_WRAPPER=${RUSTC_WRAPPER}，环境预设）${suffix}"
        else
            _sccache_emit "未生效（RUSTC_WRAPPER=${RUSTC_WRAPPER} 不可执行）${suffix}"
        fi
        return 0
    fi

    if ! command -v sccache >/dev/null 2>&1; then
        _sccache_reported=1
        _sccache_emit "未接线（未安装 sccache）${suffix}——启用方式: brew install sccache"
        return 0
    fi

    # 仓库根 = 本文件所在 lib 目录的上两级（与 scripts/lib/cargo-run.sh 同法推导）
    local root
    root="$(cd "${BASH_SOURCE[0]%/*}/../.." 2>/dev/null && pwd)" || root=""
    export RUSTC_WRAPPER="sccache"
    [ -n "${root}" ] && export SCCACHE_BASEDIR="${SCCACHE_BASEDIR:-${root}}"
    export SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-20G}"
    # 缓存面 IO 异常不得升级为构建失败（server 不可写/网络盘抖动时降级为直连编译）
    export SCCACHE_IGNORE_SERVER_IO_ERROR="${SCCACHE_IGNORE_SERVER_IO_ERROR:-1}"
    _sccache_reported=1
    _sccache_emit "已启用（RUSTC_WRAPPER=sccache, cache=${SCCACHE_CACHE_SIZE}, base=${SCCACHE_BASEDIR:-未设}）${suffix}"
}

# sccache_stats —— 打印命中统计（缺失时说明未安装，不报错）
sccache_stats() {
    if ! command -v sccache >/dev/null 2>&1; then
        _sccache_emit "未接线（未安装 sccache）——无法查询统计"
        return 1
    fi
    sccache --show-stats
}

# ── CLI / 自检 ──────────────────────────────────────────────────────────────
# 直接执行（非 source）时：`stats` = 统计；其它/无参 = 自检四条分支。
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    if [ "${1:-}" = "stats" ]; then
        sccache_stats
        exit $?
    fi

    _st_fail=0
    _st_ok() { echo "  ok: $1"; }
    _st_bad() { echo "  FAIL: $1"; _st_fail=1; }

    # 每条分支在**子 shell** 内断言（`( )` 在同一进程分支，函数已定义——MUST NOT 在子 shell 内
    # 再 source 本文件：`$0` 与 `BASH_SOURCE[0]` 相同会使下面的入口判断再次成立 ⇒ 指数级递归）。
    # shellcheck disable=SC2030,SC2031
    echo "sccache 接线自检:"
    if (
        _sccache_reported=0
        sccache_enable_if_available "自检" 2>/dev/null
        [ "${RUSTC_WRAPPER:-}" = "sccache" ] && [ -n "${SCCACHE_BASEDIR:-}" ]
    ); then _st_ok "可用时注入 RUSTC_WRAPPER=sccache 与 SCCACHE_BASEDIR"; else _st_bad "未注入（sccache 不可用？）"; fi
    if (
        _sccache_reported=0
        ALIOTH_SCCACHE=0
        sccache_enable_if_available "自检" 2>/dev/null
        [ -z "${RUSTC_WRAPPER:-}" ]
    ); then _st_ok "ALIOTH_SCCACHE=0 显式停用（不注入）"; else _st_bad "显式停用仍注入了 wrapper"; fi
    if (
        _sccache_reported=0
        RUSTC_WRAPPER=/bin/echo
        sccache_enable_if_available "自检" 2>/dev/null
        [ "${RUSTC_WRAPPER}" = "/bin/echo" ]
    ); then _st_ok "既有 RUSTC_WRAPPER 不被覆盖"; else _st_bad "覆盖了既有 wrapper"; fi
    if (
        _sccache_reported=0
        PATH=/usr/bin:/bin
        sccache_enable_if_available "自检" 2>/dev/null && [ -z "${RUSTC_WRAPPER:-}" ]
    ); then _st_ok "未安装时 fail-soft（rc=0 且不注入）"; else _st_bad "未安装分支非 fail-soft"; fi

    [ "${_st_fail}" = "0" ] && echo "自检全部通过" || { echo "自检存在失败"; exit 1; }
fi
