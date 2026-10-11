#!/usr/bin/env bash
# mise 项目配置信任状态判定（只读探针，零副作用）
#
# 单一来源：三处消费方 MUST 共用本函数，禁止各自内联同判据——
#   scripts/codegraph/setup-codegraph.sh §1「Node.js 环境一致化」
#   scripts/env/verify-env.sh            「附加检查」
#   scripts/setup/env-orchestrator.sh    eo_probe_mise_trust
#
# 用法: mise_trust_state <dir>
#   stdout: trusted | untrusted | unknown
#   rc:     恒 0（状态经 stdout 交付，ok/gap/warn 映射归调用方）
#
# 判据 = `mise -C <dir> trust --show` 的输出行（只读；形态 `<dir>: trusted` /
# `<dir>/.mise.toml: untrusted`，HOME 下的路径显示为 `~/…`）。
#
# MUST NOT 用 `mise trust --check`：本仓 mise（2026.9.x）无该标志，实测恒 rc≠0
#   （`error: unexpected argument '--check' found`），被 `&>/dev/null` 吞掉后会把已信任恒报为未信任。
# MUST NOT 改用行为式探针（`mise ls <tool>` / `mise env` 的退出码）：实测 paranoid 模式下未信任配置
#   仍 rc=0（配置被静默跳过）⇒ 假阴性，读不出未信任。
mise_trust_state() {
    local dir="${1:?mise_trust_state 需要项目目录}"
    command -v mise &>/dev/null || { echo unknown; return 0; }
    [ -f "${dir}/.mise.toml" ] || { echo unknown; return 0; }

    local out=""
    out="$(mise -C "$dir" trust --show 2>/dev/null || true)"
    [ -n "$out" ] || { echo unknown; return 0; }

    # `~/…` 显示形态 → 归一为绝对路径
    # （MUST NOT 反向用 `${dir/#$HOME/~}` 造 `~`：实测本机 bash 对替换串为 `~` 时结果与原串相同 ⇒ 恒不匹配）
    local home_dir="${HOME:-}"
    if [ -n "$home_dir" ]; then
        out="$(printf '%s\n' "$out" | sed "s|^~/|${home_dir}/|")"
    fi

    if printf '%s\n' "$out" | grep -qF "${dir}: trusted" \
        || printf '%s\n' "$out" | grep -qF "${dir}/.mise.toml: trusted"; then
        echo trusted
    elif printf '%s\n' "$out" | grep -qF "${dir}: untrusted" \
        || printf '%s\n' "$out" | grep -qF "${dir}/.mise.toml: untrusted"; then
        echo untrusted
    else
        echo unknown
    fi
}
