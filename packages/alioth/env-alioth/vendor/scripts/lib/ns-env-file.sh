#!/bin/bash
# ns-env-file.sh — namespace 环境文件分层解析（**唯一实现**）
#
# 为什么单独成 lib：env 文件此前按「dev = Deploy/{ns}/.env」硬编码在各启动/发布脚本里；
# 引入 pre 层后若各脚本自行拼 `.env.pre`，会立刻形成第二套约定（且极易漏改 ⇒ pre 启动读到
# dev env ⇒ 用 dev 库跑 pre 环境）。
#
# 用法（source 后调用）：
#   source scripts/lib/ns-env-file.sh
#   ns_env_file <ns> [tier]        # Gateway/ns 面 env：dev → .env      | pre → .env.pre
#   ns_open_env_file <ns> [tier]   # 门户面 env：      dev → Open/.env | pre → Open/.env.pre
#
# 契约：
#   · tier 缺省 dev；仅接受 dev|pre，未知 tier → rc=1（MUST NOT 静默默认为 dev）；
#   · 目标文件 MUST 存在（-f）；不存在 → rc=1 **且 MUST NOT 回落另一 tier 的文件**
#     （回落 = 用 dev 库跑 pre 环境，是本 lib 要消灭的隐式降级）。

if [[ -z "${PROJECT_ROOT:-}" ]]; then
    PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." 2>/dev/null && pwd)"
fi

# _ns_env_tier_suffix <tier> —— tier → 文件后缀（dev 空 / pre .pre）；未知 → rc=1
_ns_env_tier_suffix() {
    case "${1:-dev}" in
        dev) printf '' ;;
        pre) printf '.pre' ;;
        *)
            echo "ns-env-file: 未知环境层 '${1}'（仅 dev|pre）" >&2
            return 1
            ;;
    esac
}

# ns_env_file_in <dir> <tier> —— 「目录 + 层」→ env 文件绝对/给定路径（后缀规则**单一实现** = _ns_env_tier_suffix）
# 用途：消费方持有的是**发布包目录**（如容器的 `--pkg`）而非 `Deploy/{ns}` 的 ns 名时，MUST 走本函数取路径，
# MUST NOT 自行拼 `.env.pre`（拼接副本 = 本 lib 要消灭的第二套约定）。
# 退出：路径存在 → 打印并 rc=0；不存在 → rc=1 且 MUST NOT 回落另一层。
ns_env_file_in() {
    local dir="${1:-}" tier="${2:-dev}" suffix f
    [[ -n "$dir" ]] || { echo "ns-env-file: 缺少目录" >&2; return 1; }
    suffix="$(_ns_env_tier_suffix "$tier")" || return 1
    f="${dir%/}/.env${suffix}"
    [[ -f "$f" ]] || return 1
    printf '%s\n' "$f"
}

# ns_env_file <ns> [tier]
ns_env_file() {
    local ns="${1:-}" tier="${2:-dev}" suffix f
    [[ -n "$ns" ]] || { echo "ns-env-file: 缺少 namespace" >&2; return 1; }
    suffix="$(_ns_env_tier_suffix "$tier")" || return 1
    f="${PROJECT_ROOT}/Deploy/${ns}/.env${suffix}"
    [[ -f "$f" ]] || {
        echo "ns-env-file: ${tier} 层 env 缺失：${f}（MUST NOT 回落到其他层级的 env）" >&2
        return 1
    }
    printf '%s' "$f"
}

# ns_open_env_file <ns> [tier]
ns_open_env_file() {
    local ns="${1:-}" tier="${2:-dev}" suffix f
    [[ -n "$ns" ]] || { echo "ns-env-file: 缺少 namespace" >&2; return 1; }
    suffix="$(_ns_env_tier_suffix "$tier")" || return 1
    f="${PROJECT_ROOT}/Deploy/${ns}/Open/.env${suffix}"
    [[ -f "$f" ]] || {
        echo "ns-env-file: ${tier} 层门户 env 缺失：${f}（MUST NOT 回落到其他层级的 env）" >&2
        return 1
    }
    printf '%s' "$f"
}

# 自检：tier 归一与未知层拒绝（无文件依赖）
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    set -euo pipefail
    [[ "$(_ns_env_tier_suffix dev)" == "" ]] || { echo "自检失败: dev 后缀非空"; exit 1; }
    [[ "$(_ns_env_tier_suffix pre)" == ".pre" ]] || { echo "自检失败: pre 后缀"; exit 1; }
    if _ns_env_tier_suffix staging >/dev/null 2>&1; then
        echo "自检失败: 未知 tier 未被拒绝"; exit 1
    fi
    echo "✓ ns-env-file.sh 自检通过"
fi
