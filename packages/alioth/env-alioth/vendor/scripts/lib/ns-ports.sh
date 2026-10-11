#!/bin/bash
# ns-ports.sh — Namespace 端口槽位 / 运行时注册表 / 端口抢占 / 启动锁
#
# 声明正本 = 各 ns 仓 .alioth/platform.conf（FE_PORT/BE_PORT/SSO_PORT，move-ns-registry-to-ns-repos）；
# 平台仓 Deploy/ports.conf = **派生缓存**（refresh-ns-registry 刷新）——它同时是**行序正本**：
# 模块端口带（本库 ns_module_port_band 与 TS modulePortBand）以缓存 0 基行序为共同序，两侧 MUST 同源。
# 分配算法（ns_ports_next_slot：FE=max+1 → BE=9001+2k → SSO=BE+1）= 平台层接口，写者落 ns 声明面。
#
# add-namespace-port-slot-table：Gateway/SSO dev 端口表机器可读化。
# fix-dev-port-selfheal-gaps：对齐扩到 5 键；运行时注册表单格式 + 活性判定；
# 端口抢占（归属判定→复检）；同 ns 并发启动锁。
#
# 用法（调用方需先设 PROJECT_ROOT 或当前目录为仓库根）:
#   source scripts/lib/ns-ports.sh
#   ns_port WZ fe            # → 41719（角色: fe|be|sso）
#   ns_ports_known WZ        # 0=已声明；1=未声明
#   ns_ports_file            # 钩子表路径（仅 NS_PORTS_CONF 显式设置时有意义）
#   ns_ports_list            # 枚举 "ns fe be sso" 行（供审计工具消费）
#   ns_ports_align_env WZ Deploy/WZ/.env        # 5 键对齐槽位
#   ns_registry_write WZ <be_pid> <fe_pid> <sso_pid>
#   ns_registry_status WZ / ns_registry_prune WZ
#   ns_registry_health WZ      # 严格逐服务健康判定（--status 汇总用）；0=全活
#   ns_pid_belongs WZ <pid>
#   ns_owner_valid WZ          # owner 记录（「谁持有本 ns 槽位」唯一真相源；见 lib/ns-owner.sh）
#   ns_owner_release_request WZ [timeout_s=20]   # 经 owner 请退（SIGTERM + 有界等待）
#   ns_port_preempt WZ 41719 Gateway-FE
#   ns_lock_acquire WZ / ns_lock_release WZ
#
# 测试钩子: NS_PORTS_CONF = 旧 4 列表路径（ns fe be sso）——读写全部走该表（隔离 worktree 用）；
#           NS_REPO_ROOT_OVERRIDE = ns 仓根（ns-repo.sh 钩子，声明面定位绕过符号链接）。
# 声明面读取：subshell source 取键（无正则解析，见 NO_REGEX_FOR_PARSING.md）。

# 调用方未设 PROJECT_ROOT 时自推导（直接执行自检/独立调用场景；显式设置优先）
if [ -z "${PROJECT_ROOT:-}" ]; then
  PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." 2>/dev/null && pwd)"
fi

# SSO 形态读取器（同目录声明面定位库；embedded 下影响 `SSO_SERVICE_URL` 的取值规则）
_NS_PORTS_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -f "${_NS_PORTS_LIB_DIR}/ns-repo.sh" ]; then
  # shellcheck source=ns-repo.sh
  source "${_NS_PORTS_LIB_DIR}/ns-repo.sh"
fi
if [ -f "${_NS_PORTS_LIB_DIR}/ns-sso-mode.sh" ]; then
  # shellcheck source=ns-sso-mode.sh
  source "${_NS_PORTS_LIB_DIR}/ns-sso-mode.sh"
fi
if ! command -v ns_repo_root >/dev/null 2>&1; then
  ns_repo_root() { echo "[ports] scripts/lib/ns-repo.sh 缺失——无法定位 ns 声明面" >&2; return 1; }
fi

# ns 落点解析（日志 / 运行态归属 ns 侧：Deploy/{ns}/logs|.runtime）——单一实现同目录
if [ -f "${_NS_PORTS_LIB_DIR}/ns-deploy-dir.sh" ]; then
  # shellcheck source=ns-deploy-dir.sh
  source "${_NS_PORTS_LIB_DIR}/ns-deploy-dir.sh"
fi
# 缺依赖时的退化（保持函数存在，避免调用方 set -u 崩）：仍走 ns 侧 Deploy 路径
if ! command -v ns_runtime_dir >/dev/null 2>&1; then
  ns_runtime_dir() { printf '%s/Deploy/%s/.runtime\n' "${PROJECT_ROOT:-.}" "${1:-}"; }
  ns_log_dir()     { printf '%s/Deploy/%s/logs%s\n' "${PROJECT_ROOT:-.}" "${1:-}" "${2:+/$2}"; }
fi

# owner 记录读取器（同目录同一真相源：**所有权** = 「谁持有本 ns 槽位」，优先于注册表
# 「诊断缓存」）。缺文件时退化：一律判「无有效 owner」⇒ 调用方走 legacy 路径（不崩、不静默改判）。
if [ -f "${_NS_PORTS_LIB_DIR}/ns-owner.sh" ]; then
  # shellcheck source=ns-owner.sh
  source "${_NS_PORTS_LIB_DIR}/ns-owner.sh"
fi
if ! command -v ns_owner_valid >/dev/null 2>&1; then
  ns_owner_file() { printf '%s/owner.conf\n' "$(ns_runtime_dir "${1:-}")"; }
  ns_owner_read() { return 1; }
  ns_owner_field() { printf '%s\n' ""; }
  ns_owner_lstart() { return 1; }
  ns_owner_valid() { return 1; }
  ns_owner_pid() { return 1; }
  ns_owner_write() { return 1; }
  ns_owner_clear() { return 1; }
  ns_owner_release_request() { return 2; }
fi

# ns_sso_service_port <ns> <be_slot> <sso_slot> —— 该形态下 `SSO_SERVICE_URL` 应取的有效端口。
#
# embedded：SSO（含 `/api/ngac/pdp/*`）由 Gateway 进程内挂载，而 Gateway 的 NGAC 客户端以
#   `SSO_SERVICE_URL` 作 PDP 基址（`Gateway/backend/src/apps.rs` 与 `pep/middleware.rs`：`{base}/api/ngac/pdp/*`）
#   ⇒ 该键 MUST 指向 **Gateway 自身**端口。若照槽位表写 SSO 槽位（= BE+1，embedded 下无进程监听），
#   PDP 不可达 ⇒ 全部 NGAC 校验 **fail-close 403**（2026-09-22 实证：ns:avic 启动后
#   `/api/apps` 返回「Policy decision service unavailable」，前端因此加载不到任何应用）。
# remote：SSO 由独立进程承载 ⇒ SSO 槽位。
# 未登记形态（`ns_sso_mode` 非零）⇒ 保持槽位表值（既有行为，不静默改判）。
ns_sso_service_port() {
  local ns="$1" be="$2" sso="$3" mode=""
  if declare -F ns_sso_mode >/dev/null 2>&1; then
    mode="$(ns_sso_mode "$ns" 2>/dev/null || true)"
  fi
  case "$mode" in
    embedded) echo "$be" ;;
    *) echo "$sso" ;;
  esac
}

ns_ports_file() {
  echo "${NS_PORTS_CONF:-${PROJECT_ROOT:-.}/Deploy/ports.conf}"
}

# ns_port <ns> <fe|be|sso> —— 输出槽位端口；未声明/未知角色返回非 0。
# 值正本 = ns 声明面（.alioth/platform.conf 的 FE_PORT/BE_PORT/SSO_PORT）；
# NS_PORTS_CONF 钩子设置时走旧 4 列表（测试/隔离通道）。
ns_port() {
  local ns="$1" role="$2" n fe be sso keyname
  case "$role" in fe) keyname=FE_PORT ;; be) keyname=BE_PORT ;; sso) keyname=SSO_PORT ;; *) return 1 ;; esac
  if [ -n "${NS_PORTS_CONF:-}" ]; then
    [ -f "$(ns_ports_file)" ] || return 1
    while read -r n fe be sso; do
      [[ -z "$n" || "$n" == \#* ]] && continue
      [ "$n" = "$ns" ] || continue
      case "$role" in
        fe) echo "$fe" ;;
        be) echo "$be" ;;
        sso) echo "$sso" ;;
      esac
      return 0
    done < "$(ns_ports_file)"
    return 1
  fi
  local conf fe_v be_v sso_v val=""
  conf="$(ns_repo_conf "$ns")" || return 1
  { read -r fe_v; read -r be_v; read -r sso_v; } < <(
    (source "$conf" >/dev/null 2>&1; printf '%s\n%s\n%s\n' "${FE_PORT:-}" "${BE_PORT:-}" "${SSO_PORT:-}")
  )
  case "$role" in fe) val="$fe_v" ;; be) val="$be_v" ;; sso) val="$sso_v" ;; esac
  if [ -z "$val" ]; then
    echo "[ports] ${ns} 的 ${conf} 未声明 ${keyname}——MUST 显式声明（不做默认推断）" >&2
    return 1
  fi
  echo "$val"
}

# ns_ports_known <ns> —— 0=已声明
ns_ports_known() {
  ns_port "$1" be >/dev/null 2>&1
}

# ns_ports_list —— 枚举已声明行（"ns fe be sso"）；声明面均不可读返回 0 且无输出。
# 行序正本 = 平台派生缓存 Deploy/ports.conf（freshness 门禁保其与声明面一致）；
# 值正本 = ns 声明面（缓存漂移由门禁检出）。缓存缺失 → rc=1（fail-loud；行序无 fallback）。
ns_ports_list() {
  local n fe be sso
  if [ -n "${NS_PORTS_CONF:-}" ]; then
    [ -f "$(ns_ports_file)" ] || return 0
    while read -r n fe be sso; do
      [[ -z "$n" || "$n" == \#* ]] && continue
      printf '%s %s %s %s\n' "$n" "$fe" "$be" "$sso"
    done < "$(ns_ports_file)"
    return 0
  fi
  local cache="${PROJECT_ROOT:-.}/Deploy/ports.conf"
  if [ ! -f "$cache" ]; then
    echo "[ports] 派生缓存缺失：${cache}——运行 bash scripts/gen/refresh-ns-registry.sh 刷新" >&2
    return 1
  fi
  while read -r n fe be sso; do
    [[ -z "$n" || "$n" == \#* ]] && continue
    if fe="$(ns_port "$n" fe 2>/dev/null)" && be="$(ns_port "$n" be 2>/dev/null)" \
       && sso="$(ns_port "$n" sso 2>/dev/null)"; then
      printf '%s %s %s %s\n' "$n" "$fe" "$be" "$sso"
    fi
  done < "$cache"
}

# ns_module_port_band <ns> —— 模块 dev server 端口带（"min max"，闭区间）。
#
# 带 = `6500 + 100 * idx`，idx = 该 ns 在派生缓存的 **0 基行序**。行序正本 = Deploy/ports.conf——
# TS 侧 `check-namespace-frontend.ts::modulePortBand` 独立自算同一行序，两侧 MUST 同源
# （行序即缓存内容 ⇒ freshness 门禁守双侧一致）。为什么不从 5xxx 起：PG 5432 / Vite 默认 5173 /
# 镜像站 6464 均在 5xxx-6xxx。正本 = openspec/specs/dev-tooling :: module-dev-port-namespace-band；
# 判据 = `check-namespace-frontend.ts` 的 `dev-port.band` / `dev-port.unique`。
ns_module_port_band() {
  local ns="$1" n _fe _be _sso idx=0 base
  local cache="${PROJECT_ROOT:-.}/Deploy/ports.conf"
  if [ -n "${NS_PORTS_CONF:-}" ]; then cache="$(ns_ports_file)"; fi
  [ -f "$cache" ] || { echo "[ports] 行序正本缺失：${cache}——端口带判据不可用" >&2; return 1; }
  while read -r n _fe _be _sso; do
    [[ -z "$n" || "$n" == \#* ]] && continue
    if [ "$n" = "$ns" ]; then
      base=$((6500 + 100 * idx))
      printf '%s %s\n' "$base" "$((base + 99))"
      return 0
    fi
    idx=$((idx + 1))
  done < "$cache"
  return 1
}

# ns_absolutize_path <base_dir> <value> —— 相对路径按基准目录绝对化（已绝对 / 空值原样返回）。
#
# 用途：`Deploy/{ns}/.env` 的密钥路径**按设计写相对名**（`release-to-namespace.sh` 生成——
# 发布包自包含，启动器在 Deploy 目录内运行），而消费进程 CWD ≠ `Deploy/{ns}`（Gateway 二进制
# CWD = `Gateway/backend`）⇒ 消费侧 MUST 收口绝对化。
#
# 唯一实现：启动器（dev-gateway 等）MUST 经本函数，不得各自内联拼路径。
ns_absolutize_path() {
  local base="$1" val="${2:-}"
  [ -z "$val" ] && return 0
  case "$val" in
    /*) printf '%s' "$val" ;;
    *) printf '%s/%s' "${base%/}" "$val" ;;
  esac
}

# ns_ports_check <ns> <be_port> <fe_port> <sso_port> —— 0=与槽位一致;1=漂移(逐项输出)
# 未登记 ns: 输出提示并返回 0（调用方按其既有策略继续，如 dev-gateway 跳过校验）。
ns_ports_check() {
  local ns="$1" be_actual="$2" fe_actual="$3" sso_actual="$4"
  local eb ef es ok=1
  if ! ns_ports_known "$ns"; then
    echo "[ports] ${ns} 未声明槽位（ns 仓 .alioth/platform.conf）——跳过槽位校验（建议声明后启用）"
    return 0
  fi
  eb="$(ns_port "$ns" be)"
  ef="$(ns_port "$ns" fe)"
  es="$(ns_port "$ns" sso)"
  # SSO_SERVICE_URL 的期望值随形态（embedded ⇒ Gateway 自身端口；见 ns_sso_service_port）
  es="$(ns_sso_service_port "$ns" "$eb" "$es")"
  [ "$be_actual" != "$eb" ] && { echo "[ports] ${ns} SERVER_ADDR 端口 ${be_actual} ≠ 槽位 ${eb}"; ok=0; }
  [ "$fe_actual" != "$ef" ] && { echo "[ports] ${ns} VITE_PORT ${fe_actual} ≠ 槽位 ${ef}"; ok=0; }
  [ "$sso_actual" != "$es" ] && { echo "[ports] ${ns} SSO_SERVICE_URL 端口 ${sso_actual} ≠ ${es}"; ok=0; }
  [ "$ok" = 0 ] && return 1
  return 0
}

# ── 运行时注册表（Deploy/{ns}/.runtime/{ns}.pid）────────────────────────────
# 唯一格式：`KEY=value`（NAMESPACE/DB_NAME/SSO_PID/GATEWAY_BE_PID/GATEWAY_FE_PID）。
# 读取容忍 legacy 裸 PID（无 `=` 的行）——按其唯一语义视作 GATEWAY_BE_PID 并告警。
# 归属：运行态随 `Deploy/{ns}` 落 **ns 侧**（2026-09-25 用户裁定「NS-Log 都归属各个 NS 的路径」）。
# NS_RUNTIME_ROOT / NS_LOGS_ROOT 可覆写（测试/隔离 worktree；语义 = 一个含 `<ns>` 条目的根）。

ns_registry_file() {
  if [ -n "${NS_RUNTIME_ROOT:-}" ]; then
    echo "${NS_RUNTIME_ROOT}/${1}.pid"
  else
    echo "$(ns_runtime_dir "${1}")/${1}.pid"
  fi
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
# 判据顺序（add-ns-slot-ownership-supervisor：owner 记录是**所有权**唯一真相源，优先采信）：
#   ① owner 记录：`ns_owner_valid` ∧ pid == OWNER_PID ⇒ 归属成立。owner 的身份已由
#      「pid 活 ∧ lstart 逐字相同」双匹配保证，故无需再过 cmdline 启发式；
#   ② legacy/组件进程：一方进程身份（ns_pid_looks_first_party）∧（per-service pid 文件 ∨
#      注册表）登记——注册表降级为诊断缓存，但仍是**回退**归属判据（无 owner 记录的实例）。
ns_pid_belongs() {
  local ns="$1" pid="$2" f kv owner
  ns_alive "$pid" || return 1
  owner="$(ns_owner_pid "$ns" 2>/dev/null || true)"
  if [ -n "$owner" ] && [ "$owner" = "$pid" ]; then
    return 0
  fi
  ns_pid_looks_first_party "$ns" "$pid" || return 1
  local logs_root
  if [ -n "${NS_LOGS_ROOT:-}" ]; then logs_root="${NS_LOGS_ROOT}/${ns}"; else logs_root="$(ns_log_dir "$ns")"; fi
  for f in "${logs_root}"/*.pid; do
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

# ns_registry_health <ns> —— **严格**健康判定（供 --status 汇总行使用）。
#
# 与 ns_registry_status 的区别：后者 rc=0 只表示「**至少一个**记录进程存活」（用于
# ns_registry_prune 的"是否有活进程"语义），故 BE 已死而 FE/SSO 尚存时它也 rc=0——
# 直接拿它当汇总判据会打印 `✅ All services running` 而实际后端已死（2026-09-23 502
# 事故现场即如此）。本函数按**逐服务**判定：任一必需服务不达标即 rc=1。
#
# 0 = 全部必需服务「存活 ∧ 身份确属本项目」；1 = 存在 stale / 未运行 / PID 复用。
# stdout: 每条不达标服务一行 `<label>: <原因>`（全活时无输出）——调用方据此打印明细。
# 必需集合：GATEWAY_BE_PID / GATEWAY_FE_PID 恒必需；SSO_PID 仅 remote 形态必需
#（embedded 下 SSO 由 Gateway 进程内承载，注册表按设计写空，不得判为异常）。
ns_registry_health() {
  local ns="$1" kv mode="" bad=0 spec key label pid line
  if declare -F ns_sso_mode >/dev/null 2>&1; then
    mode="$(ns_sso_mode "$ns" 2>/dev/null || true)"
  fi
  if ! kv="$(ns_registry_read "$ns" 2>/dev/null)"; then
    echo "registry: 无记录（$(ns_registry_file "$ns")）"
    return 1
  fi
  for spec in "GATEWAY_BE_PID|Gateway BE" "GATEWAY_FE_PID|Gateway FE" "SSO_PID|SSO"; do
    key="${spec%%|*}"; label="${spec#*|}"
    pid=""
    while IFS= read -r line; do
      case "$line" in "${key}="*) pid="${line#*=}"; break ;; esac
    done <<<"$kv"
    # embedded 形态下 SSO 无独立进程：注册表空值是设计值，不算异常
    if [ "$key" = "SSO_PID" ] && [ "$mode" != "remote" ] && [ -z "$pid" ]; then
      continue
    fi
    if [ -z "$pid" ]; then
      echo "${label}: 未记录（服务未启动）"
      bad=1
    elif ! ns_alive "$pid"; then
      echo "${label}: PID ${pid} 已死（stale）"
      bad=1
    elif ! ns_pid_looks_first_party "$ns" "$pid"; then
      echo "${label}: PID ${pid} 存活但非本项目进程（疑似 PID 复用）"
      bad=1
    fi
  done
  return "$bad"
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
# 语义（2026-09-09 用户裁决；2026-09-29 owner 契约收口）：启动脚本抢占所需端口。
# 处置顺序（add-ns-slot-ownership-supervisor D3）：
#   ① 容器平面 nginx 守卫（MUST NOT 抢占）；② FE 槽位归属无法判定 ⇒ fail-loud（MUST NOT 抢占）；
#   ③ 本 ns 有**有效 owner 记录**且目标端口 ∈ owner.SLOTS ⇒ 经 `ns_owner_release_request`
#      请 owner 自清（SIGTERM + 有界等待，默认 20s，可 `NS_OWNER_RELEASE_TIMEOUT` 覆写）后重探；
#      MUST NOT 直接对 owner 或其子进程投信号（盲杀会被 supervisor 的 on-failure 策略重拉 ⇒ 抢拉互搏）；
#   ④ 其余（无有效 owner / 端口不属 owner / 请退后残留孤儿）⇒ 保留既有盲杀，但日志标记
#      `legacy takeover (…)`；`NS_NO_BLIND_PREEMPT=1` 时改判 fail-loud（非零 + 指名持有者，MUST NOT 投信号）。
# 归属判定只用于分级告警（本 ns 旧实例 / 非本 ns 进程），不改变"抢占"这一处置；
# 抢占后强制复检——抢不回即 fail-loud（supervisor 托管/权限不足的真实出口）。
# ns_port_owner <port> —— 宿主上监听该端口的进程名（无占用/无法判定 ⇒ 空输出，rc=0）
# ns_port_owner <port> —— 占用者身份（两行：`name=<可执行体名>` / `args=<命令行前 120 字>`；无占用 ⇒ 两行空值）
# 唯一实现（供 ns_port_preempt 的归属守卫与容器平面 {ns-port} 预检共用；MUST NOT 在别处再写一份
# 占用者判定）。**按可执行体/argv[0] 判定而非 `ss` 的线程名**：`ss -ltnp` 对多线程进程报的是线程名
# （实测 node 报 `MainThread`）⇒ 名字面不可靠；故取 PID 后读 `/proc/<pid>/exe` 与 `ps -o args=`。
# ns_ports_ingress_kv —— 每 ns 入站端口 `{ns-port}`（= FE 槽位）的 `ns=port` 行（唯一实现；
# 供容器平面生成器（经 CLI `ns-app-host.sh ports --json`）与预检消费）
ns_ports_ingress_kv() {
  local n fe be sso
  if [ -n "${NS_PORTS_CONF:-}" ]; then
    [ -f "$(ns_ports_file)" ] || return 1
    while read -r n fe be sso; do
      case "$n" in ''|'#'*) continue ;; esac
      [ -n "$fe" ] || continue
      printf '%s=%s\n' "$n" "$fe"
    done < "$(ns_ports_file)"
    return 0
  fi
  local rows
  rows="$(ns_ports_list)" || return 1
  while read -r n fe be sso; do
    [ -n "$n" ] || continue
    [ -n "$fe" ] || continue
    printf '%s=%s\n' "$n" "$fe"
  done <<< "$rows"
}

# ns_port_listen_addrs <port> —— 回显该端口的监听本地地址（每行一个；无监听 ⇒ 空输出）
# 唯一实现（`ns_port_listening` / `ns_port_loopback_only` / `nginx-gen` 同机拓扑判定共用；
# MUST NOT 在别处再写一份 ss/lsof 判据）：Linux 走 `ss -ltnH`；BSD/macOS **无 `ss`** ⇒ 退化
# `lsof -nP -iTCP -sTCP:LISTEN`（2026-09-27 实测：只认 `ss` 会让「已监听」在 macOS 上恒假 ——
# 占用守卫因此静默失效并直接落到抢占分支）。
# 2026-10-09 补：macOS 装了 homebrew iproute2mac 时 `command -v ss` 为真，但 shim 的 `-l`
# 不过滤 LISTEN（输出 ESTAB 行）⇒ 监听判定同样恒假 ⇒ Darwin 恒走 lsof 分支。
ns_port_listen_addrs() {
  local port="${1:-}"
  [ -n "$port" ] || return 1
  if [ "$(uname -s)" != "Darwin" ] && command -v ss >/dev/null 2>&1; then
    ss -ltnH 2>/dev/null | awk -v p=":${port}" '$4 ~ (p "$") { print $4 }'
    return 0
  fi
  command -v lsof >/dev/null 2>&1 || return 1
  lsof -nP -iTCP -sTCP:LISTEN 2>/dev/null |
    awk -v p=":${port}" 'NR > 1 { for (i = 9; i <= NF; i++) if ($i ~ (p "$")) { print $i; break } }'
}

# ns_port_listening <port> —— 宿主是否已有监听该端口的 socket（0 = 是）
# 唯一实现（容器平面 `rt_port_listening` 委托到此；MUST NOT 在别处再写一份 ss 判据）
ns_port_listening() {
  [ -n "$(ns_port_listen_addrs "${1:-}")" ]
}

# ns_port_loopback_only <port> —— 0 = 该端口**已监听**且全部监听地址均为回环
# 消费方 = `nginx-gen.ts` 的同机拓扑判定（`--listen-addr` 下应用让出对外地址给边缘块）
ns_port_loopback_only() {
  local addrs="" bad=""
  addrs="$(ns_port_listen_addrs "${1:-}")"
  [ -n "$addrs" ] || return 1
  bad="$(printf '%s\n' "$addrs" | grep -Ev '^(\[::1\]|127\.0\.0\.1|localhost):' || true)"
  [ -z "$bad" ]
}

ns_port_owner() {
  local port="$1" pid="" args="" exe="" a0=""
  pid="$(lsof -ti ":${port}" 2>/dev/null | head -1)"
  if [ -z "$pid" ] && command -v ss >/dev/null 2>&1; then
    pid="$(ss -ltnp 2>/dev/null | awk -v pat=":${port}$" '
      $4 ~ pat { if (match($0, /pid=[0-9]+/)) { print substr($0, RSTART + 4, RLENGTH - 4); exit } }')"
  fi
  if [ -n "$pid" ]; then
    args="$(ps -p "$pid" -o args= 2>/dev/null | head -1)"
    exe="$(basename "$(readlink -f "/proc/${pid}/exe" 2>/dev/null || true)" 2>/dev/null || true)"
    a0="${args%% *}"; a0="${a0##*/}"
    [ -n "$a0" ] || a0="$exe"
  fi
  printf 'name=%s\nargs=%s\n' "$a0" "$(printf '%s' "$args" | cut -c1-120)"
  return 0
}

ns_port_preempt() {
  local ns="$1" port="$2" label="${3:-dev}"
  local pids p cmd note owner owner_name owner_args
  local owner_pid owner_slots reason="" rel_rc rel_timeout
  # 容器平面入站守卫（add-per-ns-ingress-and-remote-control D13）：FE 槽位自该 change 起是双用途端口，
  # 持有者为 nginx 时 MUST NOT 抢占——抢占会杀掉对外路由（站点静默消失），且该进程属容器平面。
  owner="$(ns_port_owner "$port")"
  owner_name="$(printf '%s\n' "$owner" | sed -n 's/^name=//p')"
  owner_args="$(printf '%s\n' "$owner" | sed -n 's/^args=//p')"
  case "$owner_name" in
    nginx|nginx:*)
      echo "❌ 端口 :${port}（${label}）由 **nginx** 持有（容器平面每 ns 入站路由 {ns-port}）：两平面同机互斥。" >&2
      echo "   占用者: ${owner_args:-未知}" >&2
      echo "   处置（MUST NOT 抢占）：① 该机保留容器平面 ⇒ 改在另一台机跑 dev/pre；或 ② 先停用该 ns 的容器平面入站（撤销生成的 nginx 配置并 reload）再启动宿主进程面。" >&2
      return 1
      ;;
  esac
  # FE 槽位保守策略：占用但归属**无法判定**时 MUST NOT 抢占（无法排除持有者是容器平面 nginx）
  if ns_port_listening "$port" && [ -z "$owner_name" ]; then
    case "$label" in
      *FE*|*fe*)
        echo "❌ 端口 :${port}（${label}）被占用但归属无法判定：该槽位是容器平面 {ns-port} 候选（nginx 持有即承载对外路由）⇒ MUST NOT 抢占。" >&2
        echo "   处置：人工确认占用者（lsof -i :$port / ss -ltnp）后再决定；或改在另一台机跑该平面。" >&2
        return 1
        ;;
    esac
  fi
  # 可见占位者（legacy 盲杀的输入；owner 分支同样要求非空——无可见占位者时 MUST NOT 向 owner
  # 投递请退：无法证明其组件确实占着该端口，且"探针与 lsof 结论不一致"时不误停整个 ns 栈）
  pids="$(lsof -ti :"$port" 2>/dev/null || true)"
  # ── ③ owner 契约（唯一真相源）：有效 owner 且该端口 ∈ owner.SLOTS ⇒ 请 owner 自清 ──
  reason="no valid owner record"
  if [ -n "$pids" ] && ns_owner_valid "$ns"; then
    reason="port not in owner SLOTS"
    owner_pid="$(ns_owner_field "$ns" OWNER_PID)"
    owner_slots="$(ns_owner_field "$ns" SLOTS)"
    case ",${owner_slots}," in
      *",${port},"*)
        rel_timeout="${NS_OWNER_RELEASE_TIMEOUT:-20}"
        echo "  [port] :$port (${label}) 归属本 ns owner 记录（$(ns_owner_file "$ns")；owner_pid=${owner_pid}；SLOTS=${owner_slots}）→ 经 owner 请退" >&2
        rel_rc=0
        ns_owner_release_request "$ns" "$rel_timeout" || rel_rc=$?
        if [ "$rel_rc" = 0 ] && ! ns_port_listening "$port"; then
          echo "  [port] :$port (${label}) 经 owner 请退成功——端口已回收（owner_pid=${owner_pid}）" >&2
          return 0
        fi
        if [ "$rel_rc" = 1 ]; then
          echo "  [port] :$port (${label}) 经 owner 请退**超时**（owner_pid=${owner_pid} 未在 ${rel_timeout}s 内退出）——MUST NOT 盲杀" >&2
          echo "   理由：其子进程受 supervisor on-failure 策略托管，盲杀会被立即重拉（抢拉互搏）。" >&2
          echo "   处置：人工检查/终止该 owner（kill -TERM ${owner_pid}）后重试；或先停用本 ns 栈。" >&2
          return 1
        fi
        if ns_alive "$owner_pid"; then
          echo "  [port] :$port (${label}) owner 记录已消失（请退 rc=${rel_rc}）但 owner PID ${owner_pid} 仍存活，且端口仍被占用 ⇒ MUST NOT 盲杀" >&2
          echo "   处置：人工确认该进程归属（ps -p ${owner_pid} -o command=）后处置。" >&2
          return 1
        fi
        reason="orphan holder after owner release"
        echo "  [port] :$port (${label}) owner 已退出但端口仍被占用（孤儿占位进程）——转 legacy 判定" >&2
        ;;
    esac
  fi
  # ── ④ legacy 盲杀（无有效 owner 记录 / 端口不属 owner / 请退后孤儿）──
  [ -z "$pids" ] && return 0
  if [ "${NS_NO_BLIND_PREEMPT:-0}" = "1" ]; then
    echo "  [port] :$port (${label}) legacy takeover（${reason}）被 NS_NO_BLIND_PREEMPT=1 拒绝：不投递任何信号" >&2
    for p in $pids; do
      echo "    PID $p: $(ps -p "$p" -o command= 2>/dev/null | cut -c1-120 || echo 未知)" >&2
    done
    echo "   处置：经 owner 请退（scripts/lib/ns-owner.sh release-request <ns>）或人工确认占用者后重试" >&2
    return 1
  fi
  for p in $pids; do
    if ns_pid_belongs "$ns" "$p"; then
      note="本 ns 旧实例"
    else
      note="非本 ns 进程（可能属其他 ns / 其他 checkout / 第三方）"
    fi
    cmd="$(ps -p "$p" -o command= 2>/dev/null | cut -c1-120 || true)"
    echo "  [port] :$port (${label}) legacy takeover (${reason}) —— 被 PID $p 占用（${note}; cmd: ${cmd:-未知}）——自动抢占（用户裁决:启动脚本关闭占用进程）" >&2
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

# ── 同 ns 并发启动锁（Deploy/{ns}/.runtime/{ns}.lock）────────────────────────
# mkdir 原子创建；内容 = 持有者 PID。仅覆盖"启动窗口"（稳态运行不持锁 → 有意重启不受阻）。
ns_lock_dir() {
  if [ -n "${NS_RUNTIME_ROOT:-}" ]; then
    echo "${NS_RUNTIME_ROOT}/${1}.lock"
  else
    echo "$(ns_runtime_dir "${1}")/${1}.lock"
  fi
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
    echo "[ports] ${ns} 未声明槽位——跳过对齐（建议先经 ns 仓 .alioth/platform.conf 声明）"
    return 1
  fi
  [ -f "$env_file" ] || { echo "[ports] 环境文件不存在：${env_file}" >&2; return 1; }
  be="$(ns_port "$ns" be)"
  fe="$(ns_port "$ns" fe)"
  sso="$(ns_port "$ns" sso)"
  # SSO_SERVICE_URL 的有效端口随形态（embedded ⇒ Gateway 自身端口，见 ns_sso_service_port）
  sso="$(ns_sso_service_port "$ns" "$be" "$sso")"

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

# ns_ports_next_slot —— 依现存声明推算下一槽位（稳健取 max FE+1，容忍删行）：
# 输出 "fe be sso"（FE=41717+k、BE=9001+2k、SSO=BE+1 槽位序列）。
# 声明面/缓存均不可读时返回初槽（既有语义：首个 ns 从 41717 起）。
ns_ports_next_slot() {
  local n be fe sso maxfe=41716 row
  if [ -n "${NS_PORTS_CONF:-}" ]; then
    [ -f "$(ns_ports_file)" ] || { echo "41717 9001 9002"; return 0; }
    while read -r n fe be sso; do
      [[ -z "$n" || "$n" == \#* ]] && continue
      [ "$fe" -gt "$maxfe" ] 2>/dev/null && maxfe="$fe"
    done < "$(ns_ports_file)"
  else
    while read -r n fe be sso; do
      [ -n "$n" ] || continue
      [ "$fe" -gt "$maxfe" ] 2>/dev/null && maxfe="$fe"
    done < <(ns_ports_list 2>/dev/null)
  fi
  local new_fe=$((maxfe + 1))
  local new_be=$((9001 + 2 * (new_fe - 41717)))
  echo "${new_fe} ${new_be} $((new_be + 1))"
}

# ns_ports_register <ns> [mode] —— namespace 创建时登记（幂等）。
# 声明通道（默认）：写 ns 仓 .alioth/platform.conf 的 FE_PORT/BE_PORT/SSO_PORT（+ SSO_MODE，
#   缺省 embedded 且 MUST 显式落键），并刷新平台派生缓存（refresh-ns-registry 存在时）。
# 钩子通道（NS_PORTS_CONF 设置时）：写旧 4 列表（测试/隔离 worktree，行为同旧版）。
# 已声明 → 提示现值 rc=0；未声明 → 下一槽位写入。
ns_ports_register() {
  local ns="$1" mode="${2:-embedded}"
  if [ -n "${NS_PORTS_CONF:-}" ]; then
    local slot fe be sso
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
    echo "[ports] 已登记 ${ns} → FE=${fe} BE=${be} SSO=${sso}（NS_PORTS_CONF 钩子表）"
    return 0
  fi
  local root conf
  root="$(ns_repo_root "$ns")" || { echo "[ports] 无法定位 ${ns} 的 ns 仓根——登记失败" >&2; return 1; }
  conf="$root/.alioth/platform.conf"
  if ns_ports_known "$ns"; then
    echo "[ports] ${ns} 已声明槽位 FE=$(ns_port "$ns" fe) BE=$(ns_port "$ns" be) SSO=$(ns_port "$ns" sso)"
  else
    slot="$(ns_ports_next_slot)"
    read -r fe be sso <<<"$slot"
    local k v
    for kv in "FE_PORT=${fe}" "BE_PORT=${be}" "SSO_PORT=${sso}"; do
      k="${kv%%=*}"; v="${kv#*=}"
      if grep -q "^${k}=" "$conf" 2>/dev/null; then
        # 畸形部分键场景的替换/补齐合一：awk 重建 + mv（可移植；BSD/GNU sed -i 形态分歧见 check-shell-portability）
        local tmpf
        tmpf="$(mktemp)"
        awk -v k="$k" -v v="$v" '
          $0 ~ "^"k"=" { print k "=" v; seen=1; next }
          { print }
          END { if (!seen) print k "=" v }
        ' "$conf" > "$tmpf" && mv "$tmpf" "$conf"
      else
        printf '\n%s=%s\n' "$k" "$v" >> "$conf"
      fi
    done
    echo "[ports] 已声明 ${ns} → FE=${fe} BE=${be} SSO=${sso}（${conf}）"
  fi
  if ! grep -q "^SSO_MODE=" "$conf" 2>/dev/null; then
    printf '\nSSO_MODE=%s\n' "$mode" >> "$conf"
  fi
  if [ -f "${PROJECT_ROOT}/scripts/gen/refresh-ns-registry.sh" ]; then
    bash "${PROJECT_ROOT}/scripts/gen/refresh-ns-registry.sh" >/dev/null 2>&1 || \
      echo "[ports] ⚠ 派生缓存刷新失败——门禁将报缓存 ⇔ 声明面漂移" >&2
  fi
  return 0
}

# 直接执行 = 自检（调试/冒烟；不依赖 dev 服务环境）
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  # 档位守卫：本自检的判据根 = 平台仓派生缓存 `Deploy/ports.conf` + `Pre-Proc/{ns}` 符号链接挂载面
  # （见 ^ 用法说明与 ns-repo.sh）。ns 仓档位按设计**不持有**该判据根
  # （REPO_LAYOUT_SPEC.md 的同档位判据：「无 Deploy/ports.conf ⇒ ns 仓档位不适用」；本仓 `Pre-Proc/{ns}` 非符号链接）
  # ⇒ 在此档位运行时全部槽位读空、枚举为空，自检必假红。故按双档位惯例跳过判定，
  # 先例 = check-rust-dep-alignment.ts（判据根缺失 ⇒ ⏭ 跳过，判定归平台档位）。
  if [ ! -f "${PROJECT_ROOT}/Deploy/ports.conf" ]; then
    echo "⏭ ns-ports-selftest：ns 仓档位不持有平台判据根 Deploy/ports.conf —— 跳过判定（判定归平台档位）"
    exit 0
  fi
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
  #
  # decoy 形态 MUST 是**绝对路径系统二进制直接 exec**（2026-09-23 实测根因，勿改回子壳形态）：
  # `( cd /tmp && sleep 30 ) &` 里 bash 对子壳**不做 exec**（子壳体是 `cd &&` 列表），
  # 该子壳 argv 继承父 shell ⇒ `ps -p <pid> -o command=` 打印 `bash <本脚本路径>`。
  # 单独跑时调用形如 `bash scripts/lib/ns-ports.sh`（相对）⇒ argv 不含绝对路径，恰好通过；
  # 而 pre-commit 门禁用**绝对路径**调用（scripts/pre/gates.ts `ports-lib-selftest`：
  # `bash ${ROOT}/scripts/lib/ns-ports.sh`）⇒ argv 含 PROJECT_ROOT ⇒ 命中身份判据①
  # 「cmdline 含本仓库绝对路径」⇒ decoy 被误判一方进程（三条 FAIL 均为 期望 1 实际 0）。
  # `/bin/sleep 30 &` 由 bash 直接 exec ⇒ cmdline 恒为 `/bin/sleep 30`，与调用方式无关。
  # ⚠ 只修 fixture 形态，MUST NOT 放宽 ns_pid_looks_first_party 判据（安全生产判据）。
  /bin/sleep 30 &
  UNRELATED_PID=$!
  sleep 0.3
  decoy_cmd="$(ps -p "$UNRELATED_PID" -o command= 2>/dev/null)"
  case "$decoy_cmd" in
    *"${PROJECT_ROOT:-/nonexistent}"*)
      echo "  FAIL: decoy 命令行含仓库路径（fixture 失效——见上方注释，须用绝对路径直接 exec）"; local_fail=1 ;;
  esac
  assert "非本项目进程：身份判据拒绝" 1 ns_pid_looks_first_party WZ "$UNRELATED_PID"
  printf 'NAMESPACE=WZ\nGATEWAY_BE_PID=%s\n' "$UNRELATED_PID" > "$tmp/runtime/WZ.pid"
  assert "已登记但非本项目进程：不构成归属（不杀）" 1 ns_pid_belongs WZ "$UNRELATED_PID"
  # ── 严格健康判定（fix-status-summary-lies）──
  # 汇总行不得在「部分服务 stale」时报 ✅：旧口径（ns_registry_status 的 "≥1 存活"）会，
  # 新口径（ns_registry_health 逐服务）不会——本组断言即该事故的回归门。
  health_has() { ns_registry_health WZ 2>/dev/null | grep -qF "$1"; }
  printf 'NAMESPACE=WZ\nGATEWAY_BE_PID=111\nGATEWAY_FE_PID=%s\nSSO_PID=%s\n' "$PARTY_PID" "$PARTY_PID" > "$tmp/runtime/WZ.pid"
  assert "BE 已死 + FE/SSO 存活 → status rc=0（事故现场假 ✅ 的来源）" 0 ns_registry_status WZ
  assert "同状态 → health rc=1（汇总必须报异常）" 1 ns_registry_health WZ
  assert "health 明细点名 stale 服务（BE）" 0 health_has "Gateway BE: PID 111 已死（stale）"
  printf 'NAMESPACE=WZ\nGATEWAY_BE_PID=%s\nGATEWAY_FE_PID=%s\nSSO_PID=%s\n' "$PARTY_PID" "$PARTY_PID" "$PARTY_PID" > "$tmp/runtime/WZ.pid"
  assert "三方全活 → health rc=0（保持 ✅ 与 rc=0）" 0 ns_registry_health WZ
  printf 'NAMESPACE=WZ\nGATEWAY_BE_PID=%s\nGATEWAY_FE_PID=%s\nSSO_PID=%s\n' "$PARTY_PID" "$UNRELATED_PID" "$PARTY_PID" > "$tmp/runtime/WZ.pid"
  assert "PID 复用（存活但非本项目）→ health rc=1" 1 ns_registry_health WZ
  printf 'NAMESPACE=WZ\nGATEWAY_BE_PID=%s\nGATEWAY_FE_PID=%s\n' "$PARTY_PID" "$PARTY_PID" > "$tmp/runtime/WZ.pid"
  if [ "$(ns_sso_mode WZ 2>/dev/null)" = "remote" ]; then
    assert "remote 形态（WZ）未记录 SSO → health rc=1（必需服务缺失）" 1 ns_registry_health WZ
  else
    assert "embedded 形态（WZ）空 SSO = 设计值 → 不判异常" 0 ns_registry_health WZ
  fi
  if [ "$(ns_sso_mode Alioth 2>/dev/null)" = "embedded" ]; then
    printf 'NAMESPACE=Alioth\nGATEWAY_BE_PID=111\nGATEWAY_FE_PID=111\nSSO_PID=\n' > "$tmp/runtime/Alioth.pid"
    no_sso() { ! ns_registry_health Alioth 2>/dev/null | grep -q 'SSO'; }
    assert "embedded（Alioth）空 SSO 明细无异常行" 0 no_sso
    rm -f "$tmp/runtime/Alioth.pid"
  fi
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
  preempt_rc() { # $1 期望 rc $2 日志文件 $3 描述 ...其余 = ns_port_preempt 参数
    local want="$1" logf="$2" desc="$3" got=0
    shift 3
    ns_port_preempt "$@" >"$logf" 2>&1 || got=$?
    if [ "$got" = "$want" ]; then echo "  ok: $desc"; else echo "  FAIL: $desc (期望 $want 实际 $got)"; local_fail=1; fi
  }
  log_has() { # $1 描述 $2 文件 $3 期望子串
    if grep -qF "$3" "$2" 2>/dev/null; then echo "  ok: $1"; else echo "  FAIL: $1（日志缺 '$3'）"; local_fail=1; fi
  }
  log_lacks() { # $1 描述 $2 文件 $3 禁止子串
    if grep -qF "$3" "$2" 2>/dev/null; then echo "  FAIL: $1（日志含 '$3'）"; local_fail=1; else echo "  ok: $1"; fi
  }
  DUMMY_PORT=$(( 46000 + ($$ % 500) ))
  bun -e "Bun.serve({port:${DUMMY_PORT},hostname:'127.0.0.1',fetch:()=>new Response('ok')})" >/dev/null 2>&1 &
  DUMMY_PID=$!
  for _ in $(seq 1 30); do lsof -ti :"$DUMMY_PORT" >/dev/null 2>&1 && break; sleep 0.2; done
  if [ -n "$(lsof -ti :"$DUMMY_PORT" 2>/dev/null)" ]; then
    preempt_rc 0 "$tmp/legacy-preempt.log" "抢占持占端口 rc=0" WZ "$DUMMY_PORT" TEST
    log_has "无有效 owner 记录 ⇒ 日志标记 legacy takeover" "$tmp/legacy-preempt.log" \
      "legacy takeover (no valid owner record)"
    assert "抢占后端口自由" 0 bash -c "! lsof -ti :${DUMMY_PORT} >/dev/null 2>&1"
    assert "被抢占进程已终止" 1 ns_alive "$DUMMY_PID"
  else
    echo "  FAIL: dummy listener 未起（跳过抢占用例）"; local_fail=1
  fi
  kill "$DUMMY_PID" 2>/dev/null || true
  assert "空闲端口抢占 rc=0 无副作用" 0 ns_port_preempt WZ "$DUMMY_PORT" TEST

  # owner 契约（add-ns-slot-ownership-supervisor D3）：有效 owner 且端口 ∈ owner.SLOTS
  # ⇒ 经 owner 请退（MUST NOT 直杀）；NS_NO_BLIND_PREEMPT=1 + 无 owner ⇒ fail-loud 不投信号。
  echo "端口抢占（owner 契约）："
  OWNER_PORT=$(( DUMMY_PORT + 1 ))
  bun -e "Bun.serve({port:${OWNER_PORT},hostname:'127.0.0.1',fetch:()=>new Response('ok')})" >/dev/null 2>&1 &
  OWNER_FIXTURE_PID=$!
  for _ in $(seq 1 30); do lsof -ti :"$OWNER_PORT" >/dev/null 2>&1 && break; sleep 0.2; done
  if [ -n "$(lsof -ti :"$OWNER_PORT" 2>/dev/null)" ]; then
    assert "owner 记录写入（fixture = 持占者本身；SLOTS 含该端口）" 0 \
      ns_owner_write WZ dev "$OWNER_FIXTURE_PID" fe,be,sso "$OWNER_PORT" "$tmp/runtime/supervise.sock"
    assert "owner 记录有效（pid 活 + lstart 匹配）" 0 ns_owner_valid WZ
    preempt_rc 0 "$tmp/owner-preempt.log" "有效 owner ⇒ 经 owner 请退 rc=0" WZ "$OWNER_PORT" TEST
    log_has "日志写明「经 owner 请退」" "$tmp/owner-preempt.log" "经 owner 请退"
    log_has "日志写明请退成功" "$tmp/owner-preempt.log" "经 owner 请退成功"
    log_lacks "未落 legacy 盲杀" "$tmp/owner-preempt.log" "legacy takeover"
    assert "owner（= 持占者）已被 TERM 终止" 1 ns_alive "$OWNER_FIXTURE_PID"
    assert "端口已释放" 0 bash -c "! lsof -ti :${OWNER_PORT} >/dev/null 2>&1"
    ns_owner_clear WZ "$OWNER_FIXTURE_PID" 2>/dev/null || true
    rm -f "$tmp/runtime/WZ.owner.conf"
  else
    echo "  FAIL: owner fixture listener 未起（跳过 owner 请退用例）"; local_fail=1
  fi
  kill "$OWNER_FIXTURE_PID" 2>/dev/null || true

  echo "端口抢占（NS_NO_BLIND_PREEMPT=1 严格模式；无 owner 记录）："
  bun -e "Bun.serve({port:${OWNER_PORT},hostname:'127.0.0.1',fetch:()=>new Response('ok')})" >/dev/null 2>&1 &
  STRICT_PID=$!
  for _ in $(seq 1 30); do lsof -ti :"$OWNER_PORT" >/dev/null 2>&1 && break; sleep 0.2; done
  if [ -n "$(lsof -ti :"$OWNER_PORT" 2>/dev/null)" ]; then
    NS_NO_BLIND_PREEMPT=1
    preempt_rc 1 "$tmp/strict-preempt.log" "无 owner + 严格模式 ⇒ rc=1 fail-loud" WZ "$OWNER_PORT" TEST
    unset NS_NO_BLIND_PREEMPT
    log_has "严格模式指名持有者" "$tmp/strict-preempt.log" "PID ${STRICT_PID}"
    assert "占位者未被投信号（仍存活）" 0 ns_alive "$STRICT_PID"
    assert "端口仍被占用（未被抢占）" 0 bash -c "lsof -ti :${OWNER_PORT} >/dev/null 2>&1"
  else
    echo "  FAIL: strict fixture listener 未起（跳过严格模式用例）"; local_fail=1
  fi
  kill "$STRICT_PID" 2>/dev/null || true

  # 路径绝对化：.env 相对密钥名 → Deploy 目录（收口判据；dev-gateway 启动链回归）
  assert_eq() { # $1 描述 $2 期望 $3 实际
    if [ "$2" = "$3" ]; then echo "  ok: $1"; else echo "  FAIL: $1 (期望 '$2' 实际 '$3')"; local_fail=1; fi
  }
  echo "路径绝对化："
  assert_eq "相对名 → 基准/名" "/d/Alioth/sso_jwt_public.pem" "$(ns_absolutize_path /d/Alioth sso_jwt_public.pem)"
  assert_eq "基准带尾斜杠 → 单斜杠拼接" "/d/Alioth/sso_jwt_public.pem" "$(ns_absolutize_path /d/Alioth/ sso_jwt_public.pem)"
  assert_eq "已绝对 → 原样不动" "/x/y.pem" "$(ns_absolutize_path /d/Alioth /x/y.pem)"
  assert_eq "空值 → 空（不产出基准斜杠）" "" "$(ns_absolutize_path /d/Alioth '')"

  # 下一槽位 + 登记（钩子通道）：临时表（含 Alioth 行 → 下一槽 Cosmic-Tools 41718/9003/9004）
  printf '# 注释\nAlioth 41717 9001 9002\n' > "$tmp/ports.conf"
  export NS_PORTS_CONF="$tmp/ports.conf"
  assert_eq "模块端口带：Alioth（idx=0）⇒ 6500 6599" "6500 6599" "$(ns_module_port_band Alioth)"
  assert_eq "模块端口带：未登记 ns ⇒ 无输出" "" "$(ns_module_port_band NoSuchNs 2>/dev/null)"
  assert "next 推算 FE=41718" 0 next_is "41718 9003 9004"
  assert "register 新 ns（钩子表）" 0 ns_ports_register Cosmic-Tools
  assert_eq "模块端口带：随行序推进（Cosmic-Tools idx=1）⇒ 6600 6699" "6600 6699" "$(ns_module_port_band Cosmic-Tools)"
  assert "登记行已写入" 0 grep -q '^Cosmic-Tools 41718 9003 9004$' "$tmp/ports.conf"
  assert "register 幂等跳过" 0 ns_ports_register Cosmic-Tools
  unset NS_PORTS_CONF

  # 声明通道：临时 ns 仓 fixture（NS_REPO_ROOT_OVERRIDE）——声明面写入 + 读取回环
  echo "声明面登记（NS_REPO_ROOT_OVERRIDE fixture）："
  mkdir -p "$tmp/fake-ns-repo/.alioth" "$tmp/fake-ns-repo/Pre-Proc/NewNS"
  printf 'META_DIR=../nowhere\n' > "$tmp/fake-ns-repo/.alioth/platform.conf"
  NS_REPO_ROOT_OVERRIDE="$tmp/fake-ns-repo"
  assert "register 新 ns（声明面，缺省 embedded 显式落键）" 0 ns_ports_register NewNS
  # override 全局劫持声明面定位 ⇒ next_slot 枚举为空 ⇒ 初槽（推进逻辑由上方钩子用例覆盖）
  assert_eq "声明面 FE_PORT（初槽）" "41717" "$(ns_port NewNS fe 2>/dev/null)"
  assert_eq "声明面 BE_PORT（初槽）" "9001" "$(ns_port NewNS be 2>/dev/null)"
  assert_eq "声明面 SSO_PORT（初槽）" "9002" "$(ns_port NewNS sso 2>/dev/null)"
  assert "SSO_MODE 显式落键" 0 grep -q '^SSO_MODE=embedded$' "$tmp/fake-ns-repo/.alioth/platform.conf"
  assert "register 幂等跳过" 0 ns_ports_register NewNS remote
  assert "幂等不改已声明 SSO_MODE" 0 grep -q '^SSO_MODE=embedded$' "$tmp/fake-ns-repo/.alioth/platform.conf"
  unset NS_REPO_ROOT_OVERRIDE

  unset NS_LOGS_ROOT NS_RUNTIME_ROOT
  rm -rf "$tmp"
  # shellcheck disable=SC2015 # 自检裁决：非空即通过，失败分支只在 set -e 之外显式退出
  [ "$local_fail" = 0 ] && echo "自检全部通过" || { echo "自检存在失败"; exit 1; }
fi
