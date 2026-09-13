#!/bin/bash
# binary-freshness.sh — dev 栈 standalone 二进制的新鲜度判定（单一实现）
#
# 判定目标：二进制是否**落后于它的构建输入**。输入域必须是完整闭包，不能只看
# `*.rs`——历史漏检过的三类：
#   1) 依赖/workspace 元数据：根 `Cargo.toml`/`Cargo.lock`、`Pre-Proc/{ns}/Cargo.{toml,lock}`
#      （Pre-Proc 是独立 workspace，自带 lock）；
#   2) `include_str!` 内嵌资源：`Gateway/backend/src/i18n/locales/*.json`、
#      `SSO/backend/migrations/*.sql`、`Framework/backend/*/locales/*.json`；
#   3) build.rs 读取的 `service.json`（不在 `backend/` 子树下，须单独纳入）。
#
# 用法（调用方 source 本文件后调用）：
#   source "${PROJECT_ROOT}/scripts/lib/binary-freshness.sh"
#   reason=$(binary_stale_reason "$BIN" \
#       "file:$ROOT/Cargo.toml" "file:$ROOT/Cargo.lock" \
#       "rust-tree:$ROOT/Gateway/backend" "rust-tree:$ROOT/SSO/backend" \
#       "rust-tree:$ROOT/Framework/backend" \
#       "ns-sources:$ROOT/Pre-Proc/$NS/Sources")
#   [ -n "$reason" ] && echo "旧二进制：$reason"
#
# spec 语法：
#   file:<path>        单文件比对（Cargo.toml / Cargo.lock / build.rs…）
#   rust-tree:<dir>    目录树，按白名单扩展名统计；prune
#                      target / node_modules / dist / .git / tests / benches / examples
#   ns-sources:<dir>   目录树，只统计 `<dir>/*/backend/**` 子树 + 任意 `service.json`
#                      （前端 TS/JSON 与 package.json 不触发后端重建）
#
# 输出：空 = 新鲜；非空 = 过期原因（首个命中的路径，或 `binary missing`）。
# 退出码恒 0——判定结果走 stdout，调用方按输出是否为空分支。

# 构建输入白名单扩展名（含非 Rust 的内嵌/元数据输入）
BINARY_FRESHNESS_EXTS=(
    '*.rs' '*.toml' '*.lock' '*.sql' '*.json' '*.yaml' '*.yml' '*.proto'
)

# prune 目录（构建产物 / 依赖 / 非构建输入）
BINARY_FRESHNESS_PRUNE=(
    target node_modules dist .git tests benches examples
)

# binary_stale_reason <binary> <spec>...
binary_stale_reason() {
    local bin="$1"; shift

    if [ ! -x "$bin" ]; then
        echo "binary missing"
        return 0
    fi

    # prune 目录（构建产物 / 依赖 / 非构建输入）——尾项不加 -o
    local prune=() first=1 p
    for p in "${BINARY_FRESHNESS_PRUNE[@]}"; do
        [ "$first" = 1 ] || prune+=(-o)
        prune+=(-name "$p")
        first=0
    done

    local exts=()
    first=1
    for p in "${BINARY_FRESHNESS_EXTS[@]}"; do
        [ "$first" = 1 ] || exts+=(-o)
        exts+=(-name "$p")
        first=0
    done

    for spec in "$@"; do
        case "$spec" in
            file:*)
                path="${spec#file:}"
                if [ -f "$path" ] && [ "$path" -nt "$bin" ]; then
                    echo "$path"
                    return 0
                fi
                ;;
            rust-tree:*)
                path="${spec#rust-tree:}"
                [ -d "$path" ] || continue
                hit=$(find "$path" \( "${prune[@]}" \) -prune -o \
                        -type f \( "${exts[@]}" \) -newer "$bin" -print -quit 2>/dev/null)
                if [ -n "$hit" ]; then
                    echo "$hit"
                    return 0
                fi
                ;;
            ns-sources:*)
                path="${spec#ns-sources:}"
                [ -d "$path" ] || continue
                hit=$(find "$path" \( "${prune[@]}" \) -prune -o \
                        -type f -newer "$bin" \
                        \( -path '*/backend/*' -o -name 'service.json' \) \
                        \( "${exts[@]}" \) -print -quit 2>/dev/null)
                if [ -n "$hit" ]; then
                    echo "$hit"
                    return 0
                fi
                ;;
            *)
                echo "unknown freshness spec: $spec" >&2
                return 0
                ;;
        esac
    done

    return 0
}

# ── 自检（bash scripts/lib/binary-freshness.sh --self-test）──────────────
# 覆盖：缺失 / 新鲜 / Rust 源过期 / **非 Rust 内嵌输入过期**（历史漏检）/
#       service.json 过期 / 前端 TS·JSON 不触发。
if [ "${BASH_SOURCE[0]}" = "$0" ] && [ "${1:-}" = "--self-test" ]; then
    set -u
    TMP=$(mktemp -d)
    trap 'rm -rf "$TMP"' EXIT
    fail=0

    mkdir -p "$TMP/tree/src/i18n/locales" \
             "$TMP/ns/Apps/Services/foo/backend/src" \
             "$TMP/ns/Apps/Modules/bar/frontend/src/locales"
    echo 'fn main() {}' > "$TMP/tree/src/a.rs"
    echo '{"k":"v"}'    > "$TMP/tree/src/i18n/locales/zh-CN.json"
    echo 'pub fn f() {}' > "$TMP/ns/Apps/Services/foo/backend/src/lib.rs"
    echo '{}'           > "$TMP/ns/Apps/Services/foo/service.json"
    echo '{"k":"v"}'    > "$TMP/ns/Apps/Modules/bar/frontend/src/locales/zh-CN.json"
    echo 'export const P = 1;' > "$TMP/ns/Apps/Modules/bar/frontend/src/Page.tsx"
    echo 'x' > "$TMP/Cargo.toml"

    SPECS=("file:$TMP/Cargo.toml" "rust-tree:$TMP/tree" "ns-sources:$TMP/ns")

    check() { # <name> <expected: empty|nonempty> <actual>
        local name="$1" want="$2" got="$3"
        if [ "$want" = "empty" ] && [ -z "$got" ]; then
            echo "  ✓ $name"
        elif [ "$want" = "nonempty" ] && [ -n "$got" ]; then
            echo "  ✓ $name → ${got#$TMP/}"
        else
            echo "  ✗ $name（期望 $want，实得 '${got:-<空>}'）"
            fail=1
        fi
    }

    # 1) 二进制缺失
    check "binary missing" nonempty "$(binary_stale_reason "$TMP/bin" "${SPECS[@]}")"

    # 2) 二进制比全部输入新 → 新鲜
    BIN="$TMP/bin"; : > "$BIN"; chmod +x "$BIN"
    sleep 1
    touch "$BIN"
    sleep 1
    check "all inputs older → fresh" empty "$(binary_stale_reason "$BIN" "${SPECS[@]}")"

    # 3) Rust 源过期
    touch "$TMP/tree/src/a.rs"
    check "rust source newer → stale" nonempty "$(binary_stale_reason "$BIN" "${SPECS[@]}")"

    # 4) 只有内嵌 i18n JSON 过期（旧逻辑只看 *.rs → 漏检）
    touch "$BIN"; sleep 1; touch "$TMP/tree/src/i18n/locales/zh-CN.json"
    check "embedded locale json newer → stale" nonempty "$(binary_stale_reason "$BIN" "${SPECS[@]}")"

    # 5) 只有 service.json 过期（不在 backend/ 子树）
    touch "$BIN"; sleep 1; touch "$TMP/ns/Apps/Services/foo/service.json"
    check "service.json newer → stale" nonempty "$(binary_stale_reason "$BIN" "${SPECS[@]}")"

    # 6) 只有前端 TS / 前端 locale JSON 过期 → 新鲜（不得误触发）
    touch "$BIN"; sleep 1
    touch "$TMP/ns/Apps/Modules/bar/frontend/src/Page.tsx" \
          "$TMP/ns/Apps/Modules/bar/frontend/src/locales/zh-CN.json"
    check "frontend-only changes → fresh" empty "$(binary_stale_reason "$BIN" "${SPECS[@]}")"

    # 7) Cargo.toml（file spec）过期
    touch "$BIN"; sleep 1; touch "$TMP/Cargo.toml"
    check "cargo manifest newer → stale" nonempty "$(binary_stale_reason "$BIN" "${SPECS[@]}")"

    [ "$fail" = 0 ] && echo "binary-freshness self-test: PASS" || echo "binary-freshness self-test: FAIL"
    exit "$fail"
fi
