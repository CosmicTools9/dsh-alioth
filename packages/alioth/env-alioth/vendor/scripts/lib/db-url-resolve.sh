#!/usr/bin/env bash
# =============================================================================
# db-url-resolve.sh — `DATABASE_URL` 的**解析通道**单一真相源
#
# 为什么需要本文件（2026-09-21 实证）：
#   `DATABASE_URL` **不是仓根级变量**，也不在 `.env` 里——
#   `.env` 自 2026-09 起刻意**不再承载**该键（原 `enc:` 密文注入优先级高于 shell，
#   导致显式 `DATABASE_URL=…test` 经 `mise run` 仍连 dev；见 Meta/backend/.mise.toml:91-95）。
#   现行承载面 = **组件级 `.mise.toml [env]`**（`DATABASE_URL = { default = "postgres://…" }`）。
#   ⇒ 任何「读 `.env` 找 DATABASE_URL」或「只读 shell 的 $DATABASE_URL」的探测都会**假报未配置**
#     （本条即修此漂移：探测口径收敛到本文件的解析通道）。
#
# 用法（source 型）:
#   source scripts/lib/db-url-resolve.sh
#   url="$(db_url_resolve)"        # 解析出的 URL（无则空串）；DB_URL_SOURCE = 来源说明
#   db_url_resolve_candidates      # 诊断用：输出声明了该键的组件目录（相对仓库根）
#
# 只读契约：只读各组件 `.mise.toml`，并在组件目录内执行 `mise env`（解密/汇编 env，
# 不写仓库、不写 DB、不建库）。
# =============================================================================

DB_URL_LIB_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# 由调用方在 source 后读取（如 env-orchestrator 的 DATABASE_URL 探测行）——shellcheck 跨文件不可见
# shellcheck disable=SC2034
DB_URL_SOURCE=""

# 候选 = 在自身 `.mise.toml` 声明 `DATABASE_URL` 的组件（`[env]` 形态，含 `{ default = … }`）
db_url_resolve_candidates() {
    local mise_toml comp
    for mise_toml in "${DB_URL_LIB_ROOT}"/*/*/.mise.toml; do
        [ -f "$mise_toml" ] || continue
        grep -qE '^"?DATABASE_URL"? *=' "$mise_toml" || continue
        comp="$(dirname "$mise_toml")"
        printf '%s\n' "${comp#"${DB_URL_LIB_ROOT}"/}"
    done
}

# 解析：环境变量优先；否则按候选组件走 `mise env`（单趟 awk，ERE 可移植）
db_url_resolve() {
    DB_URL_SOURCE=""
    if [ -n "${DATABASE_URL:-}" ]; then
        DB_URL_SOURCE="环境变量"
        printf '%s' "$DATABASE_URL"
        return 0
    fi
    local comp url
    while IFS= read -r comp; do
        url="$(cd "${DB_URL_LIB_ROOT}/${comp}" && mise env 2>/dev/null \
            | awk '/^(export )?DATABASE_URL=/{sub(/^export /,""); sub(/^DATABASE_URL=/,""); print; exit}' \
            | sed -e "s/['\"]//g")"
        if [ -n "$url" ]; then
            DB_URL_SOURCE="${comp}/.mise.toml [env]（mise 解析）"
            printf '%s' "$url"
            return 0
        fi
    done < <(db_url_resolve_candidates)
    return 1
}
