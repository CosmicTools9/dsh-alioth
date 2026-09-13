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
parse_mise_server_addr() {
    local dir="${1:-}"
    [ -z "$dir" ] && return 1
    local toml="${dir}/.mise.toml"
    [ ! -f "$toml" ] && return 1
    grep -oP 'SERVER_ADDR\s*=\s*"\K[^"]+' "$toml" 2>/dev/null || return 1
}

# ── 从 vite.config.ts 解析端口 ─────────────────────────────
parse_vite_port() {
    local dir="${1:-}"
    [ -z "$dir" ] && return 1
    local config="${dir}/vite.config.ts"
    [ ! -f "$config" ] && config="${dir}/vite.config.js"
    [ ! -f "$config" ] && return 1
    grep -oP 'port:\s*\K\d+' "$config" 2>/dev/null | head -1 || return 1
}

# ── 构建所有后端（--release） ──────────────────────────────
build_all_backends() {
    log "构建所有后端服务（release 模式）..."
    log "Framework/backend..."
    (cd "${PROJECT_ROOT}/Framework/backend" && cargo build --workspace --release) || {
        err "Framework/backend 构建失败"
        exit 1
    }
    log "Meta/backend..."
    (cd "${PROJECT_ROOT}/Meta/backend" && cargo build --release --bin meta-backend) || {
        err "Meta/backend 构建失败"
        exit 1
    }
    log "SSO/backend..."
    (cd "${PROJECT_ROOT}/SSO/backend" && cargo build --release) || {
        err "SSO/backend 构建失败"
        exit 1
    }
    log "Gateway/backend..."
    # 生产口径构建：--no-default-features + {ns},sso（对齐 scripts/build-ns.sh）。
    # preproc-proxy（未认证反代 /preproc/*、/api/pre_proc/*）不在此路径提供——
    # dev 形态需显式追加该 feature（mise run -C Gateway/backend dev）。
    # NS 环境变量透传（默认 alioth）；cargo feature 名全小写（Cargo.toml [features]），
    # 显式 NS=WZ/AVIC-CAASEC 等大写值时需归一（对齐 scripts/build-ns.sh 的 NS_LOWER）。
    NS_LOWER="$(echo "${NS:-alioth}" | tr '[:upper:]' '[:lower:]')"
    (cd "${PROJECT_ROOT}/Gateway/backend" && cargo build --release -p alioth-gateway --no-default-features --features "${NS_LOWER},sso") || {
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
