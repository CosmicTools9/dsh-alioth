#!/usr/bin/env bash
# ns-repo.sh — ns 仓根定位（正本声明面 = 各 ns 仓 .alioth/platform.conf）
#
# move-ns-registry-to-ns-repos：端口槽位（FE_PORT/BE_PORT/SSO_PORT）与 SSO 形态（SSO_MODE）
# 的声明正本在 **ns 仓**；平台仓 Deploy/{ports,sso-mode}.conf 为派生缓存（refresh-ns-registry
# 刷新，禁手改）。定位机制：平台仓 `Pre-Proc/{ns}` 为指向 ns 仓的符号链接（2026-09-24 拆分；
# 实目录遮蔽挂载 ⇒ ns 构建全败，判据 = `scripts/setup.sh --check`）⇒ readlink 反推 ns 仓根。
#
# 用法（调用方需先设 PROJECT_ROOT，或当前目录为仓库根）:
#   source scripts/lib/ns-repo.sh
#   ns_repo_root WZ      # → ns 仓根绝对路径；不可定位 rc=1（fail-loud，不回落）
#   ns_repo_conf  WZ     # → <ns_repo>/.alioth/platform.conf 路径
#
# 测试钩子: NS_REPO_ROOT_OVERRIDE（绕过符号链接定位，直指定 ns 仓根——自检/隔离 worktree 用）

if [ -z "${PROJECT_ROOT:-}" ]; then
  PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." 2>/dev/null && pwd)"
fi

# ns_repo_root <ns> —— 输出 ns 仓根绝对路径；非符号链接/目标缺失/声明面缺失 → rc=1
ns_repo_root() {
  local ns="${1:-}" link target dir root
  if [ -n "${NS_REPO_ROOT_OVERRIDE:-}" ]; then
    printf '%s\n' "$NS_REPO_ROOT_OVERRIDE"
    return 0
  fi
  [ -n "$ns" ] || { echo "[ns-repo] 用法: ns_repo_root <ns>" >&2; return 1; }
  link="${PROJECT_ROOT:-.}/Pre-Proc/${ns}"
  if [ ! -L "$link" ]; then
    echo "[ns-repo] Pre-Proc/${ns} 不是符号链接（缺失/实目录遮蔽挂载）——无法定位 ns 仓根；判据: bash scripts/setup.sh --check" >&2
    return 1
  fi
  target="$(readlink "$link")" || { echo "[ns-repo] readlink Pre-Proc/${ns} 失败" >&2; return 1; }
  # 全链**物理**解析（pwd -P）+ 上溯两级：兼容两种挂载形态——
  #   ① 主检出的相对 target（`../../Alioth-NS-{ns}/Pre-Proc/{ns}`）；
  #   ② 验证队列 worktree 的**直通** target（指向主检出 `Pre-Proc/{ns}` 的绝对路径）。
  # 旧实现只对 target 取 dirname（隐含「相对 Pre-Proc/ 解析」假设），形态 ② 下退化为平台仓根
  # （2026-10-09 实测：队列 worktree 内 refresh-ns-registry --check 报「ns 仓根 <平台根> 缺
  # `.alioth/platform.conf`」⇒ push 档 ns-registry-fresh 恒红）。
  # 语义：挂载点指向 `<ns_repo>/Pre-Proc/{ns}` ⇒ 仓根 = 该物理路径上溯两级。
  resolved="$(cd "$(dirname "$link")" && cd "$target" 2>/dev/null && pwd -P)" || {
    echo "[ns-repo] 解析 Pre-Proc/${ns} → ${target} 失败（目标不存在?）" >&2
    return 1
  }
  root="$(cd "$resolved/../.." && pwd -P)"
  [ -f "$root/.alioth/platform.conf" ] || {
    echo "[ns-repo] ns 仓根 ${root} 缺 .alioth/platform.conf——声明面缺失（端口/形态 MUST 在此声明）" >&2
    return 1
  }
  printf '%s\n' "$root"
}

# ns_repo_conf <ns> —— 输出该 ns 的声明面路径
ns_repo_conf() {
  local root
  root="$(ns_repo_root "${1:-}")" || return 1
  printf '%s/.alioth/platform.conf\n' "$root"
}

# ── 自检（调试/冒烟；不依赖 dev 服务环境）──────────────────────────────────
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  _fail=0
  _assert() { # $1=描述 $2=期望 $3=实际
    if [ "$2" = "$3" ]; then echo "  ok: $1"; else echo "  FAIL: $1（期望 '$2' 实际 '$3'）"; _fail=1; fi
  }
  echo "ns 仓根定位："
  for ns in Alioth Cosmic-Tools WZ AVIC-CAASEC SE; do
    _assert "$ns 根名 = Alioth-NS-$ns" "Alioth-NS-$ns" "$(basename "$(ns_repo_root "$ns" 2>/dev/null)")"
  done
  _assert "WZ 声明面路径尾段" "Alioth-NS-WZ/.alioth/platform.conf" \
    "$(basename "$(dirname "$(dirname "$(ns_repo_conf WZ 2>/dev/null)")")")/.alioth/platform.conf"

  echo "不可定位必须失败："
  if ns_repo_root NoSuchNS >/dev/null 2>&1; then
    echo "  FAIL: 未存在 ns 未失败"; _fail=1
  else
    echo "  ok: 未存在 ns 非零退出"
  fi
  _tmp="$(mktemp -d)"
  mkdir -p "${_tmp}/Pre-Proc" "${_tmp}/fake-repo/Pre-Proc/FakeNS" "${_tmp}/fake-repo/.alioth"
  ln -s "../fake-repo/Pre-Proc/FakeNS" "${_tmp}/Pre-Proc/FakeNS"
  printf 'META_DIR=../nowhere\n' > "${_tmp}/fake-repo/.alioth/platform.conf"
  PROJECT_ROOT="${_tmp}" ns_repo_root FakeNS >/dev/null 2>&1
  _assert "FakeNS 经临时符号链接可定位" 0 "$?"
  rm -rf "${_tmp}/fake-repo/.alioth/platform.conf"
  if PROJECT_ROOT="${_tmp}" ns_repo_root FakeNS >/dev/null 2>&1; then
    echo "  FAIL: 声明面缺失未失败"; _fail=1
  else
    echo "  ok: 声明面缺失非零退出"
  fi
  rm -rf "${_tmp}"

  [ "${_fail}" = "0" ] && echo "自检全部通过" || { echo "自检存在失败"; exit 1; }
fi
