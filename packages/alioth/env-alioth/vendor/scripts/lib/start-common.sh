#!/usr/bin/env bash
# start-common.sh — 组件启动脚本共享库（gateway/meta/sso start.sh source）
#
# 提供：颜色 + log/ok/warn/err（前缀 COMPONENT_TAG，默认时间戳）、
#       load_env_file、check_database、kill_by_port
#
# 用法：
#   source "${PROJECT_ROOT}/scripts/lib/start-common.sh"
#   COMPONENT_TAG="my-service"   # 可选：日志前缀（默认 [HH:MM:SS]）
set -euo pipefail

# ── 颜色（重复 source 无害） ─────────────────────────────
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m'

# ── 日志（前缀 = COMPONENT_TAG，未设则时间戳） ─────────────
_start_prefix() {
    if [ -n "${COMPONENT_TAG:-}" ]; then
        echo "${COMPONENT_TAG}"
    else
        date '+%H:%M:%S'
    fi
}
log() { echo -e "${BLUE}[$(_start_prefix)]${NC} $*"; }
ok()  { log "${GREEN}✓${NC} $*"; }
warn() { log "${YELLOW}⚠${NC} $*"; }
err() { log "${RED}✗${NC} $*" >&2; }

# ── .env 加载（set -a 包裹，保留原 allexport 状态） ───────
load_env_file() {
    local env_file="$1"
    local allexport_was_on=false
    [[ $- == *a* ]] && allexport_was_on=true
    set -a
    # shellcheck disable=SC1090
    source "$env_file"
    if [ "$allexport_was_on" = false ]; then
        set +a
    fi
}

# ── 数据库连接检查（dev 自动创建 / prod 失败退出） ───────
# check_database <dev|prod>
check_database() {
    local mode="$1"
    local db_url="${DATABASE_URL:-}"

    if [ -z "$db_url" ]; then
        warn "DATABASE_URL 未设置，跳过数据库检查"
        return 0
    fi

    local db_display
    db_display=$(echo "$db_url" | sed -E 's|://([^:]+):[^@]+@|://\1:****@|')
    log "数据库: ${db_display}"

    local pg_host pg_port pg_db
    pg_host=$(echo "$db_url" | sed -n 's|.*@\([^:/]*\).*|\1|p')
    pg_port=$(echo "$db_url" | sed -n 's|.*:\([0-9]\{4,5\}\)/.*|\1|p')
    pg_db=$(echo "$db_url" | sed -n 's|.*/\([^?]*\).*|\1|p')
    pg_host="${pg_host:-localhost}"
    pg_port="${pg_port:-5432}"

    if command -v pg_isready >/dev/null 2>&1; then
        if pg_isready -h "$pg_host" -p "$pg_port" -d "$pg_db" -q 2>/dev/null; then
            ok "数据库连接正常 (${pg_host}:${pg_port}/${pg_db})"
            return 0
        fi
    fi

    if command -v psql >/dev/null 2>&1; then
        if PGCONNECT_TIMEOUT=3 psql "$db_url" -c "SELECT 1" >/dev/null 2>&1; then
            ok "数据库连接正常 (${pg_host}:${pg_port}/${pg_db})"
            return 0
        fi
    fi

    if [ "$mode" = "prod" ]; then
        err "无法连接数据库 (${pg_host}:${pg_port}/${pg_db})，生产模式不可继续"
        exit 1
    fi

    warn "数据库 ${pg_db} 不可用，尝试创建..."
    local admin_url
    admin_url=$(echo "$db_url" | sed -E 's|/[^/?]*(\?|$)|/postgres\1|')

    if command -v createdb >/dev/null 2>&1; then
        createdb -h "$pg_host" -p "$pg_port" "$pg_db" 2>/dev/null && {
            ok "数据库 ${pg_db} 创建成功"
            return 0
        }
    fi

    if command -v psql >/dev/null 2>&1; then
        PGCONNECT_TIMEOUT=3 psql "$admin_url" -c "CREATE DATABASE \"${pg_db}\"" >/dev/null 2>&1 && {
            ok "数据库 ${pg_db} 创建成功"
            return 0
        }
    fi

    warn "无法创建数据库 ${pg_db}，请手动执行: createdb ${pg_db}"
    return 1
}

# ── 端口占用释放 ─────────────────────────────────────────
kill_by_port() {
    # shellcheck disable=SC2034  # name 参数保留签名兼容（调用方传日志标签）
    local port="$1" name="$2"
    if command -v lsof >/dev/null 2>&1; then
        local pid
        pid=$(lsof -ti :"${port}" 2>/dev/null || true)
        if [ -n "$pid" ]; then
            warn "端口 ${port} 被占用 (PID: ${pid})，正在释放..."
            kill "$pid" 2>/dev/null || true
            sleep 1
            if ps -p "$pid" > /dev/null 2>&1; then
                kill -9 "$pid" 2>/dev/null || true
            fi
        fi
    fi
}
