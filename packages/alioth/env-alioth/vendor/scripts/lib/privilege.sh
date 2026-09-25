#!/usr/bin/env bash
# =============================================================================
# privilege.sh — 「能否执行特权安装」判据的唯一实现
#
# 消费方（禁止各写一份）：`scripts/setup.sh`、`scripts/setup/install-dev-tools.sh`
#
# 为什么需要（2026-09-22 实证，全新 Linux 主机）：
#   交互式 sudo 在**非 TTY** 的脚本里必然失败（`sudo: a terminal is required to read
#   the password`），而 `set -e`（+ pipefail）会把这次失败升级成「整轮安装终止」——
#   用户拿到被截断的日志，而非「哪些装上了、哪些没装上」。故安装脚本 MUST 先用本判据
#   探明「特权是否真的可用」，再决定走系统包管理器还是无 root 通道（brew）。
#
# 用法（source 型）:
#   source scripts/lib/privilege.sh
#   if can_privileged_install; then SP="$(priv_prefix)"; $SP apt-get install -y <pkg>; fi
# =============================================================================

# can_privileged_install → 0 = root 或**免密** sudo 可用（可执行特权安装）
can_privileged_install() {
    [ "$(id -u)" -eq 0 ] && return 0
    command -v sudo >/dev/null 2>&1 && sudo -n true >/dev/null 2>&1
}

# priv_prefix → 特权命令前缀（root ⇒ 空串；否则 `sudo`）。仅在 can_privileged_install 通过后调用。
priv_prefix() {
    [ "$(id -u)" -eq 0 ] && return 0
    printf '%s' 'sudo'
}
