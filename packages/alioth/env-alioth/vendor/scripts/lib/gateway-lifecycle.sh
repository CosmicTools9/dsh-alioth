#!/bin/bash
# gateway-lifecycle.sh — Gateway 生命周期的**唯一实现**（进程发现 / 停止序列 / 健康等待 / 注册表读写）
#
# 契约正本 = openspec capability `gateway-lifecycle`（change consolidate-gateway-lifecycle）。
# 消费方（MUST 经本库，MUST NOT 内联第二份）：
#   · `scripts/gateway/ctl.sh`（统一 CLI，唯一编排入口）
#   · `scripts/gateway/{start,stop}.sh`（发布面启动器 / 宿主启停）
#   · `scripts/gateway/{restart,restart-gateway}.sh`（薄 facade → ctl.sh）
#   · 未迁移面（登记）：`scripts/gateway/verify-health.sh` 自持 curl 轮询（含线性退避节拍），
#     尚未接线 `gw_wait_http`——`gw_wait_http` 的 `--linear` 分支即为其预留（迁移留待后续）。
#
# 发现顺序（`gw_discover_backend_pids` / `gw_discover_frontend_pids` 同序）：
#   ① 注册表 `Deploy/{ns}/.runtime/{ns}.pid`（格式与 legacy 裸 PID 归一唯一实现 = `ns-ports.sh`）
#   ② 端口（`lsof -ti tcp:<port>`）
#   ③ pgrep（**锚定本 checkout** 的路径正则；防并行 checkout / 无关进程误杀）
#   ⇒ PID 记录通道**只有** ns 运行时注册表一处（`.gateway-dev.pid` 已退役，见 change）。
#
# 停止序列（唯一实现）：SIGTERM → 有界等待（`GW_STOP_GRACE`，默认 10s）→ SIGKILL
# 健康等待（唯一实现）：`gw_wait_http <url> <max_attempts> [interval_s] [--linear]`
#
# 边界（唯一豁免）：`scripts/gateway/serve-frontend.sh` 是**随包自含**的派生副本
# （V6 包自含契约 + `check-derived-copies` 逐字节认证，运行在无仓库的目标机）⇒ MUST NOT source
# 本库（包内无 `scripts/lib/`），自持其 FE 静态服务的 pid/停止序列。其余面 MUST 走本库。
#
# 用法（调用方需先设 PROJECT_ROOT；未设时按本文件位置自推导）：
#   source scripts/lib/gateway-lifecycle.sh
#   gw_discover_backend_pids WZ 9005 [pattern]      # 有序去重 PID 列表（每行一个）
#   gw_discover_frontend_pids WZ 41719 [dist]
#   gw_stop_pid Gateway-BE 1234 [--force]
#   gw_wait_http http://127.0.0.1:9005/health 15 1
#   gw_registry_set_backend WZ 1234 aliothstudio_dev

if [ -z "${PROJECT_ROOT:-}" ]; then
  PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." 2>/dev/null && pwd)"
fi

_GW_LIFECYCLE_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# 注册表契约（ns_registry_read/write/pid/prune）+ 落点（ns_runtime_dir / ns_log_dir）
# + 活性判定（ns_alive）/ 归属判定（ns_pid_belongs）：单一事实源，MUST NOT 另立格式。
# shellcheck source=scripts/lib/ns-ports.sh
source "${_GW_LIFECYCLE_LIB_DIR}/ns-ports.sh"

# ── 日志（颜色变量由调用方定义；未定义则无色）─────────────────────────────
gw_log()  { echo -e "${BLUE:-}[gateway-lifecycle]${NC:-} $*"; }
gw_ok()   { echo -e "${GREEN:-}[gateway-lifecycle]${NC:-} ✓ $*"; }
gw_warn() { echo -e "${YELLOW:-}[gateway-lifecycle]${NC:-} ⚠ $*" >&2; }
gw_err()  { echo -e "${RED:-}[gateway-lifecycle]${NC:-} ✗ $*" >&2; }

# ── 活性 ─────────────────────────────────────────────────────────────────
# gw_alive <pid> —— 0=进程存在且非僵尸（委托 ns-ports.sh::ns_alive，唯一实现）
gw_alive() { ns_alive "${1:-}"; }

# ── 端口 / 进程发现原语 ──────────────────────────────────────────────────
# gw_port_pid <port> —— stdout = 该端口首个监听 PID；rc=1 无监听/无 lsof
gw_port_pid() {
  local port="${1:-}" pid=""
  [ -n "$port" ] || return 1
  command -v lsof >/dev/null 2>&1 || return 1
  pid="$(lsof -ti "tcp:${port}" 2>/dev/null | head -1 || true)"
  [ -n "$pid" ] || return 1
  printf '%s' "$pid"
}

# gw_pgrep_any <egrep-pattern> —— stdout = 命令行匹配的 PID（pgrep 缺失时退化 ps；每行一个）
gw_pgrep_any() {
  local pattern="${1:?gw_pgrep_any 需要 pattern}" found=""
  if command -v pgrep >/dev/null 2>&1; then
    found="$(pgrep -f "$pattern" 2>/dev/null || true)"
  fi
  if [ -z "$found" ]; then
    found="$(ps ax -o pid=,command= 2>/dev/null | grep -E "$pattern" | grep -v grep | awk '{print $1}' || true)"
  fi
  [ -n "$found" ] && printf '%s\n' "$found"
  return 0
}

# gw_pgrep_checkout <relative-pattern> —— 同上，但把 pattern 锚定**本 checkout** 绝对路径
# （防并行 checkout / 并行会话的无关进程被误判为本仓库 Gateway —— 2026-09-25 实测误杀）
gw_pgrep_checkout() {
  gw_pgrep_any "${PROJECT_ROOT}/${1:?gw_pgrep_checkout 需要相对 pattern}"
}

# gw_add_pid <newline-list> <pid> —— stdout = list（去重；仅收存活 PID）
gw_add_pid() {
  local list="$1" pid="$2" x
  [ -n "$pid" ] || { printf '%s' "$list"; return 0; }
  gw_alive "$pid" || { printf '%s' "$list"; return 0; }
  while IFS= read -r x; do
    [ "$x" = "$pid" ] && { printf '%s' "$list"; return 0; }
  done <<<"$list"
  printf '%s%s' "${list:+${list}
}" "$pid"
}

# gw_pid_belongs <ns> <pid> —— 归属校验（owner 记录 / 一方身份 + ns 记录；委托 ns-ports.sh）
gw_pid_belongs() { ns_pid_belongs "${1:?gw_pid_belongs 需要 ns}" "${2:?gw_pid_belongs 需要 pid}"; }

# gw_registry_pid_guarded <ns> <KEY> [label] —— 注册表取值 + 存活 + **归属校验**
#   注册表是诊断缓存（ENVIRONMENT_SPEC：MUST NOT 作为投递信号的依据）⇒ 取值必须过归属校验，
#   否则 stale 记录 + PID 复用会把无关进程当靶（MUST NOT 仅凭「存活」接受）。
gw_registry_pid_guarded() {
  local ns="$1" key="$2" label="${3:-PID}" pid
  pid="$(ns_registry_pid "$ns" "$key" 2>/dev/null || true)"
  [ -n "$pid" ] || return 1
  gw_alive "$pid" || return 1
  if ! ns_pid_belongs "$ns" "$pid"; then
    gw_warn "注册表 ${key}=${pid} 未通过归属校验（疑似 PID 复用 / 非本 ns 进程）——不作为${label}停止目标"
    return 1
  fi
  printf '%s' "$pid"
}

# ── 后端发现（顺序：注册表 → 端口 → pgrep）───────────────────────────────
# gw_discover_backend_pids <ns> [port] [pgrep-pattern]
#   pattern 缺省 = 本 checkout 锚定的组合根/框架目录名（dev 形态：`cargo run … gateway-host`）；
#   发布包形态的 ns 二进制名（`bin/{ns_lower}-server`）由调用方显式传入（`Deploy/{ns}` 是 ns 仓
#   挂载路径，MUST NOT 锚定平台仓根）。
gw_discover_backend_pids() {
  local ns="${1:?gw_discover_backend_pids 需要 ns}" port="${2:-}" pattern="${3:-}"
  local pids="" p
  [ -n "$pattern" ] || pattern="${PROJECT_ROOT}/Gateway/(host|backend)"
  pids="$(gw_add_pid "$pids" "$(gw_registry_pid_guarded "$ns" GATEWAY_BE_PID "Gateway BE" || true)")"
  if [ -n "$port" ]; then
    pids="$(gw_add_pid "$pids" "$(gw_port_pid "$port" || true)")"
  fi
  for p in $(gw_pgrep_any "$pattern"); do
    pids="$(gw_add_pid "$pids" "$p")"
  done
  [ -n "$pids" ] && printf '%s\n' "$pids"
  return 0
}

# gw_discover_backend_pid <ns> [port] [pattern] —— 首个匹配（restart 语义：只处理一个旧实例）
gw_discover_backend_pid() { gw_discover_backend_pids "$@" | head -1; }

# ── 前端发现（顺序：注册表 → pgrep frontend-server.js(+端口) → 兜底档(FE dist 锚)）──
# gw_fe_port_matches <pid> <port> —— cmdline 含 frontend-server.js 的实例按 environ 端口归属；
#   无 /proc（macOS）时 grep -zq 恒失败 ⇒ 视为不匹配（既有语义，MUST NOT 放宽为「全收」）。
gw_fe_port_matches() {
  local pid="$1" port="$2"
  [ -z "$port" ] && return 0
  grep -zq "FRONTEND_PORT=${port}" "/proc/${pid}/environ" 2>/dev/null
}

gw_discover_frontend_pids() {
  local ns="${1:-}" port="${2:-}" dist="${3:-}"
  local pids="" p
  [ -n "$ns" ] && pids="$(gw_add_pid "$pids" "$(gw_registry_pid_guarded "$ns" GATEWAY_FE_PID "Gateway FE" || true)")"
  for p in $(gw_pgrep_any 'frontend-server\.js'); do
    gw_fe_port_matches "$p" "$port" || continue
    pids="$(gw_add_pid "$pids" "$p")"
  done
  # 兜底档 FE（start.sh 的 npx serve / python3 http.server 分支）：cmdline 无 frontend-server.js
  # ⇒ 以「服务目录 = FRONTEND_DIST」为身份锚（防误伤同机其它静态服务）。
  if [ -n "$dist" ]; then
    for p in $(gw_pgrep_any 'npx serve|http\.server'); do
      gw_alive "$p" || continue
      ps -o command= -p "$p" 2>/dev/null | grep -qF "$dist" || continue
      pids="$(gw_add_pid "$pids" "$p")"
    done
  fi
  [ -n "$pids" ] && printf '%s\n' "$pids"
  return 0
}

# ── 停止序列（唯一实现：TERM → 有界等待 → KILL）──────────────────────────
# gw_stop_pid <label> <pid> [--force]
gw_stop_pid() {
  local label="$1" pid="$2" mode="${3:-}" grace="${GW_STOP_GRACE:-10}" i=0
  gw_alive "$pid" || return 0
  if [ "$mode" = "--force" ]; then
    gw_log "强制停止 ${label} (PID: ${pid})..."
    kill -9 "$pid" 2>/dev/null || true
    return 0
  fi
  gw_log "优雅停止 ${label} (PID: ${pid})..."
  kill "$pid" 2>/dev/null || true
  while [ "$i" -lt "$grace" ]; do
    gw_alive "$pid" || return 0
    sleep 1
    i=$((i + 1))
  done
  gw_warn "进程未响应 (PID ${pid})，强制终止..."
  kill -9 "$pid" 2>/dev/null || true
  sleep 1
  return 0
}

# gw_stop_pids <label> <newline-list> [--force] —— stdout: 被处理的 PID 数（每 PID 一行日志）
gw_stop_pids() {
  local label="$1" list="$2" mode="${3:-}" pid n=0
  while IFS= read -r pid; do
    [ -n "$pid" ] || continue
    gw_stop_pid "$label" "$pid" "$mode"
    n=$((n + 1))
  done <<<"$list"
  printf '%s\n' "$n"
}

# ── 健康等待（唯一实现）─────────────────────────────────────────────────
# gw_wait_http <url> <max_attempts> [interval_s] [--linear]
#   --linear：第 i 次失败后 sleep i 秒（verify-health.sh 的既有节拍）；缺省 = 固定 interval。
gw_wait_http() {
  local url="${1:?gw_wait_http 需要 url}" max="${2:-15}" interval="${3:-1}" mode="${4:-}" i=1
  while [ "$i" -le "$max" ]; do
    if curl -sf -m 3 "$url" >/dev/null 2>&1; then return 0; fi
    if [ "$mode" = "--linear" ]; then sleep "$i"; else sleep "$interval"; fi
    i=$((i + 1))
  done
  return 1
}

# gw_http_ok <url> —— 单次探针（0=200）
gw_http_ok() { curl -sf -m 3 "${1:?gw_http_ok 需要 url}" >/dev/null 2>&1; }

# gw_wait_health <SERVER_ADDR> <max_attempts> [interval_s] —— 探 `http://<addr>/health`
gw_wait_health() {
  local addr="${1:?gw_wait_health 需要 SERVER_ADDR}"
  shift
  gw_wait_http "http://${addr}/health" "$@"
}

# gw_wait_process_http <pid> <url> <max_attempts> [interval_s]
#   0=就绪（进程存活 ∧ URL 200）；2=进程提前退出；1=超时
gw_wait_process_http() {
  local pid="$1" url="${2:?}" max="${3:-30}" interval="${4:-1}" i=1
  while [ "$i" -le "$max" ]; do
    gw_alive "$pid" || return 2
    if gw_http_ok "$url"; then return 0; fi
    sleep "$interval"
    i=$((i + 1))
  done
  return 1
}

# gw_wait_process_health <pid> <SERVER_ADDR> <max_attempts> [interval_s] —— 同上，探 /health
gw_wait_process_health() {
  local pid="$1" addr="${2:?}"
  shift 2
  gw_wait_process_http "$pid" "http://${addr}/health" "$@"
}

# ── 注册表读写（通道唯一 = ns-ports.sh 契约）─────────────────────────────
# gw_registry_set_backend <ns> <be_pid> [db_name] —— 写 BE PID，保留既有 FE/SSO/DB_NAME 键
gw_registry_set_backend() {
  local ns="${1:?gw_registry_set_backend 需要 ns}" be="${2:-}" db="${3:-}" fe sso
  fe="$(ns_registry_pid "$ns" GATEWAY_FE_PID 2>/dev/null || true)"
  sso="$(ns_registry_pid "$ns" SSO_PID 2>/dev/null || true)"
  [ -n "$db" ] || db="$(ns_registry_pid "$ns" DB_NAME 2>/dev/null || true)"
  ns_registry_write "$ns" "$be" "$fe" "$sso" "$db"
}

# gw_db_name <DATABASE_URL> —— 库名（去 query；空入参 ⇒ 空出参）
gw_db_name() {
  local url="${1:-}"
  [ -n "$url" ] || return 0
  printf '%s' "$url" | sed -n 's/.*\/\([^?]*\).*/\1/p'
}

# ── gateway 重启锁（目录原子锁；持有者已死可回收）───────────────────────
# 语义与既有 `scripts/gateway/.restart.lock.d` 一致（覆盖**整轮 restart**，与 ns 启动窗口锁
# `ns_lock_acquire` 不同轴）。MUST NOT 用 flock（macOS 无 util-linux）。
gw_restart_lock_dir() { printf '%s' "${GW_RESTART_LOCK_DIR:-${PROJECT_ROOT}/scripts/gateway/.restart.lock.d}"; }

# gw_restart_lock_acquire —— 0=已持锁；1=失败（已打印持有者）
gw_restart_lock_acquire() {
  local d holder
  d="$(gw_restart_lock_dir)"
  while ! mkdir "$d" 2>/dev/null; do
    holder="$(cat "${d}/pid" 2>/dev/null || true)"
    if [ -n "$holder" ] && ! kill -0 "$holder" 2>/dev/null; then
      gw_log "回收陈旧重启锁（持有者 PID ${holder} 已退出）"
      rm -rf "$d"
      continue
    fi
    gw_log "检测到另一个 Gateway 重启正在进行（持有者 ${holder:-未知}），等待其完成..."
    sleep 1
  done
  echo $$ > "${d}/pid"
  return 0
}

gw_restart_lock_release() { rm -rf "$(gw_restart_lock_dir)" 2>/dev/null || true; }

# ── SSO 依赖前置（package restart 语义；唯一实现）────────────────────────
# gw_ensure_sso <ns> [tier] —— SSO_SERVICE_URL 指向独立进程时探测 /health，未就绪自动拉起
#   （embedded / 自指 / 未配置 = 无需独立 SSO ⇒ 幂等返回 0）。形态单一事实源 = sso-mode.conf。
gw_ensure_sso() {
  local ns="${1:-}" tier="${2:-dev}"
  [ -n "$ns" ] || return 0
  local env_file
  if [ -f "${_GW_LIFECYCLE_LIB_DIR}/ns-env-file.sh" ]; then
    # shellcheck source=scripts/lib/ns-env-file.sh
    . "${_GW_LIFECYCLE_LIB_DIR}/ns-env-file.sh"
    env_file="$(ns_env_file "$ns" "$tier")" || env_file="${PROJECT_ROOT}/Deploy/${ns}/.env"
  else
    env_file="${PROJECT_ROOT}/Deploy/${ns}/.env"
  fi
  if [ -f "$env_file" ]; then
    set -a
    # shellcheck disable=SC1090
    source "$env_file" 2>/dev/null || true
    set +a
  fi

  # embedded = Gateway 内嵌 PDP（feature `sso`），无独立 gateway-sso 进程
  if [ "${SSO_MODE:-}" = "embedded" ]; then
    gw_log "SSO_MODE=embedded（内嵌 PDP），跳过独立 SSO 检查/拉起"
    return 0
  fi
  local sso_url="${SSO_SERVICE_URL:-}" server_addr="${SERVER_ADDR:-}"
  if [ -z "$sso_url" ]; then
    gw_log "SSO_SERVICE_URL 未配置（standalone 形态），跳过 SSO 检查"
    return 0
  fi
  local sso_port="${sso_url##*:}" self_port="${server_addr##*:}"
  sso_port="${sso_port%/}"
  if [ "$sso_port" = "$self_port" ]; then
    gw_log "SSO_SERVICE_URL 指向 Gateway 自身（内嵌 PDP），跳过独立 SSO 检查"
    return 0
  fi
  local health_url="${sso_url%/}/health"
  if gw_http_ok "$health_url"; then
    gw_ok "SSO 依赖健康 (${health_url})"
    return 0
  fi
  gw_warn "SSO 未就绪 (${health_url})，自动拉起独立 SSO..."
  local sso_script="${PROJECT_ROOT}/scripts/sso/restart.sh"
  if [ ! -f "$sso_script" ]; then
    gw_err "SSO 拉起脚本不存在: ${sso_script}"
    return 1
  fi
  # ns 发布包形态（remote SSO）：二进制/keys 在包内 ⇒ 传 --deploy-dir + 层 env 文件
  local sso_args=(--daemon)
  if [ -f "${PROJECT_ROOT}/Deploy/${ns}/bin/gateway-sso" ]; then
    sso_args+=(--deploy-dir "${PROJECT_ROOT}/Deploy/${ns}" --env-file "$env_file")
    gw_log "SSO 走 ns 发布包: Deploy/${ns}/bin/gateway-sso（env=${env_file#"${PROJECT_ROOT}"/}）"
  fi
  if ! bash "$sso_script" "${sso_args[@]}"; then
    gw_err "SSO 拉起失败: ${sso_script} ${sso_args[*]}"
    return 1
  fi
  if [ -n "$sso_port" ] && ! gw_http_ok "$health_url"; then
    gw_err "SSO 拉起后仍不可达: ${health_url}"
    return 1
  fi
  # 二次存活验证：sso restart.sh 返回 0 仅代表健康检查曾通过（SIGKILL 时 stdout 缓冲丢失，
  # sso.log 无 panic 痕迹）⇒ 等 2s 后同时校验进程与健康端点。
  sleep 2
  if [ -z "$(gw_pgrep_any 'gateway-sso')" ]; then
    gw_err "SSO 进程未存活（拉起后即退出），检查日志: ${PROJECT_ROOT}/scripts/sso/logs/sso.log"
    return 1
  fi
  if gw_http_ok "$health_url"; then
    gw_ok "SSO 已就绪 (${health_url})"
    return 0
  fi
  gw_err "SSO 拉起后仍不可达: ${health_url}"
  return 1
}
