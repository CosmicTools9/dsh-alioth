#!/usr/bin/env bash
# env-common.sh — 非开发环境（pre/prod）启动共享库
# 被 scripts/pre/start.sh 和 scripts/prod/start.sh source
# 提供 nohup 后台启动模式 + 端口发现 + PID 管理

set -euo pipefail
# ── 路径定位（不依赖 CWD） ─────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
LIB_DIR="${SCRIPT_DIR}"

# ── 颜色（被 source 后也生效） ─────────────────────────────
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m'

log()  { echo -e "${BLUE}[env]${NC} $*"; }
ok()   { echo -e "${GREEN}[env]${NC} $*"; }
warn() { echo -e "${YELLOW}[env]${NC} $*" >&2; }
err()  { echo -e "${RED}[env]${NC} $*" >&2; }

# ── 加载 mise 环境 ────────────────────────────────────────
if command -v mise &>/dev/null; then
    eval "$(mise env)" 2>/dev/null || true
fi

# ── 端口释放 ──────────────────────────────────────────────
free_port() {
    local port="$1"
    if [ -z "$port" ]; then return; fi
    local pid
    pid=$(lsof -ti ":$port" 2>/dev/null || true)
    if [ -n "$pid" ]; then
        kill "$pid" 2>/dev/null || true
        sleep 1
        # 若未正常退出则强制 kill
        kill -9 "$pid" 2>/dev/null || true
    fi
}

# ── 从 .mise.toml 解析 SERVER_ADDR ─────────────────────────
# 解析工具 MUST 为 POSIX（BSD grep 无 -P：/usr/bin/grep -oP 报 `invalid option -- P`，
# 真机 macOS 上原实现恒失败 ⇒ 端口静默回落默认值）。
parse_mise_server_addr() {
    local dir="${1:-}"
    [ -z "$dir" ] && return 1
    local toml="${dir}/.mise.toml"
    [ ! -f "$toml" ] && return 1
    local addr
    addr="$(sed -n -E 's/^[[:space:]]*SERVER_ADDR[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "$toml" 2>/dev/null | head -1)"
    [ -n "$addr" ] || return 1
    printf '%s' "$addr"
}

# ── 从 vite.config.ts 解析端口 ─────────────────────────────
parse_vite_port() {
    local dir="${1:-}"
    [ -z "$dir" ] && return 1
    local config="${dir}/vite.config.ts"
    [ ! -f "$config" ] && config="${dir}/vite.config.js"
    [ ! -f "$config" ] && return 1
    local port
    port="$(sed -n -E 's/.*port:[[:space:]]*([0-9]+).*/\1/p' "$config" 2>/dev/null | head -1)"
    [ -n "$port" ] || return 1
    printf '%s' "$port"
}

# ── 构建所有后端（--release） ──────────────────────────────
# 目标目录一律经映射（scripts/lib/cargo-target-dirs.sh）：Framework/SSO 是 root workspace
# 成员 ⇒ 用途 check；Meta 独立 workspace ⇒ 用途 meta。MUST NOT 依赖 cwd 默认推导。
build_all_backends() {
    # sccache 编译缓存接线（唯一实现 = scripts/lib/sccache.sh；幂等、fail-soft，未安装不阻断构建）
    # shellcheck source=scripts/lib/sccache.sh
    source "${LIB_DIR}/sccache.sh"
    sccache_enable_if_available "env-common build_all_backends"
    log "构建所有后端服务（release 模式）..."
    # 本步 cd 落点 = Framework/backend，但该目录**没有** Cargo.toml/workspace 根 ⇒ cargo 上溯到
    # 仓库根 workspace ⇒ 实际编译**平台面全量**（Framework + Ext-adapter + SSO + Gateway lib +
    # OpenActivity），产物落用途 check。命名如实反映实际编译集：ns 二进制与组合根 gateway-host
    # 均不在此步（见下方 build-ns.sh）。
    log "root workspace（Framework + Ext-adapter + SSO + Gateway lib + OpenActivity）..."
    (cd "${PROJECT_ROOT}/Framework/backend" && env CARGO_TARGET_DIR="$(bash "${PROJECT_ROOT}/scripts/cargo-target.sh" check)" cargo build --workspace --release) || {
        err "root workspace 构建失败（包含 Framework / SSO / Gateway lib / OpenActivity）"
        exit 1
    }
    log "Meta/backend..."
    (cd "${PROJECT_ROOT}/Meta/backend" && env CARGO_TARGET_DIR="$(bash "${PROJECT_ROOT}/scripts/cargo-target.sh" meta)" cargo build --release --bin meta-backend) || {
        err "Meta/backend 构建失败"
        exit 1
    }
    log "SSO/backend..."
    (cd "${PROJECT_ROOT}/SSO/backend" && env CARGO_TARGET_DIR="$(bash "${PROJECT_ROOT}/scripts/cargo-target.sh" check)" cargo build --release) || {
        err "SSO/backend 构建失败"
        exit 1
    }
    log "Gateway/backend..."
    # canonical 入口（compilation capability `release-build-via-script`）：target 隔离
    # （--target-dir Deploy/{ns}/target）、feature 门控（--no-default-features --features
    # {ns},sso）、macOS 签名单点实现——三者只在 scripts/build-ns.sh 一处，本行不内联 cargo。
    # 生产口径不含 preproc-proxy（未认证反代 /preproc/*、/api/pre_proc/*）——该 feature
    # 仅 dev 形态需要，由 dev 任务显式追加（mise run -C Gateway/backend dev）。
    # NS 未设时默认 alioth（build-ns.sh 侧已做大小写归一，统一落 Deploy/Alioth/）。
    bash "${PROJECT_ROOT}/scripts/build-ns.sh" "${NS:-alioth}" release || {
        err "Gateway/backend 构建失败"
        exit 1
    }
    ok "所有后端构建完成"
}

# ── 构建所有前端 ──────────────────────────────────────────
build_all_frontends() {
    log "构建所有前端..."
    log "Framework/frontend..."
    (cd "${PROJECT_ROOT}/Framework/frontend" && pnpm run -r build) || {
        err "Framework/frontend 构建失败"
        exit 1
    }
    log "Meta/frontend..."
    (cd "${PROJECT_ROOT}/Meta/frontend" && pnpm build) || {
        err "Meta/frontend 构建失败"
        exit 1
    }
    log "Gateway/frontend..."
    (cd "${PROJECT_ROOT}/Gateway/frontend" && pnpm build) || {
        err "Gateway/frontend 构建失败"
        exit 1
    }
    ok "所有前端构建完成"
}

# ── SYSTEM_CONFIG_ENC_KEY 初始化 ───────────────────────────
# 自包含 helper（PROJECT_ROOT 由调用方设定）
source "${LIB_DIR}/system-config-enc-key.sh"

# ── nohup 后台启动服务 ────────────────────────────────────
# 用法：nohup_start <log_tag> <work_dir> <env_file_path> <command...>
# 返回：通过 $NOHUP_PID 输出 PID
nohup_start() {
    local tag="$1"; shift
    local work_dir="$1"; shift
    local env_file="$1"; shift
    local log_dir="${PROJECT_ROOT}/logs/${ENV_NAME}"
    mkdir -p "$log_dir"
    local log_file="${log_dir}/${tag}.log"
    local pid_file="${log_dir}/${tag}.pid"

    cd "$work_dir"

    # 加载解密环境变量
    if [ -f "$env_file" ]; then
        export $(grep -v '^\s*#' "$env_file" | grep -v '^\s*$' | xargs) 2>/dev/null || true
    fi

    log "启动 $tag → ${log_file}"

    # nohup 启动，stderr 也重定向到日志
    nohup "$@" > "$log_file" 2>&1 &
    local pid=$!
    echo "$pid" > "$pid_file"
    NOHUP_PID=$pid

    cd "$PROJECT_ROOT"
    ok "$tag 已启动 (PID: $pid)"
}

# ── 停止所有后台服务 ──────────────────────────────────────
stop_all_services() {
    local log_dir="${PROJECT_ROOT}/logs/${ENV_NAME}"
    log "停止所有 ${ENV_NAME} 服务..."
    if [ -d "$log_dir" ]; then
        for pid_file in "$log_dir"/*.pid; do
            [ -f "$pid_file" ] || continue
            local pid
            pid=$(cat "$pid_file" 2>/dev/null || true)
            local svc_name
            svc_name=$(basename "$pid_file" .pid)
            if [ -n "$pid" ]; then
                log "停止 $svc_name (PID: $pid)..."
                kill "$pid" 2>/dev/null && ok "$svc_name 已停止" || warn "$svc_name 未运行"
            fi
            rm -f "$pid_file"
        done
    fi
    ok "所有 ${ENV_NAME} 服务已停止"
}
