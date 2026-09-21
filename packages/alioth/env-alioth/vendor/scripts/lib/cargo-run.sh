#!/usr/bin/env bash
# cargo-run.sh — cargo 执行外壳：target 目录锁规避 + 源码漂移守卫（唯一实现）
#
# 两件事：
#   ① target 选择（自 scripts/cargo-check.sh 内聚而来，行为不变）：
#      默认 /tmp/alioth-check → 被活 cargo 占用则会话稳定目录 → 仍占用则冷目录。
#   ② 漂移守卫（fix-cargo-run-drift-guard）：编译前后各取一次源码内容指纹；不一致说明
#      编译读到的是**变化中的树**，结论不可信——共享 checkout 下并行会话的 stash/checkout
#      曾致「符号明明存在却报找不到」的瞬时假错 ⇒ 自动**重跑一次**，两次结论都打印。
#      源未变的真实失败**不重跑**（不掩盖真错）。
#
# 用法（见 scripts/cargo-check.sh / scripts/cargo-test.sh）：
#   source scripts/lib/src-fingerprint.sh
#   source scripts/lib/cargo-run.sh
#   cargo_run_guarded check -p wz-service-isahl-db
#   cargo_run_guarded test  -p wz-service-isahl-db --test org_subjects

# ── 锁检测 ──────────────────────────────────────────────────────────
# target 目录的 .cargo-lock 被活着的 cargo 进程持有 → 视为占用（死锁文件顺手清理）
cargo_target_locked() {
    local lock_file="$1/.cargo-lock"
    [ ! -f "${lock_file}" ] && return 1
    if lsof "${lock_file}" 2>/dev/null | grep -qE '\bcargo\b'; then
        return 0
    fi
    rm -f "${lock_file}"
    return 1
}

# ── 占用报告（面向人的只读诊断）──────────────────────────────────────
# 与 cargo_target_locked 的分工：后者是写作路径上的静默判定（占用 ⇒ 换目录 / 交给 cargo 排队），
# 本函数面向人，把「cargo 静默排队」变成可解释的等待——打印持有者 PID 与命令行，供
# dev 启动链等入口在构建前调一次（2026-09-20 实证：并发 `cargo test -p app-agent` 与
# `mise run meta` 的 dev 构建共用 Meta/backend/target，表现为启动长时间无输出、页面打不开）。
# 用法：cargo_target_conflict_report <target_dir> [标签]；有占用返回 0，空闲返回 1。
cargo_target_conflict_report() {
    local dir="${1:-}" label="${2:-}"
    [ -n "${dir}" ] || return 1
    local lock_file="${dir}/.cargo-lock"
    [ -f "${lock_file}" ] || return 1
    local pids
    pids="$(lsof -t "${lock_file}" 2>/dev/null | sort -u)"
    [ -n "${pids}" ] || return 1
    echo "⚠️  ${label:-${dir}} 正被其它 cargo 占用——构建会排队等待（明显变慢）："
    local pid
    for pid in ${pids}; do
        echo "      PID ${pid}  $(ps -o command= -p "${pid}" 2>/dev/null | cut -c1-140)"
    done
    echo "      如需真正并行：把另一侧换到不同用途目录（映射见 docs/specs/COMPILATION_GUIDE.md）。"
    return 0
}

# ── 会话标识（多 agent 并发时同一终端 tab 视为同一会话）──────────────
cargo_session_key() {
    local raw=""
    if [ -n "${TERM_SESSION_ID:-}" ]; then
        raw="${TERM_SESSION_ID}"
    else
        local t
        t="$(tty 2>/dev/null || true)"
        case "${t}" in /dev/*) raw="${t}" ;; esac
    fi
    [ -z "${raw}" ] && return 1
    printf '%s' "${raw}" | shasum | cut -c1-8
}

# ── 机会式 GC（只清「不再复用」的临时目录）──────────────────────────
# 固定热缓存 /tmp/alioth-check 与 /tmp/alioth-check-gate 不匹配下列 glob，永不被清
cargo_gc_targets() {
    local freed_kb=0 removed=0
    _cargo_gc_sweep() { # $1=超龄天数 $2=要清的空格分隔目录串
        local days="$1" d
        shift
        for d in $1; do
            [ -d "${d}" ] || continue
            find "${d}" -maxdepth 0 -mtime "+${days}" | grep -q . || continue
            cargo_target_locked "${d}" && continue
            local kb
            kb="$(du -sk "${d}" 2>/dev/null | cut -f1)"
            rm -rf "${d}"
            freed_kb=$((freed_kb + ${kb:-0}))
            removed=$((removed + 1))
        done
    }
    _cargo_gc_sweep 3 '/tmp/alioth-check-sess-*'
    _cargo_gc_sweep 1 '/tmp/alioth-check-[0-9]*-[0-9]*'
    _cargo_gc_sweep 7 '/tmp/alioth-check-gate-*'
    _cargo_gc_sweep 3 '/tmp/alioth-check-gate-*/*-sess-*'
    _cargo_gc_sweep 1 '/tmp/alioth-check-gate-*/*-fallback-*'
    if [ "${removed}" -gt 0 ]; then
        echo "[cargo-run] 🧹 GC: 清理 ${removed} 个超龄目录，释放 $((freed_kb / 1024)) MB" >&2
    fi
}

# ── target 选择（两级 fallback；结果写 CARGO_TARGET_DIR）─────────────
# 默认目录取自唯一映射（scripts/lib/cargo-target-dirs.sh 的 check 用途）；
# 该 lib 缺失时退化为内置默认——两者语义一致，lib 是文档表的镜像。
_CARGO_RUN_LIB_DIR="${BASH_SOURCE[0]%/*}"
if [ -f "${_CARGO_RUN_LIB_DIR}/cargo-target-dirs.sh" ]; then
    # shellcheck source=scripts/lib/cargo-target-dirs.sh
    source "${_CARGO_RUN_LIB_DIR}/cargo-target-dirs.sh"
fi
cargo_select_target() {
    local default_target="/tmp/alioth-check"
    if command -v cargo_target_dir >/dev/null 2>&1; then
        default_target="$(cargo_target_dir check)"
    fi
    local target="${CARGO_TARGET_DIR:-${default_target}}"
    if cargo_target_locked "${target}"; then
        local key sess
        key="$(cargo_session_key || true)"
        sess=""
        [ -n "${key}" ] && sess="/tmp/alioth-check-sess-${key}"
        if [ -n "${sess}" ] && ! cargo_target_locked "${sess}"; then
            echo "[cargo-run] ⚠️  ${target} 被其他 cargo 进程占用，改用会话目录 ${sess}" >&2
            target="${sess}"
        else
            local fallback
            fallback="/tmp/alioth-check-$(date +%s)-$$"
            echo "[cargo-run] ⚠️  ${target} 被其他 cargo 进程占用，改用冷目录 ${fallback}" >&2
            target="${fallback}"
        fi
    fi
    export CARGO_TARGET_DIR="${target}"
    mkdir -p "${CARGO_TARGET_DIR}"
}

# cargo_ensure_source_config [root] — 项目级依赖源配置缺失时择源一次（多镜像回退；非阻断）。
#
# 为什么：用户级 `~/.cargo/config.toml` 把 crates-io 指向 `mirror.isahl.com`，该域名在部分网络下
# 解析到不可达内网地址 ⇒ cargo 逐 crate 超时重试并**长时间持有包缓存锁**（构建 0% CPU 卡死、
# 多会话互相阻塞、LSP 派生的 `cargo metadata` 同样中招）。任何直接跑 cargo 的入口在调用前
# source 本文件并调用本函数即可；**稳态（配置已在）零开销**，环境变量 `CARGO_MIRROR_SKIP=1` 跳过。
cargo_ensure_source_config() {
    local root="${1:-${PROJECT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}}"
    [ -f "${root}/.cargo/config.toml" ] && return 0
    [ -f "${root}/scripts/lib/cargo-mirror-fallback.sh" ] || return 0
    # shellcheck source=./cargo-mirror-fallback.sh
    # shellcheck disable=SC1091
    source "${root}/scripts/lib/cargo-mirror-fallback.sh"
    # 验证预算放宽到 90s：择源验证自身是 `cargo fetch`，在并发 cargo 持包缓存锁时会排队——
    # 30s（脚本默认）下会把「排队中」误判为「该源不可用」并删掉配置（实测）。
    CARGO_MIRROR_VERIFY_TIMEOUT="${CARGO_MIRROR_VERIFY_TIMEOUT:-90}" cargo_mirror_select >&2 || true
    return 0
}

# ── 守卫执行 ────────────────────────────────────────────────────────
# cargo_run_guarded <check|test|build|...> [cargo args...]
cargo_run_guarded() {
    local sub="$1"
    shift
    cargo_gc_targets
    cargo_select_target
    cargo_ensure_source_config

    local fp_dirs=()
    # CARGO_RUN_FP_DIRS：指纹根覆盖（空格分隔；测试与受限场景用；缺省 = 整个编译面）
    if [ -n "${CARGO_RUN_FP_DIRS:-}" ]; then
        # shellcheck disable=SC2206
        fp_dirs=(${CARGO_RUN_FP_DIRS})
    fi
    local fp_before fp_after
    if [ "${#fp_dirs[@]}" -gt 0 ]; then
        fp_before="$(src_fingerprint "${fp_dirs[@]}")"
    else
        fp_before="$(src_fingerprint)"
    fi
    cargo "${sub}" "$@"
    local rc=$?
    if [ "${#fp_dirs[@]}" -gt 0 ]; then
        fp_after="$(src_fingerprint "${fp_dirs[@]}")"
    else
        fp_after="$(src_fingerprint)"
    fi

    if [ -n "${fp_before}" ] && [ "${fp_before}" != "${fp_after}" ]; then
        echo "" >&2
        echo "[cargo-run] ⚠️  编译期间源码发生变化（fp ${fp_before:0:8} → ${fp_after:0:8}）" >&2
        echo "[cargo-run]     首轮结论（exit=${rc}）不可信——共享 checkout 下他人 checkout/stash 可致瞬时假错；自动重跑一次" >&2
        fp_before="${fp_after}"
        cargo "${sub}" "$@"
        rc=$?
        if [ "${#fp_dirs[@]}" -gt 0 ]; then
            fp_after="$(src_fingerprint "${fp_dirs[@]}")"
        else
            fp_after="$(src_fingerprint)"
        fi
        if [ "${fp_before}" != "${fp_after}" ]; then
            echo "[cargo-run] ⚠️  重跑期间源码仍在变化（fp ${fp_before:0:8} → ${fp_after:0:8}）——结论仅供参照，请稳定工作树后复跑" >&2
        else
            echo "[cargo-run] ✅ 重跑完成（源码已稳定，exit=${rc}）" >&2
        fi
    fi
    return "${rc}"
}
