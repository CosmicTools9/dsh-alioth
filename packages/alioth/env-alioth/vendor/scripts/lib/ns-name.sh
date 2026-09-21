#!/bin/bash
# ns-name.sh — namespace 名归一的唯一实现
#
# namespace 名在不同用途下需要两种形态，历史上各脚本自行转换，导致同一字面值
# 在不同调用点解析出不同目录（例如 `Deploy/alioth/target` 与 `Deploy/Alioth/target`
# 并存，编译隔离被静默破坏）：
#   - 规范目录名：Alioth / WZ / AVIC-CAASEC / SE / Cosmic-Tools（`Deploy/{ns}`、`Pre-Proc/{ns}`）
#   - cargo feature 名：全小写（alioth / wz / avic-caasec / se / cosmic-tools）
#
# 用法（调用方 source 本文件后使用）：
#   ns_canon alioth       # → Alioth
#   ns_canon WZ           # → WZ
#   ns_lower Alioth       # → alioth
#
# 未登记的名字原样返回，交由调用方既有的报错分支处理（本库不判合法性）。
# 实现只用参数扩展与 tr：MUST NOT 使用 `${var,,}`/`${var^^}`（macOS bash 3.2 报
# `bad substitution`，同缺陷已在 restart-gateway.sh 实测触发）。

# ns_canon <name> —— 输出规范 namespace 名（大小写不敏感）
ns_canon() {
    local name="${1:-}" key
    key="$(printf '%s' "$name" | tr '[:lower:]' '[:upper:]')"
    case "$key" in
        ALIOTH) printf 'Alioth' ;;
        WZ) printf 'WZ' ;;
        AVIC-CAASEC) printf 'AVIC-CAASEC' ;;
        SE) printf 'SE' ;;
        COSMIC-TOOLS) printf 'Cosmic-Tools' ;;
        *) printf '%s' "$name" ;;
    esac
}

# ns_lower <name> —— 输出 cargo feature 名（全小写）
ns_lower() {
    printf '%s' "${1:-}" | tr '[:upper:]' '[:lower:]'
}
