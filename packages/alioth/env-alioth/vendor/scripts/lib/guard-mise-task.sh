#!/bin/bash
# guard-mise-task.sh — mise task 启动守卫包装器
#
# 供各 Backend `.mise.toml` 的 `dev` / `test` 任务引用，
# 在 `cargo run` / `cargo test` 之前校验 DATABASE_URL 指向正确的 DB 实例
# （ENVIRONMENT_SPEC §6.7）。
#
# Usage:
#   bash guard-mise-task.sh <Component> [--mode dev|test] -- <command...>
#
# 示例（在 .mise.toml 中）：
#   dev = "bash ../../scripts/lib/guard-mise-task.sh Gateway -- cargo run"
#   test = "bash ../../scripts/lib/guard-mise-task.sh Gateway --mode test -- cargo test"
#
# 行为:
#   1. 解析 Component 名与可选 mode（默认 dev）
#   2. 检查 DATABASE_URL 已设置（源自 .mise.toml _.file + decrypt-env.sh）
#   3. source guard-database-tier.sh
#   4. 调用 guard_database_tier <mode> <Component> <env_file>
#   5. 失败 exit 1（阻断 cargo run 启动）
#   6. 成功 exec <command...> 替换当前进程

set -e

# ── 参数解析 ──────────────────────────────────────────────
if [[ $# -lt 1 ]]; then
    echo "usage: guard-mise-task.sh <Component> [--mode dev|pre|prod|test] -- <command...>" >&2
    exit 2
fi

COMPONENT="$1"
shift

MODE="dev"
while [[ $# -gt 0 && "$1" != "--" ]]; do
    case "$1" in
        --mode)
            MODE="$2"
            shift 2
            ;;
        *)
            echo "[guard] unknown arg: $1" >&2
            exit 2
            ;;
    esac
done

if [[ $# -lt 1 || "$1" != "--" ]]; then
    echo "[guard] missing '--' separator before command" >&2
    exit 2
fi
shift  # drop "--"

if [[ $# -lt 1 ]]; then
    echo "[guard] no command to exec" >&2
    exit 2
fi

# ── Resolve helper script location ──
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# ── Load tier guard ──
# shellcheck source=guard-database-tier.sh
source "${SCRIPT_DIR}/guard-database-tier.sh"

# ── 启动前环境自愈（ensure-db-url.sh：解密/PGUSER 兜底/连接检测，add-startup-env-self-heal）──
# shellcheck source=../env/ensure-db-url.sh
source "${PROJECT_ROOT}/scripts/env/ensure-db-url.sh" "${COMPONENT}/backend" || exit 1

# ── 校验 DATABASE_URL 已设置（源自 .mise.toml → .env → decrypt） ──
if [[ -z "${DATABASE_URL:-}" ]]; then
    echo "[guard] ❌ DATABASE_URL not set after .env loading + decryption" >&2
    echo "[guard]    Check that .mise.toml's _.file and _.source are working correctly" >&2
    exit 1
fi

# ── 推断 .env 路径（仅用于诊断日志） ──
ENV_FILE=""
for candidate in \
    "${PROJECT_ROOT}/${COMPONENT}/backend/.env" \
    "${PROJECT_ROOT}/${COMPONENT}/.env" \
    "${PROJECT_ROOT}/Gateway/backend/.env" \
    "${PROJECT_ROOT}/Meta/backend/.env" \
    "${PROJECT_ROOT}/SSO/backend/.env"; do
    if [[ -f "$candidate" ]]; then
        ENV_FILE="$candidate"
        break
    fi
done
ENV_FILE="${ENV_FILE:-(not found)}"

# ── Run tier guard ──
if ! guard_database_tier "$MODE" "$COMPONENT" "$ENV_FILE"; then
    exit 1
fi

# ── 透传剩余环境给 exec ──
exec "$@"
