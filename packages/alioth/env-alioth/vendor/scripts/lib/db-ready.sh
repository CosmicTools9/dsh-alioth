#!/usr/bin/env bash
# db-ready.sh — DB 可达判据（pre-commit 探活门与全仓审计 runner 共用单一实现）
#
# 为什么不用 `timeout 2 nc -z host port`（原实现，2026-09-13 实证为缺陷）：
#   macOS 无 coreutils `timeout` 二进制——本机 `command -v timeout` 只解析到**工具 shell 的
#   函数**，git 拉起的 hook 子进程拿不到 → `timeout 2 nc …` 恒以 127 退出 → 探活恒判
#   「不可达」→ DB 类门禁（code-table-refs / ontology-coords / task-hierarchy /
#   model-seed-contract / dk 码表）在 commit 与 push 时被**静默跳过**（表现为
#   "skipped (DB unreachable, fast-fail)"），而 psql / bun 脚本本身连库正常。
#
# 判据（按序；两者都自带超时，不依赖外部 `timeout`）：
#   1. pg_isready -d <URL> -t 2 —— PG 客户端自带，直接解析 URL（TCP / unix socket / 主机名）
#   2. PGCONNECT_TIMEOUT=2 psql -X -tAc 'select 1' <URL> —— 回退（-c 内联 = 内容可见通道）
#
# DSN 取值（2026-09-28 起**按仓角色**，唯一实现 = `scripts/lib/db-url-resolve.sh`）：
#   · 显式 `DATABASE_URL` 环境变量优先；
#   · **ns 仓**（仓根含 `.alioth/platform.conf`）⇒ 本 ns dev 库（`Deploy/{ns}/.env`）；解析不到即判
#     **不可达**（MUST NOT 回落平台共享库缺省——以平台库作 ns 面判定面属错误判定，劣于诚实 skip）；
#   · **平台仓** ⇒ 组件 `.mise.toml [env]` 通道，无声明时用平台 dev 缺省（行为与拆分前一致）。
#
# 用法：
#   # shellcheck source=scripts/lib/db-ready.sh
#   source "$PROJECT_ROOT/scripts/lib/db-ready.sh"
#   if db_ready; then DB_READY=1; fi
#
# 注意：调用方脚本若开了 `set -e`，请以上面的 if 形式调用（函数以返回码表达结论，
# 直接裸调在 set -e 下会退出）。
db_ready() {
    local lib_dir url root
    lib_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

    if [ -n "${DATABASE_URL:-}" ]; then
        url="$DATABASE_URL"
    else
        # shellcheck source=scripts/lib/db-url-resolve.sh
        . "${lib_dir}/db-url-resolve.sh"
        url="$(db_url_resolve 2>/dev/null || true)"
        if [ -z "$url" ]; then
            root="$(repo_role_root "$PWD" 2>/dev/null || true)"
            if [ -n "$root" ] && [ "$(repo_role "$root")" = "ns" ]; then
                # ns 仓：本 ns env 不可得 ⇒ 不可达（不得回落平台共享库）
                return 1
            fi
            url="${ALIOTH_PLATFORM_DEV_DSN:-postgres://isahl@localhost:5432/aliothstudio_dev}"
        fi
    fi

    if command -v pg_isready >/dev/null 2>&1; then
        pg_isready -d "$url" -t 2 >/dev/null 2>&1
        return $?
    fi
    PGCONNECT_TIMEOUT=2 psql -X -tAc 'select 1' "$url" >/dev/null 2>&1
}
