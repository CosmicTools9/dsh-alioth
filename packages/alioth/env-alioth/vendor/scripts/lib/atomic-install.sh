#!/bin/bash
# atomic-install.sh — 可执行文件落盘的唯一实现（同名替换不打断运行中实例）
#
# 背景：直接 `cp <src> <dst>` 覆盖**正被运行中实例执行**的文件会 ETXTBSY
# （`cp: 无法创建普通文件 '<dst>': 文本文件忙`）——内核禁止对已建立可执行映像的
# 文件开写入态。落点目录含运行中实例（`Deploy/{ns}/bin/`、发布目标目录）时构建/发布
# 会整体失败，而停服既非必要（老进程持有旧 inode 即可继续跑）也非本类脚本的职责。
#
# 实现 = 「同目录临时文件 + rename」：
#   1) 临时文件与目标同目录（mktemp）⇒ rename 落在同一文件系统，是原子的目录项替换；
#   2) 运行中实例继续持有旧 inode 直至其退出，无需停服、不受影响；
#   3) 目标路径任一时刻只指向完整文件（可执行位置已在 rename 前置好）⇒ 无半成品窗口。
#
# 用法（调用方 source 本文件后使用）：
#   source "${PROJECT_ROOT}/scripts/lib/atomic-install.sh"
#   install_executable_atomic "$SRC" "$DST"
#
# 前置：目标目录已存在（调用方 mkdir -p）。失败时返回非 0 并清理临时文件。
# 自检：bash scripts/lib/atomic-install.sh --self-test

# install_executable_atomic <src> <dst>
install_executable_atomic() {
    local src="$1" dst="$2" tmp
    tmp="$(mktemp "${dst}.XXXXXX")" || {
        echo "❌ install_executable_atomic: 无法在 $(dirname "$dst") 创建临时文件" >&2
        return 1
    }
    if ! cp "$src" "$tmp"; then
        rm -f "$tmp"
        echo "❌ install_executable_atomic: 复制失败 ${src} → ${dst}" >&2
        return 1
    fi
    # 可执行位在 rename 之前置好：目标路径永远指向完整且可执行的文件
    chmod +x "$tmp"
    mv -f "$tmp" "$dst"
}

# ── 自检（bash scripts/lib/atomic-install.sh --self-test）──────────────────
# 覆盖：前提复现（直接 cp 运行中的可执行文件 → ETXTBSY）/ 原子安装成功 /
#      运行中实例不被打断 / 目标内容确实换新。
if [ "${BASH_SOURCE[0]}" = "$0" ] && [ "${1:-}" = "--self-test" ]; then
    set -uo pipefail
    tmpd="$(mktemp -d)"
    trap 'rm -rf "$tmpd"' EXIT
    fails=0

    # MUST NOT 用 `command -v`：bash 的 sleep/true 可能解析为同名内建，非可执行文件路径。
    sleep_bin="$(type -P sleep)" || { echo "✗ 缺外部命令 sleep"; exit 1; }
    true_bin="$(type -P true)" || { echo "✗ 缺外部命令 true"; exit 1; }

    # 前提探测 MUST 用**独立副本 + 独立进程**：探测本身就是「对运行中可执行文件就地写入」，
    # 在允许该写入的平台上会致旧映像失效/进程被内核终止（macOS 实测 ``Killed: 9``）。
    # 若与正式用例共用 runner，探测会把正式用例的进程杀掉 ⇒ 后续 `kill -0` 断言必然假失败
    # （Linux 上探测报 ETXTBSY、进程不受影响，才掩盖了这一结构性缺陷）。
    cp "$sleep_bin" "$tmpd/probe"
    "$tmpd/probe" 30 &
    probe_pid=$!
    sleep 0.5   # 等 exec 完成（映像建立后写入才触发 ETXTBSY）

    etxtbsy_enforced=false
    if cp "$true_bin" "$tmpd/probe" 2>/dev/null; then
        echo "⚠ 前提未复现：本平台允许写入运行中的可执行文件（ETXTBSY 未强制）"
    else
        echo "✓ 前提复现：直接 cp 覆盖运行中的可执行文件被拒（ETXTBSY）"
        etxtbsy_enforced=true
    fi
    kill "$probe_pid" 2>/dev/null || true
    wait "$probe_pid" 2>/dev/null || true

    # 正式用例：全新副本 + 全新进程（与探测互不影响）
    cp "$sleep_bin" "$tmpd/runner"
    "$tmpd/runner" 30 &
    runner_pid=$!
    sleep 0.5

    if install_executable_atomic "$true_bin" "$tmpd/runner"; then
        echo "✓ 原子安装成功（落盘无 ETXTBSY 失败、无半成品窗口）"
    else
        echo "✗ 原子安装失败"
        fails=$((fails + 1))
    fi

    if [ "$etxtbsy_enforced" = true ]; then
        if kill -0 "$runner_pid" 2>/dev/null; then
            echo "✓ 运行中实例存活（旧 inode 未被替换）"
        else
            echo "✗ 运行中实例被中断"
            fails=$((fails + 1))
        fi
    else
        # 前提未复现（本平台允许对运行中可执行文件就地写入）⇒「rename 保活」这一契约的
        # 前提不成立，断言在此平台上无定义（且探测本身会终止旧映像）。此处显式标注未断言，
        # MUST NOT 记为失败——避免把「平台模型不同」伪装成「实现回归」。
        echo "⚠ 跳过「运行中实例存活」断言：本平台不强制 ETXTBSY，该断言的前提未复现"
    fi

    if cmp -s "$tmpd/runner" "$true_bin"; then
        echo "✓ 目标路径内容已换新"
    else
        echo "✗ 目标路径内容未换新"
        fails=$((fails + 1))
    fi

    if [ -x "$tmpd/runner" ]; then
        echo "✓ 目标可执行位就绪"
    else
        echo "✗ 目标缺可执行位"
        fails=$((fails + 1))
    fi

    kill "$runner_pid" 2>/dev/null || true
    wait "$runner_pid" 2>/dev/null || true

    if [ "$fails" -eq 0 ]; then
        echo "✅ atomic-install self-test passed"
        exit 0
    fi
    echo "❌ atomic-install self-test failed: ${fails} 项"
    exit 1
fi
