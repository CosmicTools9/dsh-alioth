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
#   url="$(db_url_resolve)"          # 解析出的 URL（无则空串）；直接调用时 DB_URL_SOURCE = 来源说明
#   db_url_resolve_report            # 两值一次取回：stdout 两行 `url=<…>` / `source=<…>`
#   db_url_resolve_candidates        # 诊断用：输出声明了该键的组件目录（相对仓库根）
#
# 只读契约：只读各组件 `.mise.toml`，并在组件目录内执行 `mise env`（解密/汇编 env，
# 不写仓库、不写 DB、不建库）。
# =============================================================================

DB_URL_LIB_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# 由**直接调用** db_url_resolve 的消费方在 source 后读取；command substitution 中取值时
# 本变量在子 shell 内被赋值、随子 shell 消亡 ⇒ 需要来源的消费方 MUST 改用 db_url_resolve_report
# （shellcheck 跨文件不可见）
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

# 解析扫描（**唯一实现**）：stdout 两行 `url=<…>` 与 `source=<…>`。
# 为什么两项一起输出：消费方以 `url="$(db_url_resolve)"` 取值时函数在**子 shell** 中执行，
# 其中的全局赋值随子 shell 消亡 ⇒ 来源恒空（2026-09-22 实证：报告输出 `已解析（）但库不可达`）。
# 两项只能由同一次调用的 stdout 一并带回，故扫描是唯一实现、两个门面共用。
db_url_resolve_scan() {
    local comp url
    if [ -n "${DATABASE_URL:-}" ]; then
        printf 'url=%s\nsource=%s\n' "$DATABASE_URL" "环境变量"
        return 0
    fi
    while IFS= read -r comp; do
        url="$(cd "${DB_URL_LIB_ROOT}/${comp}" && mise env 2>/dev/null \
            | awk '/^(export )?DATABASE_URL=/{sub(/^export /,""); sub(/^DATABASE_URL=/,""); print; exit}' \
            | sed -e "s/['\"]//g")"
        if [ -n "$url" ]; then
            printf 'url=%s\nsource=%s\n' "$url" "${comp}/.mise.toml [env]（mise 解析）"
            return 0
        fi
    done < <(db_url_resolve_candidates)
    return 1
}

# 单值门面（既有消费方签名不变）：stdout = URL；直接调用时同时设置 DB_URL_SOURCE。
db_url_resolve() {
    local out
    out="$(db_url_resolve_scan)" || { DB_URL_SOURCE=""; return 1; }
    DB_URL_SOURCE="$(printf '%s\n' "$out" | sed -n 's/^source=//p')"
    printf '%s' "$(printf '%s\n' "$out" | sed -n 's/^url=//p')"
}

# 两值门面：stdout = `url=<…>` / `source=<…>` 两行（需要来源的消费方用；一次扫描取回两项）。
db_url_resolve_report() {
    db_url_resolve_scan
}
