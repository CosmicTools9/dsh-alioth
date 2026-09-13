#!/bin/bash
# ns-ports.sh — Namespace 端口槽位 / 运行时注册表 / 端口抢占 / 启动锁（单一事实源 Deploy/ports.conf）
#
# add-namespace-port-slot-table：Gateway/SSO dev 端口表机器可读化。
# fix-dev-port-selfheal-gaps：对齐扩到 5 键；运行时注册表单格式 + 活性判定；
# 端口抢占（归属判定→复检）；同 ns 并发启动锁。
#
# 用法（调用方需先设 PROJECT_ROOT 或当前目录为仓库根）:
#   source scripts/lib/ns-ports.sh
#   ns_port WZ fe            # → 41719（角色: fe|be|sso）
#   ns_ports_known WZ        # 0=已登记；1=未登记
#   ns_ports_file            # 打印表路径（可覆写 NS_PORTS_CONF 测试）
#   ns_ports_list            # 枚举 "ns fe be sso" 行（供审计工具消费）
#   ns_ports_align_env WZ Deploy/WZ/.env        # 5 键对齐槽位
#   ns_registry_write WZ <be_pid> <fe_pid> <sso_pid>
#   ns_registry_status WZ / ns_registry_prune WZ
#   ns_pid_belongs WZ <pid>
#   ns_port_preempt WZ 41719 Gateway-FE
#   ns_lock_acquire WZ / ns_lock_release WZ
#
# 表格式：空白分隔 4 列（ns FE BE SSO）；# 开头/空行忽略。
# 纯 bash read 逐行解析（非正则模拟解析器）。

# 调用方未设 PROJECT_ROOT 时自推导（直接执行自检/独立调用场景；显式设置优先）
if [ -z "${PROJECT_ROOT:-}" ]; then
  PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." 2>/dev/null && pwd)"
fi

ns_ports_file() {
  echo "${NS_PORTS_CONF:-${PROJECT_ROOT:-.}/Deploy/ports.conf}"
}

# ns_port <ns> <fe|be|sso> —— 输出槽位端口；未登记/未知角色返回非 0
ns_port() {
  local ns="$1" role="$2" n fe be sso
  [ -f "$(ns_ports_file)" ] || return 1
  while read -r n fe be sso; do
    [[ -z "$n" || "$n" == \#* ]] && continue
    if [ "$n" = "$ns" ]; then
      case "$role" in
        fe) echo "$fe"; return 0 ;;
        be) echo "$be"; return 0 ;;
        sso) echo "$sso"; return 0 ;;
        *) return 1 ;;
      esac
    fi
  done < "$(ns_ports_file)"
  return 1
}

# ns_ports_known <ns> —— 0=已登记
ns_ports_known() {
  ns_port "$1" be >/dev/null 2>&1
}

# ns_ports_list —— 枚举已登记行（"ns fe be sso"）；无表/空表返回 0 且无输出
ns_ports_list() {
  local n fe be sso
  [ -f "$(ns_ports_file)" ] || return 0
  while read -r n fe be sso; do
    [[ -z "$n" || "$n" == \#* ]] && continue
    printf '%s %s %s %s\n' "$n" "$fe" "$be" "$sso"
  done < "$(ns_ports_file)"
}

# ns_ports_check <ns> <be_port> <fe_port> <sso_port> —— 0=与槽位一致;1=漂移(逐项输出)
# 未登记 ns: 输出提示并返回 0（调用方按其既有策略继续，如 dev-gateway 跳过校验）。
ns_ports_check() {
  local ns="$1" be_actual="$2" fe_actual="$3" sso_actual="$4"
  local eb ef es ok=1
  if ! ns_ports_known "$ns"; then
    echo "[ports] ${ns} 未在 Deploy/ports.conf 登记槽位——跳过槽位校验（建议登记后启用）"
    return 0
  fi
  eb="$(ns_port "$ns" be)"
  ef="$(ns_port "$ns" fe)"
  es="$(ns_port "$ns" sso)"
  [ "$be_actual" != "$eb" ] && { echo "[ports] ${ns} SERVER_ADDR 端口 ${be_actual} ≠ 槽位 ${eb}"; ok=0; }
  [ "$fe_actual" != "$ef" ] && { echo "[ports] ${ns} VITE_PORT ${fe_actual} ≠ 槽位 ${ef}"; ok=0; }
  [ "$sso_actual" != "$es" ] && { echo "[ports] ${ns} SSO_SERVICE_URL 端口 ${sso_actual} ≠ 槽位 ${es}"; ok=0; }
  [ "$ok" = 0 ] && return 1
  return 0
}

# ── 运行时注册表（.runtime/gateway/{ns}.pid）─────────────────────────────
# 唯一格式：`KEY=value`（NAMESPACE/DB_NAME/SSO_PID/GATEWAY_BE_PID/GATEWAY_FE_PID）。
# 读取容忍 legacy 裸 PID（无 `=` 的行）——按其唯一语义视作 GATEWAY_BE_PID 并告警。
# NS_RUNTIME_ROOT / NS_LOGS_ROOT 可覆写（测试/隔离 worktree）。

ns_registry_file() {
  echo "${NS_RUNTIME_ROOT:-${PROJECT_ROOT:-.}/.runtime/gateway}/${1}.pid"
}

# ns_registry_write <ns> <be_pid> <fe_pid> <sso_pid> [db_name] —— 原子写（tmp + mv）
ns_registry_write() {
  local ns="$1" be="$2" fe="$3" sso="$4" db="${5:-}"
  local f tmp
  f="$(ns_registry_file "$ns")"
  mkdir -p "$(dirname "$f")" || return 1
  tmp="${f}.tmp.$$"
  {
    printf '# Gateway+SSO instance for namespace: %s\n' "$ns"
    printf '# Started: %s\n' "$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
    printf 'NAMESPACE=%s\n' "$ns"
    printf 'DB_NAME=%s\n' "$db"
    printf 'SSO_PID=%s\n' "$sso"
    printf 'GATEWAY_BE_PID=%s\n' "$be"
    printf 'GATEWAY_FE_PID=%s\n' "$fe"
  } > "$tmp" || { rm -f "$tmp"; return 1; }
  mv -f "$tmp" "$f"
}

# ns_registry_read <ns> —— 输出 KEY=value（legacy 裸 PID 归一为 GATEWAY_BE_PID）；rc=1 无文件
ns_registry_read() {
  local ns="$1" f line legacy=0
  f="$(ns_registry_file "$ns")"
  [ -f "$f" ] || return 1
  while IFS= read -r line; do
    [[ -z "${line// /}" || "$line" == \#* ]] && continue
    if [[ "$line" == *=* ]]; then
      printf '%s\n' "$line"
    else
      legacy=1
      printf 'GATEWAY_BE_PID=%s\n' "${line// /}"
    fi
  done < "$f"
  [ "$legacy" = 1 ] && echo "[registry] ${ns}: legacy 裸 PID 注册表（读作 GATEWAY_BE_PID）——下次启动将归一为 KEY=value" >&2
  return 0
}

# ns_registry_pid <ns> <KEY> —— 取单键值（无文件/无键 → 空 + rc=1）
ns_registry_pid() {
  local ns="$1" key="$2" kv
  while IFS= read -r kv; do
    case "$kv" in
      "${key}="*) echo "${kv#*=}"; return 0 ;;
    esac
  done < <(ns_registry_read "$ns" 2>/dev/null)
  return 1
}

# ns_alive <pid> —— 0=进程存在且非僵尸（僵尸不占端口、不可再操作）
ns_alive() {
  local pid="$1" state
  [ -n "$pid" ] || return 1
  kill -0 "$pid" 2>/dev/null || return 1
  state="$(ps -p "$pid" -o state= 2>/dev/null | tr -d ' ')"
  [ -n "$state" ] && [ "${state#Z}" != "$state" ] && return 1
  return 0
}

# ns_pid_looks_first_party <ns> <pid> —— 0=该进程确属本项目（防 stale 注册表 + PID 复用误杀）
# 判据（任一）：① cmdline 含本仓库绝对路径（Deploy/{ns}/bin、target/release、node_modules vite 等）；
#              ② vite 进程且 cwd 在本仓库内；③ 一方进程名（*-server / gateway-sso / openactivity-server）
#              且监听端口落在本 ns 槽位（BE/FE/SSO）。都不满足 → 判定非本项目（调用方不得据此杀进程）。
ns_pid_looks_first_party() {
  local ns="$1" pid="$2" cmd cwd port role
  ns_alive "$pid" || return 1
  cmd="$(ps -p "$pid" -o command= 2>/dev/null || true)"
  [ -z "$cmd" ] && return 1
  case "$cmd" in *"${PROJECT_ROOT:-/nonexistent}"*) return 0 ;; esac
  case "$cmd" in
    *vite*)
      cwd="$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -1)"
      case "$cwd" in "${PROJECT_ROOT:-/nonexistent}"*) return 0 ;; esac
      return 1
      ;;
    *gateway-sso*|*openactivity-server*|*-server*)
      for role in be fe sso; do
        port="$(ns_port "$ns" "$role" 2>/dev/null || true)"
        [ -z "$port" ] && continue
        if lsof -ti :"$port" 2>/dev/null | grep -qx "$pid"; then
          return 0
        fi
      done
      return 1
      ;;
  esac
  return 1
}

# ns_pid_belongs <ns> <pid> —— 0=活进程 ∧ 本 ns 实例记录 ∧ 身份确属本项目
ns_pid_belongs() {
  local ns="$1" pid="$2" f kv
  ns_alive "$pid" || return 1
  ns_pid_looks_first_party "$ns" "$pid" || return 1
  local logs_root="${NS_LOGS_ROOT:-${PROJECT_ROOT:-.}/logs/dev}"
  for f in "${logs_root}/${ns}"/*.pid; do
    [ -f "$f" ] || continue
    [ "$(cat "$f" 2>/dev/null)" = "$pid" ] && return 0
  done
  if kv="$(ns_registry_read "$ns" 2>/dev/null)"; then
    while IFS= read -r f; do
      case "$f" in
        *=*) [ "${f#*=}" = "$pid" ] && return 0 ;;
      esac
    done <<<"$kv"
  fi
  return 1
}

# ns_registry_status <ns> —— 打印人类可读状态；0=有活进程，1=无/无文件
ns_registry_status() {
  local ns="$1" kv pid label live=1 found=0 f
  if ! kv="$(ns_registry_read "$ns" 2>/dev/null)"; then
    echo "   registry: 无记录（$(ns_registry_file "$ns")）"
    return 1
  fi
  while IFS= read -r f; do
    case "$f" in
      NAMESPACE=*) echo "   NAMESPACE: ${f#*=}" ;;
      DB_NAME=*)   echo "   DB:        ${f#*=}" ;;
      *PID=*)
        label="${f%%=*}"; pid="${f#*=}"
        found=1
        if [ -z "$pid" ]; then
          echo "   ${label}: 未记录"
        elif ns_alive "$pid"; then
          if ns_pid_looks_first_party "$ns" "$pid"; then
            echo "   ${label}: PID ${pid} 运行中"
          else
            echo "   ${label}: PID ${pid} 存活但非本项目进程（疑似 PID 复用——--stop 不会据此杀进程）"
          fi
          live=0
        else
          echo "   ${label}: PID ${pid} 已死（stale）"
        fi
        ;;
    esac
  done <<<"$kv"
  [ "$found" = 0 ] && echo "   registry: 无 PID 记录"
  return "$live"
}

# ns_registry_prune <ns> —— 无活进程 → 删除注册表文件；0=已删，1=保留
ns_registry_prune() {
  local ns="$1" f
  f="$(ns_registry_file "$ns")"
  [ -f "$f" ] || return 1
  if ns_registry_status "$ns" >/dev/null 2>&1; then
    return 1
  fi
  rm -f "$f"
  return 0
}

# ── 端口抢占 ────────────────────────────────────────────────────────────
# ns_port_preempt <ns> <port> <label> —— 0=端口已自由（含回收成功）；1=仍被占（已打印存活者）
# 语义（2026-09-09 用户裁决）：启动脚本抢占所需端口、关闭占用进程。
# 归属判定只用于分级告警（本 ns 旧实例 / 非本 ns 进程），不改变"抢占"这一处置；
# 抢占后强制复检——抢不回即 fail-loud（supervisor 托管/权限不足的真实出口）。
ns_port_preempt() {
  local ns="$1" port="$2" label="${3:-dev}"
  local pids p cmd note
  pids="$(lsof -ti :"$port" 2>/dev/null || true)"
  [ -z "$pids" ] && return 0
  for p in $pids; do
    if ns_pid_belongs "$ns" "$p"; then
      note="本 ns 旧实例"
    else
      note="非本 ns 进程（可能属其他 ns / 其他 checkout / 第三方）"
    fi
    cmd="$(ps -p "$p" -o command= 2>/dev/null | cut -c1-120 || true)"
    echo "  [port] :$port (${label}) 被 PID $p 占用（${note}; cmd: ${cmd:-未知}）——自动抢占（用户裁决:启动脚本关闭占用进程）" >&2
  done
  # shellcheck disable=SC2086 # 有意按空白切分多个 PID
  kill $pids 2>/dev/null || true
  sleep 1
  local remaining
  remaining="$(lsof -ti :"$port" 2>/dev/null || true)"
  if [ -n "$remaining" ]; then
    # shellcheck disable=SC2086 # 同上
    kill -9 $remaining 2>/dev/null || true
    sleep 0.5
  fi
  remaining="$(lsof -ti :"$port" 2>/dev/null || true)"
  if [ -n "$remaining" ]; then
    echo "  [port] :$port (${label}) 抢占失败——端口仍被占用：" >&2
    for p in $remaining; do
      echo "    PID $p: $(ps -p "$p" -o command= 2>/dev/null | cut -c1-120 || echo 未知)" >&2
    done
    echo "  处置：先停用托管该进程的会话/进程（如 supervisor、其他 checkout 的 dev），再重新启动" >&2
    return 1
  fi
  echo "  [port] :$port (${label}) 已抢回" >&2
  return 0
}

# ── 同 ns 并发启动锁（.runtime/gateway/{ns}.lock）────────────────────────
# mkdir 原子创建；内容 = 持有者 PID。仅覆盖"启动窗口"（稳态运行不持锁 → 有意重启不受阻）。
ns_lock_dir() {
  echo "${NS_RUNTIME_ROOT:-${PROJECT_ROOT:-.}/.runtime/gateway}/${1}.lock"
}

# ns_lock_acquire <ns> —— 0=已持锁；1=并发启动中（已打印持有者）；2=无法创建
ns_lock_acquire() {
  local ns="$1" d holder
  d="$(ns_lock_dir "$ns")"
  mkdir -p "$(dirname "$d")" 2>/dev/null || return 2
  if mkdir "$d" 2>/dev/null; then
    echo $$ > "$d/pid"
    return 0
  fi
  holder="$(cat "$d/pid" 2>/dev/null || true)"
  if ns_alive "$holder"; then
    echo "[lock] ${ns} 并发启动中（PID ${holder}，锁 ${d}）——本次启动中止；确认无并发启动后可 rm -rf \"$d\" 后重试" >&2
    return 1
  fi
  echo "[lock] ${ns} 残留锁（持有者 ${holder:-未知} 已死）——自动窃取" >&2
  rm -rf "$d"
  if mkdir "$d" 2>/dev/null; then
    echo $$ > "$d/pid"
    return 0
  fi
  return 2
}

# ns_lock_release <ns> —— 释放本进程持有的锁（非本进程持有的锁不动）
ns_lock_release() {
  local ns="$1" d holder
  d="$(ns_lock_dir "$ns")"
  [ -d "$d" ] || return 0
  holder="$(cat "$d/pid" 2>/dev/null || true)"
  if [ "$holder" = "$$" ]; then
    rm -rf "$d"
  fi
  return 0
}

# ── 环境文件端口对齐（5 键）──────────────────────────────────────────────
# 派生键变换：保留宿主（含 scheme 与路径），只换端口——避免把
# `http://localhost`（无端口）或 `http://host:9001/api` 误改。
_ns_align_rehost() { # $1=原值 $2=新端口
  local v="$1" port="$2" scheme rest host path=""
  case "$v" in
    *"://"*)
      scheme="${v%%://*}"
      rest="${v#*://}"
      host="${rest%%:*}"
      case "$rest" in */*) path="/${rest#*/}" ;; esac
      echo "${scheme}://${host}:${port}${path}"
      ;;
    *) echo "$v" ;;
  esac
}

_ns_align_origin_port() { # $1=CORS_ALLOWED_ORIGINS 原值 $2=新端口
  local v="$1" port="$2" out="" item host
  local IFS=','
  for item in $v; do
    host="${item#*://}"
    host="${host%%:*}"
    case "$host" in
      localhost|127.0.0.1|0.0.0.0) item="${item%%://*}://${host}:${port}" ;;
    esac
    out="${out:+${out},}${item}"
  done
  echo "$out"
}

# 端口载体键的期望值：字面值 / __DERIVED__（需按宿主派生）/ 空（非端口键）
_ns_align_expect() { # $1=key $2=be $3=fe $4=sso
  case "$1" in
    SERVER_ADDR) echo "127.0.0.1:${2}" ;;
    VITE_PORT) echo "${3}" ;;
    SSO_SERVICE_URL) echo "http://127.0.0.1:${4}" ;;
    VITE_API_URL|CORS_ALLOWED_ORIGINS|ALLOWED_ORIGINS) echo "__DERIVED__" ;;
    *) echo "" ;;
  esac
}

_ns_align_derive() { # $1=key $2=原值 $3=be $4=fe
  case "$1" in
    VITE_API_URL) _ns_align_rehost "$2" "$3" ;;
    CORS_ALLOWED_ORIGINS|ALLOWED_ORIGINS) _ns_align_origin_port "$2" "$4" ;;
    *) echo "$2" ;;
  esac
}

# ns_ports_align_env <ns> <env_file> —— 对齐 6 个端口载体；其余行一律不动。
# SERVER_ADDR/VITE_PORT/SSO_SERVICE_URL：直接取槽位值。
# VITE_API_URL：保留 scheme+host，端口换成槽位 BE。
# CORS 双键 CORS_ALLOWED_ORIGINS / ALLOWED_ORIGINS（分别由 config.rs 与
# common::build_cors 读取；缺 ALLOWED_ORIGINS 的 env 会让服务启动即 panic）：
# 同一 origin 规则——保留非回环 origin 原样，仅把 localhost/127.0.0.1 项换成槽位 FE。
ns_ports_align_env() {
  local ns="$1" env_file="$2" be fe sso aligned="" cur tmp line key val
  if ! ns_ports_known "$ns"; then
    echo "[ports] ${ns} 未登记槽位——跳过对齐（建议先向 Deploy/ports.conf 登记）"
    return 1
  fi
  [ -f "$env_file" ] || { echo "[ports] 环境文件不存在：${env_file}" >&2; return 1; }
  be="$(ns_port "$ns" be)"
  fe="$(ns_port "$ns" fe)"
  sso="$(ns_port "$ns" sso)"

  tmp="${env_file}.tmp.$$"
  : > "$tmp" || return 1
  while IFS= read -r line || [ -n "$line" ]; do
    if [[ "$line" == *=* && "$line" != \#* ]]; then
      key="${line%%=*}"
      val="${line#*=}"
      if [ -n "$(_ns_align_expect "$key" "$be" "$fe" "$sso")" ]; then
        cur="$(_ns_align_expect "$key" "$be" "$fe" "$sso")"
        if [ "$cur" = "__DERIVED__" ]; then
          cur="$(_ns_align_derive "$key" "$val" "$be" "$fe")"
        fi
        if [ "$val" != "$cur" ]; then
          printf '%s=%s\n' "$key" "$cur" >> "$tmp"
          aligned="${aligned} ${key}"
          continue
        fi
      fi
    fi
    printf '%s\n' "$line" >> "$tmp"
  done < "$env_file"
  # 缺失键补齐（CORS 双键同规则）
  for key in SERVER_ADDR VITE_PORT SSO_SERVICE_URL VITE_API_URL CORS_ALLOWED_ORIGINS ALLOWED_ORIGINS; do
    grep -q "^${key}=" "$tmp" 2>/dev/null && continue
    cur="$(_ns_align_expect "$key" "$be" "$fe" "$sso")"
    case "$cur" in
      __DERIVED__)
        case "$key" in
          VITE_API_URL) cur="http://127.0.0.1:${be}" ;;
          CORS_ALLOWED_ORIGINS|ALLOWED_ORIGINS) cur="http://localhost:${fe},http://127.0.0.1:${fe}" ;;
        esac
        ;;
    esac
    printf '%s=%s\n' "$key" "$cur" >> "$tmp"
    aligned="${aligned} ${key}(新增)"
  done
  mv -f "$tmp" "$env_file"
  if [ -n "$aligned" ]; then
    echo "[ports] ${ns} .env 端口对齐槽位（Deploy/ports.conf）:${aligned}"
  fi
  return 0
}

# ns_ports_next_slot —— 依现存登记推算下一槽位（稳健取 max FE+1，容忍删行）：
# 输出 "fe be sso"（FE=41717+k、BE=9001+2k、SSO=BE+1 槽位序列）。
ns_ports_next_slot() {
  local n be fe sso maxfe=41716
  [ -f "$(ns_ports_file)" ] || { echo "41717 9001 9002"; return 0; }
  while read -r n fe be sso; do
    [[ -z "$n" || "$n" == \#* ]] && continue
    [ "$fe" -gt "$maxfe" ] 2>/dev/null && maxfe="$fe"
  done < "$(ns_ports_file)"
  local new_fe=$((maxfe + 1))
  local new_be=$((9001 + 2 * (new_fe - 41717)))
  echo "${new_fe} ${new_be} $((new_be + 1))"
}

# ns_ports_register <ns> —— namespace 创建时自动登记（幂等）：已登记 → 提示现值 rc=0；
# 未登记 → 按下一槽位追加到 Deploy/ports.conf 并回显。表缺失 rc=1。
ns_ports_register() {
  local ns="$1" slot fe be sso
  if ns_ports_known "$ns"; then
    echo "[ports] ${ns} 已登记槽位 FE=$(ns_port "$ns" fe) BE=$(ns_port "$ns" be) SSO=$(ns_port "$ns" sso)"
    return 0
  fi
  slot="$(ns_ports_next_slot)"
  read -r fe be sso <<<"$slot"
  if ! printf '%s %s %s %s\n' "$ns" "$fe" "$be" "$sso" >> "$(ns_ports_file)"; then
    echo "[ports] 登记失败——无法写入 $(ns_ports_file)" >&2
    return 1
  fi
  echo "[ports] 已登记 ${ns} → FE=${fe} BE=${be} SSO=${sso}（Deploy/ports.conf；后续：Pre-Proc/${ns}、Gateway feature、mise dev 任务、release 生成 .env）"
  return 0
}

# 直接执行 = 自检（调试/冒烟；不依赖 dev 服务环境）
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  local_fail=0
  assert() { # $1 描述 $2 期望值（0/1） ... 实际经 run
    local desc="$1" want="$2" got
    shift 2
    got=0; "$@" >/dev/null 2>&1 || got=$?
    if [ "$got" = "$want" ]; then echo "  ok: $desc"; else echo "  FAIL: $desc (期望 $want 实际 $got)"; local_fail=1; fi
  }
  list_has() { ns_ports_list | grep -q "$1"; }
  next_is() { [ "$(ns_ports_next_slot)" = "$1" ]; }
  reg_is() { [ "$(ns_registry_pid "$1" "$2")" = "$3" ]; }
  echo "槽位读取："
  for ns in Alioth Cosmic-Tools WZ AVIC-CAASEC SE; do
    echo "  $ns: FE=$(ns_port "$ns" fe) BE=$(ns_port "$ns" be) SSO=$(ns_port "$ns" sso)"
  done
  assert "未知 ns 拒绝" 1 ns_ports_known NoSuchNS
  assert "WZ 槽位一致 rc=0" 0 ns_ports_check WZ 9005 41719 9006
  echo "槽位枚举："
  ns_ports_list | sed 's/^/  /'
  assert "枚举含 WZ 行" 0 list_has '^WZ 41719 9005 9006$'
  echo "漂移检测（期望 FAIL 行 + rc=1）："
  if ns_ports_check WZ 9005 41717 9006; then echo "  FAIL: 漂移未检出"; local_fail=1; else echo "  ok: FE 漂移 41717≠41719 被检出"; fi

  tmp="$(mktemp -d)"
  mkdir -p "$tmp/logs/dev/WZ" "$tmp/runtime"
  NS_LOGS_ROOT="$tmp/logs/dev" NS_RUNTIME_ROOT="$tmp/runtime"
  export NS_LOGS_ROOT NS_RUNTIME_ROOT

  echo "注册表："
  ns_registry_write WZ 111 222 333 wz 2>/dev/null
  assert "写入后可读 BE" 0 reg_is WZ GATEWAY_BE_PID 111
  assert "写入后可读 FE" 0 reg_is WZ GATEWAY_FE_PID 222
  assert "写入后可读 SSO" 0 reg_is WZ SSO_PID 333
  assert "死 PID 无活进程 → status rc=1" 1 ns_registry_status WZ
  assert "prune 删除死注册表" 0 ns_registry_prune WZ
  assert "prune 后文件消失" 1 test -f "$tmp/runtime/WZ.pid"
  # legacy 裸 PID
  printf '9999\n' > "$tmp/runtime/WZ.pid"
  assert "legacy 裸 PID 读作 BE" 0 reg_is WZ GATEWAY_BE_PID 9999
  rm -f "$tmp/runtime/WZ.pid"
  # PID 归属：身份 + 登记 + 活性
  bash -c "exec -a '${PROJECT_ROOT}/Deploy/WZ/bin/wz-server' sleep 30" &
  PARTY_PID=$!
  sleep 0.3
  assert "一方进程：身份判据通过" 0 ns_pid_looks_first_party WZ "$PARTY_PID"
  assert "一方进程但未登记 → 拒绝" 1 ns_pid_belongs WZ "$PARTY_PID"
  assert "pid 非本 ns 记录拒绝" 1 ns_pid_belongs WZ 8888
  echo "$PARTY_PID" > "$tmp/logs/dev/WZ/gateway-frontend.pid"
  assert "per-service pidfile 登记 → 归属成立" 0 ns_pid_belongs WZ "$PARTY_PID"
  rm -f "$tmp/logs/dev/WZ/gateway-frontend.pid"
  printf 'NAMESPACE=WZ\nGATEWAY_FE_PID=%s\n' "$PARTY_PID" > "$tmp/runtime/WZ.pid"
  assert "registry 汇总登记 → 归属成立" 0 ns_pid_belongs WZ "$PARTY_PID"
  printf 'NAMESPACE=WZ\nGATEWAY_FE_PID=7777\n' > "$tmp/runtime/WZ.pid"
  assert "死 PID 不构成归属" 1 ns_pid_belongs WZ 7777
  # 非本项目进程（无仓库路径、非 vite、非 *-server）→ 身份拒绝
  ( cd /tmp && sleep 30 ) &
  UNRELATED_PID=$!
  sleep 0.3
  assert "非本项目进程：身份判据拒绝" 1 ns_pid_looks_first_party WZ "$UNRELATED_PID"
  printf 'NAMESPACE=WZ\nGATEWAY_BE_PID=%s\n' "$UNRELATED_PID" > "$tmp/runtime/WZ.pid"
  assert "已登记但非本项目进程：不构成归属（不杀）" 1 ns_pid_belongs WZ "$UNRELATED_PID"
  kill "$PARTY_PID" "$UNRELATED_PID" 2>/dev/null || true
  rm -f "$tmp/runtime/WZ.pid"

  echo "启动锁："
  assert "acquire 成功" 0 ns_lock_acquire WZ
  assert "并发 acquire 被拒" 1 ns_lock_acquire WZ
  assert "release 成功" 0 ns_lock_release WZ
  assert "release 后可再 acquire" 0 ns_lock_acquire WZ
  ns_lock_release WZ
  printf '99999999\n' > "$tmp/runtime/WZ.lock/pid"
  assert "残留锁（持有者已死）自动窃取" 0 ns_lock_acquire WZ
  ns_lock_release WZ
  assert "锁目录已清理" 1 test -d "$tmp/runtime/WZ.lock"

  echo "端口对齐（6 键，CORS 双键同规则）："
  printf 'NAMESPACE=WZ\nSERVER_ADDR=127.0.0.1:9001\nVITE_PORT=41717\nVITE_API_URL=http://127.0.0.1:9001\nCORS_ALLOWED_ORIGINS=http://localhost:41717,http://192.168.1.7:5173\nALLOWED_ORIGINS=http://localhost:41717,http://192.168.1.7:5173\nSSO_SERVICE_URL=http://127.0.0.1:9002\nDATABASE_URL=postgresql://localhost:5432/wz\nMY_KEY=keepme\n' > "$tmp/wz.env"
  assert "对齐 rc=0" 0 ns_ports_align_env WZ "$tmp/wz.env"
  echo "对齐后内容："
  grep -E "^(SERVER_ADDR|VITE_PORT|SSO_SERVICE_URL|VITE_API_URL|CORS_ALLOWED_ORIGINS|ALLOWED_ORIGINS|DATABASE_URL|MY_KEY)=" "$tmp/wz.env" | sed 's/^/  /'
  assert "SERVER_ADDR 对齐 9005" 0 grep -q '^SERVER_ADDR=127.0.0.1:9005$' "$tmp/wz.env"
  assert "VITE_PORT 对齐 41719" 0 grep -q '^VITE_PORT=41719$' "$tmp/wz.env"
  assert "SSO_SERVICE_URL 对齐 9006" 0 grep -q '^SSO_SERVICE_URL=http://127.0.0.1:9006$' "$tmp/wz.env"
  assert "VITE_API_URL 对齐 9005（保留 scheme/host）" 0 grep -q '^VITE_API_URL=http://127.0.0.1:9005$' "$tmp/wz.env"
  assert "CORS loopback 对齐 41719" 0 grep -q '^CORS_ALLOWED_ORIGINS=http://localhost:41719,http://192.168.1.7:5173$' "$tmp/wz.env"
  assert "ALLOWED_ORIGINS loopback 对齐 41719（双键同规则）" 0 grep -q '^ALLOWED_ORIGINS=http://localhost:41719,http://192.168.1.7:5173$' "$tmp/wz.env"
  assert "DATABASE_URL 未动" 0 grep -q '^DATABASE_URL=postgresql://localhost:5432/wz$' "$tmp/wz.env"
  assert "MY_KEY 未动" 0 grep -q '^MY_KEY=keepme$' "$tmp/wz.env"
  # 缺失键补齐
  printf 'NAMESPACE=WZ\n' > "$tmp/empty.env"
  assert "缺失键补齐 rc=0" 0 ns_ports_align_env WZ "$tmp/empty.env"
  assert "补齐 VITE_API_URL" 0 grep -q '^VITE_API_URL=http://127.0.0.1:9005$' "$tmp/empty.env"
  assert "补齐 CORS_ALLOWED_ORIGINS" 0 grep -q '^CORS_ALLOWED_ORIGINS=http://localhost:41719,http://127.0.0.1:41719$' "$tmp/empty.env"
  assert "补齐 ALLOWED_ORIGINS" 0 grep -q '^ALLOWED_ORIGINS=http://localhost:41719,http://127.0.0.1:41719$' "$tmp/empty.env"

  echo "端口抢占（dummy listener）："
  DUMMY_PORT=$(( 46000 + ($$ % 500) ))
  bun -e "Bun.serve({port:${DUMMY_PORT},hostname:'127.0.0.1',fetch:()=>new Response('ok')})" >/dev/null 2>&1 &
  DUMMY_PID=$!
  for _ in $(seq 1 30); do lsof -ti :"$DUMMY_PORT" >/dev/null 2>&1 && break; sleep 0.2; done
  if [ -n "$(lsof -ti :"$DUMMY_PORT" 2>/dev/null)" ]; then
    assert "抢占持占端口 rc=0" 0 ns_port_preempt WZ "$DUMMY_PORT" TEST
    assert "抢占后端口自由" 0 bash -c "! lsof -ti :${DUMMY_PORT} >/dev/null 2>&1"
    assert "被抢占进程已终止" 1 ns_alive "$DUMMY_PID"
  else
    echo "  FAIL: dummy listener 未起（跳过抢占用例）"; local_fail=1
  fi
  kill "$DUMMY_PID" 2>/dev/null || true
  assert "空闲端口抢占 rc=0 无副作用" 0 ns_port_preempt WZ "$DUMMY_PORT" TEST

  # 下一槽位 + 登记：临时表（含 Alioth 行 → 下一槽 Cosmic-Tools 41718/9003/9004）
  printf '# 注释\nAlioth 41717 9001 9002\n' > "$tmp/ports.conf"
  export NS_PORTS_CONF="$tmp/ports.conf"
  assert "next 推算 FE=41718" 0 next_is "41718 9003 9004"
  assert "register 新 ns" 0 ns_ports_register Cosmic-Tools
  assert "登记行已写入" 0 grep -q '^Cosmic-Tools 41718 9003 9004$' "$tmp/ports.conf"
  assert "register 幂等跳过" 0 ns_ports_register Cosmic-Tools
  unset NS_PORTS_CONF
  unset NS_LOGS_ROOT NS_RUNTIME_ROOT
  rm -rf "$tmp"
  # shellcheck disable=SC2015 # 自检裁决：非空即通过，失败分支只在 set -e 之外显式退出
  [ "$local_fail" = 0 ] && echo "自检全部通过" || { echo "自检存在失败"; exit 1; }
fi
