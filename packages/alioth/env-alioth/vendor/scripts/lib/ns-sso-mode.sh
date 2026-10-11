#!/usr/bin/env bash
# ns-sso-mode.sh — Namespace SSO 形态读取器（声明正本 = 各 ns 仓 .alioth/platform.conf 的 SSO_MODE）
#
# 契约: openspec change move-ns-registry-to-ns-repos（D1:A 声明载体 / D2:A 派生缓存 / D3:A 函数内切源）
#       openspec/specs/sso-remote-auth-proxy/spec.md :: per-namespace-sso-mode-contract
#   门禁: scripts/check/check-sso-mode-conf.ts（派生缓存 ⇔ 声明面一致 / 结构 / env 镜像 / remote 专属）
#
# 用法（调用方需先设 PROJECT_ROOT，或当前目录为仓库根）:
#   source scripts/lib/ns-sso-mode.sh
#   ns_sso_mode WZ            # → remote；未声明 ns → rc=1（MUST NOT 静默返回默认）
#   ns_sso_feature WZ         # → sso-remote（embedded → sso）
#   ns_sso_modes_list         # 枚举 "ns mode" 行（供审计工具消费）
#   ns_sso_mode_check WZ remote
#   bash scripts/lib/ns-sso-mode.sh   # 自检
#
# 读取通道:
#   默认   —— ns 仓声明面（ns_repo_root 定位 → subshell source 取 SSO_MODE，无正则解析）
#   覆写   —— NS_SSO_MODE_CONF 指向旧 2 列表格式（ns mode）时走旧解析（测试/隔离 worktree 钩子，
#             语义 = 表缺失/未登记/非法值 rc=1 不变）
#
# 为什么未声明 ns 必须失败：形态是部署契约的一部分，静默默认会把 remote 的 ns
# 降级为 embedded（本变更要消灭的隐式推断正是此形态的静默降级）。

if [ -z "${PROJECT_ROOT:-}" ]; then
  PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." 2>/dev/null && pwd)"
fi

_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -f "${_LIB_DIR}/ns-repo.sh" ]; then
  # shellcheck source=ns-repo.sh
  source "${_LIB_DIR}/ns-repo.sh"
fi
if ! command -v ns_repo_root >/dev/null 2>&1; then
  ns_repo_root() { echo "[sso-mode] scripts/lib/ns-repo.sh 缺失——无法定位 ns 声明面" >&2; return 1; }
fi

# ns_sso_mode_file —— 覆写表路径（仅 NS_SSO_MODE_CONF 显式设置时有意义；测试/隔离用）
ns_sso_mode_file() {
  echo "${NS_SSO_MODE_CONF:-${PROJECT_ROOT:-.}/Deploy/sso-mode.conf}"
}

# _ns_sso_mode_from_table <ns> —— 旧 2 列表格式解析（NS_SSO_MODE_CONF 钩子通道）
_ns_sso_mode_from_table() {
  local ns="$1" n mode found=""
  [ -f "$(ns_sso_mode_file)" ] || {
    echo "[sso-mode] 形态表缺失：$(ns_sso_mode_file)" >&2
    return 1
  }
  while read -r n mode; do
    case "$n" in ''|\#*) continue ;; esac
    [ "$n" = "$ns" ] || continue
    found="$mode"
    break
  done < "$(ns_sso_mode_file)"
  if [ -z "$found" ]; then
    echo "[sso-mode] namespace '${ns}' 未在 $(ns_sso_mode_file) 登记——MUST 显式登记（不做默认推断）" >&2
    return 1
  fi
  printf '%s\n' "$found"
}

# ns_sso_mode <ns> —— 打印形态（embedded|remote）；未声明/声明面不可达/形态非法 → rc=1
ns_sso_mode() {
  local ns="$1" mode=""
  if [ -n "${NS_SSO_MODE_CONF:-}" ]; then
    mode="$(_ns_sso_mode_from_table "$ns")" || return 1
  else
    local conf
    conf="$(ns_repo_conf "$ns")" || return 1
    mode="$( (source "$conf" >/dev/null 2>&1; printf '%s' "${SSO_MODE:-}") )"
    if [ -z "$mode" ]; then
      echo "[sso-mode] namespace '${ns}' 的 ${conf} 未声明 SSO_MODE——MUST 显式声明（不做默认推断）" >&2
      return 1
    fi
  fi
  case "$mode" in
    embedded|remote) echo "$mode"; return 0 ;;
    *)
      echo "[sso-mode] namespace '${ns}' 的形态 '${mode}' 非法（仅 embedded|remote）" >&2
      return 1
      ;;
  esac
}

# ns_sso_feature <ns> —— 打印 Gateway feature 名（embedded→sso / remote→sso-remote）
ns_sso_feature() {
  local mode
  mode="$(ns_sso_mode "$1")" || return 1
  case "$mode" in
    embedded) echo "sso" ;;
    remote)   echo "sso-remote" ;;
  esac
}

# ns_sso_known <ns> —— 0=已声明
ns_sso_known() {
  ns_sso_mode "$1" >/dev/null 2>&1
}

# ns_sso_modes_list —— 枚举已声明行（"ns mode"）；无声明面返回 0 且无输出。
# 枚举序 = Pre-Proc 目录 glob 序（消费方均顺序不敏感；平台派生缓存的行序正本见 ns-ports.sh）。
ns_sso_modes_list() {
  local d ns mode
  for d in "${PROJECT_ROOT:-.}"/Pre-Proc/*/; do
    [ -d "$d" ] || continue
    ns="$(basename "$d")"
    mode="$(ns_sso_mode "$ns" 2>/dev/null)" || continue
    printf '%s %s\n' "$ns" "$mode"
  done
}

# ns_sso_mode_check <ns> <expected> —— 0=一致；1=不一致（打印实际值）
ns_sso_mode_check() {
  local ns="$1" expected="$2" actual
  actual="$(ns_sso_mode "$ns")" || return 1
  if [ "$actual" != "$expected" ]; then
    echo "[sso-mode] ${ns} 形态 ${actual} ≠ 期望 ${expected}"
    return 1
  fi
  return 0
}

# ── 自检（调试/冒烟；不依赖 dev 服务环境）──────────────────────────────────
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  _fail=0
  _assert() { # $1=描述 $2=期望 $3=实际
    if [ "$2" = "$3" ]; then echo "  ok: $1"; else echo "  FAIL: $1（期望 '$2' 实际 '$3'）"; _fail=1; fi
  }
  echo "声明面读取："
  ns_sso_modes_list | sed 's/^/  /'
  _assert "WZ = remote" "remote" "$(ns_sso_mode WZ 2>/dev/null)"
  _assert "Alioth = embedded" "embedded" "$(ns_sso_mode Alioth 2>/dev/null)"
  _assert "WZ feature = sso-remote" "sso-remote" "$(ns_sso_feature WZ 2>/dev/null)"
  _assert "Alioth feature = sso" "sso" "$(ns_sso_feature Alioth 2>/dev/null)"
  _assert "已声明行数 = 5" "5" "$(ns_sso_modes_list | wc -l | tr -d ' ')"

  echo "未声明 ns 必须失败："
  if ns_sso_mode NoSuchNS >/dev/null 2>&1; then
    echo "  FAIL: 未声明 ns 未失败（静默默认即违规）"; _fail=1
  else
    echo "  ok: 未声明 ns 非零退出"
  fi
  if ns_sso_feature NoSuchNS >/dev/null 2>&1; then
    echo "  FAIL: 未声明 ns 的 feature 未失败"; _fail=1
  else
    echo "  ok: 未声明 ns 的 feature 非零退出"
  fi

  echo "形态校验："
  if ns_sso_mode_check WZ remote >/dev/null 2>&1; then echo "  ok: WZ=remote 一致"; else echo "  FAIL: WZ=remote 判定失败"; _fail=1; fi
  if ns_sso_mode_check WZ embedded >/dev/null 2>&1; then echo "  FAIL: 期望 embedded 却判为一致"; _fail=1; else echo "  ok: WZ≠embedded 被检出"; fi

  echo "非法形态 / 缺表（NS_SSO_MODE_CONF 覆写通道） / 声明面缺键："
  _tmp="$(mktemp -d)"
  printf 'WZ broken\n' > "${_tmp}/sso-mode.conf"
  NS_SSO_MODE_CONF="${_tmp}/sso-mode.conf"
  if ns_sso_mode WZ >/dev/null 2>&1; then echo "  FAIL: 非法形态未被拒"; _fail=1; else echo "  ok: 非法形态被拒"; fi
  NS_SSO_MODE_CONF="${_tmp}/absent.conf"
  if ns_sso_mode WZ >/dev/null 2>&1; then echo "  FAIL: 表缺失未被拒"; _fail=1; else echo "  ok: 表缺失被拒"; fi
  unset NS_SSO_MODE_CONF
  # 声明面缺 SSO_MODE 键（NS_REPO_ROOT_OVERRIDE 指临时 ns 仓 fixture）
  mkdir -p "${_tmp}/fake-repo/.alioth"
  printf 'META_DIR=../nowhere\n' > "${_tmp}/fake-repo/.alioth/platform.conf"
  NS_REPO_ROOT_OVERRIDE="${_tmp}/fake-repo"
  if ns_sso_mode FakeNS >/dev/null 2>&1; then echo "  FAIL: 声明面缺键未被拒"; _fail=1; else echo "  ok: 声明面缺键被拒"; fi
  printf 'SSO_MODE=broken\n' >> "${_tmp}/fake-repo/.alioth/platform.conf"
  if ns_sso_mode FakeNS >/dev/null 2>&1; then echo "  FAIL: 声明面非法值未被拒"; _fail=1; else echo "  ok: 声明面非法值被拒"; fi
  unset NS_REPO_ROOT_OVERRIDE
  rm -rf "${_tmp}"

  [ "${_fail}" = "0" ] && echo "自检全部通过" || { echo "自检存在失败"; exit 1; }
fi
