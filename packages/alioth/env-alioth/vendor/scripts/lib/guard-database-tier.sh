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

    # 仓库根（`BASH_SOURCE` 锚定，**MUST NOT** 依赖调用方 cwd）——dev 分支的 ns env 枚举用。
    # 为什么：原实现写 `../../Deploy/*.env`（相对 cwd），从仓库根调用（如 restart-gateway.sh
    # 的 `cd "$PROJECT_ROOT"` 后调用）解析到仓外 ⇒ 合法集退化为 {aliothstudio_dev}，契约要求的
    # per-ns 绑定反被判违规（fix-ns-package-env-db-binding 实测 A/B：同 URL 双 cwd 结论相反）。
    local _guard_root
    _guard_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." 2>/dev/null && pwd)"

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
            for ns_env in "${_guard_root}"/Deploy/*/.env; do
                [[ -f "$ns_env" ]] || continue
                local ns_db
                ns_db="$(grep -E '^DATABASE_URL=' "$ns_env" | head -1 | cut -d= -f2-)"
                ns_db="${ns_db%%\?*}"
                ns_db="${ns_db%/}"
                ns_db="${ns_db##*/}"
                [[ -n "$ns_db" ]] && expected_dbs+=("$ns_db")
            done
            ;;
        pre)
            # 平台 pre 库 + **ns pre 库**（`{ns}_pre`，层映射唯一实现 = scripts/db/lib/ns-db-name.sh）：
            # per-ns pre 层（strengthen-pre-env-and-promotion-gate 方案 A）下，ns 组件在 pre 层
            # 绑定的是自己的 pre 库而非平台库；不收 ns pre 名会把合法 pre 启动判成跨层污染。
            expected_dbs+=("aliothstudio_pre")
            if [[ -n "${NAMESPACE:-}" ]]; then
                local _guard_lib_dir ns_pre_db
                _guard_lib_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
                if [[ -f "${_guard_lib_dir}/../db/lib/ns-db-name.sh" ]]; then
                    # shellcheck source=scripts/db/lib/ns-db-name.sh
                    source "${_guard_lib_dir}/../db/lib/ns-db-name.sh"
                    ns_pre_db="$(ns_name_to_db "${NAMESPACE}" pre 2>/dev/null || true)"
                    [[ -n "$ns_pre_db" ]] && expected_dbs+=("$ns_pre_db")
                fi
            fi
            ;;
        prod) expected_dbs+=("aliothstudio")      ;;
        test)
            # 平台测试库 + **ns 测试库**（`{ns}_test`，层映射唯一实现 = scripts/db/lib/ns-db-name.sh）：
            # per-ns 测试库隔离（enforce-ns-isolation-across-axes）下，ns 组件测试绑定自身测试库；
            # 只认 aliothstudio_test 会把合法 ns 测试运行判成跨层污染。
            expected_dbs+=("aliothstudio_test")
            if [[ -n "${NAMESPACE:-}" ]]; then
                local _guard_t_lib ns_test_db
                _guard_t_lib="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
                if [[ -f "${_guard_t_lib}/../db/lib/ns-db-name.sh" ]]; then
                    # shellcheck source=scripts/db/lib/ns-db-name.sh
                    source "${_guard_t_lib}/../db/lib/ns-db-name.sh"
                    ns_test_db="$(ns_name_to_db "${NAMESPACE}" test 2>/dev/null || true)"
                    [[ -n "$ns_test_db" ]] && expected_dbs+=("$ns_test_db")
                fi
            fi
            ;;
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
