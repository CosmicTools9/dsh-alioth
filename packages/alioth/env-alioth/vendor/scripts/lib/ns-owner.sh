#!/bin/bash
# ns-owner.sh —— Namespace 槽位所有权（owner 记录）单一实现
#
# 契约来源：`add-ns-slot-ownership-supervisor`（spec delta `ns-slot-ownership-single-owner`，
# 设计 D2/D3/D4）。**owner 记录 = 「谁持有本 ns 槽位」的唯一真相源**：本层 supervisor
# （随层存亡，不常驻于层之外）在启动时写入、退出时清除；任何需要该槽位的请求方 MUST 先读本
# 记录 → 验身份（`OWNER_PID` 存活 ∧ `ps -p PID -o lstart=` 与 `OWNER_START` 逐字相同）
# → 经 `ns_owner_release_request` 请 owner 自清；MUST NOT 按进程名/端口盲杀。
#
# 与 `ns-ports.sh::ns_registry_file`（`Deploy/{ns}/.runtime/{ns}.pid`）的区别：
#   · owner.conf    = **所有权**（谁持有槽位、可被请退、**唯一判据，优先采信**）；
#   · {ns}.pid      = **诊断缓存**（`--status` 显示面；由 current owner 在子进程 pid 变化时
#                     回写；MUST NOT 作为投递信号的依据——它可留僵尸 PID，见 proposal 实证）。
# 两者格式不同、生命周期不同，故不共用解析路径：本文件是 owner 记录格式的唯一实现，
# 调用方（bash 与 TS）MUST 经本文件读写，MUST NOT 自行拼写。
#
# 身份判据为什么必须含 `ps -p PID -o lstart=`（`ns_owner_lstart`）：
# PID 复用是本机实测过的真实失效面（同 ns 旧实例 PID 被无关进程重用后 kill 会误杀）。
# 只判「进程存活」无法区分「还是那个 owner」与「PID 已被别的进程复用」；`lstart` 是进程
# **启动时刻**的内核事实（同一 PID 的两次不同生命不会给出相同启动时刻），且以**原样字符串**
# 比较 ⇒ 零解析、零时区错误、macOS/Linux 一致。因此 `ns_owner_valid` 恒为
# 「pid 活 ∧ lstart 逐字相同」，任一不满足即判「无有效 owner」，请求方 MUST NOT 投信号。
# 归一：`ps -o lstart=` 的**渲染随调用方 locale 变化**（实测同一进程：`LC_ALL=C` 下
# `Tue Sep 29 02:45:32 2026`，CJK locale 下 `二  9月/29 02:45:34 2026` + 字段填充尾随空格）
# ⇒ 若不同时钉死，写者（supervisor）与读者（dev-gateway/抢占链）locale 环境不同会把同一 owner
# 判为「无有效 owner」并退回 legacy 盲杀。故 `ns_owner_lstart` 内**固定 `LC_ALL=C LANG=C`**，
# 并在比较两侧做首尾空白归一——"逐字相同"对**规范串**成立（不涉及任何日期解析）。
#
# 用法（调用方 source 后使用；亦可直接执行——见文末 CLI 与 `selftest`）:
#   source scripts/lib/ns-owner.sh
#   ns_owner_file WZ                          # → 记录路径
#   ns_owner_read WZ / ns_owner_field WZ OWNER_PID
#   ns_owner_lstart <pid>                     # → `ps -p PID -o lstart=` 原样
#   ns_owner_valid WZ / ns_owner_pid WZ
#   ns_owner_write WZ dev <pid> fe,be,sso 41719,9005,9006 /abs/.../supervise.sock
#   ns_owner_clear WZ [expect_pid]
#   ns_owner_release_request WZ [timeout_s=20]
#
# 记录格式（行式 KEY=value；解析风格同 `ns_registry_read`：纯 bash `read`、跳空行与 # 注释）:
#   LAYER=dev
#   NAMESPACE=WZ
#   OWNER_PID=12345
#   OWNER_START=<ps -p PID -o lstart= 的原样字符串>
#   OWNER_EXE=<ps -p PID -o comm=>
#   SLOTS=41719,9005,9006
#   ROLES=fe,be,sso
#   CONTROL_SOCK=/abs/path/Deploy/WZ/.runtime/supervise.sock
#   UPDATED_AT=2026-09-29T03:00:00Z
#
# 测试/隔离：`NS_RUNTIME_ROOT` 可覆写根目录（语义同 `ns_registry_file` / `ns_lock_dir`：
# 一个含 `<ns>` 条目的根；owner 条目 = `<ns>.owner.conf`）⇒ 单测不触碰真实 `Deploy/{ns}`。
# 纯 bash（无 jq/python），macOS bash 3.2 兼容（无 mapfile / wait -n）。

# 调用方未设 PROJECT_ROOT 时自推导（直接执行/独立调用场景；显式设置优先）
if [ -z "${PROJECT_ROOT:-}" ]; then
  PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." 2>/dev/null && pwd)"
fi

# ns 落点解析（运行态归属 ns 侧：Deploy/{ns}/.runtime）——单一实现同目录
_NS_OWNER_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -f "${_NS_OWNER_LIB_DIR}/ns-deploy-dir.sh" ]; then
  # shellcheck source=ns-deploy-dir.sh
  source "${_NS_OWNER_LIB_DIR}/ns-deploy-dir.sh"
fi
# 缺依赖时的退化（保持函数存在，避免调用方 set -u 崩）：仍走 ns 侧 Deploy 路径
if ! command -v ns_runtime_dir >/dev/null 2>&1; then
  ns_runtime_dir() { printf '%s/Deploy/%s/.runtime\n' "${PROJECT_ROOT:-.}" "${1:-}"; }
fi

# ns_owner_file <ns> —— owner 记录路径（NS_RUNTIME_ROOT 覆写；语义同 ns_registry_file）
ns_owner_file() {
  local ns="${1:-}"
  if [ -n "${NS_RUNTIME_ROOT:-}" ]; then
    printf '%s/%s.owner.conf\n' "${NS_RUNTIME_ROOT%/}" "$ns"
  else
    printf '%s/owner.conf\n' "$(ns_runtime_dir "$ns")"
  fi
}

# ns_owner_read <ns> —— 输出 KEY=value 行（空行/`#` 注释忽略）；无文件 rc=1
ns_owner_read() {
  local ns="${1:-}" f line
  f="$(ns_owner_file "$ns")"
  [ -f "$f" ] || return 1
  while IFS= read -r line; do
    [[ -z "${line// /}" || "$line" == \#* ]] && continue
    printf '%s\n' "$line"
  done < "$f"
  return 0
}

# ns_owner_field <ns> <KEY> —— 取单键值；缺失 → 空串 rc=0（不判存在性，供可选字段读取）
ns_owner_field() {
  local ns="${1:-}" key="${2:-}" kv
  while IFS= read -r kv; do
    case "$kv" in
      "${key}="*) printf '%s\n' "${kv#*=}"; return 0 ;;
    esac
  done < <(ns_owner_read "$ns" 2>/dev/null)
  printf '%s\n' ""
  return 0
}

# _ns_owner_trim <str> —— 去首尾空白（`ps` 字段填充随 locale 变化，CJK locale 带尾随空格）
_ns_owner_trim() {
  local s="${1:-}"
  s="${s#"${s%%[![:space:]]*}"}"
  s="${s%"${s##*[![:space:]]}"}"
  printf '%s\n' "$s"
}

# ns_owner_lstart <pid> —— `ps -p PID -o lstart=` 原样输出（`LC_ALL=C` 规范串 + 首尾空白归一）；
# 进程不存在 rc=1。不做任何解析/格式化：写侧与验侧都经本函数 ⇒ 规范串逐字比较成立。
ns_owner_lstart() {
  local pid="${1:-}" out
  [ -n "$pid" ] || return 1
  # locale 钉死：否则同一进程在不同调用方 locale 下渲染不同 ⇒ 身份判据跨 locale 误判（见文首）
  out="$(LC_ALL=C LANG=C ps -p "$pid" -o lstart= 2>/dev/null)" || return 1
  [ -n "$out" ] || return 1
  _ns_owner_trim "$out"
  return 0
}

# _ns_owner_alive <pid> —— 0=进程存在且非僵尸（轮询用；语义同 `ns-ports.sh::ns_alive`——
# 僵尸已不可再操作，视作已退出。此处内联以避免 owner 层反向依赖 ns-ports.sh）。
_ns_owner_alive() {
  local pid="${1:-}" state
  [ -n "$pid" ] || return 1
  kill -0 "$pid" 2>/dev/null || return 1
  state="$(ps -p "$pid" -o state= 2>/dev/null | tr -d ' ')"
  [ -n "$state" ] && [ "${state#Z}" != "$state" ] && return 1
  return 0
}

# ns_owner_valid <ns> —— 0 = 记录存在 ∧ OWNER_PID 存活 ∧ lstart 逐字匹配（首尾空白归一）；否则非 0
ns_owner_valid() {
  local ns="${1:-}" pid start live
  pid="$(ns_owner_field "$ns" OWNER_PID)"
  [ -n "$pid" ] || return 1
  start="$(_ns_owner_trim "$(ns_owner_field "$ns" OWNER_START)")"
  [ -n "$start" ] || return 1
  live="$(ns_owner_lstart "$pid" 2>/dev/null)" || return 1
  [ "$live" = "$start" ] || return 1
  return 0
}

# ns_owner_pid <ns> —— 有效 → 打印 pid rc=0；否则 rc=1
ns_owner_pid() {
  local ns="${1:-}" pid
  ns_owner_valid "$ns" || return 1
  pid="$(ns_owner_field "$ns" OWNER_PID)"
  [ -n "$pid" ] || return 1
  printf '%s\n' "$pid"
  return 0
}

# ns_owner_write <ns> <layer> <pid> <roles_csv> <slots_csv> <sock> —— 原子写（tmp + mv）
# OWNER_PID 必须存活（否则 OWNER_START 无从取值 ⇒ 记录必为无效）⇒ 死 pid fail-loud rc=1。
ns_owner_write() {
  local ns="${1:-}" layer="${2:-}" pid="${3:-}" roles="${4:-}" slots="${5:-}" sock="${6:-}"
  local f tmp start exe
  [ -n "$ns" ] || { echo "[owner] 缺 namespace 参数" >&2; return 1; }
  [ -n "$pid" ] || { echo "[owner] ${ns}: 缺 OWNER_PID 参数" >&2; return 1; }
  start="$(ns_owner_lstart "$pid" 2>/dev/null)" || {
    echo "[owner] ${ns}: PID ${pid} 不存在——拒绝写入 owner 记录（身份判据不可满足）" >&2
    return 1
  }
  exe="$(ps -p "$pid" -o comm= 2>/dev/null | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')" || exe=""
  [ -n "$exe" ] || exe="unknown"
  f="$(ns_owner_file "$ns")"
  mkdir -p "$(dirname "$f")" || return 1
  tmp="${f}.tmp.$$"
  {
    printf 'LAYER=%s\n' "$layer"
    printf 'NAMESPACE=%s\n' "$ns"
    printf 'OWNER_PID=%s\n' "$pid"
    printf 'OWNER_START=%s\n' "$start"
    printf 'OWNER_EXE=%s\n' "$exe"
    printf 'SLOTS=%s\n' "$slots"
    printf 'ROLES=%s\n' "$roles"
    printf 'CONTROL_SOCK=%s\n' "$sock"
    printf 'UPDATED_AT=%s\n' "$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
  } > "$tmp" || { rm -f "$tmp"; return 1; }
  mv -f "$tmp" "$f" || { rm -f "$tmp"; return 1; }
  return 0
}

# ns_owner_clear <ns> [expect_pid] —— 删除记录；给 expect_pid 时仅当 OWNER_PID 逐字等于它才删
# （防「他人已接管后本进程退出清掉新 owner 记录」）。无文件：无参 rc=0（幂等）；
# 带 expect_pid rc=1（无从匹配，调用方据此判定"我是否仍是记录持有者"）。
ns_owner_clear() {
  local ns="${1:-}" expect="${2:-}" f cur
  f="$(ns_owner_file "$ns")"
  if [ ! -f "$f" ]; then
    [ -n "$expect" ] && return 1
    return 0
  fi
  if [ -n "$expect" ]; then
    cur="$(ns_owner_field "$ns" OWNER_PID)"
    if [ "$cur" != "$expect" ]; then
      echo "[owner] ${ns}: OWNER_PID=${cur:-（无）} ≠ 期望 ${expect}——不删（防误清他人记录）" >&2
      return 1
    fi
  fi
  rm -f "$f"
  return 0
}

# ns_owner_release_request <ns> [timeout_s=20] —— 请 owner 自清（SIGTERM + 有界等待）
# rc: 0 = owner 已退出或记录已消失（超时内）；1 = 超时（owner/记录仍在）；2 = **无有效 owner
# 记录**（请求方自行决定 legacy 处置，MUST NOT 据此投递任何信号）。
# 轮询间隔 0.5s（≤0.5s 契约）；成功判据 = 进程死亡（非僵尸）**或**记录消失。
ns_owner_release_request() {
  local ns="${1:-}" timeout="${2:-${NS_OWNER_RELEASE_TIMEOUT:-20}}"
  local pid start ticks i
  case "$timeout" in ''|*[!0-9]*) timeout=20 ;; esac
  # 身份校验（pid 活 ∧ lstart 逐字匹配）——不通过即"无有效 owner"，绝不对 pid 投信号
  if ! ns_owner_valid "$ns"; then
    return 2
  fi
  pid="$(ns_owner_field "$ns" OWNER_PID)"
  start="$(_ns_owner_trim "$(ns_owner_field "$ns" OWNER_START)")"
  [ -n "$pid" ] || return 2
  echo "[owner] ${ns}: 经 owner 请退 → SIGTERM PID ${pid}（layer=$(ns_owner_field "$ns" LAYER)；lstart=${start}）" >&2
  kill -TERM "$pid" 2>/dev/null || true
  ticks=$(( timeout * 2 ))   # 0.5s × ticks = timeout 秒
  [ "$ticks" -ge 1 ] || ticks=1
  i=0
  while [ "$i" -lt "$ticks" ]; do
    if ! _ns_owner_alive "$pid"; then
      echo "[owner] ${ns}: owner PID ${pid} 已退出（请退完成）" >&2
      return 0
    fi
    if [ ! -f "$(ns_owner_file "$ns")" ]; then
      echo "[owner] ${ns}: owner 记录已消失（请退完成）" >&2
      return 0
    fi
    sleep 0.5
    i=$(( i + 1 ))
  done
  echo "[owner] ${ns}: 请退超时（${timeout}s）——owner PID ${pid} 仍存活/记录仍在" >&2
  return 1
}

# ── 用法（CLI 与自检共用）────────────────────────────────────────────────────
ns_owner_usage() {
  cat <<'EOF'
用法:
  bash scripts/lib/ns-owner.sh selftest              # 自检（默认动作；无参数亦同；幂等）
  bash scripts/lib/ns-owner.sh file <ns>             # 打印 owner 记录路径
  bash scripts/lib/ns-owner.sh read <ns>             # 打印记录行（无文件 rc=1）
  bash scripts/lib/ns-owner.sh field <ns> <KEY>      # 打印单键值（缺失 → 空串 rc=0）
  bash scripts/lib/ns-owner.sh lstart <pid>          # `ps -p PID -o lstart=` 原样（无进程 rc=1）
  bash scripts/lib/ns-owner.sh valid <ns>            # 0=有效 owner（pid 活 + lstart 匹配）
  bash scripts/lib/ns-owner.sh pid <ns>              # 有效 → 打印 OWNER_PID
  bash scripts/lib/ns-owner.sh write <ns> <layer> <pid> <roles_csv> <slots_csv> <sock>
  bash scripts/lib/ns-owner.sh clear <ns> [expect_pid]
  bash scripts/lib/ns-owner.sh release-request <ns> [timeout_s]
                       # 0=请退完成；1=超时；2=无有效 owner 记录
环境:
  NS_RUNTIME_ROOT      覆写记录根目录（测试/隔离；条目 = <ns>.owner.conf）
  NS_OWNER_RELEASE_TIMEOUT  release-request 默认超时秒数（默认 20）
EOF
}

# ── CLI / 自检（直接执行；幂等、不依赖仓库状态）──────────────────────────────
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  ns_owner_selftest() {
    local fail=0 tmp p1
    sa() { # $1 描述 $2 期望 rc，其后为命令
      local desc="$1" want="$2" got
      shift 2
      got=0; "$@" >/dev/null 2>&1 || got=$?
      if [ "$got" = "$want" ]; then echo "  ok: $desc"; else echo "  FAIL: $desc (期望 $want 实际 $got)"; fail=1; fi
    }
    assert_eq() { # $1 描述 $2 期望 $3 实际
      if [ "$2" = "$3" ]; then echo "  ok: $1"; else echo "  FAIL: $1 (期望 '$2' 实际 '$3')"; fail=1; fi
    }

    tmp="$(mktemp -d)"
    NS_RUNTIME_ROOT="$tmp"   # 隔离：记录落 tmp（不动真实 Deploy/{ns}）
    export NS_RUNTIME_ROOT

    echo "记录路径："
    assert_eq "ns_owner_file 落在 NS_RUNTIME_ROOT 下" "$tmp/WZ.owner.conf" "$(ns_owner_file WZ)"

    echo "缺记录语义："
    assert_eq "field 缺记录 → 空串" "" "$(ns_owner_field WZ OWNER_PID)"
    assert_eq "read 缺记录 → 空输出" "" "$(ns_owner_read WZ 2>/dev/null || true)"
    sa "valid 缺记录 → rc=1" 1 ns_owner_valid WZ
    sa "pid 缺记录 → rc=1" 1 ns_owner_pid WZ
    sa "release_request 缺记录 → rc=2（调用方 legacy 处置）" 2 ns_owner_release_request WZ 1

    echo "写入 / 读取往返（fixture = 真实活进程）："
    bash -c 'exec sleep 30' &   # 真实活进程；文末 kill 收尾
    p1=$!
    sleep 0.3
    sa "write 活进程 rc=0" 0 ns_owner_write WZ dev "$p1" fe,be,sso 41719,9005,9006 "$tmp/supervise.sock"
    assert_eq "read 含 OWNER_PID 行" "OWNER_PID=$p1" "$(ns_owner_read WZ | grep '^OWNER_PID=')"
    assert_eq "NAMESPACE 回读" "WZ" "$(ns_owner_field WZ NAMESPACE)"
    assert_eq "LAYER 回读" "dev" "$(ns_owner_field WZ LAYER)"
    assert_eq "SLOTS 回读" "41719,9005,9006" "$(ns_owner_field WZ SLOTS)"
    assert_eq "CONTROL_SOCK 回读" "$tmp/supervise.sock" "$(ns_owner_field WZ CONTROL_SOCK)"
    assert_eq "OWNER_START = ns_owner_lstart 原样" "$(ns_owner_lstart "$p1")" "$(ns_owner_field WZ OWNER_START)"
    sa "valid 全对 → rc=0" 0 ns_owner_valid WZ
    assert_eq "pid 打印 OWNER_PID" "$p1" "$(ns_owner_pid WZ)"
    sa "write 死 PID → rc=1（拒写）" 1 ns_owner_write WZ dev 99999999 fe 1 "$tmp/s.sock"

    echo "locale 无关性（ps 的 lstart 渲染随调用方 locale 变化 ⇒ 单实现内钉死 LC_ALL=C）："
    assert_eq "非 C locale 下 lstart 仍为 C 规范串" "$(ns_owner_lstart "$p1")" \
      "$(export LC_ALL=zh_CN.UTF-8 LANG=zh_CN.UTF-8; ns_owner_lstart "$p1")"
    locale_reader() { (export LC_ALL=zh_CN.UTF-8 LANG=zh_CN.UTF-8; ns_owner_valid WZ); }
    sa "非 C locale 读者判同一记录有效（交叉 locale）" 0 locale_reader

    echo "身份判据（lstart 不匹配 ⇒ MUST NOT 投信号）："
    printf 'NAMESPACE=WZ\nOWNER_PID=%s\nOWNER_START=Thu Jan  1 00:00:00 1970\n' "$p1" > "$tmp/WZ.owner.conf"
    sa "valid 伪造 lstart → rc=1" 1 ns_owner_valid WZ
    sa "release_request 伪造 lstart → rc=2（不发信号）" 2 ns_owner_release_request WZ 1
    sa "伪造期间活 pid 仍存活（未被投信号）" 0 _ns_owner_alive "$p1"

    echo "locale 归一（ps 字段填充在 CJK locale 带尾随空格 ⇒ 记录带尾空白仍应有效）："
    printf 'NAMESPACE=WZ\nOWNER_PID=%s\nOWNER_START=%s  \n' "$p1" "$(ns_owner_lstart "$p1")" > "$tmp/WZ.owner.conf"
    sa "记录 OWNER_START 带尾随空白 → valid rc=0（两侧归一比较）" 0 ns_owner_valid WZ

    echo "清理（expect_pid 语义）："
    sa "clear expect_pid 不匹配 → rc=1" 1 ns_owner_clear WZ 424242
    sa "不匹配时记录仍在" 0 test -f "$tmp/WZ.owner.conf"
    sa "clear expect_pid 匹配 → rc=0" 0 ns_owner_clear WZ "$p1"
    sa "匹配后记录已删" 1 test -f "$tmp/WZ.owner.conf"

    echo "请退（有效 owner）："
    sa "重写 owner 记录" 0 ns_owner_write WZ dev "$p1" fe,be,sso 9005 "$tmp/supervise.sock"
    sa "release_request 有效 owner → rc=0" 0 ns_owner_release_request WZ 5
    sa "owner 已退出（僵尸视作已退出）" 1 _ns_owner_alive "$p1"

    kill "$p1" 2>/dev/null || true
    wait 2>/dev/null || true
    unset NS_RUNTIME_ROOT
    rm -rf "$tmp"
    # shellcheck disable=SC2015 # 自检裁决：非空即通过，失败分支显式返回 1
    [ "$fail" = 0 ] && echo "自检全部通过" || { echo "自检存在失败"; return 1; }
  }

  case "${1:-selftest}" in
    --help|-h|help)   ns_owner_usage; exit 0 ;;
    selftest)         ns_owner_selftest; exit $? ;;
    file)             ns_owner_file "${2:-}"; exit $? ;;
    read)             ns_owner_read "${2:-}"; exit $? ;;
    field)            ns_owner_field "${2:-}" "${3:-}"; exit $? ;;
    lstart)           ns_owner_lstart "${2:-}"; exit $? ;;
    valid)            ns_owner_valid "${2:-}"; exit $? ;;
    pid)              ns_owner_pid "${2:-}"; exit $? ;;
    write)            shift; ns_owner_write "$@"; exit $? ;;
    clear)            shift; ns_owner_clear "$@"; exit $? ;;
    release-request)  shift; ns_owner_release_request "$@"; exit $? ;;
    *)                echo "未知子命令: $1" >&2; ns_owner_usage >&2; exit 2 ;;
  esac
fi
