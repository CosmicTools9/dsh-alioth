#!/bin/bash
# container-runtime.sh — namespace 应用容器托管（runtime hosting）单一实现库
#
# 目标：把 `Deploy/{ns}/` 发布包作为**容器实例**动态起停（一个 namespace 可并存多实例），
# 并以单一入站端口按二级域名（`pre-<ns>[-<label>].<domain_suffix>`）路由到对应实例。
#
# 唯一事实源:
#   · 配置表      = Deploy/runtime.conf（本库 rt_conf_get 是唯一 bash 解析实现）
#   · 端口槽位    = Deploy/ports.conf（经 scripts/lib/ns-ports.sh，本库不复制解析）
#   · SSO 形态    = Deploy/sso-mode.conf（经 scripts/lib/ns-sso-mode.sh）
#   · 实例注册表  = .runtime/runtime/<key>.json（唯一写者 = rt_registry_write；原子 tmp+mv）
#
# 容器后端（backend）:
#   · podman（Linux 默认；rootless 优先）——两种运行形态：
#       native    : 系统提供 newuidmap/newgidmap（uidmap 包）⇒ 直接调用 podman
#       single-id : 缺 newuidmap 且 /etc/subuid 有本人区间 ⇒ 经 userns 影子（单 ID 映射）
#                   运行 podman（scripts/runtime/podman-single-id-inner.sh），规避
#                   「rootless mode with multiple IDs: newuidmap not found」
#   · docker（macOS OrbStack / Docker Desktop，或 Linux Docker Engine）
#   选择：ALIOTH_CONTAINER_RUNTIME=podman|docker 显式指定；否则 PATH 上 podman 优先，再 docker。
#
# 网络与端口口径（add-per-ns-ingress-and-remote-control 起，用户裁决 2026-09-26）:
#   · 两种后端一律 `--network=host`（Linux；rootless pasta 下容器端口即宿主端口）
#   · **容器实例端口 = `Deploy/ports.conf` 的该 ns 三元组（BE/FE/SSO）**，与 dev 宿主进程面完全同构
#     ⇒ 零环境差异与漂移（同一份 env：SERVER_ADDR=127.0.0.1:<be>、FRONTEND_PORT=<fe>、SSO=<sso>）
#   · 由此：① 端口不再「分配」（原 instance_port_band 退役）；② 同机**同 ns 只能一个实例**（端口唯一）；
#     ③ 该 ns 的 dev / 容器 / 发布面静态服务三者同机互斥（抢占保护见 ENVIRONMENT_SPEC §6.9）；
#     ④ 对外入口 = 边缘 nginx（`listen {ns-port}` → `proxy_pass http://<container_host>:<{ns-port}>`，见 nginx-gen.ts）
#
# 用法:
#   source scripts/lib/container-runtime.sh
#   rt_config_dump                       # 打印配置表 key=value
#   rt_resolve_host pre-se-a.isahl.com   # → 实例 key（空 = 未登记/不合法）
#   rt_start SE [--label a] [--image X] [--pkg Deploy/SE]
#   rt_stop se-a / rt_status se-a / rt_list / rt_logs se-a --tail 50
#   rt_instance_host SE a                # → pre-se-a.isahl.com
#
# 退出码：0 成功；1 失败（fail-loud，错误写 stderr）

set -o pipefail

if [ -z "${PROJECT_ROOT:-}" ]; then
  PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." 2>/dev/null && pwd)"
fi

_RT_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# 端口槽位读取器（唯一实现；未登记 ns 由该库判定）
# shellcheck source=scripts/lib/ns-ports.sh
source "${_RT_LIB_DIR}/ns-ports.sh"
# SSO 形态读取器（唯一实现）
# shellcheck source=scripts/lib/ns-sso-mode.sh
source "${_RT_LIB_DIR}/ns-sso-mode.sh"
# env 分层解析（唯一实现；层→文件后缀规则 `ns_env_file_in`）——容器平面的 `--tier` 复用同一规则
if [ -f "${_RT_LIB_DIR}/../db/lib/ns-db-name.sh" ]; then
  # shellcheck source=scripts/db/lib/ns-db-name.sh
  source "${_RT_LIB_DIR}/../db/lib/ns-db-name.sh"
fi
if [ -f "${_RT_LIB_DIR}/ns-env-file.sh" ]; then
  # shellcheck source=scripts/lib/ns-env-file.sh
  source "${_RT_LIB_DIR}/ns-env-file.sh"
fi

RT_DEFAULT_IMAGE="${RT_DEFAULT_IMAGE:-alioth-ns-runtime:latest}"

# ── 配置表 ────────────────────────────────────────────────────────────────

rt_config_file() {
  echo "${RT_CONF_FILE:-${PROJECT_ROOT}/Deploy/runtime.conf}"
}

# rt_conf_get <key> —— 打印该键的值（多值原样空格分隔）；缺表/缺键 → 非 0
rt_conf_get() {
  # 与 `rt_conf_set` 同口径：解析钉 C locale（真 bash 侧字节透明；仿真 shell 的内建重编码风险见其注释）
  local LC_ALL=C
  local key="$1" file line k v
  file="$(rt_config_file)"
  [ -f "$file" ] || return 1
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in ''|'#'*) continue ;; esac
    # shellcheck disable=SC2086
    set -- $line
    k="$1"
    shift
    [ "$k" = "$key" ] || continue
    v="$*"
    [ -n "$v" ] || return 1
    printf '%s\n' "$v"
    return 0
  done < "$file"
  return 1
}

# rt_conf_set <key> <value> —— 就地修改 `Deploy/runtime.conf`（原子 tmp+mv）
#
# 契约：
#   · **只允许改已登记键**（`rt_conf_get` 读不到 ⇒ fail-loud）：新增键 MUST 同步门禁
#     `scripts/check/check-runtime-conf.ts`（REQUIRED_KEYS + 形态断言）与文档，MUST NOT 由本函数静默扩键。
#   · 值形态：`*_port` → 1024..65535 整数；`control_server_names` → 空格分隔域名表（非空）；
#     其余键禁空白且非空。
#   · 注释与键序保持不动；写失败不改原文件。
rt_conf_set() {
  # 读写循环 MUST 与 locale 无关，且**调用入口 MUST 是真 bash**（2026-09-29 实测定位）：
  #   · 真 bash（`bash -c …`，本文件的两条路径）在 LC_ALL=C 下字节透明 ⇒ 只改目标键那一行，
  #     其余行逐字节不变（空写 0 差异）；
  #   · 仿真 shell（工具链的 Bun 内建 shell，`/proc/$$/exe` = bun）的 `printf`/`read` 内建会在
  #     UTF-8 locale 下把文本按 latin1 重编码 ⇒ 同值空写即把中文注释打成 `åº…` 乱码数十行，
  #     且该内建**不理会**下方 `local LC_ALL=C` ⇒ 此类 shell MUST NOT 直接调用本函数。
  # `local LC_ALL` 由 bash 视为 locale 作用域赋值（保护真 bash 侧的确定性）。
  local LC_ALL=C
  local key="$1" value="$2" file tmp line k found=0 rc=0
  [ -n "$key" ] || { echo "❌ rt_conf_set: 缺 key" >&2; return 2; }
  case "$key" in
    *_port)
      case "$value" in ''|*[!0-9]*) echo "❌ $key 必须为整数（收到 '$value'）" >&2; return 2 ;; esac
      if [ "$value" -lt 1024 ] || [ "$value" -gt 65535 ]; then
        echo "❌ $key 越界（1024..65535，收到 ${value}）" >&2; return 2
      fi
      ;;
    control_server_names)
      [ -n "$value" ] || { echo "❌ control_server_names 不能为空" >&2; return 2; }
      ;;
    *)
      case "$value" in ''|*[[:space:]]*) echo "❌ $key 值禁空白/空（收到 '$value'）" >&2; return 2 ;; esac
      ;;
  esac
  file="$(rt_config_file)"
  [ -f "$file" ] || { echo "❌ 缺配置源: $file" >&2; return 1; }
  rt_conf_get "$key" >/dev/null 2>&1 || {
    echo "❌ 未知键 '$key'（runtime.conf 未登记；新增键 MUST 同步门禁 check-runtime-conf.ts 与文档）" >&2
    return 2
  }
  tmp="${file}.tmp.$$"
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in ''|'#'*) printf '%s\n' "$line"; continue ;; esac
    # shellcheck disable=SC2086
    set -- $line
    k="$1"
    if [ "$k" = "$key" ]; then printf '%s %s\n' "$key" "$value"; found=1; else printf '%s\n' "$line"; fi
  done < "$file" > "$tmp" || rc=1
  if [ "$rc" != 0 ] || [ "$found" != 1 ]; then
    rm -f "$tmp"
    echo "❌ runtime.conf 写入失败（key=$key found=${found}）" >&2
    return 1
  fi
  mv -f "$tmp" "$file"
  printf '✅ %s = %s（%s）\n' "$key" "$value" "${file}"
}

# ── 容器服务环境自检与开关（单一实现；默认关闭 + 仅 Linux）──────────────
#
# 语义（用户裁决 2026-09-27）：
#   · **默认关闭**：`container_service` 缺省 off ⇒ `start`/`build-image` 等**驱动面** MUST fail-closed 拒绝
#     （原 `serve` 子命令已随控制面迁入 Meta 退役，不在驱动面之列）；
#     只读/清理面（status/list/logs/down/doctor/backend/config）MUST 仍可用（否则关掉开关后无法收尾）。
#   · **仅 Linux**：非 Linux（Darwin 等）即使开关置 on 也 MUST 拒绝（宿主网络/userns/uidmap 语义为 Linux 特有）。
#   · 开关优先级：env `ALIOTH_CONTAINER_SERVICE`(on|off) > `Deploy/runtime.conf::container_service` > 缺省 off。
#   · 测试/仿真注入：`ALIOTH_RT_OS_OVERRIDE` 覆写 OS 判定（MUST NOT 用于生产）。
rt_os() {
  if [ -n "${ALIOTH_RT_OS_OVERRIDE:-}" ]; then printf '%s\n' "$ALIOTH_RT_OS_OVERRIDE"; return 0; fi
  uname -s 2>/dev/null || printf 'unknown\n'
}

# rt_containers_supported —— 0=本机可跑容器实例（仅 Linux）
rt_containers_supported() {
  [ "$(rt_os)" = "Linux" ]
}

# rt_container_service_setting —— 打印 on|off（含来源；`--source` 时打印 "<value> <source>"）
rt_container_service_setting() {
  local want_source="${1:-}" v
  if [ -n "${ALIOTH_CONTAINER_SERVICE:-}" ]; then
    case "$ALIOTH_CONTAINER_SERVICE" in
      on|off) v="$ALIOTH_CONTAINER_SERVICE" ;;
      *) v="off" ;;   # 非法值 ⇒ fail-closed 视为 off
    esac
    if [ "$want_source" = "--source" ]; then printf '%s env\n' "$v"; else printf '%s\n' "$v"; fi
    return 0
  fi
  v="$(rt_conf_get container_service 2>/dev/null || true)"
  case "$v" in
    on|off) : ;;
    *) v="off" ;;     # 缺键/非法 ⇒ 默认关闭
  esac
  if [ "$want_source" = "--source" ]; then printf '%s conf\n' "$v"; else printf '%s\n' "$v"; fi
}

# rt_container_service_enabled —— 0=已开启（仅开关；不含 OS 判据）
rt_container_service_enabled() {
  [ "$(rt_container_service_setting)" = "on" ]
}

# rt_require_container_service <动作名> —— 驱动面前置门：非 Linux 或未开启 ⇒ 打诊断 + 非零
# 诊断 MUST 含：当前 OS、开关值与来源、开启方式（三层：env / runtime.conf / 门禁）
rt_require_container_service() {
  local action="${1:-容器操作}" os setting source
  os="$(rt_os)"
  setting="$(rt_container_service_setting --source)"
  if [ "$os" != "Linux" ]; then
    {
      echo "❌ ${action}不可用：容器实例仅支持 Linux（当前 OS=${os}）"
      echo "   原因：实例以宿主网络 + rootless userns（uidmap / single-id 影子）托管，语义为 Linux 特有。"
      echo "   处置：在 Linux 主机上运行；非 Linux 请改用进程面（bash scripts/gateway/restart.sh --namespace <ns>）。"
    } >&2
    return 3
  fi
  case "$setting" in
    "on "*) return 0 ;;
    *)
      {
        echo "❌ ${action}不可用：容器服务**默认关闭**（当前 container_service=${setting}）"
        echo "   开启方式（三选一，均需 Linux）："
        echo "     · 持久（推荐）：Deploy/runtime.conf 置 'container_service on'"
        echo "     · 单次：ALIOTH_CONTAINER_SERVICE=on <命令>"
        echo "     · 门禁：bun scripts/check/check-runtime-conf.ts --fail（键形态校验）"
      } >&2
      return 3
    ;;
  esac
}

rt_conf_require() { # <key>
  local v
  v="$(rt_conf_get "$1")" || { echo "❌ Deploy/runtime.conf 缺少键: $1" >&2; return 1; }
  printf '%s\n' "$v"
}

rt_ingress_port() { rt_conf_require ingress_port; }
rt_host_prefix() { rt_conf_require host_prefix; }
rt_domain_suffix() { rt_conf_require domain_suffix; }

# rt_config_dump —— key=value 行（供 CLI/服务面消费，避免二次解析）
rt_config_dump() {
  local f; f="$(rt_config_file)"
  [ -f "$f" ] || { echo "❌ 配置表不存在: $f" >&2; return 1; }
  local line
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in ''|'#'*) continue ;; esac
    line="${line%%#*}"                       # 去尾注释
    line="${line#"${line%%[![:space:]]*}"}"  # ltrim
    [ -n "$line" ] || continue
    printf '%s\n' "${line%%[[:space:]]*}=${line#*[[:space:]]}"
  done < "$f"
}

# ── 命名 / 主机名 ─────────────────────────────────────────────────────────

# rt_ns_lower <ns>
rt_ns_lower() { printf '%s\n' "$1" | tr '[:upper:]' '[:lower:]'; }

# rt_instance_key <ns> [<label>] —— 缺省 label = ns 小写；输出 <ns-lower>[-<label>]
rt_instance_key() {
  local ns="$1" label="${2:-}" lower
  lower="$(rt_ns_lower "$ns")"
  if [ -n "$label" ] && [ "$label" != "$lower" ]; then
    printf '%s-%s\n' "$lower" "$label"
  else
    printf '%s\n' "$lower"
  fi
}

# rt_instance_name <key> —— 容器名（全库唯一形）
rt_instance_name() { printf 'alioth-ns-%s\n' "$1"; }

# rt_instance_host <ns> [<label>] —— 入站主机名（不含端口）
rt_instance_host() {
  printf '%s-%s.%s\n' "$(rt_host_prefix)" "$(rt_instance_key "$1" "${2:-}")" "$(rt_domain_suffix)"
}

# rt_origin <host> —— 浏览器可见 origin 集合（CORS 双键值，逗号分隔）
rt_origin() {
  local host="$1" port; port="$(rt_ingress_port)"
  printf 'https://%s,http://%s,http://%s:%s\n' "$host" "$host" "$host" "$port"
}

# rt_resolve_host <host> [--port <p>] —— 主机名 → 实例 key；0 = 已登记且合法
# 判据：后缀匹配 domain_suffix ∧ 前缀 <host_prefix>- ∧ 余段按「最长 ns 前缀」切分；
#       余段为空或以 '-' 开头（label）。未登记 ns / 非法 label → 非 0。
rt_resolve_host() {
  local raw="$1" port="${2:-}"
  local host suffix prefix body
  host="$(rt_ns_lower "${raw%%:*}")"
  # 端口判定 MUST 在大小写归一之后（否则大写 Host 会被当成「端口 = 主机名」而判失败）
  if [ -z "$port" ]; then
    if [ "${raw##*:}" != "${raw%%:*}" ]; then port="${raw##*:}"; else port="$(rt_ingress_port)"; fi
  fi
  suffix="$(rt_domain_suffix)"; prefix="$(rt_host_prefix)"
  case "$host" in
    *".$suffix") host="${host%".$suffix"}" ;;
    *) return 1 ;;
  esac
  case "$host" in
    "$prefix"-*) body="${host#"$prefix"-}" ;;
    *) return 1 ;;
  esac
  [ -n "$body" ] || return 1
  # 最长 ns 前缀匹配（ns 名可含 '-'，故必须最长优先）
  local ns ns_lower rest best="" best_lower=""
  while IFS= read -r ns; do
    [ -n "$ns" ] || continue
    ns_lower="$(rt_ns_lower "$ns")"
    case "$body" in
      "$ns_lower"|"$ns_lower"-*)
        if [ "${#ns_lower}" -gt "${#best_lower}" ]; then best="$ns"; best_lower="$ns_lower"; fi
        ;;
    esac
  done < <(ns_ports_list 2>/dev/null | awk 'NF{print $1}')
  [ -n "$best" ] || return 1
  rest="${body#"$best_lower"}"
  rest="${rest#-}"
  if [ -n "$rest" ] && ! rt_label_valid "$rest"; then return 1; fi
  printf '%s\n' "$(rt_instance_key "$best" "$rest")"
  [ "$(rt_ingress_port)" = "$port" ] || return 3   # 端口不符（调用方可据此提示）
  return 0
}

# rt_label_valid <label> —— ^[a-z0-9]([a-z0-9-]{0,38}[a-z0-9])?$
rt_label_valid() {
  local l="$1"
  [ "${#l}" -le 40 ] || return 1
  case "$l" in
    [a-z0-9]) return 0 ;;
    [a-z0-9]*[a-z0-9]) case "$l" in *[!a-z0-9-]*) return 1 ;; *) return 0 ;; esac ;;
    *) return 1 ;;
  esac
}

# ── 后端探测 ──────────────────────────────────────────────────────────────

rt_backend() {
  if [ -n "${ALIOTH_CONTAINER_RUNTIME:-}" ]; then
    case "$ALIOTH_CONTAINER_RUNTIME" in
      podman|docker) printf '%s\n' "$ALIOTH_CONTAINER_RUNTIME" ;;
      *) echo "❌ ALIOTH_CONTAINER_RUNTIME 非法: ${ALIOTH_CONTAINER_RUNTIME}（只允许 podman|docker）" >&2; return 1 ;;
    esac
    return 0
  fi
  if command -v podman >/dev/null 2>&1; then echo podman; return 0; fi
  if command -v docker >/dev/null 2>&1; then echo docker; return 0; fi
  echo "❌ 未找到容器后端（podman / docker 皆不在 PATH）；安装其一或用 ALIOTH_CONTAINER_RUNTIME 指定" >&2
  return 1
}

# rt_podman_mode —— native | single-id
# ALIOTH_RT_PODMAN_MODE=native|single-id 可强制形态（**诊断/测试用**：复现目标主机形态、
# 让测试与验收在两种形态各跑一次——2026-09-25 R8 ❌-13「换形态即红的测试」根因处置）。
# 强制 native 而 newuidmap 缺失 MUST fail-loud（该形态在缺 uidmap 的主机上根本跑不起来，
# 静默降级会把「形态不可用」伪装成「已按 native 跑」）。
rt_podman_mode() {
  if [ -n "${ALIOTH_RT_PODMAN_MODE:-}" ]; then
    case "$ALIOTH_RT_PODMAN_MODE" in
      native)
        if command -v newuidmap >/dev/null 2>&1 && command -v newgidmap >/dev/null 2>&1; then
          echo native; return 0
        fi
        echo "❌ ALIOTH_RT_PODMAN_MODE=native 但缺 newuidmap/newgidmap：该形态不可用（装 uidmap 或用 single-id）" >&2
        return 1
        ;;
      single-id) echo single-id; return 0 ;;
      *) echo "❌ ALIOTH_RT_PODMAN_MODE 非法: ${ALIOTH_RT_PODMAN_MODE}（只允许 native|single-id）" >&2; return 1 ;;
    esac
  fi
  if command -v newuidmap >/dev/null 2>&1 && command -v newgidmap >/dev/null 2>&1; then
    echo native
    return 0
  fi
  local user; user="$(id -un)"
  if [ -r /etc/subuid ] && grep -q "^${user}:" /etc/subuid 2>/dev/null; then
    echo single-id
    return 0
  fi
  echo "❌ podman rootless 不可用：缺 newuidmap/newgidmap（uidmap 包）且 $user 无 /etc/subuid 区间" >&2
  echo "   处置：sudo apt-get install -y uidmap（或安装 podman 发行包自带依赖）" >&2
  return 1
}

# rt_backend_info —— key=value 行（backend/mode/name/version/storage_root）
rt_backend_info() {
  local backend mode name version root
  backend="$(rt_backend)" || return 1
  if [ "$backend" = podman ]; then
    mode="$(rt_podman_mode)" || return 1
  else
    mode="native"
  fi
  name="$backend"
  # 后端 CLI 缺失时 `--version` 会 127；本函数仍须给出 backend/mode（探测结论）⇒ 容错
  version="$(_rt_cli_raw --version 2>/dev/null | head -1 || true)"
  # 存储根按形态取实际值（报错值会误导排障）：podman native = podman GraphRoot；
  # docker = 守护进程 DockerRootDir；single-id 影子 = 注入的专用根（2026-09-26 实测三者互不相同）
  local root=""
  # 后端 CLI 可能缺失（如只探形态时）：命令替换失败会因 pipefail 中止脚本 ⇒ 一律容错。
  # 查询失败时 MUST NOT 回落 podman 影子根（那会把 docker 的存储根报成 podman 的，误导排障）
  # ——2026-09-26 R13 ⚠️-23 实测：守护不可达时原先报 ~/.local/share/alioth-container-runtime/storage
  if [ "$backend" = podman ] && [ "$mode" = single-id ]; then
    root="$(_rt_podman_storage_root)"
  elif [ "$backend" = podman ]; then
    root="$(podman info --format '{{.Store.GraphRoot}}' 2>/dev/null | head -1 || true)"
    root="${root:-unavailable}"
  elif [ "$backend" = docker ]; then
    root="$(docker info --format '{{.DockerRootDir}}' 2>/dev/null | head -1 || true)"
    root="${root:-unavailable}"
  fi
  [ -n "$root" ] || root="unavailable"
  local uidmap_state="present" uidmap_cmd=""
  if ! rt_uidmap_present; then
    uidmap_state="missing"
    uidmap_cmd="$(rt_uidmap_install_cmd 2>/dev/null || true)"
  fi
  printf 'backend=%s\nmode=%s\ncli=%s\nversion=%s\nstorage_root=%s\nuidmap=%s\nuidmap_install_cmd=%s\nstorage_root_override=%s\n' \
    "$backend" "$mode" "$name" "$version" "${root:-}" "$uidmap_state" "$uidmap_cmd" "${ALIOTH_RT_STORAGE_ROOT:-}"
  printf 'os=%s\n' "$(rt_os)"
  printf 'containers_supported=%s\n' "$(rt_containers_supported && echo true || echo false)"
  printf 'container_service=%s\n' "$(rt_container_service_setting)"
  # 引擎作用域（2026-09-27 用户裁决：仅**本机** Meta 可管理容器服务，异机/远端 Meta 只能消费只读面）：
  # `remote=true` ⟺ 显式配置了引擎端点（`RT_ENGINE_ENDPOINT` 非空）⇒ 调用方 MUST NOT 视作本机默认引擎
  # （口径与 `launch-image.sh` 的 `remote=` 输出一致：端点的**显式性**即「非本机默认」判据）。
  printf 'engine_endpoint=%s\n' "${RT_ENGINE_ENDPOINT:-}"
  if [ -n "${RT_ENGINE_ENDPOINT:-}" ]; then printf 'remote=true\n'; else printf 'remote=false\n'; fi
}

_rt_podman_storage_root() {
  printf '%s\n' "${ALIOTH_RT_STORAGE_ROOT:-${XDG_DATA_HOME:-${HOME}/.local/share}/alioth-container-runtime/storage}"
}
# 运行根 MUST 与影子脚本同口径（且落在 tmpfs-/run 之外）：两处不一致即
# `database run root ... does not match our run root`（容器库 DB 记录了首建时的 runroot）
_rt_podman_run_root() {
  printf '%s\n' "${ALIOTH_RT_RUN_ROOT:-${HOME}/.local/share/alioth-container-runtime/run}"
}

# rt_run_extra_args —— run/create 专用附加参数（每行一个）
# 单 ID 影子形态下 podman 以 euid 0 运行却无 /sys/fs/cgroup 写权限 ⇒ 容器创建会以
# `create /sys/fs/cgroup/libpod_parent: Permission denied` 失败；本形态实例不需要 cgroup
# 限额，故显式 --cgroups=disabled（native/rootless 形态保持默认，限额能力不丢）。
rt_run_extra_args() {
  local backend mode
  backend="$(rt_backend 2>/dev/null || true)"
  [ "$backend" = podman ] || return 0
  mode="$(rt_podman_mode 2>/dev/null || true)"
  [ "$mode" = single-id ] || return 0
  printf '%s\n' --cgroups=disabled
  # devpts 默认挂载带 gid=5（tty 组），单 ID 映射下 gid 5 未映射 ⇒ crun
  # `mount devpts to dev/pts: Invalid argument`；改用 tmpfs 覆盖 /dev/pts
  # （本形态实例是无 tty 的常驻服务；`exec -it` 交互式 tty 在该形态不可用，见规约）
  printf '%s\n' --mount type=tmpfs,destination=/dev/pts
  return 0
}

# _rt_cli_raw <args...> —— 按后端/形态拼出真实命令并执行
# 端点在此**唯一派发点**消费：`RT_ENGINE_ENDPOINT`（由 rt_engine_resolve 设置）非空 ⇒ docker
# `DOCKER_HOST=` / podman `CONTAINER_HOST=` + `--remote`；为空 ⇒ 既有本地路径（含 single-id 影子）。
# 若端点解析不被本函数消费，探针/状态/日志会绕过远端端点走本地引擎（= 第二派发路径）。
_rt_cli_raw() {
  local backend="${RT_ENGINE:-}"
  [ -n "$backend" ] || backend="$(rt_backend)" || return 1
  if [ -n "${RT_ENGINE_ENDPOINT:-}" ]; then
    case "$backend" in
      docker) DOCKER_HOST="$RT_ENGINE_ENDPOINT" docker "$@" ;;
      podman) CONTAINER_HOST="$RT_ENGINE_ENDPOINT" podman --remote "$@" ;;
      *) echo "❌ 端点已设置但引擎非法: $backend" >&2; return 1 ;;
    esac
    return $?
  fi
  if [ "$backend" = podman ]; then
    local mode; mode="$(rt_podman_mode)" || return 1
    if [ "$mode" = single-id ]; then
      RT_STORAGE_ROOT="$(_rt_podman_storage_root)" \
        RT_RUN_ROOT="$(_rt_podman_run_root)" \
        unshare --user --map-root-user --mount \
          "${PROJECT_ROOT}/scripts/runtime/podman-single-id-inner.sh" "$@"
    else
      podman "$@"
    fi
  else
    docker "$@"
  fi
}

# rt_cli <args...> —— 容器 CLI 透传（对外统一入口）
rt_cli() { _rt_cli_raw "$@"; }

# rt_image_present <image> —— 0 = 当前形态的引擎存储中已存在该镜像（`<repo>:<tag>` 形引用）
# 形态分支处置与 smoke.sh 同口径：podman 支持 `image exists`；docker 无该子命令（rc=1 恒非「不存在」）
# ⇒ 退回 `images --format` 逐行**精确**匹配（非正则解析，仅字面比对）。
rt_image_present() {
  local img="$1"
  [ -n "$img" ] || return 1
  if rt_cli image exists "$img" >/dev/null 2>&1; then return 0; fi
  rt_cli images --format '{{.Repository}}:{{.Tag}}' 2>/dev/null | grep -qxF "$img"
}

# ── 实例注册表 ────────────────────────────────────────────────────────────

rt_runtime_root() { printf '%s\n' "${RT_RUNTIME_ROOT:-${PROJECT_ROOT}/.runtime/runtime}"; }
rt_registry_file() { printf '%s\n' "$(rt_runtime_root)/$1.json"; }

# rt_registry_write <key> <ns> <label> <container> <image> <be> <fe> <sso> <pkg> <backend> <mode> <started_at>
rt_registry_write() {
  local key="$1" ns="$2" label="$3" container="$4" image="$5" be="$6" fe="$7" sso="$8" pkg="$9" backend="${10}" mode="${11}" started="${12}" tier="${13:-dev}"
  local dir file tmp host
  dir="$(rt_runtime_root)"; mkdir -p "$dir"
  file="$(rt_registry_file "$key")"
  tmp="${file}.tmp.$$"
  host="$(rt_instance_host "$ns" "$label")"
  {
    printf '{\n'
    printf '  "key": "%s",\n' "$key"
    printf '  "namespace": "%s",\n' "$ns"
    printf '  "label": "%s",\n' "$label"
    printf '  "host": "%s",\n' "$host"
    printf '  "container": "%s",\n' "$container"
    printf '  "image": "%s",\n' "$image"
    printf '  "bePort": %s,\n' "$be"
    printf '  "fePort": %s,\n' "$fe"
    printf '  "ssoPort": %s,\n' "$sso"
    printf '  "deployPath": "%s",\n' "$pkg"
    printf '  "backend": "%s",\n' "$backend"
    printf '  "backendMode": "%s",\n' "$mode"
    printf '  "tier": "%s",\n' "$tier"
    printf '  "startedAt": "%s"\n' "$started"
    printf '}\n'
  } > "$tmp"
  mv "$tmp" "$file"
}

# _rt_json_runner —— 注册表 JSON 解析器（bun 优先，其次 node）
# 纪律（NO_REGEX_FOR_PARSING）：JSON 是结构化容器，MUST 用标准解析器——MUST NOT 用 sed/awk
# 正则抠字段（那既是第二份实现，也会在值含引号/转义时静默错读）。两者皆缺 ⇒ fail-loud。
_rt_json_runner() {
  if command -v bun >/dev/null 2>&1; then printf 'bun\n'; return 0; fi
  if command -v node >/dev/null 2>&1; then printf 'node\n'; return 0; fi
  echo "❌ 需要 bun 或 node 解析实例注册表 JSON（NO_REGEX_FOR_PARSING）" >&2
  return 1
}

# rt_registry_field <key> <field> —— 读 JSON 字段（标准解析器；缺失/空 → rc=1）
# 输出 MUST 以换行结尾：唯一多值消费方 rt_ports_used 以「逐行聚合」判定占用，
# 缺尾随换行会把多条记录的端口串接成一个 token（实测 391003910139102），
# 使 rt_ports_next 的占用判据永不命中（2026-09-25 spec-audit ❌-11）。
rt_registry_field() {
  local file field runner; file="$(rt_registry_file "$1")"; field="$2"
  [ -f "$file" ] || return 1
  runner="$(_rt_json_runner)" || return 1
  "$runner" -e '
    const fs = require("fs");
    const o = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    const v = o[process.argv[2]];
    if (v === undefined || v === null) process.exit(1);
    process.stdout.write(String(v) + "\n");
  ' "$file" "$field"
}

# rt_registry_kv <key> —— 一条注册表记录的全部字段（`k=v` 行，标准解析器一次读完）
# 供 rt_status 等高频消费方使用，避免逐字段多次派生解析器
rt_registry_kv() {
  local file runner; file="$(rt_registry_file "$1")"
  [ -f "$file" ] || return 1
  runner="$(_rt_json_runner)" || return 1
  "$runner" -e '
    const fs = require("fs");
    const o = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    for (const k of Object.keys(o)) process.stdout.write(`${k}=${o[k] ?? ""}\n`);
  ' "$file"
}

# rt_url_rehost <url> <new_host> —— 以标准 URL 解析器替换宿主名（保留 scheme/凭据/端口/路径）
# 纪律（NO_REGEX_FOR_PARSING）：URL 是结构化容器——MUST 用 URL 解析器，MUST NOT 用 sed 正则改
# （口令含字面 `@localhost` 时正则会错位改写；2026-09-25 spec-audit ⚠️-10）。解析失败 ⇒ 非零且原样返回。
rt_url_rehost() {
  local url="$1" host="$2" runner out
  runner="$(_rt_json_runner)" || { printf '%s' "$url"; return 1; }
  if out="$("$runner" -e '
    const u = new URL(process.argv[1]);
    u.hostname = process.argv[2];
    process.stdout.write(u.toString());
  ' "$url" "$host" 2>/dev/null)" && [ -n "$out" ]; then
    printf '%s' "$out"
    return 0
  fi
  printf '%s' "$url"   # 解析失败：原样返回 + 非零（调用方据此决定是否采用改写值）
  return 1
}

rt_registry_remove() { rm -f "$(rt_registry_file "$1")"; }

# rt_url_host <url> —— URL 宿主名（标准 URL 解析器；解析失败 ⇒ 非零 + 空输出）
# 用于通用启动器的 DB 宿主名判定（MUST NOT 用 sed/awk 拆 URL —— NO_REGEX_FOR_PARSING）
rt_url_host() {
  local url="$1" runner
  runner="$(_rt_json_runner)" || return 1
  "$runner" -e '
    try { process.stdout.write(new URL(process.argv[1]).hostname + "\n"); }
    catch { process.exit(1); }
  ' "$url"
}

# rt_recorded_form <key> —— 三态返回（**MUST NOT 把「读不出」当「未登记」**，2026-09-26 R10 ❌-15）：
#   0 = 有登记且读出（stdout "backend mode"）
#   1 = 未登记（注册表无该条目）
#   3 = 有登记但读不出（缺 bun/node 解析器 / JSON 破损）⇒ 调用方 MUST fail-loud 且 MUST NOT 清注册表
rt_recorded_form() {
  local key="$1" file kv backend mode
  file="$(rt_registry_file "$key")"
  [ -f "$file" ] || return 1
  kv="$(rt_registry_kv "$key" 2>/dev/null)" || return 3
  backend="$(printf '%s\n' "$kv" | sed -n 's/^backend=//p')"
  mode="$(printf '%s\n' "$kv" | sed -n 's/^backendMode=//p')"
  [ -n "$backend" ] || return 3
  printf '%s %s\n' "$backend" "$mode"
}

# rt_require_form <key> —— 统一的形态可用性门禁（0 = 可继续；非 0 = 已打印原因且 MUST NOT 清注册表）
# 成功后把形态写入 RT_FORM_BACKEND / RT_FORM_MODE（空 = 未登记 ⇒ 按当前形态）
rt_require_form() {
  local key="$1" form rc
  RT_FORM_BACKEND=""; RT_FORM_MODE=""
  form="$(rt_recorded_form "$key")"; rc=$?
  case "$rc" in
    0)
      RT_FORM_BACKEND="${form%% *}"; RT_FORM_MODE="${form##* }"
      if rt_form_available "$RT_FORM_BACKEND" "$RT_FORM_MODE"; then return 0; fi
      echo "❌ 实例 $key 登记为 ${RT_FORM_BACKEND}/${RT_FORM_MODE}（该形态在本机不可用）：MUST NOT 清理注册表。请以 $(_rt_form_hint "$RT_FORM_BACKEND" "$RT_FORM_MODE") 恢复该形态后重试" >&2
      return 3 ;;
    1)
      return 0 ;;   # 未登记：调用方按当前形态
    *)
      echo "❌ 实例 $key 的注册表条目不可读（缺 bun/node 解析器或 JSON 破损）：MUST NOT 清理注册表；请修复解析器或人工核对 $(rt_registry_file "$key")" >&2
      return 3 ;;
  esac
}

# rt_form_available <backend> <mode> —— 0 = 该形态在本机可用
rt_form_available() {
  local backend="$1" mode="$2"
  # 进程内缓存：同一次 CLI 调用里同一形态只探一次——探针含 `podman info`/`docker info`（数百 ms ~ 2s），
  # 每键各探会把 `list`/`status` 拖成 N×探针耗时（2026-09-26 实测：`list --no-health` 冷启动仍 ~2s）。
  if [ "$backend" = docker ] && [ -n "${_RT_FORM_DOCKER+x}" ]; then [ "$_RT_FORM_DOCKER" = 1 ]; return; fi
  if [ "$backend" = podman ] && [ "$mode" = native ] && [ -n "${_RT_FORM_PODMAN_NATIVE+x}" ]; then [ "$_RT_FORM_PODMAN_NATIVE" = 1 ]; return; fi
  if [ "$backend" = podman ] && [ "$mode" = single-id ] && [ -n "${_RT_FORM_PODMAN_SINGLE+x}" ]; then [ "$_RT_FORM_PODMAN_SINGLE" = 1 ]; return; fi
  local ok=1
  case "$backend" in
    docker)
      command -v docker >/dev/null 2>&1 || ok=0
      # 守护可达性 MUST 一并判定：CLI 在而守护不可达（Desktop 未启 / rootless 守护未起 / 未导出
      # DOCKER_HOST）时，`inspect` 必然失败 ⇒ 会把「形态不可用」误判为「容器不存在」并清注册表
      # （2026-09-26 R14 ❌-16 实测：条目被静默删除且不可恢复）。fail-loud 优于静默错判。
      # 探测 MUST 有界（守护半死/指向无响应地址时 CLI 连接重试可达数秒 ⇒ MUST NOT 无界等待）。
      [ "$ok" = 1 ] && { timeout 2 docker info >/dev/null 2>&1 || ok=0; } ;;
    podman)
      command -v podman >/dev/null 2>&1 || ok=0
      if [ "$ok" = 1 ]; then
        case "$mode" in
          native) { command -v newuidmap >/dev/null 2>&1 && command -v newgidmap >/dev/null 2>&1 \
                    && timeout 2 podman info >/dev/null 2>&1; } || ok=0 ;;
          # 单 ID 影子形态的前置 = 进入 user namespace 的 `unshare` + 影子内运行时 `crun`
          # （podman-single-id-inner.sh:18-19/69-70 均 fail-loud 要求）⇒ 缺一即视为**形态不可用**：
          # 否则影子不可运行（unshare/crun 缺失或被替换、升级失配）时条目会被误清成孤儿（R15 ❌-18）
          single-id) { command -v unshare >/dev/null 2>&1 && command -v crun >/dev/null 2>&1; } || ok=0 ;;
          "") : ;;
          *) ok=0 ;;
        esac
      fi ;;
    *) ok=0 ;;
  esac
  # 记入进程内缓存（后端/模式键）
  case "$backend/$mode" in
    docker/*) _RT_FORM_DOCKER="$ok" ;;
    podman/native) _RT_FORM_PODMAN_NATIVE="$ok" ;;
    podman/single-id) _RT_FORM_PODMAN_SINGLE="$ok" ;;
  esac
  [ "$ok" = 1 ]
}

# _rt_form_hint <backend> <mode> —— 该形态不可用时的补救提示（人类可读，含空格；用于 stderr）
_rt_form_hint() {
  case "$1" in
    docker) printf 'ALIOTH_CONTAINER_RUNTIME=docker（并安装 Docker Engine/OrbStack）' ;;
    podman) printf 'ALIOTH_RT_PODMAN_MODE=%s' "${2:-native}" ;;
    *) printf '恢复该形态能力' ;;
  esac
}

# _rt_form_hint_token <backend> <mode> —— 同上但**无空格**（单行 kv 面要求值不含空格）
_rt_form_hint_token() {
  case "$1" in
    docker) printf 'ALIOTH_CONTAINER_RUNTIME=docker' ;;
    podman) printf 'ALIOTH_RT_PODMAN_MODE=%s' "${2:-native}" ;;
    *) printf 'restore-form' ;;
  esac
}

# _rt_cli_in_form <backend> <mode> <args...> —— 以**指定形态**执行后端 CLI（跨形态收口用：
# 清理另一形态遗下的同名容器）。与 `rt_cli_for_key`（按**登记形态**）互补。
_rt_cli_in_form() {
  local b="$1" m="$2"; shift 2
  case "$b" in
    docker) ALIOTH_CONTAINER_RUNTIME=docker _rt_cli_raw "$@" ;;
    podman) ALIOTH_CONTAINER_RUNTIME=podman ALIOTH_RT_PODMAN_MODE="$m" _rt_cli_raw "$@" ;;
    *) return 1 ;;
  esac
}

# rt_cli_for_key <key> <args...> —— 以**登记形态**执行后端 CLI
# 为什么必须按登记形态：本机形态可因装 uidmap 而改变（§5/§8 引导的路径），若按当前形态定位容器，
# 会把「容器在另一形态的存储里」误判为不存在 ⇒ 静默清注册表 + 留下孤儿容器（2026-09-26 R9 ❌-14）。
# 登记形态不可用时 MUST fail-loud 给补救命令，MUST NOT 静默改用当前形态。
rt_cli_for_key() {
  local key="$1"; shift
  rt_require_form "$key" || return 3
  local backend="$RT_FORM_BACKEND" mode="$RT_FORM_MODE"
  if [ -z "$backend" ]; then rt_cli "$@"; return $?; fi   # 未登记：按当前形态
  # MUST 同时强制**后端本身**：只设 podman mode 时，当前环境的 ALIOTH_CONTAINER_RUNTIME（如 docker）
  # 会泄漏进按登记形态的查询 ⇒ 另一后端托管的实例被判 absent 并被清注册表（2026-09-26 实测复现）。
  case "$backend" in
    docker) ALIOTH_CONTAINER_RUNTIME=docker _rt_cli_raw "$@" ;;
    podman) ALIOTH_CONTAINER_RUNTIME=podman ALIOTH_RT_PODMAN_MODE="${mode:-native}" _rt_cli_raw "$@" ;;
    *) echo "❌ 实例 $key 登记的后端非法: $backend" >&2; return 1 ;;
  esac
}

# _rt_follow_recorded_form <key> —— 既有实例的形态以**注册表登记**为准：把本次操作的形态选择 env
# 切到登记形态（MUST 在调用方计算 backend/mode **之前**调用）。未登记 / 登记形态不可用 ⇒ 无操作。
# 动因（2026-09-29 实测）：调用方环境的默认形态 MUST NOT 静默改写既有实例形态——否则对 single-id
# 实例执行默认（native）start 会走「清理同名已停止容器 ⇒ 按当前形态重建」分支，把实例从影子 DB
# 摘除并重建到 native 根，而原容器进程树/端口三元组仍存活（孤儿 + 入站身份串台）。
_rt_follow_recorded_form() {
  local key="$1" rec rb rm
  rec="$(rt_recorded_form "$key" 2>/dev/null || true)"
  [ -n "$rec" ] || return 0
  read -r rb rm <<<"$rec"
  rt_form_available "$rb" "$rm" || return 0
  case "$rb" in
    docker) export ALIOTH_CONTAINER_RUNTIME=docker ;;
    podman) export ALIOTH_CONTAINER_RUNTIME=podman ALIOTH_RT_PODMAN_MODE="${rm:-native}" ;;
  esac
  return 0
}

# ── 按 key 定位容器（带登记形态）───────────────────────────────────────────
rt_key_container_exists() { # <key> —— 0 = 在该实例登记形态下存在
  rt_cli_for_key "$1" inspect --type container "$(rt_instance_name "$1")" >/dev/null 2>&1
}
rt_key_container_state() { # <key> —— 打印登记形态下的容器状态（不可用/不存在 → 空）
  rt_cli_for_key "$1" inspect --format '{{.State.Status}}' "$(rt_instance_name "$1")" 2>/dev/null | head -1
}

# rt_registry_keys —— 逐个打印已登记实例 key（字典序）
rt_registry_keys() {
  local dir; dir="$(rt_runtime_root)"
  [ -d "$dir" ] || return 0
  local f
  for f in "$dir"/*.json; do
    [ -f "$f" ] || continue
    basename "$f" .json
  done | LC_ALL=C sort
}

# ── 端口分配 ──────────────────────────────────────────────────────────────

# rt_port_listening <port> —— 宿主是否已监听该端口（**委托** ns-ports.sh::ns_port_listening，单一实现）
rt_port_listening() { ns_port_listening "$1"; }

# rt_port_owner <port> —— 占用者身份（**委托** ns-ports.sh::ns_port_owner：`name=`/`args=` 两行）
# 未复用说明：不为容器平面另写一份占用者判定（`ss -ltnp` 的线程名不可靠，已由 ns_port_owner 处置）。
rt_port_owner() { ns_port_owner "$1"; }

# rt_deploy_tier —— 容器实例缺省环境层（dev|pre；缺键 fail-loud，词表与 ns-env-file.sh 同源）
rt_deploy_tier() { rt_conf_require deploy_tier; }

# rt_host_namespaces —— 启动时自动托管的 namespace 列表（`Deploy/runtime.conf::host_namespaces`）
# 可选键：缺省/空 ⇒ 无输出 + rc 0（调用方据此判「不自动托管」）。
# 逐行打印一个 ns（消费方 MUST NOT 自解析配置表）；「逐项已在 Deploy/ports.conf 登记 + 无重复」
# 由门禁 check-runtime-conf.ts 保证；未登记项由调用方经 ns_ports_known 判定（本函数不复制登记判据）。
rt_host_namespaces() {
  local v ns
  v="$(rt_conf_get host_namespaces 2>/dev/null || true)"
  [ -n "$v" ] || return 0
  # 空白分隔多值（词表与门禁 splitFields 同构）
  # shellcheck disable=SC2086
  set -- $v
  for ns in "$@"; do [ -n "$ns" ] && printf '%s\n' "$ns"; done
  return 0
}

# rt_instance_env_file <pkg> <tier> —— 实例 env 文件（dev → `{pkg}/.env`；pre → `{pkg}/.env.pre`）
# 路径规则唯一实现 = scripts/lib/ns-env-file.sh::ns_env_file_in（MUST NOT 在本库/调用方拼后缀）；
# 目标层缺失 MUST fail-loud 且 MUST NOT 回落另一层（回落 = 用 dev 库跑 pre 环境）。
rt_instance_env_file() {
  local pkg="$1" tier="$2" f
  f="$(ns_env_file_in "$pkg" "$tier")" || {
    echo "❌ 实例 env 缺目标层文件：$pkg/.env$([ "$tier" = pre ] && printf '.pre')（tier=${tier}）" >&2
    echo "   MUST NOT 回落另一层；pre 层 env 生成：bash scripts/pre/provision-ns-env.sh <ns>" >&2
    return 1
  }
  printf '%s\n' "$f"
}

# rt_ns_ingress_port <ns> —— 每 ns 入站端口 `{ns-port}`（= Deploy/ports.conf 的 FE 槽位）
# 唯一读取实现 = ns-ports.sh::ns_port（本函数 MUST NOT 二次解析该表）
rt_ns_ingress_port() {
  local p
  p="$(ns_port "$1" fe)" || { echo "❌ namespace 未在 Deploy/ports.conf 登记（无 {ns-port}）: $1" >&2; return 1; }
  printf '%s\n' "$p"
}

# rt_control_public_port / rt_control_server_names —— 控制面对外端口与域名（缺键 fail-loud，MUST NOT 静默默认）
rt_control_public_port() { rt_conf_require control_public_port; }
rt_control_server_names() { rt_conf_require control_server_names; }

# rt_instance_ports <ns> —— 该 ns 的实例端口三元组（= Deploy/ports.conf 的 BE/FE/SSO；唯一读取实现）
# 原 `instance_port_band` 与端口分配已退役：端口取自表，不再"分配"（防环境差异与漂移）。
rt_instance_ports() {
  local ns="$1" be fe sso
  be="$(ns_port "$ns" be)" || { echo "❌ namespace 未登记（Deploy/ports.conf，无端口三元组）: $ns" >&2; return 1; }
  fe="$(ns_port "$ns" fe)" || return 1
  sso="$(ns_port "$ns" sso)" || return 1
  printf '%s %s %s\n' "$be" "$fe" "$sso"
}

# ── 发布包 env → 容器 -e 参数 ─────────────────────────────────────────────

# rt_pkg_env_exports <pkg_dir> [tier=dev] —— 以子 shell 解密并打印该包**目标层** env 的 KEY=VALUE（明文）
# tier 缺省 = Deploy/runtime.conf 的 deploy_tier（容器面缺省层）；文件由 rt_instance_env_file 解析。
rt_pkg_env_exports() {
  # 注意：`local a=x b="$a/y"` 在同一 local 语句内前向引用**兄弟变量**会失败（bash 先展开全部
  # 赋值词再落盘；`set -u` 下报 unbound variable）——故拆成两条 local（2026-09-25 R4 实证：
  # 该 bug 曾被下游 `|| true` 掩盖 ⇒ 发布包 .env 实际从未注入容器）
  local pkg="$1" tier="${2:-}"
  [ -n "$tier" ] || tier="$(rt_deploy_tier)" || return 1
  local envf
  envf="$(rt_instance_env_file "$pkg" "$tier")" || return 1
  PROJECT_ROOT="$PROJECT_ROOT" RT_ENV_FILE="$envf" bash -c '
    set -e
    cd "$PROJECT_ROOT"
    # 1) 先按 .env 原文导出（与 dev-gateway/发布启动器同惯例：set -a + source）
    set -a
    . "$RT_ENV_FILE"
    set +a
    # 2) enc: 值经仓库解密器覆盖为明文；**无 .encryption_key 时该库直接 return 0 且不导出任何值**
    #    ⇒ 步骤 1 是必需的前置（2026-09-25 R4 实证：只调解密器会让整包 env 变空）
    # shellcheck source=scripts/lib/decrypt-env-values.sh
    source scripts/lib/decrypt-env-values.sh
    decrypt_env_file "$RT_ENV_FILE" >/dev/null 2>&1 || true
    # 只回显文件里出现的键，避免把父进程环境整包泄进容器；值空也如实回显（= 文件自身的声明）
    keys="$(sed -n "s/^\([A-Za-z_][A-Za-z0-9_]*\)=.*/\1/p" "$RT_ENV_FILE" | LC_ALL=C sort -u)"
    for k in $keys; do
      printf "%s=%s\n" "$k" "${!k-}"
    done
  '
}

# ── uidmap 按需自举 ───────────────────────────────────────────────────────
# 背景：podman rootless 需要 newuidmap/newgidmap（Debian/Ubuntu 的 uidmap 包）。缺失时 podman
# 走单 ID 映射形态（可用但拉不了多属主镜像层、无 tty、无 cgroup 限额，见规约 §5）。
# 语义：**按需**执行——非交互（`sudo -n`）能装就装；需要交互密码则 fail-loud 给出确切命令，
# MUST NOT 挂起等待密码输入。开关：ALIOTH_RT_INSTALL_UIDMAP=1 自动尝试；=0 关闭。

# rt_uidmap_present —— 0 = newuidmap/newgidmap 均在
rt_uidmap_present() {
  command -v newuidmap >/dev/null 2>&1 && command -v newgidmap >/dev/null 2>&1
}

# rt_uidmap_pm —— 打印本机包管理器的 uidmap 安装命令（不含 sudo）；无法识别 → 非 0
rt_uidmap_pm() {
  if command -v apt-get >/dev/null 2>&1; then
    printf 'apt-get install -y uidmap\n'
  elif command -v dnf >/dev/null 2>&1; then
    printf 'dnf install -y shadow-utils\n'
  elif command -v pacman >/dev/null 2>&1; then
    printf 'pacman -S --noconfirm shadow\n'
  else
    return 1
  fi
}

# rt_uidmap_install_cmd —— 打印将执行的完整命令（含 `sudo -n`，非交互）
rt_uidmap_install_cmd() {
  local pm; pm="$(rt_uidmap_pm)" || return 1
  printf 'sudo -n %s\n' "$pm"
}

# rt_install_uidmap [--dry-run] —— 按需安装 uidmap（非交互）；成功后再探一次 newuidmap
rt_install_uidmap() {
  local dry=0
  [ "${1:-}" = --dry-run ] && dry=1
  if rt_uidmap_present; then
    echo "ℹ️  uidmap 已就位（newuidmap/newgidmap 存在，无需安装）"
    return 0
  fi
  local -a pm_argv=()
  local pm=""
  if pm="$(rt_uidmap_pm)"; then
    if [ "$dry" = 1 ]; then printf 'sudo -n %s\n' "$pm"; return 0; fi
    pm_argv=(sudo -n)
    # shellcheck disable=SC2206  # 包管理器命令含参数，按空白切分为 argv（来源为受控字面量）
    pm_argv+=($pm)
  else
    echo "❌ 未识别的包管理器：请手工安装提供 newuidmap/newgidmap 的包（Debian/Ubuntu: uidmap；Fedora: shadow-utils；Arch: shadow）" >&2
    return 1
  fi
  echo "▶ 按需安装 uidmap（非交互）: ${pm_argv[*]}" >&2
  if "${pm_argv[@]}" >/dev/null 2>&1; then
    if rt_uidmap_present; then
      echo "✅ uidmap 安装完成：podman 形态将自动切为 native（无需改任何配置）" >&2
      return 0
    fi
    echo "❌ 安装命令成功但 newuidmap 仍不可见（PATH? 预期 /usr/bin/newuidmap）" >&2
    return 1
  fi
  echo "❌ 按需安装失败：sudo 需要交互密码或权限不足（MUST NOT 挂起等待密码）。" >&2
  echo "   请在有 root 的环境执行：$(rt_uidmap_pm 2>/dev/null || echo '<包管理器> install uidmap')" >&2
  echo "   完成后 podman 形态自动变 native；不想安装时用 --rootfs-mode host（免 root 镜像路径）。" >&2
  return 1
}

# rt_ensure_uidmap —— 0 = 可用；1 = 不可用（含按需安装失败/被显式关闭）
rt_ensure_uidmap() {
  rt_uidmap_present && return 0
  case "${ALIOTH_RT_INSTALL_UIDMAP:-1}" in
    0|off|false|no) echo "ℹ️  跳过 uidmap 按需安装（ALIOTH_RT_INSTALL_UIDMAP=0）" >&2; return 1 ;;
  esac
  rt_install_uidmap
}

# ── 实例操作 ──────────────────────────────────────────────────────────────

# rt_instance_pkg <ns> —— 发布包目录（Deploy/{ns}）
rt_instance_pkg() { printf '%s\n' "${RT_PKG_ROOT:-${PROJECT_ROOT}/Deploy}/$1"; }

# rt_start <ns> [--label L] [--image IMG] [--pkg DIR] [--no-wait] [--dry-run]
# --dry-run：只打印将要执行的容器 argv（一行一参数）+ 覆盖键名（**不打印值**，避免凭据外泄），
#            不建 env 文件、不启动容器、不写注册表——用于后端差异审计与手工复核。
# _rt_container_exists_in_form <backend> <mode> <key> —— 指定形态下是否存在同名容器
# （用于 rt_start 的「跨形态重复实例」前置：容器名同形，另一形态存在时新建 = 两实例抢同一 host）
_rt_container_exists_in_form() {
  ALIOTH_CONTAINER_RUNTIME="$1" ALIOTH_RT_PODMAN_MODE="$2" \
    rt_cli inspect --type container "$(rt_instance_name "$3")" >/dev/null 2>&1
}

rt_start() {
  local ns="$1"; shift || true
  local label="" image="$RT_DEFAULT_IMAGE" pkg="" wait=1 dry=0 tier=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --label) label="$2"; shift 2 ;;
      --image) image="$2"; shift 2 ;;
      --pkg) pkg="$2"; shift 2 ;;
      --no-wait) wait=0; shift ;;
      --tier) tier="$2"; shift 2 ;;
      --dry-run) dry=1; shift ;;
      *) echo "❌ rt_start 未知参数: $1" >&2; return 1 ;;
    esac
  done
  ns_ports_known "$ns" || { echo "❌ 未登记的 namespace: ${ns}（Deploy/ports.conf）" >&2; return 1; }
  label="$(rt_ns_lower "${label:-$(rt_ns_lower "$ns")}")"
  rt_label_valid "$label" || { echo "❌ label 非法: ${label}（^[a-z0-9]([a-z0-9-]{0,38}[a-z0-9])?\$）" >&2; return 1; }
  # 环境层：--tier > 配置表 deploy_tier（词表 dev|pre，与 ns-env-file.sh 同源）
  [ -n "$tier" ] || tier="$(rt_deploy_tier)" || return 1
  case "$tier" in dev|pre) ;; *) echo "❌ --tier 非法: ${tier}（只允许 dev|pre）" >&2; return 1 ;; esac
  [ -n "$pkg" ] || pkg="$(rt_instance_pkg "$ns")"
  # 挂载源 MUST 为绝对路径：`-v` 的相对路径会被 podman/docker 当作**命名卷**（实测报
  # `creating named volume "Deploy/WZ": names must match …`）⇒ 统一绝对化（realpath 优先）。
  if [ "${pkg#/}" = "$pkg" ]; then
    pkg="$(cd "$pkg" 2>/dev/null && pwd)" || { echo "❌ --pkg 目录不可解析为绝对路径: $pkg" >&2; return 1; }
  fi
  [ -d "$pkg" ] || { echo "❌ 发布包目录不存在: ${pkg}（先 bash scripts/build-ns.sh $ns release + release-to-namespace.sh）" >&2; return 1; }
  [ -x "$pkg/bin/$(rt_ns_lower "$ns")-server" ] || { echo "❌ 发布包缺二进制: $pkg/bin/$(rt_ns_lower "$ns")-server" >&2; return 1; }

  local key container backend mode be fe sso host ports
  key="$(rt_instance_key "$ns" "$label")"
  container="$(rt_instance_name "$key")"
  # 形态稳定性（2026-09-29 实测修复）：既有实例的形态以注册表登记为准，调用方默认形态 MUST NOT
  # 静默改写它（否则 single-id 实例会被「重建成 native」：原容器从影子 DB 摘除、进程树与端口三元组
  # 仍被占 ⇒ 孤儿 + 同 ns 下次 start 失败 + 入站身份串台）。MUST 在计算 backend/mode 之前调用。
  _rt_follow_recorded_form "$key"
  backend="$(rt_backend)"; if [ "$backend" = podman ]; then mode="$(rt_podman_mode)"; else mode=native; fi
  host="$(rt_instance_host "$ns" "$label")"

  # 同名实例已登记时按**登记形态**判定（容器可能在另一形态的存储里；R9 ❌-14 / R10 ❌-15）
  rt_require_form "$key" || return 3
  local recorded; recorded="$(rt_recorded_form "$key" 2>/dev/null || true)"
  if rt_key_container_exists "$key"; then
    # MUST 用进程权威状态（rt_instance_state），MUST NOT 用 raw podman 状态：单 ID 影子形态下
    # podman 会报 `exited` 而实例进程仍在服务（2026-09-29 实测）⇒ 用 raw 会把「已在运行」误判为
    # 「已停止」并走重建分支（实例被摘除重建、端口被残留占用）。
    local st; st="$(rt_instance_state "$key")"
    if [ "$st" = running ]; then
      # 条目缺失却存在运行中容器 ⇒ MUST NOT 静默返回 0：该实例不可查、入站不可路由（R14 ⚠️-24）
      if [ -z "$recorded" ]; then
        echo "❌ 存在运行中的同名容器但注册表条目缺失: $container" >&2
        echo "   实例将不可查/不可路由 ⇒ 处置：bash scripts/runtime/ns-app-host.sh stop $key 后重起（或人工重建 .runtime/runtime/$key.json）" >&2
        return 1
      fi
      echo "ℹ️  实例已在运行: ${key}（登记形态 ${recorded}）"
      return 0
    fi
    echo "▶ 清理同名的已停止容器: $container" >&2
    rt_cli_for_key "$key" rm -f "$container" >/dev/null 2>&1 || true
  else
    # 当前形态无同名容器 ⇒ MUST 先探**其余全部可用形态**：容器名同形，任一形态已持有该实例时新建
    # 即得**重复实例**（两容器抢同一 host/端口）。MUST NOT 只探某一个形态（R14 ⚠️-24 / R15 ❌-17：
    # 先前硬编码只探 docker/native，single-id 方向漏探 ⇒ 实测可造出重复容器）。
    local ob om cand
    for cand in "docker native" "podman native" "podman single-id"; do
      read -r ob om <<<"$cand"
      [ "$ob" = "$backend" ] && [ "$om" = "$mode" ] && continue
      if rt_form_available "$ob" "$om" && _rt_container_exists_in_form "$ob" "$om" "$key"; then
        echo "❌ 同名容器存在于另一形态（$ob/${om}）: $container —— MUST NOT 新建重复实例（会与既有实例抢同一 host/端口）" >&2
        echo "   处置：ALIOTH_CONTAINER_RUNTIME=$ob bash scripts/runtime/ns-app-host.sh stop $key 后再 start" >&2
        return 1
      fi
    done
  fi

  # 端口 = 该 ns 在 Deploy/ports.conf 的三元组（同构 dev；原 instance_port_band 已退役）
  ports="$(rt_instance_ports "$ns")" || return 1
  be="${ports%% *}"; ports="${ports#* }"
  fe="${ports%% *}"; sso="${ports##* }"

  # 同机同 ns 单实例守卫：端口三元组唯一 ⇒ 第二个实例必然失败；提前 fail-loud（可诊断）
  local other st
  while IFS= read -r other; do
    [ -n "$other" ] || continue
    [ "$other" = "$key" ] && continue
    [ "$(rt_registry_field "$other" namespace 2>/dev/null || true)" = "$ns" ] || continue
    st="$(rt_registry_field "$other" state 2>/dev/null || true)"
    if [ "$st" = running ] || [ "$(rt_key_container_state "$other" 2>/dev/null || true)" = running ]; then
      echo "❌ 同 namespace 已有运行中实例（端口三元组唯一，同机同 ns 仅允许一个）: $other" >&2
      echo "   处置：先 bash scripts/runtime/ns-app-host.sh stop ${other}，或改用另一台机/另一 ns。" >&2
      return 1
    fi
  done < <(rt_registry_keys)

  # env：发布包 .env（解密后）→ 实例化覆盖
  # 注入面 MUST 用 `--env-file`（0600、用后即删），MUST NOT 用逐条 `-e K=V`：
  # 后者会把解密后的整包 .env（含 DATABASE_URL 口令）摆上宿主进程表（`ps` 可见），
  # 与仓库既有惯例（scripts/gateway|sso/deploy-docker.sh 的 --env-file）一致。
  # 宿主网络 ⇒ 容器内绑定地址即宿主地址绑定；一律回环（对外由边缘 nginx 承载）
  local bind_addr="127.0.0.1"
  # SSO 形态 fail-loud（move-ns-registry-to-ns-repos）：读取失败（ns 未声明/声明面不可达）
  # MUST NOT 静默回落 embedded——那会把 remote 的 ns 悄悄降级（openspec mode-lookup-never-defaults-silently）。
  local sso_mode
  sso_mode="$(ns_sso_mode "$ns")" || { echo "❌ 容器实例启动中止：${ns} 的 SSO 形态不可读（ns 仓 .alioth/platform.conf）" >&2; return 1; }

  # 发布包 env **只读一次**且 fail-loud（下游 docker 分支的 DATABASE_URL 改写与合并步共用同一份；
  # 二次读取/吞错会让「包 env 读不到」在某条分支上静默，2026-09-26 R10 ❌-15 同类吞错面自查）
  local pkg_env
  pkg_env="$(rt_pkg_env_exports "$pkg" "$tier")" || { echo "❌ 读取发布包 env 失败（tier=${tier}）" >&2; return 1; }

  local -a net_args vol_args extra_args
  # 两种后端一律 `--network=host`：实例端口三元组 = 宿主端口（与 dev 同构，防漂移）。
  # 前提（实测口径）：本模型要求「容器 --network=host 的 netns = 真实宿主网络」。
  #   · podman rootless（pasta）✅：容器共享宿主地址 ⇒ 容器内绑端口即宿主端口（实测 9005/41719/9006 见于宿主 ss）
  #   · docker rootless（rootlesskit + slirp4netns）❌：容器 "host" = 守护进程所在 slirp netns
  #     （容器内仅见 tap0 10.0.2.100 / docker0；宿主 ss 看不到该端口）⇒ 该形态下本模型不成立，
  #     须改用 rootful Engine 或使守护进程的 host 网络为真实宿主（如 rootlesskit --net=host）。
  #     就绪探针会以「宿主探不到端口」暴露该缺口，rt_start 失败路径会打印本条提示。
  # （2026-09-27 清理）此处曾有 bridge 时代的 `--add-host host.docker.internal:<host-gateway>` 分支，
  # 现两种后端**一律** `--network=host` ⇒ `localhost` 即宿主，无需别名；bridge/slirp 形态（rootless
  # slirp4netns 的 "host" 非真实宿主）**不支持**本模型（见上方前提），故该分支为死代码已删除。
  net_args=(--network=host)
  vol_args=(
    -v "${pkg}:/app:ro"
    --tmpfs /app/logs
    --tmpfs /app/.pids
    --tmpfs /tmp
  )
  # tmpfs 挂载点位于**只读 bind 内** ⇒ MUST 先在宿主侧建目录：容器运行时不保证能对 ro 文件系统
  # 建挂载点（2026-09-26 实测：docker 报 `make mountpoint "/app/.pids": read-only file system`，
  # 而 podman 侧容错 ⇒ 该缺陷在 podman 两形态被掩盖）。目录内容仍由 tmpfs 承载（实例级隔离），
  # 故发布包内容对容器仍不可改写（与仓库启动器 serve-frontend.sh 自建 .pids/logs 同语义）。
  mkdir -p "$pkg/logs" "$pkg/.pids" || { echo "❌ 无法在发布包内准备可写点目录（logs/.pids）: $pkg" >&2; return 1; }
  while IFS= read -r line; do [ -n "$line" ] && extra_args+=("$line"); done < <(rt_run_extra_args)

  local env_dir env_file
  env_dir="$(rt_runtime_root)/env"
  env_file="$env_dir/${key}.env"

  # 实例化覆盖块（**单一来源**：dry-run 的 override_keys 与落盘文件由同一份内容派生，
  # MUST NOT 另写一份键名清单——副本会静默少报，2026-09-25 spec-audit ⚠️-14）
  local db rewritten
  local ovr_content
  # SSO 服务地址按模式取真值（2026-09-26 真包实测）：remote ⇒ 独立 SSO 端口（入口亦规整为同值，
  # 但容器级 env MUST 自身真实，MUST NOT 依赖入口规整掩盖）；embedded ⇒ 网关自身（SSO 进程内挂载）。
  local sso_ep="$be"
  [ "$sso_mode" = remote ] && sso_ep="$sso"
  ovr_content="$(printf 'NAMESPACE=%s\nDEPLOY_PATH=/app\nSERVER_ADDR=%s:%s\nVITE_PORT=%s\nFRONTEND_PORT=%s\nFRONTEND_DIR=/app/frontend\nFRONTEND_LABEL=alioth-%s\nVITE_API_URL=http://127.0.0.1:%s\nSSO_MODE=%s\nSSO_SERVICE_URL=http://127.0.0.1:%s\nCORS_ALLOWED_ORIGINS=%s\nALLOWED_ORIGINS=%s\nALIOTH_INSTANCE_KEY=%s\nALIOTH_INGRESS_HOST=%s\nALIOTH_SSO_PORT=%s\nALIOTH_DEPLOY_TIER=%s\n' \
    "$ns" "$bind_addr" "$be" "$fe" "$fe" "$key" "$be" "$sso_mode" "$sso_ep" \
    "$(rt_origin "$host")" "$(rt_origin "$host")" "$key" "$host" "$sso" "$tier")"
  local ovr_keys
  ovr_keys="$(printf '%s\n' "$ovr_content" | sed -n 's/^\([A-Za-z_][A-Za-z0-9_]*\)=.*/\1/p' | LC_ALL=C sort | tr '\n' ' ' | sed 's/ $//')"

  local -a run_argv=(
    run -d --name "$container"
    --label "alioth.runtime=1"
    --label "alioth.namespace=$ns"
    --label "alioth.instance=$key"
    --restart unless-stopped
    "${extra_args[@]}"
    "${net_args[@]}"
    "${vol_args[@]}"
    --env-file "$env_file"
    "$image"
  )

  if [ "$dry" = 1 ]; then
    printf 'backend=%s\nmode=%s\ntier=%s\nimage=%s\nbe=%s\nfe=%s\nsso=%s\nhost=%s\nmount=%s:/app:ro\nenv_file=%s\noverride_keys=%s\nargv:\n' \
      "$backend" "$mode" "$tier" "$image" "$be" "$fe" "$sso" "$host" "$pkg" "$env_file" "$ovr_keys"
    printf '  %s\n' "${run_argv[@]}"
    return 0
  fi

  # host 网络预检（本模型前提：容器端口 = 宿主端口）——不满足 ⇒ 环境缺口（rc=2），MUST NOT 误判为产物缺陷
  if [ "${ALIOTH_RT_SKIP_NETNS_CHECK:-0}" != 1 ]; then
    local nrc=0
    # 探针端口自选（MUST NOT 用实例端口：探针残留会以 EADDRINUSE 阻塞本次启动）
    rt_host_network_shared "$image" || nrc=$?
    if [ "$nrc" = 1 ]; then
      echo "❌ 该容器引擎的 \`--network=host\` **不是真实宿主网络**（预检端口 :$be 在宿主不可见）" >&2
      echo "   实测口径：podman rootless(pasta) ✅ 满足；docker rootless(rootlesskit+slirp4netns) ❌ 不满足" >&2
      echo "   （容器 "host" = 守护进程的 slirp netns）。处置：改用 rootful Engine / rootlesskit --net=host，或该 ns 走 podman。" >&2
      return 2
    fi
    [ "$nrc" = 2 ] && echo "⚠️  host 网络预检无法判定（探测容器未能启动：镜像/引擎异常）——继续，就绪失败时按环境缺口排查" >&2
  fi


  # 0600 env 注入文件（唯一实现 = rt_env_file_write；通用启动路径同调，MUST NOT 第二份实现）——
  # 先删再建（umask 077 只作用于新建文件，残留同名文件会保留旧权限）；包内 env 去掉被覆盖键后追加覆盖块。
  # 复用上方**唯一一次**读取的 ${pkg_env}（fail-loud 已在读取处收口；此处不再二次读取也不吞错）。
  rt_env_file_write "$env_file" "$ovr_content" "$pkg_env"

  echo "🚀 启动实例 ${key}（backend=$backend/$mode image=$image BE=$be FE=$fe SSO=$sso host=${host}）" >&2
  local cid started_at
  cid="$(rt_cli "${run_argv[@]}" 2>&1)" || {
    echo "❌ 容器启动失败：$cid" >&2
    rm -f "$env_file"
    return 1
  }
  rm -f "$env_file"   # 容器已创建（env 已落入容器配置），宿主不留明文副本
  started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  rt_registry_write "$key" "$ns" "$label" "$container" "$image" "$be" "$fe" "$sso" "$pkg" "$backend" "$mode" "$started_at" "$tier"

  if [ "$wait" = 1 ]; then
    if ! rt_wait_healthy "$be" "$fe" 90; then
      echo "❌ 实例 $key 未在 90s 内就绪：先取证据再**停止实例**（2026-09-29 实测修复——" >&2
      echo "   原实现「保留容器供排查」在 --restart 策略下成无限重启循环，每轮反复抢端口三元组、" >&2
      echo "   状态随机读作 exited，把「发布面缺件」这类真实根因掩盖成运行时异常）" >&2
      if [ "$backend" = docker ]; then
        echo "   ⚠️  docker 后端常见根因：守护进程的 host 网络不是**真实宿主**（rootlesskit+slirp4netns 下容器" >&2
        echo "      '--network=host' = 守护进程的 slirp netns，宿主探不到该端口）⇒ 改用 rootful Engine 或" >&2
        echo "      rootlesskit --net=host 形态；podman rootless(pasta) 不受此限（容器端口即宿主端口）。" >&2
      fi
      rt_cli logs --tail 40 "$container" >&2 || true
      rt_stop "$key" >/dev/null 2>&1 || true
      return 1
    fi
    echo "✅ 实例就绪: $key  →  http://${host}:$(rt_ingress_port)  （直连 FE http://127.0.0.1:${fe}）" >&2
  fi
  printf '%s\n' "$key"
}

# rt_wait_healthy <be_port> <fe_port> <timeout_s>
# 就绪判据 = 后端 /health 且**静态面**可服务：入站代理路由到的是静态面端口，只等后端会把
# 「实例已就绪」提前到静态面起来之前（2026-09-25 冒烟实证：返回后立即访问 FE 得到 connection refused）
rt_wait_healthy() {
  local be="$1" fe="$2" timeout="${3:-90}" waited=0
  local path="${SEED_HEALTH_PATH:-/health}"
  while [ "$waited" -lt "$timeout" ]; do
    if curl -fsS -m 3 -o /dev/null "http://127.0.0.1:${be}${path}" 2>/dev/null \
       && curl -fsS -m 3 -o /dev/null "http://127.0.0.1:${fe}/" 2>/dev/null; then
      return 0
    fi
    sleep 2; waited=$((waited + 2))
  done
  return 1
}

# rt_instance_process_pids <key> —— 打印携带实例身份 env（`ALIOTH_INSTANCE_KEY=<key>`，由 rt_start 的
# 实例化覆盖块注入、被子进程继承）的进程 pid。
# 判据口径：**按 env 身份**而非进程名——容器内进程名各异（wz-server / MainThread / gateway-sso），
# 且 `alioth-ns-entrypoint` 的 cmdline 不含实例 key（key 只出现在 conmon 的 `-n` 参数里）。
rt_instance_process_pids() {
  local key="$1" p pid
  for p in /proc/[0-9]*; do
    pid="${p#/proc/}"
    [ -r "$p/environ" ] || continue
    # 他人进程：`-r` 可能判真而 open 仍被 ptrace 规则拒绝（实测 `Permission denied`）；重定向失败的
    # 报错发生在 tr 之前，`tr … 2>/dev/null` 盖不住 ⇒ MUST 在子壳内读并对子壳整体抑制 stderr。
    ( tr '\0' '\n' < "$p/environ" 2>/dev/null ) 2>/dev/null \
      | grep -qx "ALIOTH_INSTANCE_KEY=${key}" || continue
    printf '%s\n' "$pid"
  done
}

# rt_reap_instance_processes <key> —— 回收实例的容器进程树（TERM → 有界等待 → KILL）。
# 场景（2026-09-29 实测）：`stop`/`rm` 后引擎 DB 已无该容器，而 entrypoint 及其子进程成孤儿
# （ppid=1）继续占着宿主端口三元组 ⇒ 同 ns 下次 start 必失败（state=exited）+ 入站身份串台
# （A 的 FE 响应里出现 `instance=se-smoke-x`）。
rt_reap_instance_processes() {
  local key="$1" pid waited=0 pids
  pids="$(rt_instance_process_pids "$key" | tr '\n' ' ')"
  [ -n "$pids" ] || return 0
  echo "▶ 回收实例进程（引擎 DB 已无该容器）：$key → ${pids% }" >&2
  # shellcheck disable=SC2086  # 空白分隔的 pid 列表（有意分词）
  kill -TERM $pids 2>/dev/null || true
  while [ "$waited" -lt 10 ]; do
    [ -z "$(rt_instance_process_pids "$key")" ] && break
    sleep 1; waited=$((waited + 1))
  done
  pids="$(rt_instance_process_pids "$key" | tr '\n' ' ')"
  if [ -n "$pids" ]; then
    # shellcheck disable=SC2086
    kill -KILL $pids 2>/dev/null || true
    sleep 1
  fi
  return 0
}

rt_backend_container_exists() {
  local name="$1"
  rt_cli inspect --type container "$name" >/dev/null 2>&1
}
rt_backend_container_state() {
  rt_cli inspect --format '{{.State.Status}}' "$1" 2>/dev/null | head -1
}

# rt_container_init_pid <key> —— 容器 PID 1 的宿主 PID（权威来源 = /proc 反查）
# 依据：/proc/<pid>/status 的 NSpid 末位为 1（该进程是某 PID namespace 的 init）
#       且 /proc/<pid>/environ 含 ALIOTH_INSTANCE_KEY=<key>（身份唯一，多实例不误判）。
# 为什么不用 `.State.Pid`：单 ID 形态实测该字段与 conmon pidfile 不一致（state 记 2125868、
# pidfile 记 2126445），据此派信号 ⇒ 打到已退出进程，容器永久停留 stopping（2026-09-25 实证）。
rt_container_init_pid() {
  local key="$1" p nspid
  for p in /proc/[0-9]*; do
    [ -r "$p/status" ] || continue
    nspid="$(sed -n 's/^NSpid:[[:space:]]*//p' "$p/status" 2>/dev/null | head -1)"
    [ -n "$nspid" ] || continue
    [ "${nspid##*[[:space:]]}" = "1" ] || continue
    if ( tr '\0' '\n' < "$p/environ" 2>/dev/null ) 2>/dev/null \
         | grep -qx "ALIOTH_INSTANCE_KEY=${key}"; then
      basename "$p"
      return 0
    fi
  done
  return 1
}

# rt_instance_state <key> —— 实例状态（单 ID 形态下以 /proc 权威判据覆盖 podman 陈旧状态）
# podman 单 ID 形态下 conmon 无法回收容器 init（实测：init 已消失而 state 仍报 running/stopping）
# ⇒ 以「容器 PID 1 是否存活」为准，否则 status/stop 的收敛判定会永远等不到 exited。
rt_instance_state() {
  local key="$1" raw
  raw="$(rt_key_container_state "$key")"
  # 单 ID 影子形态实测（2026-09-29）：podman 状态可与事实相反——`ps`/`inspect` 报 `exited`、
  # `State.Pid=0`，而容器进程（conmon + entrypoint + 业务进程 + node 静态面）仍在且占着宿主端口
  # 三元组（同源缺陷曾产出 `alioth-netns-probe-*` 孤儿；`--restart` 策略下还成重启循环）。
  # 此时 MUST 以**进程事实**为准：否则「已在运行」幂等判据、入站路由、stop 收敛判据全部失真。
  if [ "$raw" != running ]; then
    if [ -n "$(rt_instance_process_pids "$key" 2>/dev/null | head -1)" ]; then
      printf 'running\n'
      return 0
    fi
  fi
  if [ "$raw" = running ] && [ -z "$(rt_container_init_pid "$key" 2>/dev/null)" ]; then
    printf 'exited\n'
    return 0
  fi
  printf '%s\n' "$raw"
}

# rt_backend_stop <key> —— 有界停止容器（**按登记形态**，不依赖运行时信号路径）
# 单 ID 形态下 `podman stop` 不收敛：podman 记录的 `.State.Pid` 与 conmon pidfile 不一致（实证
# 2125868 vs 2126445），据此派信号打到已退出进程 ⇒ 容器永久停留 stopping。
# 故该形态按**权威 PID**（rt_container_init_pid：/proc 反查 NSpid 末位=1 + 实例 env 身份）
# 直投 SIGTERM，有界等待进程消失后升级 SIGKILL；若 conmon 仍未回收（state 不复位），
# 收口时终止 conmon（此时容器 init 已死，conmon 仅剩监控职责）再交 rm -f。
# 形态取自注册表（rt_cli_for_key）——本机形态变化时不得改用当前形态（R9 ❌-14）。
rt_backend_stop() {
  local key="$1" container; container="$(rt_instance_name "$key")"
  local form backend mode pid waited=0
  form="$(rt_recorded_form "$key" || true)"
  backend="${form%% *}"; mode="${form##* }"
  [ -n "$backend" ] || backend="$(rt_backend 2>/dev/null || true)"
  [ -n "$mode" ] || { mode="native"; [ "$backend" = podman ] && mode="$(rt_podman_mode 2>/dev/null || true)"; }
  if [ "$backend" = podman ] && [ "$mode" = single-id ]; then
    # 1) 先摘重启策略：`--restart unless-stopped` 会在我们杀掉容器 init 后立刻重启之，
    #    使 stop 永远收敛不了（实测：TERM init 后 1s 内出现新 init，状态恒 running）。
    #    `podman update` 的资源更新段在本形态必然失败（无 cgroup），但重启策略已在 podman
    #    侧生效——故忽略其非零退出，由下面的状态判据与 conmon 收口兜底。
    rt_cli_for_key "$key" update --restart=no "$container" >/dev/null 2>&1 || true
    # 2) 按权威 PID 投递信号，有界等待进程消失（以 /proc 为准，不依赖 podman 状态复位）
    pid="$(rt_container_init_pid "$key" 2>/dev/null || true)"
    if [ -z "$pid" ]; then
      pid="$(rt_cli_for_key "$key" inspect --format '{{.State.Pid}}' "$container" 2>/dev/null | head -1)"
    fi
    if [ -n "$pid" ] && [ "$pid" != "0" ]; then
      kill -TERM "$pid" 2>/dev/null || true
      while [ "$waited" -lt 20 ]; do
        [ -d "/proc/$pid" ] || break
        sleep 1; waited=$((waited + 1))
      done
      if [ -d "/proc/$pid" ]; then
        kill -KILL "$pid" 2>/dev/null || true
        waited=0
        while [ "$waited" -lt 10 ]; do
          [ -d "/proc/$pid" ] || break
          sleep 1; waited=$((waited + 1))
        done
      fi
    fi
    # 3) conmon 未回收（podman 状态不复位 / 仍有重启循环）⇒ 有界终止 conmon
    local state conmon
    state="$(rt_key_container_state "$key")"
    if [ "$state" != exited ] && [ "$state" != created ]; then
      conmon="$(rt_cli_for_key "$key" inspect --format '{{.ConmonPidFile}}' "$container" 2>/dev/null | head -1)"
      if [ -n "$conmon" ] && [ -r "$conmon" ]; then
        conmon="$(cat "$conmon" 2>/dev/null || true)"
        if [ -n "$conmon" ] && [ -d "/proc/$conmon" ]; then
          kill -TERM "$conmon" 2>/dev/null || true
          sleep 2
          [ -d "/proc/$conmon" ] && kill -KILL "$conmon" 2>/dev/null || true
        fi
      fi
    fi
    # 4) 兜底：若期间发生过重启，再收一次新 init
    pid="$(rt_container_init_pid "$key" 2>/dev/null || true)"
    if [ -n "$pid" ] && [ -d "/proc/$pid" ]; then
      kill -TERM "$pid" 2>/dev/null || true
      sleep 2
      [ -d "/proc/$pid" ] && kill -KILL "$pid" 2>/dev/null || true
    fi
    return 0
  fi
  rt_cli_for_key "$key" stop -t 10 "$container" >/dev/null 2>&1 || true
  return 0
}

# rt_stop <key>
rt_stop() {
  local key="$1" container state form
  container="$(rt_instance_name "$key")"
  # 登记形态不可用 / 注册表条目读不出 ⇒ fail-loud 且 MUST NOT 清注册表（R9 ❌-14 / R10 ❌-15）
  rt_require_form "$key" || return 3
  # 跨形态 + 进程面收口（2026-09-29 实测修复）：容器名同形 ⇒ 任一**可用形态**持有同 key 容器都 MUST
  # 被清除；引擎 DB 已无容器而进程树存活（孤儿 entrypoint + 子进程仍占宿主端口三元组）时 MUST 按实例
  # 身份（env `ALIOTH_INSTANCE_KEY`）回收，并在未收口时 fail-loud（MUST NOT 静默清注册表）。
  local found=0 b m cand leftover_ctr="" leftover_pids
  if rt_key_container_exists "$key"; then
    found=1
    rt_backend_stop "$key"
    state="$(rt_instance_state "$key")"
    if [ "$state" != exited ] && [ "$state" != created ]; then
      echo "⚠️  stop 未收敛（state=${state}），改用 kill 强制收口: $container" >&2
      rt_cli_for_key "$key" kill "$container" >/dev/null 2>&1 || true
    fi
    if ! rt_cli_for_key "$key" rm -f "$container" >/dev/null 2>&1; then
      echo "❌ 容器删除失败: $container" >&2
      return 1
    fi
  fi
  for cand in "docker native" "podman native" "podman single-id"; do
    read -r b m <<<"$cand"
    rt_form_available "$b" "$m" || continue
    _rt_container_exists_in_form "$b" "$m" "$key" || continue
    found=1
    echo "▶ 清理另一形态（${b}/${m}）的同名容器: $container" >&2
    _rt_cli_in_form "$b" "$m" rm -f "$container" >/dev/null 2>&1 || true
  done
  rt_reap_instance_processes "$key"
  for cand in "docker native" "podman native" "podman single-id"; do
    read -r b m <<<"$cand"
    rt_form_available "$b" "$m" || continue
    _rt_container_exists_in_form "$b" "$m" "$key" && leftover_ctr="${leftover_ctr}容器(${b}/${m}) "
  done
  leftover_pids="$(rt_instance_process_pids "$key" | tr '\n' ' ')"
  if [ -n "$leftover_ctr" ] || [ -n "$leftover_pids" ]; then
    echo "❌ 实例 $key 未收口：${leftover_ctr}${leftover_pids}（MUST NOT 静默清注册表——否则入站/端口归属失真）" >&2
    return 1
  fi
  rt_registry_remove "$key"
  if [ "$found" = 0 ]; then
    echo "ℹ️  实例不存在（已清理注册表）: $key"
  else
    echo "🛑 已停止实例: $key"
  fi
}

# rt_status <key> —— 输出 "key=... state=... health=... host=... be=... fe=... sso=... image=... startedAt=..."
# ── 宿主进程面实例（进程面演示/发布）的派生视图 ───────────────────────
#
# 定位：容器注册表（`$key.json`）只记**容器**实例；进程面实例（`scripts/gateway/{start,restart}.sh`、
# `scripts/sso/*`、`scripts/dev/dev-gateway.sh`）的单一事实源 = 宿主注册表
# `Deploy/{ns}/.runtime/{ns}.pid`（读写唯一实现 = ns-ports.sh 的 `ns_registry_*`）。
# 本组函数把宿主注册表**派生**为与容器实例同构的记录，供 `list`/`status`/`ps` 统一可见。
# MUST NOT 反向写入容器注册表（两平面注册表不合并）。两平面互斥（同 ns 同端口三元组）；
# 若两者同时存在 ⇒ **容器条目优先**（避免同一 ns 出现两条互相矛盾的记录）。

# rt_host_ns_all —— 已登记 ns 的**规范名**逐行（`ns_ports_list` 首字段；缺失 ⇒ 空）
rt_host_ns_all() {
  ns_ports_list 2>/dev/null | awk 'NF { print $1 }'
}

# rt_host_state <ns> → running|degraded|stale|absent
#   running  = ns_registry_health 严格通过（逐服务存活且身份确属本项目）
#   degraded = 至少一个记录进程存活（严格判定不通过：FE/SSO 未记录/已死）
#   stale    = 有注册表文件但无任何记录进程存活
rt_host_state() {
  local ns="$1"
  [ -f "$(ns_registry_file "$ns")" ] || { printf 'absent\n'; return 0; }
  if ns_registry_health "$ns" >/dev/null 2>&1; then printf 'running\n'; return 0; fi
  if ns_registry_status "$ns" >/dev/null 2>&1; then printf 'degraded\n'; return 0; fi
  printf 'stale\n'
}

# rt_host_tier <ns> <db> → dev|pre|unknown（层→库名唯一实现 = ns-db-name.sh）
rt_host_tier() {
  local ns="$1" db="$2" t v
  [ -n "$db" ] || { printf 'unknown\n'; return 0; }
  command -v ns_name_to_db >/dev/null 2>&1 || { printf 'unknown\n'; return 0; }
  for t in dev pre; do
    v="$(ns_name_to_db "$ns" "$t" 2>/dev/null || true)"
    [ -n "$v" ] && [ "$v" = "$db" ] && { printf '%s\n' "$t"; return 0; }
  done
  printf 'unknown\n'
}

# rt_host_hostname <ns> —— 该 ns 的入站主机名（`<host_prefix>-<ns_lower>.<domain_suffix>`；缺配置 ⇒ 空）
rt_host_hostname() {
  local ns="$1" hp ds
  hp="$(rt_host_prefix 2>/dev/null || true)"
  ds="$(rt_domain_suffix 2>/dev/null || true)"
  [ -n "$hp" ] && [ -n "$ds" ] || return 0
  printf '%s-%s.%s\n' "$hp" "$(rt_ns_lower "$ns")" "$ds"
}

# rt_host_kv <ns> —— 与 rt_status 同构的 k=v 行（+ `backend=host`）；无宿主记录 ⇒ rc=1
rt_host_kv() {
  local ns="$1" st be fe sso db tier health=unknown
  st="$(rt_host_state "$ns")"
  [ "$st" = absent ] && return 1
  be="$(ns_port "$ns" be 2>/dev/null || true)"
  fe="$(ns_port "$ns" fe 2>/dev/null || true)"
  sso="$(ns_port "$ns" sso 2>/dev/null || true)"
  db="$(ns_registry_pid "$ns" DB_NAME 2>/dev/null || true)"
  tier="$(rt_host_tier "$ns" "$db")"
  if [ "${RT_SKIP_HEALTH:-0}" != 1 ]; then
    health=down
    # degraded 也可能 BE 健康（FE/SSO 缺失或未记录）⇒ 只要非 stale 且有 BE 端口即探针
    if [ "$st" != stale ] && [ -n "$be" ] && \
       curl -fsS -m 3 -o /dev/null "http://127.0.0.1:${be}${SEED_HEALTH_PATH:-/health}" 2>/dev/null; then
      health=up
    fi
  fi
  printf 'key=%s namespace=%s label=%s tier=%s state=%s health=%s host=%s be=%s fe=%s sso=%s image=%s startedAt=%s backend=host\n' \
    "$(rt_ns_lower "$ns")" "$ns" host "$tier" "$st" "$health" "$(rt_host_hostname "$ns")" \
    "$be" "$fe" "$sso" "-" ""
}

# rt_host_ns_for_key <key> —— key（容器键或 ns 名，大小写不敏感）→ 规范 ns 名；仅当该 ns
# **有宿主记录且无容器条目**时命中（容器条目优先）
rt_host_ns_for_key() {
  local key="$1" ns lower
  [ -n "$key" ] || return 1
  lower="$(rt_ns_lower "$key")"
  while IFS= read -r ns; do
    [ -n "$ns" ] || continue
    [ "$(rt_ns_lower "$ns")" = "$lower" ] || continue
    [ -f "$(ns_registry_file "$ns")" ] || return 1
    [ -f "$(rt_registry_file "$lower")" ] && return 1
    printf '%s\n' "$ns"; return 0
  done < <(rt_host_ns_all)
  return 1
}

rt_status() {
  local key="$1" container state health be file kv form host_ns
  # ① 进程面实例：单一事实源 = 宿主注册表（不依赖容器引擎可用性；容器条目存在时让位）
  host_ns="$(rt_host_ns_for_key "$key" 2>/dev/null || true)"
  if [ -n "$host_ns" ]; then
    rt_host_kv "$host_ns"
    return $?
  fi
  # ② 容器实例
  container="$(rt_instance_name "$key")"
  file="$(rt_registry_file "$key")"
  # 登记形态不可用 / 注册表条目读不出 ⇒ 可诊断状态且 MUST NOT 清注册表（R9 ❌-14 / R10 ❌-15）
  # 注意：**不得**抑制 stderr——守卫的人读理由（为什么不可用 / 怎么补救）是人读面唯一来源（R11 ⚠️-21）
  if ! rt_require_form "$key"; then
    local form_field
    if [ -n "$RT_FORM_BACKEND" ]; then form_field="$RT_FORM_BACKEND/$RT_FORM_MODE"; else form_field="unreadable"; fi
    printf 'key=%s state=unknown-form-unavailable health=unknown registeredForm=%s hint=%s\n' \
      "$key" "$form_field" "$(_rt_form_hint_token "$RT_FORM_BACKEND" "$RT_FORM_MODE")"
    return 1
  fi
  if ! rt_key_container_exists "$key"; then
    [ -f "$file" ] && rt_registry_remove "$key"
    printf 'key=%s state=absent health=unknown\n' "$key"
    return 1
  fi
  state="$(rt_instance_state "$key")"
  kv="$(rt_registry_kv "$key" 2>/dev/null || true)"
  be="$(printf '%s\n' "$kv" | sed -n 's/^bePort=//p')"
  health=unknown
  # `RT_SKIP_HEALTH=1`（由 CLI `--no-health` 设置）：**路由/快照面**不做逐键健康探针——
  # 逐键探针让快照刷新耗时 ≈ 探针超时且（同步子进程）阻塞在线服务事件循环 ⇒ 请求被卡
  # （2026-09-26 实测：5 实例平面下 `/health` 冷请求连续超时、冒烟判「控制面未就绪」）。
  if [ "${RT_SKIP_HEALTH:-0}" != 1 ]; then
    health=down
    if [ "$state" = running ] && [ -n "$be" ] && curl -fsS -m 3 -o /dev/null "http://127.0.0.1:${be}${SEED_HEALTH_PATH:-/health}" 2>/dev/null; then
      health=up
    fi
  fi
  printf 'key=%s namespace=%s label=%s tier=%s state=%s health=%s host=%s be=%s fe=%s sso=%s image=%s startedAt=%s\n' \
    "$key" "$(printf '%s\n' "$kv" | sed -n 's/^namespace=//p')" "$(printf '%s\n' "$kv" | sed -n 's/^label=//p')" \
    "$(printf '%s\n' "$kv" | sed -n 's/^tier=//p')" \
    "$state" "$health" "$(printf '%s\n' "$kv" | sed -n 's/^host=//p')" "$be" "$(printf '%s\n' "$kv" | sed -n 's/^fePort=//p')" \
    "$(printf '%s\n' "$kv" | sed -n 's/^ssoPort=//p')" \
    "$(printf '%s\n' "$kv" | sed -n 's/^image=//p')" "$(printf '%s\n' "$kv" | sed -n 's/^startedAt=//p')"
}

# rt_list —— 每行一个实例的状态输出
# 并发探测（2026-09-26 实测：串行时每键最多一个探针超时 ⇒ N 个实例 = N×3s，平面 7 实例即 19.5s，
# 入站快照与控制面列表被拖垮）⇒ 并发后总耗时 ≈ 单次探针超时，输出顺序与 rc 语义保持不变。
rt_list() {
  local key rc=0 tmpd i=0 ns lower st
  tmpd="$(mktemp -d "${TMPDIR:-/tmp}/rt-list.XXXXXX")" || return 1
  local -a keys=()
  while IFS= read -r key; do [ -n "$key" ] && keys+=("$key"); done < <(rt_registry_keys)
  # bash 3.2（macOS /bin/bash）在 `set -u` 下展开**空数组** `"${keys[@]}"` 会报
  # `keys[@]: unbound variable` ⇒ 零实例时整个 `list` 失败（实测 2026-09-27：`rt_list` rc=1
  # ⇒ router 快照刷新失败并保留旧路由 = 热摘除失效）。故 MUST 先判元素数（`${#keys[@]}` 对空数组安全）。
  if [ "${#keys[@]}" -gt 0 ]; then
    for key in "${keys[@]}"; do
      ( rt_status "$key" >"$tmpd/$i.out" 2>"$tmpd/$i.err"; echo "$?" >"$tmpd/$i.rc" ) &
      i=$((i + 1))
    done
  fi
  wait
  for ((i = 0; i < ${#keys[@]}; i++)); do
    cat "$tmpd/$i.out"
    if [ "$(cat "$tmpd/$i.rc" 2>/dev/null || echo 1)" != 0 ]; then
      rc=1
      cat "$tmpd/$i.err" >&2
    fi
  done
  rm -rf "$tmpd"
  # 进程面实例（宿主注册表派生）：未被同名容器条目覆盖的 ns
  while IFS= read -r ns; do
    [ -n "$ns" ] || continue
    lower="$(rt_ns_lower "$ns")"
    [ -f "$(rt_registry_file "$lower")" ] && continue
    st="$(rt_host_state "$ns")"
    [ "$st" = absent ] && continue
    rt_host_kv "$ns" || true
  done < <(rt_host_ns_all)
  return "$rc"
}

# rt_logs <key> [--tail N] —— 按**登记形态**取日志（跨形态可用）
rt_logs() {
  local key="$1"; shift
  rt_cli_for_key "$key" logs "$@" "$(rt_instance_name "$key")"
}

# rt_ps —— **当前形态**的托管容器一览（按 label 过滤）；跨形态全量视图 = `list`（注册表驱动）
rt_ps() {
  local backend mode ns st be fe sso db
  backend="$(rt_backend 2>/dev/null || true)"
  mode="native"; [ "$backend" = podman ] && mode="$(rt_podman_mode 2>/dev/null || true)"
  printf '# form=%s/%s（跨形态全量请用 list）\n' "$backend" "$mode"
  rt_cli ps -a --filter "label=alioth.runtime=1" \
    --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}\t{{.Image}}'
  # 进程面实例（宿主注册表派生；与容器实例统一视图）
  printf '\n# host（进程面；单一事实源 = Deploy/{ns}/.runtime/{ns}.pid）\n'
  printf '%-16s %-9s %-20s %s\n' NAMESPACE STATE be/fe/sso DB
  while IFS= read -r ns; do
    [ -n "$ns" ] || continue
    st="$(rt_host_state "$ns")"
    [ "$st" = absent ] && continue
    be="$(ns_port "$ns" be 2>/dev/null || true)"
    fe="$(ns_port "$ns" fe 2>/dev/null || true)"
    sso="$(ns_port "$ns" sso 2>/dev/null || true)"
    db="$(ns_registry_pid "$ns" DB_NAME 2>/dev/null || true)"
    printf '%-16s %-9s %-20s %s\n' "$ns" "$st" "${be}/${fe}/${sso}" "$db"
  done < <(rt_host_ns_all)
}

# ── 通用启动（验证平面）原语 ──────────────────────────────────────────────
#
# 定位：供 `scripts/runtime/launch-image.sh` 使用（change add-generic-container-launch）。
# 与入站托管平面**共享**本 lib 的引擎探测/派发/JSON/端口/env 原语，但**不共享**实例注册表与
# Host 路由语义（design D1/D3/D8/D9）。端点解析的**唯一消费者** = `_rt_cli_raw`。
#
# 未复用说明（REUSE_FIRST §3：新实现 MUST 与现成实现显式区分）：
#   · `rt_engine_reachable` **未复用** `rt_form_available`——后者按 `(backend,mode)` 进程内缓存、
#     不含端点维度；用它探远端端点会返回本地陈旧结论。
#   · `rt_generic_probe_exec` **未复用** `rt_wait_healthy`——后者是宿主侧 `curl 127.0.0.1`，
#     远端引擎下端口发布在目标机 ⇒ 宿主回环不可达。
#   · 端口 **取自** `Deploy/ports.conf` 三元组（`rt_instance_ports`；单一来源，不做分配）；占用判定复用 `rt_port_listening`。
#   · env 落盘 **复用** `rt_env_file_write`（自 `rt_start` 抽出，两处同调，MUST NOT 第二份 0600 实现）。

# 敏感键标记（与 `Meta/backend/app-agent/src/trace.rs::SENSITIVE_KEY_MARKERS` 同口径；
# 跨语言无共享机制，双处 MUST 同步）
RT_SENSITIVE_KEY_MARKERS="api_key apikey secret password passwd token private_key authorization dsn database_url connection_string encryption_key credential 密码 密钥 凭据"

# _rt_endpoint_parse <uri> —— 标准 URL 解析器（NO_REGEX_FOR_PARSING）
# 输出 3 行：scheme / host / port（非法 URI 或不支持的 scheme ⇒ rc=1）
_rt_endpoint_parse() {
  local uri="$1" runner
  runner="$(_rt_json_runner)" || return 1
  "$runner" -e '
    const raw = process.argv[1];
    let u;
    try { u = new URL(raw); } catch { process.stderr.write(`❌ 非法端点 URI: ${raw}\n`); process.exit(1); }
    const scheme = u.protocol.replace(/:$/, "");
    if (scheme !== "unix" && scheme !== "ssh" && scheme !== "tcp") {
      process.stderr.write(`❌ 不支持的端点 scheme: ${scheme}（只允许 unix:// | ssh:// | tcp://）\n`);
      process.exit(1);
    }
    process.stdout.write(`${scheme}\n${u.hostname}\n${u.port}\n`);
  ' "$uri"
}

# rt_engine_resolve <engine|""> <host|"">
# 设置 RT_ENGINE / RT_ENGINE_ENDPOINT / RT_ENGINE_REMOTE / RT_ENGINE_SCHEME / RT_ENGINE_HOST_ALIAS
# 非法引擎、非法/不支持 scheme、`tcp://` 未显式放行（RT_ALLOW_INSECURE_TCP=1）⇒ rc=1 fail-loud
rt_engine_resolve() {
  local engine="${1:-}" host="${2:-}" ep parsed scheme ephost epport="" _unused
  RT_ENGINE=""; RT_ENGINE_ENDPOINT=""; RT_ENGINE_REMOTE=0; RT_ENGINE_SCHEME=""; RT_ENGINE_HOST_ALIAS=""
  if [ -n "$engine" ]; then
    case "$engine" in
      podman|docker) RT_ENGINE="$engine" ;;
      *) echo "❌ --engine 非法: ${engine}（只允许 podman|docker）" >&2; return 1 ;;
    esac
  else
    RT_ENGINE="$(rt_backend)" || return 1
  fi
  ep="$host"
  if [ -z "$ep" ]; then
    if [ "$RT_ENGINE" = docker ]; then ep="${DOCKER_HOST:-}"; else ep="${CONTAINER_HOST:-}"; fi
  fi
  [ -n "$ep" ] || return 0                      # 本机默认端点
  parsed="$(_rt_endpoint_parse "$ep")" || return 1
  { IFS= read -r scheme; IFS= read -r ephost; IFS= read -r epport; } <<<"$parsed"
  if [ "$scheme" = tcp ] && [ "${RT_ALLOW_INSECURE_TCP:-0}" != 1 ]; then
    echo "❌ 拒绝明文容器引擎端点: $ep" >&2
    echo "   该传输无加密且通常无认证（凭据与镜像上下文会明文过网）；推荐 --host ssh://user@host。" >&2
    echo "   确需使用时 MUST 显式传 --allow-insecure-tcp。" >&2
    return 1
  fi
  [ "$scheme" = tcp ] && echo "⚠️  使用明文容器引擎端点（--allow-insecure-tcp）: $ep" >&2
  RT_ENGINE_ENDPOINT="$ep"
  RT_ENGINE_SCHEME="$scheme"
  case "$scheme" in ssh|tcp) RT_ENGINE_REMOTE=1 ;; *) RT_ENGINE_REMOTE=0 ;; esac
  RT_ENGINE_HOST_ALIAS="${ephost:-}"
  return 0
}

# rt_engine_reachable —— 0 = 引擎可用；2 = 端点不可达（环境缺口）；1 = 其他
# 缓存键 = 端点（**未复用** rt_form_available 的 (backend,mode) 缓存，见文件头未复用说明）
rt_engine_reachable() {
  local key="${RT_ENGINE:-none}|${RT_ENGINE_ENDPOINT:-local}" backend rc=0
  if [ -n "${_RT_REACH_KEY:-}" ] && [ "$_RT_REACH_KEY" = "$key" ]; then return "$_RT_REACH_VAL"; fi
  if [ -n "${RT_ENGINE_ENDPOINT:-}" ]; then
    case "${RT_ENGINE}" in
      docker) timeout 8 env DOCKER_HOST="${RT_ENGINE_ENDPOINT}" docker info >/dev/null 2>&1 || rc=2 ;;
      podman) timeout 8 env CONTAINER_HOST="${RT_ENGINE_ENDPOINT}" podman --remote info >/dev/null 2>&1 || rc=2 ;;
      *) rc=2 ;;
    esac
  else
    backend="${RT_ENGINE:-}"
    [ -n "$backend" ] || backend="$(rt_backend 2>/dev/null || true)"
    case "$backend" in
      docker) timeout 5 docker info >/dev/null 2>&1 || rc=2 ;;
      podman)
        if timeout 5 podman info >/dev/null 2>&1; then
          rc=0
        elif rt_form_available podman single-id; then
          timeout 30 env RT_STORAGE_ROOT="$(_rt_podman_storage_root)" RT_RUN_ROOT="$(_rt_podman_run_root)" \
            unshare --user --map-root-user --mount "${PROJECT_ROOT}/scripts/runtime/podman-single-id-inner.sh" info >/dev/null 2>&1 || rc=2
        else
          rc=2
        fi ;;
      *) rc=2 ;;
    esac
  fi
  _RT_REACH_KEY="$key"; _RT_REACH_VAL="$rc"
  return "$rc"
}

# rt_sha256_file <path> —— 标准哈希工具（coreutils → shasum → openssl）
rt_sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}'
  else openssl dgst -sha256 "$1" | awk '{print $NF}'; fi
}

# rt_artifact_fingerprint <pkg_dir> —— 产物指纹（sha256(apps.json? + 排序后的 bin/*-server) 前 8 位）
# 输入面 = 业务二进制（必有）+ apps.json（发布面必有；夹具包可缺，缺则只按二进制派生）
rt_artifact_fingerprint() {
  local pkg="$1" acc="" f n=0
  if [ -f "$pkg/apps.json" ]; then
    acc="$(rt_sha256_file "$pkg/apps.json")"
    n=$((n + 1))
  fi
  for f in $(ls -1 "$pkg"/bin/*-server 2>/dev/null | LC_ALL=C sort); do
    [ -f "$f" ] || continue
    acc="$acc$(rt_sha256_file "$f")"
    n=$((n + 1))
  done
  if [ "$n" = 0 ]; then
    echo "❌ 发布包缺指纹输入（bin/*-server 或 apps.json）: $pkg" >&2
    return 1
  fi
  printf '%s' "$acc" | { if command -v sha256sum >/dev/null 2>&1; then sha256sum; elif command -v shasum >/dev/null 2>&1; then shasum -a 256; else openssl dgst -sha256; fi; } | awk '{print substr($1,1,8)}'
}

# rt_ports_explicit <be> <fe> <sso> —— 校验显式三元组（正整数、互异）；打印 "be fe sso"
rt_ports_explicit() {
  local be="$1" fe="$2" sso="$3" p
  for p in "$be" "$fe" "$sso"; do
    case "$p" in ''|*[!0-9]*) echo "❌ --port-* 必须为正整数（收到 '$p'）" >&2; return 1 ;; esac
    if [ "$p" -lt 1 ] || [ "$p" -gt 65535 ]; then echo "❌ --port-* 超出 1-65535: $p" >&2; return 1; fi
  done
  if [ "$be" = "$fe" ] || [ "$be" = "$sso" ] || [ "$fe" = "$sso" ]; then
    echo "❌ --port-be/--port-fe/--port-sso 必须互异（收到 $be/$fe/${sso}）" >&2; return 1
  fi
  printf '%s %s %s\n' "$be" "$fe" "$sso"
}

# rt_ports_generic <ns|""> <be|""> <fe|""> <sso|""> —— 通用（验证平面）实例端口三元组
# 语义（band 退役后）：显式 ⇒ 校验；给了 ns ⇒ **取自 Deploy/ports.conf 三元组**（与托管/dev 同构）；
# 两者皆无（如 `--image` 任意镜像）⇒ fail-loud 要求显式端口（不再自动分配）。
rt_ports_generic() {
  local ns="$1" be="$2" fe="$3" sso="$4"
  if [ -n "$be" ] || [ -n "$fe" ] || [ -n "$sso" ]; then
    if [ -z "$be" ] || [ -z "$fe" ] || [ -z "$sso" ]; then
      echo "❌ 显式端口 MUST 三者齐备（--port-be/--port-fe/--port-sso）" >&2; return 1
    fi
    rt_ports_explicit "$be" "$fe" "$sso" || return 1   # 校验失败 MUST NOT 被吞（明示端口是最强的用户意图）
    return 0
  fi
  if [ -n "$ns" ]; then rt_instance_ports "$ns"; return $?; fi
  echo "❌ 未给 --ns 时 MUST 显式指定端口（--port-be/--port-fe/--port-sso）：端口不再自动分配（防环境差异与漂移）" >&2
  return 1
}

# _rt_extra_args_quoted —— 把形态附加参数拼为 shell 安全串（供 setsid bash -c 内联使用）
_rt_extra_args_quoted() {
  local out="" line
  while IFS= read -r line; do
    [ -n "$line" ] && out="$out $(printf '%q' "$line")"
  done < <(rt_run_extra_args)
  printf '%s' "${out# }"
}

# rt_probe_free_port —— 选一个**当前未被监听**的探针端口（MUST NOT 复用实例端口：探针残留会以
# `EADDRINUSE` 阻塞真实实例启动——2026-09-27 实测：single-id 影子内探针容器删除失败时，
# 探针进程仍占着实例 BE 端口 ⇒ 随后 start 报 EADDRINUSE 且真因不可见）。
rt_probe_free_port() {
  local p="${1:-42100}" limit=$(( ${1:-42100} + 300 ))
  while [ "$p" -lt "$limit" ]; do
    rt_port_listening "$p" || { printf '%s\n' "$p"; return 0; }
    p=$((p + 1))
  done
  return 1
}

# rt_host_network_shared <image> [probe_port] —— 0 = 该引擎的 `--network=host` 即**真实宿主网络**
# 为什么需要：本模型要求「容器端口 = 宿主端口」（见文件头）。podman rootless(pasta) 满足；
#   docker rootless(rootlesskit+slirp4netns) **不满足**（容器 "host" = 守护进程的 slirp netns，
#   实测容器内仅见 tap0 10.0.2.100/docker0，宿主 ss 看不到端口）⇒ 必须启动前判别，否则会以
#   「容器内探针通过但宿主不可达」或「连不上宿主 DB」的形式误判为产物缺陷。
# 做法：起一次性容器在 `--network=host` 下绑 `<probe_port>`，看**宿主**是否观察到该监听。
# 返回：0 共享；1 不共享（环境缺口）；2 无法探测（镜像缺失/引擎异常 ⇒ 亦按环境缺口处理）
rt_host_network_shared() {
  local image="$1" port="${2:-}" cname="alioth-netns-probe-$$" w=0
  local -a EXTRA=() line
  # 缺省自选空闲端口（调用方 MUST NOT 传实例端口：探针残留会以 EADDRINUSE 阻塞真实实例）
  [ -n "$port" ] || port="$(rt_probe_free_port)" || return 2
  # 形态附加参数 MUST 带上（single-id 下缺它 ⇒ crun 建 cgroup 失败 ⇒ 预检恒「无法判定」）
  while IFS= read -r line; do [ -n "$line" ] && EXTRA+=("$line"); done < <(rt_run_extra_args)
  rt_container_host_kill "$cname" >/dev/null 2>&1 || true   # 同名残留先清（幂等）
  # MUST 覆盖 ENTRYPOINT（运行时镜像的 entrypoint 是发布包启动器，会吞掉命令行参数 ⇒ 探测容器立刻退出）
  if ! rt_cli run -d --name "$cname" --network=host "${EXTRA[@]}" --entrypoint node "$image" \
       -e "require('net').createServer().listen(${port}, '127.0.0.1')" >/dev/null 2>&1; then
    rt_container_host_kill "$cname" >/dev/null 2>&1 || true
    return 2
  fi
  local rc=1
  while [ "$w" -lt 12 ]; do
    if rt_port_listening "$port"; then rc=0; break; fi
    sleep 0.3; w=$((w + 1))
  done
  # 收口：宿主兜底清理（single-id 影子内 `podman stop/rm` 不收敛 ⇒ 残留会占端口/名字）
  rt_container_host_kill "$cname" >/dev/null 2>&1 || true
  # 退出前确认端口已释放：仍未释放 ⇒ 按环境缺口处理（rc=2，MUST NOT 误判为「不共享」）
  w=0
  while [ "$w" -lt 10 ]; do
    rt_port_listening "$port" || break
    sleep 0.3; w=$((w + 1))
  done
  if rt_port_listening "$port"; then
    echo "⚠️  探针端口 $port 未释放（探针容器残留）——按环境缺口处理" >&2
    rc=2
  fi
  return "$rc"
}

# rt_generic_name <slug> —— 通用实例容器名（独立前缀 `alioth-rt-<slug>`，与托管平面 `alioth-ns-<key>` 不冲突）
rt_generic_name() {
  local slug; slug="$(rt_ns_lower "$1")"
  slug="$(printf '%s' "$slug" | tr -c 'a-z0-9-' '-' | sed 's/-\{2,\}/-/g; s/^-//; s/-$//' | cut -c1-40)"
  slug="${slug%-}"
  [ -n "$slug" ] || { echo "❌ 无法从 '$1' 派生通用实例名" >&2; return 1; }
  printf 'alioth-rt-%s\n' "$slug"
}

# rt_env_file_write <env_file> <ovr_content> <pkg_env> —— 0600 env 注入文件（唯一实现）
# 语义：包内 env 去掉被覆盖键 → 覆盖块追加在后；先删再建（umask 077 只作用于新建文件）
rt_env_file_write() {
  local env_file="$1" ovr_content="$2" pkg_env="$3" ovr_file="${1}.overrides.tmp"
  local dir; dir="$(dirname "$env_file")"; mkdir -p "$dir"
  rm -f "$env_file" "$ovr_file"
  ( umask 077; : > "$env_file"; printf '%s\n' "$ovr_content" > "$ovr_file" )
  printf '%s\n' "$pkg_env" \
    | awk -F= 'NR==FNR { ov[$1]=1; next } !($1 in ov)' "$ovr_file" - >> "$env_file"
  cat "$ovr_file" >> "$env_file"
  rm -f "$ovr_file"
}

# rt_probe_runtime <container> —— 容器内可用的探针运行时（node|curl|空）
rt_probe_runtime() {
  rt_cli exec "$1" sh -c 'command -v node >/dev/null 2>&1 && printf node || { command -v curl >/dev/null 2>&1 && printf curl || printf ""; }' 2>/dev/null | head -1
}

# rt_generic_probe_exec <container> <url> —— 容器内 HTTP 探针（引擎/远端无关；未复用宿主侧 rt_wait_healthy）
# 输出 "<status> <duration_ms>"；0 = 2xx/3xx；2 = 探针运行时缺失（环境缺口）
rt_generic_probe_exec() {
  local c="$1" url="$2" kind out=""
  kind="$(rt_probe_runtime "$c")"
  case "$kind" in
    node)
      out="$(rt_cli exec "$c" node -e '
const h = require("http");
const t = Date.now();
const done = (s) => { process.stdout.write(`${s} ${Date.now() - t}`); process.exit(s > 0 && s < 400 ? 0 : 1); };
const r = h.get(process.argv[1], (res) => { res.resume(); done(res.statusCode); });
r.on("error", () => done(0));
r.setTimeout(5000, () => { r.destroy(); done(0); });
' "$url" 2>/dev/null || true)" ;;
    curl)
      out="$(rt_cli exec "$c" curl -s -o /dev/null -m 5 -w '%{http_code} %{time_total}' "$url" 2>/dev/null \
        | awk '{printf "%s %d", $1, $2 * 1000}' || true)" ;;
    *)
      echo "❌ 容器 $c 内既无 node 也无 curl：无法执行就绪探针（探针运行时缺失 = 环境缺口，退出码 2）" >&2
      return 2 ;;
  esac
  out="$(printf '%s' "$out" | tr -d '\r' | head -1)"
  [ -n "$out" ] || out="0 0"
  printf '%s\n' "$out"
  case "${out%% *}" in 1??|2??|3??) return 0 ;; *) return 1 ;; esac
}

# rt_logs_redacted <container> <tail> —— 脱敏 + 限行日志摘要（供证据与回灌；MUST NOT 原样透出）
rt_logs_redacted() {
  local c="$1" tail="${2:-40}"
  rt_cli logs --tail "$tail" "$c" 2>&1 | MARKERS="$RT_SENSITIVE_KEY_MARKERS" awk '
    BEGIN { n = split(ENVIRON["MARKERS"], m, " "); }
    {
      low = tolower($0); hit = 0
      for (i = 1; i <= n; i++) if (index(low, m[i]) > 0) { hit = 1; break }
      if (!hit) { print; next }
      if (match($0, /^[[:space:]]*[A-Za-z0-9_.-]+[=:]/)) { print substr($0, 1, RLENGTH) "***"; next }
      p = index($0, "://"); a = index($0, "@")
      if (p > 0 && a > p) { print substr($0, 1, p + 2) "***" substr($0, a); next }
      print "[redacted 命中敏感标记]"
    }
  ' | tail -n "$tail"
}

# rt_evidence_json —— stdin 收 `k=v` 行 ⇒ 单行证据 JSON（标准序列化器；未知键 fail-loud）
rt_evidence_json() {
  local runner; runner="$(_rt_json_runner)" || return 1
  "$runner" -e '
const fs = require("fs");
const allowed = new Set(["verdict","container","engine","endpoint","remote","image_ref","image_digest","port_be","port_fe","port_sso","logs_cmd","duration_ms","started_at"]);
const probeIdx = [];
const kv = {};
for (const line of fs.readFileSync(0, "utf8").split("\n")) {
  if (!line) continue;
  const i = line.indexOf("=");
  if (i < 0) { process.stderr.write(`❌ 证据行缺少 "=": ${line}\n`); process.exit(1); }
  const k = line.slice(0, i);
  const isProbe =
    k.startsWith("probe_url_") || k.startsWith("probe_status_") || k.startsWith("probe_ms_") || k.startsWith("probe_via_");
  if (!allowed.has(k) && !isProbe) { process.stderr.write(`❌ 未知证据字段: ${k}\n`); process.exit(1); }
  if (k.startsWith("probe_url_")) probeIdx.push(Number.parseInt(k.slice(10), 10));
  kv[k] = line.slice(i + 1);
}
const num = (k) => Number.parseInt(kv[k] || "0", 10) || 0;
const idx = [...new Set(probeIdx)].sort((a, b) => a - b);
process.stdout.write(JSON.stringify({
  verdict: kv.verdict || "unknown",
  container: kv.container || "",
  engine: kv.engine || "",
  endpoint: kv.endpoint || "",
  remote: kv.remote === "1",
  image: { ref: kv.image_ref || "", digest: kv.image_digest || "" },
  ports: { be: num("port_be"), fe: num("port_fe"), sso: num("port_sso") },
  // `via` 标明探针**观测点**：exec（容器内）/ host（宿主侧复核）/ external（`--probe-url`）——
  // 同一 URL 可能同时有 exec 与 host 两条（容器内通过 ≠ 宿主可达；见 host 网络预检与假就绪防护）
  probes: idx.map((i) => ({
    url: kv[`probe_url_${i}`] || "",
    via: kv[`probe_via_${i}`] || "exec",
    status: num(`probe_status_${i}`),
    duration_ms: num(`probe_ms_${i}`),
  })),
  logs_cmd: kv.logs_cmd || "",
  duration_ms: num("duration_ms"),
  startedAt: kv.started_at || "",
}) + "\n");
'
}

# rt_kv_to_json —— stdin 收 `k=v` 行 ⇒ 单行 JSON（值均为字符串；用于 config/status 面）
rt_kv_to_json() {
  local runner; runner="$(_rt_json_runner)" || return 1
  "$runner" -e '
const fs = require("fs");
const out = {};
for (const line of fs.readFileSync(0, "utf8").split("\n")) {
  if (!line) continue;
  const i = line.indexOf("=");
  if (i < 0) continue;
  out[line.slice(0, i)] = line.slice(i + 1);
}
process.stdout.write(JSON.stringify(out) + "\n");
'
}

# ── 镜像 rootfs 组装（export/import）───────────────────────────────────────────
# 用途：在**无 OCI runtime 可用**的形态（podman single-id 单 ID 映射：`podman build` 的
# `RUN` 步骤会以 "did not get container create message from subprocess: EOF" 失败）下，
# 仍能产出镜像——`create`+`export` 取基础镜像 rootfs，宿主侧改完再 `import`（不需要 RUN）。
# 与 `build-image.sh` 的 `--rootfs-mode host` 同源思路（该路径自组装 rootfs，也不跑 RUN）。

# rt_container_host_kill <container> —— 宿主侧**强制收口**一个容器（0 = 已不存在）
# 为什么：single-id 单 ID 影子内 `podman stop/rm` 会因「记录 PID 与新命名空间不可达」而不收敛
# （实测 `given PID did not die within timeout`）⇒ 容器永久停在 `Stopping` 并残留（占端口/名字）。
# 做法：按容器名取 `.State.Pid`（宿主可见）与 conmon（pidfile）直投 TERM→KILL，再 `rm -f --time 0`。
rt_container_host_kill() {
  local c="$1" pid="" cf="" cp="" w=0
  pid="$(rt_cli inspect --format '{{.State.Pid}}' "$c" 2>/dev/null | head -1 || true)"
  if [ -n "$pid" ] && [ "$pid" != "0" ] && [ -d "/proc/$pid" ]; then
    kill -TERM "$pid" 2>/dev/null || true
    while [ "$w" -lt 10 ]; do [ -d "/proc/$pid" ] || break; sleep 0.5; w=$((w + 1)); done
    [ -d "/proc/$pid" ] && kill -KILL "$pid" 2>/dev/null || true
  fi
  cf="$(rt_cli inspect --format '{{.ConmonPidFile}}' "$c" 2>/dev/null | head -1 || true)"
  if [ -n "$cf" ] && [ -r "$cf" ]; then
    cp="$(cat "$cf" 2>/dev/null || true)"
    if [ -n "$cp" ] && [ -d "/proc/$cp" ]; then
      kill -TERM "$cp" 2>/dev/null || true
      sleep 1
      [ -d "/proc/$cp" ] && kill -KILL "$cp" 2>/dev/null || true
    fi
  fi
  rt_cli rm -f --time 0 "$c" >/dev/null 2>&1 || rt_cli rm -f "$c" >/dev/null 2>&1 || true
  rt_cli inspect --type container "$c" >/dev/null 2>&1 && return 1
  return 0
}

# rt_image_rootfs_export <image> <dest-dir>
#   导出镜像 merged rootfs 到 <dest-dir>（引擎为事实源）。失败 ⇒ 清理临时容器并 rc≠0。
rt_image_rootfs_export() {
  local image="$1" dest="$2" c uid rc=0
  uid="$$-${RANDOM}"
  c="alioth-export-${uid}"
  rt_cli create --name "$c" "$image" true >/dev/null 2>&1 \
    || { echo "❌ 导出 rootfs 失败：create ${image}（引擎不可用或缺镜像）" >&2; return 1; }
  # `export` 输出为 tar 流；解包失败 ⇒ 仍须清理容器（MUST NOT 留残留）
  rt_cli export "$c" 2>/dev/null | tar -C "$dest" -xf - || rc=1
  rt_cli rm -f "$c" >/dev/null 2>&1 || true
  [ "$rc" = 0 ] || { echo "❌ 导出 rootfs 失败：export $image" >&2; return 1; }
  return 0
}

# rt_image_meta_changes <image>
#   打印可传给 `rt_cli import` 的 `--change` 参数（每行一个，成对出现）——继承基础镜像的
#   Config（ENV/ENTRYPOINT/CMD/WORKDIR/USER）。`import` 不继承元数据 ⇒ 不继承会丢 entrypoint。
rt_image_meta_changes() {
  local image="$1" e ep cmd wd user
  while IFS= read -r e; do
    [ -n "$e" ] && printf -- '--change\nENV %s\n' "$e"
  done < <(rt_cli inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$image" 2>/dev/null || true)
  # 三条可选字段：`null`/`[]`/空 ⇒ 不设（MUST NOT 写 `--change 'ENTRYPOINT null'`）
  ep="$(rt_cli inspect --format '{{json .Config.Entrypoint}}' "$image" 2>/dev/null || true)"
  case "$ep" in ""|null|"[]") : ;; *) printf -- '--change\nENTRYPOINT %s\n' "$ep" ;; esac
  cmd="$(rt_cli inspect --format '{{json .Config.Cmd}}' "$image" 2>/dev/null || true)"
  case "$cmd" in ""|null|"[]") : ;; *) printf -- '--change\nCMD %s\n' "$cmd" ;; esac
  wd="$(rt_cli inspect --format '{{.Config.WorkingDir}}' "$image" 2>/dev/null || true)"
  [ -n "$wd" ] && printf -- '--change\nWORKDIR %s\n' "$wd"
  user="$(rt_cli inspect --format '{{.Config.User}}' "$image" 2>/dev/null || true)"
  [ -n "$user" ] && printf -- '--change\nUSER %s\n' "$user"
  return 0
}

# rt_image_import <image> <rootfs-dir> [--change …]…
#   打包 rootfs（属主归 0，与镜像层语义一致）→ 引擎 import。唯一实现：调用方把自身的
#   元数据（`rt_image_meta_changes` 输出 + 追加项）透传为 `--change`。
rt_image_import() {
  local image="$1" root="$2"; shift 2
  local tar_file rc=0
  tar_file="$(mktemp "${TMPDIR:-/tmp}/alioth-image-import.XXXXXX")"
  tar -C "$root" --owner=0 --group=0 -cf "$tar_file" . \
    || { rm -f "$tar_file"; echo "❌ 打包 rootfs 失败: $root" >&2; return 1; }
  rt_cli import "$@" - "$image" < "$tar_file" || rc=1
  rm -f "$tar_file"
  [ "$rc" = 0 ] || { echo "❌ 镜像导入失败: $image" >&2; return 1; }
  return 0
}

# rt_ps_json <engine> —— 通用实例一览（标签 `alioth.launch=1`）⇒ 单行 JSON 数组
# 两个引擎统一用 `--format {{json .}}`（每行一个 JSON 对象）⇒ 由标准解析器组装，MUST NOT 手工拼 JSON
rt_ps_json() {
  local runner; runner="$(_rt_json_runner)" || return 1
  rt_cli ps -a --filter "label=alioth.launch=1" --format '{{json .}}' 2>/dev/null | "$runner" -e '
const fs = require("fs");
const rows = [];
for (const line of fs.readFileSync(0, "utf8").split("\n")) {
  const s = line.trim();
  if (!s || s[0] !== "{") continue;
  let o; try { o = JSON.parse(s); } catch { continue; }
  const name = Array.isArray(o.Names) ? o.Names.join(",") : o.Names || o.Name || "";
  rows.push({
    container: name,
    state: o.State || o.Status || "",
    image: o.Image || "",
    ports: o.Ports || "",
    created: o.CreatedAt || o.Created || "",
  });
}
rows.sort((a, b) => (a.container < b.container ? -1 : a.container > b.container ? 1 : 0));
process.stdout.write(JSON.stringify(rows) + "\n");
'
}

# rt_launch_contract_json —— CLI 契约（动词/退出码/证据字段）机器可读单一源
# 门禁与文档 MUST 由该源派生（MUST NOT 正则刮 Markdown 取契约项）
rt_launch_contract_json() {
  local runner; runner="$(_rt_json_runner)" || return 1
  "$runner" -e '
process.stdout.write(JSON.stringify({
  cli: "scripts/runtime/launch-image.sh",
  verbs: ["up","down","status","logs","list","config","--print-contract"],
  exit_codes: { "0": "ready", "1": "artifact_or_runtime_failure", "2": "environment_unreachable", "3": "container_service_unavailable" },
  evidence_fields: ["verdict","container","engine","endpoint","remote","image","ports","probes","logs_cmd","duration_ms","startedAt"],
  verdicts: ["ready","failed","started","environment_unreachable"],
  label: "alioth.launch=1",
  name_prefix: "alioth-rt-",
}) + "\n");
'
}

# ── 自检（直接执行时）────────────────────────────────────────────────────

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  set -e
  echo "== config =="; rt_config_dump
  echo "== backend =="
  if ! rt_backend_info; then
    # 门禁口径：无容器后端的主机 MUST 显式跳过该段而非失败（与 check-* 的 [skipped] 约定一致）
    echo "  [skipped] 本机无可用容器后端（podman/docker 皆不在 PATH）——后端相关断言跳过"
  fi
  # ns 依赖段（SE 的 host / resolve / 端口三元组）：部分检出副本（仅挂本 ns）无 SE 端口声明 ⇒ SE
  # 不可判定，跳过该段而非失败（与「无容器后端主机打印 [skipped] 段」同口径，MUST NOT 静默跳过）。
  if [ -L "${PROJECT_ROOT:-.}/Pre-Proc/SE" ]; then
    echo "== hosts =="
    echo "  RT SE        → $(rt_instance_host SE)"
    echo "  RT SE --label alioth → $(rt_instance_host SE alioth)"
    echo "== resolve =="
    for h in "pre-se.isahl.com" "pre-se-demo.isahl.com" "pre-cosmic-tools-x.isahl.com" "pre-nope.isahl.com" "se.isahl.com"; do
      if k="$(rt_resolve_host "$h" 2>/dev/null)"; then echo "  $h → $k"; else echo "  $h → ✗"; fi
    done
    # 实例端口三元组 MUST = Deploy/ports.conf 的该 ns 三元组（单一来源；band 已退役 ⇒ 不再"分配"）
    trip="$(rt_instance_ports SE)"
    exp="$(ns_port SE be) $(ns_port SE fe) $(ns_port SE sso)"
    if [ "$trip" = "$exp" ]; then
      echo "  ✅ 实例端口三元组与 ports.conf 同源（SE → ${trip}）"
    else
      echo "  ❌ 实例端口三元组漂移：rt_instance_ports=$trip ≠ ports.conf=$exp"
      exit 1
    fi
  else
    echo "  [skipped] SE 未挂载（Pre-Proc/SE 非符号链接）——SE 依赖断言跳过（部分检出副本）"
  fi
  # 未登记 ns ⇒ MUST fail-loud（不再回落到任何默认端口）
  if rt_instance_ports NoSuchNs >/dev/null 2>&1; then
    echo "  ❌ 未登记 ns 的端口三元组未 fail-loud"
    exit 1
  fi
  echo "  ✅ 未登记 ns 的端口三元组 fail-loud（不回落到默认端口）"
  # 以下两条断言验证的是**容器实例面的机制**（单 ID 影子前置、形态探针进程内缓存）：
  # 它们只在宿主可跑容器实例（Linux）时有意义——非 Linux 宿主上 `start`/`build-image` 整面被
  # 产品 fail-loud 拒绝（「容器实例仅支持 Linux」），形态探针链也随之走宿主特有分支而不可复现
  # （实测 macOS：桩链不执行 ⇒ 探测 0 次）。故按平台跳过并显式打印原因，与同文件既有语义一致
  # （「无容器后端主机打印 [skipped] 段而非失败」）；MUST NOT 静默跳过。
  if rt_containers_supported; then
    # 形态可用性判据 MUST 含影子前置（R15 ❌-18）：PATH 仅留 podman ⇒ unshare/crun 不可见 ⇒ single-id 必须判不可用
    # （否则影子不可运行时 status 会把「形态不可用」当「容器不存在」并清条目 ⇒ 孤儿容器）
    formtb="$(mktemp -d "${TMPDIR:-/tmp}/rt-formtest.XXXXXX")"
    unset _RT_FORM_PODMAN_SINGLE   # 清进程内缓存：否则本次断言可能被同进程早先的调用命中
    ln -sf "$(command -v podman)" "$formtb/podman"
    if PATH="$formtb" rt_form_available podman single-id; then
      echo "  ❌ single-id 形态可用性未含影子前置（unshare/crun 不可见仍判可用）"
      rm -rf "$formtb"; exit 1
    fi
    rm -rf "$formtb"
    echo "  ✅ single-id 形态可用性含影子前置（unshare/crun 不可见 ⇒ 判不可用）"
    # 形态探针 MUST 按进程内缓存（R18 ⚠️-29）：同一次调用里 3 次问同一形态 ⇒ 只探 1 次
    # 注入面 MUST 宿主无关（R8 ❌-13 同类）：`podman native` 探针的前置链 = `timeout` + `newuidmap`/`newgidmap`
    # + `podman info`，三者任一在宿主缺失都会让探针短路且桩从不执行 ⇒ 断言恒失败。故三者一并桩化，
    # 使该断言只测**缓存语义**，与宿主能力解耦。
    pcd="$(mktemp -d "${TMPDIR:-/tmp}/rt-probecnt.XXXXXX")"
    unset _RT_FORM_PODMAN_NATIVE
    printf '#!/bin/sh\necho x >> "%s/cnt"\nexit 0\n' "$pcd" > "$pcd/podman"
    printf '#!/bin/sh\nexit 0\n' > "$pcd/newuidmap"
    printf '#!/bin/sh\nexit 0\n' > "$pcd/newgidmap"
    printf '#!/bin/sh\nshift\nexec "$@"\n' > "$pcd/timeout"
    chmod +x "$pcd/podman" "$pcd/newuidmap" "$pcd/newgidmap" "$pcd/timeout"
    PATH="$pcd:$PATH" rt_form_available podman native || true
    PATH="$pcd:$PATH" rt_form_available podman native || true
    PATH="$pcd:$PATH" rt_form_available podman native || true
    pc="$(grep -c . "$pcd/cnt" 2>/dev/null || echo 0)"
    pc="${pc//[!0-9]/}"
    rm -rf "$pcd"
    if [ "$pc" = 1 ]; then
      echo "  ✅ 形态探针进程内缓存（3 次问同一形态 ⇒ 1 次探测）"
    else
      echo "  ❌ 形态探针缓存失效（探测 $pc 次，期望 1）"
      exit 1
    fi
  else
    echo "  [skipped] 单 ID 影子前置与形态探针缓存断言（宿主不支持容器实例：$(rt_os) 非 Linux）"
  fi
  echo "== registry =="; rt_registry_keys | sed 's/^/  /'

  # 零实例（空注册表）⇒ `list` MUST 成功（bash 3.2 下空数组展开 `"${keys[@]}"` 会 unbound variable
  # ⇒ router 快照刷新失败并保留旧路由 = 热摘除静默失效；2026-09-27 实测回归面）
  _empty_root="$(mktemp -d "${TMPDIR:-/tmp}/rt-emptyreg.XXXXXX")"
  if _lr="$(NS_RUNTIME_ROOT="$_empty_root" rt_list 2>&1)"; then
    [ -z "$_lr" ] && echo "  ✅ 空注册表 ⇒ list rc=0 且无输出（bash 3.2 空数组展开安全）" || {
      echo "  ❌ 空注册表 ⇒ list 输出非空：$_lr"; rm -rf "$_empty_root"; exit 1; }
  else
    echo "  ❌ 空注册表 ⇒ list 失败：$_lr"
    rm -rf "$_empty_root"; exit 1
  fi
  rm -rf "$_empty_root"

  # ── 宿主进程面实例派生视图（list/status/ps 统一可见；不依赖容器引擎）──
  echo "== host instances（进程面派生） =="
  _hi_root="$(mktemp -d "${TMPDIR:-/tmp}/rt-hostreg.XXXXXX")"
  printf 'NAMESPACE=WZ\nDB_NAME=wz_pre\nSSO_PID=\nGATEWAY_BE_PID=999999\nGATEWAY_FE_PID=\n' > "${_hi_root}/WZ.pid"
  if [ "$(NS_RUNTIME_ROOT="$_hi_root" rt_host_state WZ)" = "stale" ]; then
    echo "  ✅ 宿主注册表 ⇒ 状态派生（dead pid ⇒ stale）"
  else
    echo "  ❌ 宿主状态派生失败：$(NS_RUNTIME_ROOT="$_hi_root" rt_host_state WZ)"
    rm -rf "$_hi_root"; exit 1
  fi
  _hi_kv="$(NS_RUNTIME_ROOT="$_hi_root" rt_host_kv WZ || true)"
  case "$_hi_kv" in
    *"backend=host"*) echo "  ✅ 宿主实例 kv 带 backend=host" ;;
    *) echo "  ❌ 宿主实例 kv 缺 backend=host：$_hi_kv"; rm -rf "$_hi_root"; exit 1 ;;
  esac
  case "$_hi_kv" in
    *"tier=pre"*) echo "  ✅ 层由 DB_NAME 派生（wz_pre ⇒ pre，复用 ns_name_to_db）" ;;
    *) echo "  ❌ 层派生失败：$_hi_kv"; rm -rf "$_hi_root"; exit 1 ;;
  esac
  _hi_empty="$(mktemp -d "${TMPDIR:-/tmp}/rt-hostreg-empty.XXXXXX")"
  if [ "$(RT_RUNTIME_ROOT="$_hi_empty" NS_RUNTIME_ROOT="$_hi_root" rt_host_ns_for_key WZ)" = "WZ" ] && \
     [ "$(RT_RUNTIME_ROOT="$_hi_empty" NS_RUNTIME_ROOT="$_hi_root" rt_host_ns_for_key wz)" = "WZ" ]; then
    echo "  ✅ 宿主 key 解析（大小写归一：WZ/wz 同源；容器条目存在时让位）"
  else
    echo "  ❌ 宿主 key 解析失败"; rm -rf "$_hi_root" "$_hi_empty"; exit 1
  fi
  # 容器条目优先：同名容器注册条目存在 ⇒ 宿主条目不出现在 status（两平面互斥，容器优先）
  printf '{"key":"wz","ns":"WZ"}' > "${_hi_empty}/wz.json"
  if [ -z "$(RT_RUNTIME_ROOT="$_hi_empty" NS_RUNTIME_ROOT="$_hi_root" rt_host_ns_for_key wz 2>/dev/null || true)" ]; then
    echo "  ✅ 容器条目优先（同名容器注册存在 ⇒ 宿主派生让位）"
  else
    echo "  ❌ 容器优先级失效"; rm -rf "$_hi_root" "$_hi_empty"; exit 1
  fi
  rm -rf "$_hi_root" "$_hi_empty"
fi
