#!/bin/bash
# guard-database-tier.sh — ENVIRONMENT_SPEC §6.7 启动守卫
#
# 在每个后端启动前调用，验证 DATABASE_URL 指向的数据库实例与启动模式匹配。
# 失败时输出诊断并 exit 1，防止跨层连接污染生产/预发库。
#
# Usage:
#   source "$(dirname "${BASH_SOURCE[0]}")/guard-database-tier.sh"
#   guard_database_tier "dev" "Gateway" "Gateway/backend/.env"
#
# 参数：
#   $1 — 启动模式 ("dev" | "pre" | "prod" | "test")
#   $2 — 组件名（仅用于日志）
#   $3 — .env 文件路径（仅用于日志）
#
# 要求调用前已 export DATABASE_URL（或 .env 已解密）。

set -e

guard_database_tier() {
    local mode="$1"
    local component="${2:-?}"
    local env_file="${3:-?}"

    # ── Skip when no DATABASE_URL (e.g. server-less tools) ──
    if [[ -z "${DATABASE_URL:-}" ]]; then
        echo "[guard] ${component}: DATABASE_URL not set, skipping tier check"
        return 0
    fi

    # ── Skip when psql missing (degrade gracefully) ──
    if ! command -v psql >/dev/null 2>&1; then
        echo "[guard] ${component}: psql not found, skipping tier check"
        return 0
    fi

    # ── Resolve expected database name(s) from launch mode ──
    # dev 模式（2026-08-14 决策）：默认 ns 库（Deploy/{NAMESPACE}/.env 原样，
    # namespace 隔离，每 ns 独立库）；无 NAMESPACE 或非 ns 组件回退共享 dev 库
    # （aliothstudio_dev）。pre/prod/test 仍严格绑定四层库。
    local -a expected_dbs=()
    case "$mode" in
        dev)
            expected_dbs+=("aliothstudio_dev")
            for ns_env in ../../Deploy/*/.env; do
                [[ -f "$ns_env" ]] || continue
                local ns_db
                ns_db="$(grep -E '^DATABASE_URL=' "$ns_env" | head -1 | cut -d= -f2-)"
                ns_db="${ns_db%%\?*}"
                ns_db="${ns_db%/}"
                ns_db="${ns_db##*/}"
                [[ -n "$ns_db" ]] && expected_dbs+=("$ns_db")
            done
            ;;
        pre)  expected_dbs+=("aliothstudio_pre")  ;;
        prod) expected_dbs+=("aliothstudio")      ;;
        test) expected_dbs+=("aliothstudio_test") ;;
        *)
            echo "[guard] ${component}: unknown launch mode '${mode}', skipping tier check" >&2
            return 0
            ;;
    esac

    # ── Query current database ──
    local actual_db
    actual_db="$(psql "${DATABASE_URL}" -tA -c "SELECT current_database();" 2>/dev/null || echo "ERROR")"
    if [[ "$actual_db" == "ERROR" || -z "$actual_db" ]]; then
        echo "[guard] ${component}: cannot connect to '${DATABASE_URL}'" >&2
        echo "[guard] Aborting ${component} startup (mode=${mode}, expected=${expected_dbs[*]})" >&2
        return 1
    fi

    if [[ ! " ${expected_dbs[*]} " =~ " $actual_db " ]]; then
        cat >&2 <<EOF
[guard] ❌ DATABASE tier mismatch for ${component} (env file: ${env_file})

  Mode:           ${mode}
  Expected DB(s): ${expected_dbs[*]}
  Actual DB:      ${actual_db}
  DATABASE_URL:   ${DATABASE_URL}

[guard] Refusing to start — this would connect a dev/pre/prod process to the wrong tier.
[guard] Fix the DATABASE_URL in ${env_file} before retrying.
[guard] See docs/specs/ENVIRONMENT_SPEC.md §6.7 for the binding rule.
EOF
        return 1
    fi

    echo "[guard] ✓ ${component} → ${actual_db} (mode=${mode})"
    return 0
}
