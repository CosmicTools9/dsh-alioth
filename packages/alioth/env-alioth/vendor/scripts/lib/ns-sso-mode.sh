#!/usr/bin/env bash
# ns-sso-mode.sh — Namespace SSO 形态读取器（单一事实源 = Deploy/sso-mode.conf）
#
# 契约: openspec/changes/add-per-namespace-sso-mode（design D1–D7）
#       openspec/specs/sso-remote-auth-proxy/spec.md :: per-namespace-sso-mode-contract
#   门禁: scripts/check/check-sso-mode-conf.ts（结构 / 齐备 / env 镜像 / remote 专属）
#
# 用法（调用方需先设 PROJECT_ROOT，或当前目录为仓库根）:
#   source scripts/lib/ns-sso-mode.sh
#   ns_sso_mode WZ            # → remote；未登记 ns → rc=1（MUST NOT 静默返回默认）
#   ns_sso_feature WZ         # → sso-remote（embedded → sso）
#   ns_sso_modes_list         # 枚举 "ns mode" 行（供审计工具消费）
#   ns_sso_mode_check WZ remote
#   bash scripts/lib/ns-sso-mode.sh   # 自检
#
# 表格式：空白分隔 2 列（ns mode）；# 开头/空行忽略。纯 bash `read` 逐行解析
# （非正则模拟解析器，与 scripts/lib/ns-ports.sh 同法，见 NO_REGEX_FOR_PARSING.md）。
#
# 为什么未登记 ns 必须失败：形态是部署契约的一部分，静默默认会把 remote 的 ns
# 降级为 embedded（本变更要消灭的隐式推断正是此形态的静默降级）。

if [ -z "${PROJECT_ROOT:-}" ]; then
  PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." 2>/dev/null && pwd)"
fi

ns_sso_mode_file() {
  echo "${NS_SSO_MODE_CONF:-${PROJECT_ROOT:-.}/Deploy/sso-mode.conf}"
}

# ns_sso_mode <ns> —— 打印形态（embedded|remote）；未登记/表缺失/形态非法 → rc=1
ns_sso_mode() {
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
  case "$found" in
    embedded|remote) echo "$found"; return 0 ;;
    *)
      echo "[sso-mode] namespace '${ns}' 的形态 '${found}' 非法（仅 embedded|remote）" >&2
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

# ns_sso_known <ns> —— 0=已登记
ns_sso_known() {
  ns_sso_mode "$1" >/dev/null 2>&1
}

# ns_sso_modes_list —— 枚举已登记行（"ns mode"）；表缺失返回 0 且无输出
ns_sso_modes_list() {
  local n mode
  [ -f "$(ns_sso_mode_file)" ] || return 0
  while read -r n mode; do
    case "$n" in ''|\#*) continue ;; esac
    printf '%s %s\n' "$n" "$mode"
  done < "$(ns_sso_mode_file)"
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
  echo "形态读取："
  ns_sso_modes_list | sed 's/^/  /'
  _assert "WZ = remote" "remote" "$(ns_sso_mode WZ 2>/dev/null)"
  _assert "Alioth = embedded" "embedded" "$(ns_sso_mode Alioth 2>/dev/null)"
  _assert "WZ feature = sso-remote" "sso-remote" "$(ns_sso_feature WZ 2>/dev/null)"
  _assert "Alioth feature = sso" "sso" "$(ns_sso_feature Alioth 2>/dev/null)"
  _assert "已登记行数 = 5" "5" "$(ns_sso_modes_list | wc -l | tr -d ' ')"

  echo "未登记 ns 必须失败："
  if ns_sso_mode NoSuchNS >/dev/null 2>&1; then
    echo "  FAIL: 未登记 ns 未失败（静默默认即违规）"; _fail=1
  else
    echo "  ok: 未登记 ns 非零退出"
  fi
  if ns_sso_feature NoSuchNS >/dev/null 2>&1; then
    echo "  FAIL: 未登记 ns 的 feature 未失败"; _fail=1
  else
    echo "  ok: 未登记 ns 的 feature 非零退出"
  fi

  echo "形态校验："
  if ns_sso_mode_check WZ remote >/dev/null 2>&1; then echo "  ok: WZ=remote 一致"; else echo "  FAIL: WZ=remote 判定失败"; _fail=1; fi
  if ns_sso_mode_check WZ embedded >/dev/null 2>&1; then echo "  FAIL: 期望 embedded 却判为一致"; _fail=1; else echo "  ok: WZ≠embedded 被检出"; fi

  echo "非法形态 / 缺表："
  _tmp="$(mktemp -d)"
  printf 'WZ broken\n' > "${_tmp}/sso-mode.conf"
  NS_SSO_MODE_CONF="${_tmp}/sso-mode.conf"
  if ns_sso_mode WZ >/dev/null 2>&1; then echo "  FAIL: 非法形态未被拒"; _fail=1; else echo "  ok: 非法形态被拒"; fi
  NS_SSO_MODE_CONF="${_tmp}/absent.conf"
  if ns_sso_mode WZ >/dev/null 2>&1; then echo "  FAIL: 表缺失未被拒"; _fail=1; else echo "  ok: 表缺失被拒"; fi
  rm -rf "${_tmp}"

  [ "${_fail}" = "0" ] && echo "自检全部通过" || { echo "自检存在失败"; exit 1; }
fi
