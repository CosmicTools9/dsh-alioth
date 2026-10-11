#!/usr/bin/env bash
# ns-auth-providers.sh — ns 登录来源声明面 → 发布面 `.env` 凭据占位物化
#
# 声明正本 = 各 ns 仓 `Pre-Proc/{ns}/gateway-host/auth-providers.json`
#   （门禁 = scripts/check/check-ns-auth-provider-declaration.ts；规约 = ns-auth-provider-declaration）。
# 背景（change `declare-ns-auth-providers`，G8）：ns 的登录来源凭据（如 WZ
#   `WECHAT_MINIAPP_DRIVER_APPID`/`_SECRET`、Cosmic `BVL_WECHAT_*`）承载于 `Deploy/{ns}/.env`，
#   但发布脚本此前**无任何 per-ns 凭据物化通道**——运维从发布面无从得知「本 ns 需要哪些键」。
#   实测：`Deploy/WZ/.env` 的 `grep -c WECHAT` = 0 ⇒ 来源不注册，登录 fail-closed 400。
#
# 本 lib 提供 `ns_auth_provider_materialize_env <ns> <env_file>`：按声明面声明的凭据键，在 `.env`
# **缺失**该键时追加**注释占位行**（含 `secret` 标注与声明文件出处），使运维从发布面即可见所需键。
#   幂等：以 managed 块（BEGIN/END 标记）承载——每次运行先摘除旧块再按当前缺失键重建；
#   绝不覆盖已有真值（`.env` 已有 `KEY=` 行 ⇒ 该键不入占位块）；二次运行结果稳定。
#
# 用法（调用方需先设 PROJECT_ROOT；或当前目录为仓库根）:
#   source scripts/lib/ns-auth-providers.sh
#   ns_auth_provider_materialize_env WZ /path/to/Deploy/WZ/.env
#
# 自检：bash scripts/lib/ns-auth-providers.sh

if [ -z "${PROJECT_ROOT:-}" ]; then
  PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." 2>/dev/null && pwd)"
fi

# managed 块标记（awk -v 精确匹配，纯 ASCII 以免 locale 干扰）
NSAP_BEGIN='# ns-auth-provider-cred BEGIN'
NSAP_END='# ns-auth-provider-cred END'

# _nsap_decl_path <ns> —— 声明面路径
_nsap_decl_path() {
  printf '%s/Pre-Proc/%s/gateway-host/auth-providers.json\n' "$PROJECT_ROOT" "$1"
}

# ns_auth_provider_credentials <ns> —— 逐行打印声明凭据：`KEY<TAB>fromType<TAB>secret|plain`
# 未声明 / 文件缺失 ⇒ 空输出 rc=0（无可物化面）；jq 缺失 / 解析失败 ⇒ rc=1（fail-loud）。
ns_auth_provider_credentials() {
  local ns="${1:-}" f
  [ -n "$ns" ] || { echo "[ns-auth-providers] 用法: ns_auth_provider_credentials <ns>" >&2; return 1; }
  f="$(_nsap_decl_path "$ns")"
  [ -f "$f" ] || return 0
  if ! command -v jq >/dev/null 2>&1; then
    echo "[ns-auth-providers] 缺 jq——无法读取声明面 ${f}" >&2
    return 1
  fi
  jq -r '.loginSources[]? | .fromType as $ft | .credentials[]?
         | "\(.key)\t\($ft)\t\(if .secret == true then "secret" else "plain" end)"' "$f" || return 1
}

# ns_auth_provider_materialize_env <ns> <env_file> —— 声明面凭据键 → `.env` 注释占位（幂等、不覆盖真值）
ns_auth_provider_materialize_env() {
  local ns="${1:-}" env_file="${2:-}" decl
  [ -n "$ns" ] && [ -n "$env_file" ] || {
    echo "[ns-auth-providers] 用法: ns_auth_provider_materialize_env <ns> <env_file>" >&2
    return 1
  }
  [ -f "$env_file" ] || return 0 # `.env` 未生成 ⇒ 无物化面（发布链后续创建）
  decl="$(_nsap_decl_path "$ns")"
  [ -f "$decl" ] || return 0 # 未声明 ⇒ 无可物化（声明面门禁另处守）

  # 1) 摘除既有 managed 块（幂等：重跑先清后建，绝不累积；连块前单个空行一并吞掉）
  if grep -qF "$NSAP_BEGIN" "$env_file" 2>/dev/null; then
    local tmp="${env_file}.nsap.tmp"
    if awk -v b="$NSAP_BEGIN" -v e="$NSAP_END" '
          { lines[NR] = $0 }
          $0 == b { bstart = NR }
          $0 == e { bend = NR }
          END {
            from = bstart; to = bend
            if (bstart > 1 && lines[bstart - 1] == "") from = bstart - 1
            for (i = 1; i <= NR; i++) {
              if (i >= from && i <= to) continue
              print lines[i]
            }
          }
        ' "$env_file" > "$tmp" && mv "$tmp" "$env_file"; then
      :
    else
      rm -f "$tmp" 2>/dev/null
      echo "[ns-auth-providers] 摘除旧占位块失败：${env_file}" >&2
      return 1
    fi
  fi

  # 2) 收集缺失键（.env 无 `^KEY=` 真值行者）→ 注释占位行
  local lines=() key from secret note
  while IFS=$'\t' read -r key from secret; do
    [ -n "$key" ] || continue
    grep -q "^${key}=" "$env_file" 2>/dev/null && continue # 已有真值 ⇒ 不占位、不覆盖
    note="来源 ${from}"
    [ "$secret" = "secret" ] && note="${note}（secret：脱敏处理，勿入日志/不入库）"
    lines+=("# ${key}=           # ${note}")
  done < <(ns_auth_provider_credentials "$ns") || return 1

  # 3) 追加新块（仅当存在缺失键）
  if [ "${#lines[@]}" -gt 0 ]; then
    {
      echo ""
      echo "$NSAP_BEGIN"
      echo "# 本 ns 登录来源声明（Pre-Proc/${ns}/gateway-host/auth-providers.json）所需但 .env 未配置的键："
      echo "# MUST 由运维填入真实值后删除本注释（缺失 ⇒ 对应来源不注册，登录 fail-closed 400）。"
      local l
      for l in "${lines[@]}"; do echo "$l"; done
      echo "$NSAP_END"
    } >> "$env_file"
  fi
  return 0
}

# ── 自检（正/负例；不依赖 dev 服务环境）──────────────────────────────────
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  _fail=0
  _t="$(mktemp -d)"
  trap 'rm -rf "$_t"' EXIT
  mkdir -p "$_t/Pre-Proc/TestNS/gateway-host" "$_t/Deploy/TestNS"
  cat >"$_t/Pre-Proc/TestNS/gateway-host/auth-providers.json" <<'JSON'
{
  "loginSources": [
    { "fromType": "testApp",
      "implPath": "Pre-Proc/TestNS/gateway-host/src/lib.rs",
      "credentials": [ { "key": "APPID_X" }, { "key": "SECRET_X", "secret": true } ] }
  ],
  "dbProviders": []
}
JSON
  printf 'NAMESPACE=TestNS\nSECRET_X=already-set\n' >"$_t/Deploy/TestNS/.env"

  _env="$_t/Deploy/TestNS/.env"
  PROJECT_ROOT="$_t" ns_auth_provider_materialize_env TestNS "$_env" >/dev/null

  _assert() { # $1=描述 $2=期望（1=期望为真） $3=命令结果（grep 退出码）
    local _hit _want
    _hit=$([ "${3:-}" = "0" ] && echo 1 || echo 0)
    _want="${2:-}"
    if [ "$_hit" = "$_want" ]; then
      echo "  ok: $1"
    else
      echo "  FAIL: ${1}（期望 ${_want} 实际 ${_hit}）"; _fail=1
    fi
  }

  echo "物化行为："
  grep -q '^# APPID_X=' "$_env"; _assert "缺失键 APPID_X 追加为注释占位" 1 "$?"
  grep -q '^# SECRET_X=' "$_env"; _assert "已有真值 SECRET_X 不产占位" 0 "$?"
  grep -q '^SECRET_X=already-set$' "$_env"; _assert "已有真值 SECRET_X 未被覆盖" 1 "$?"
  grep -qF "$NSAP_BEGIN" "$_env"; _assert "managed 块已写入" 1 "$?"
  _cnt="$(grep -cF "$NSAP_BEGIN" "$_env")"; _assert "块唯一（1）" 1 "$([ "$_cnt" = "1" ] && echo 0 || echo 1)"

  echo "幂等（二次运行）："
  PROJECT_ROOT="$_t" ns_auth_provider_materialize_env TestNS "$_env" >/dev/null
  _cnt2="$(grep -cF "$NSAP_BEGIN" "$_env")"; _assert "二次运行块仍唯一（1）" 1 "$([ "$_cnt2" = "1" ] && echo 0 || echo 1)"
  _a2="$(grep -c '^# APPID_X=' "$_env")"; _assert "二次运行 APPID_X 占位不重复（1）" 1 "$([ "$_a2" = "1" ] && echo 0 || echo 1)"
  _b1="$(head -1 "$_env")"; _assert "首行未被破坏（NAMESPACE）" 1 "$([ "$_b1" = "NAMESPACE=TestNS" ] && echo 0 || echo 1)"

  echo "未声明 ns："
  PROJECT_ROOT="$_t" ns_auth_provider_materialize_env NoSuchNS "$_env" >/dev/null; _assert "未声明 ns 零动作 rc=0" 1 "$?"

  [ "$_fail" = "0" ] && echo "自检全部通过" || { echo "自检存在失败"; exit 1; }
fi
