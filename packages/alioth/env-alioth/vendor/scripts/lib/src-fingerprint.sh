#!/usr/bin/env bash
# src-fingerprint.sh — 源码**内容**指纹（判定「构建/编译期间源是否变化」的唯一实现）
#
# 事故背景（2026-09-15，fix-cargo-run-drift-guard / fix-watcher-stale-binary）：
#   dev 后端 watcher 用 mtime stamp 判漂移，且把 stamp 锚定在构建**结束**时刻 ⇒
#   构建期间落盘的编辑被静默跳过，二进制与源码永久不一致（实证：watcher 产出的
#   `Deploy/WZ/bin/wz-server` 仍含中间态 SQL ⇒ 活栈端点 500 `字段 c.tableoid 不存在`）。
#   并行共享 checkout 下 mtime 语义同样脆弱（他人 checkout/stash 会刷新 mtime 而不改内容）。
#   同一根因在「编译期读到变化中的树」时给出**瞬时假错**（实测：identity-org 报
#   `cannot find common::leaf_relname`，串行重跑即通过）。
#
# 指纹 = 构建输入闭包的内容哈希（与 mtime 无关）：
#   find（构建输入扩展名）→ LC_ALL=C sort -z（路径序稳定）→ cat（内容流）→ sha256
#
# 域 MUST 与构建输入闭包一致（与 `scripts/lib/binary-freshness.sh` 同口径）：
#   扩展名 = *.rs / Cargo.toml / build.rs / include_str! 内嵌资源（*.json/*.sql/*.yaml），
#   prune  = target/ node_modules/ dist/ .git/ tests/ benches/ examples/
# 实测事故（2026-09-15，本次修正的由来）：早期版本把 tests/ 纳入域，而 watcher 的 mtime 触发
# 与 binary-freshness.sh 都 prune 它 ⇒ 并行会话**只改测试**时，指纹把「非构建输入的变化」判成
# 漂移 ⇒ 有界轮数用尽、永不锚定 stamp ⇒ 无谓重建（实测：identity-org/tests/*.rs 的他人编辑
# 让一次真实构建判为未收敛）。域比触发器宽 = 假漂移，域比构建输入窄 = 陈旧二进制，两者都禁止。
#
# 用法：
#   source scripts/lib/src-fingerprint.sh
#   fp=$(src_fingerprint)                 # 默认：整个编译面（Framework/Gateway/SSO/Meta/OpenActivity/Pre-Proc Services）
#   fp=$(src_fingerprint "${WATCH_DIRS[@]}")   # 指定根（watcher 即传自己的监视集合）
src_fingerprint() {
    local root="${PROJECT_ROOT:?PROJECT_ROOT 未设置（调用方需先 export）}"
    local dirs=()
    if [ "$#" -gt 0 ]; then
        dirs=("$@")
    else
        dirs=(
            "${root}/Framework/backend"
            "${root}/Gateway/backend/src"
            "${root}/SSO/backend/src"
            "${root}/Meta/backend/src"
            "${root}/OpenActivity/backend/src"
        )
        local d
        for d in "${root}"/Pre-Proc/*/Sources/Apps/Services; do
            [ -d "${d}" ] && dirs+=("${d}")
        done
    fi
    # 不存在的根会被 find 忽略（stderr 丢弃）；数组为空（bash 3.2 下 @ 展开不安全）时退回仓库根
    [ "${#dirs[@]}" -gt 0 ] || dirs=("${root}")
    find "${dirs[@]}" -type f \
        \( -name '*.rs' -o -name '*.toml' -o -name '*.lock' -o -name '*.sql' \
           -o -name '*.json' -o -name '*.yaml' -o -name '*.yml' -o -name '*.proto' \) \
        -not -path '*/target/*' -not -path '*/node_modules/*' -not -path '*/dist/*' \
        -not -path '*/.git/*' \
        -not -path '*/tests/*' -not -path '*/benches/*' -not -path '*/examples/*' \
        -print0 2>/dev/null \
        | LC_ALL=C sort -z \
        | xargs -0 cat 2>/dev/null \
        | shasum -a 256 | awk '{print $1}'
}
