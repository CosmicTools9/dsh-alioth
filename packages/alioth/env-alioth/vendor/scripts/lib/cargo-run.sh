#!/usr/bin/env bash
# cargo-run.sh — cargo 执行外壳：target 目录锁规避 + 源码漂移守卫（唯一实现）
#
# 两件事：
#   ① target 选择（自 scripts/cargo-check.sh 内聚而来，行为不变）：
#      默认 <仓父目录>/.tmp/alioth-check → 被活 cargo 占用则会话稳定目录 → 仍占用则冷目录。
#   ② 漂移守卫（fix-stale-build-detection）：命令前后各取一次源码内容指纹；不一致说明
#      命令读到的是**变化中的树**，结论不可信——共享 checkout 下并行会话的 stash/checkout
#      曾致「符号明明存在却报找不到」的瞬时假错 ⇒ 自动**重跑一次**，两次结论都打印。
#      源未变的真实失败**不重跑**（不掩盖真错）。
#
# 用法（见 scripts/cargo-check.sh / scripts/cargo-test.sh / scripts/test-all.sh）：
#   source scripts/lib/cargo-run.sh      # src-fingerprint.sh 缺省时由本文件自补
#   cargo_run_guarded check -p wz-service-isahl-db     # cargo 专用 = target 选择 + 守卫
#   cargo_run_guarded test  -p wz-service-isahl-db --test org_subjects
#   run_guarded env CARGO_TARGET_DIR=/tmp/x cargo test --workspace   # 通用形态（全量测试入口）

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
# 固定热缓存（临时根下的 `alioth-check` / `alioth-check-gate`）不匹配下列 glob，永不被清。
# 临时根 = 仓父目录 `/.tmp`（跨平台统一；见 cargo-target-dirs.sh::cargo_tmp_root）
cargo_gc_targets() {
    local freed_kb=0 removed=0 _tmp
    _tmp="$(_cargo_tmp_root)"
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
    _cargo_gc_sweep 3 "${_tmp}/alioth-check-sess-"*
    _cargo_gc_sweep 1 "${_tmp}/alioth-check-"[0-9]*"-"[0-9]*
    _cargo_gc_sweep 7 "${_tmp}/alioth-check-gate-"*
    _cargo_gc_sweep 3 "${_tmp}/alioth-check-gate-"*"/"*"-sess-"*
    _cargo_gc_sweep 1 "${_tmp}/alioth-check-gate-"*"/"*"-fallback-"*
    if [ "${removed}" -gt 0 ]; then
        echo "[cargo-run] 🧹 GC: 清理 ${removed} 个超龄目录，释放 $((freed_kb / 1024)) MB" >&2
    fi
}

# ── target 选择（两级 fallback；结果写 CARGO_TARGET_DIR）─────────────
# 默认目录取自唯一映射（scripts/lib/cargo-target-dirs.sh 的 check 用途）；
# 该 lib 缺失时退化为内置默认——两者语义一致，lib 是文档表的镜像。
# _cargo_tmp_root —— 临时根（优先映射实现；lib 缺失时内联推导「仓父目录/.tmp」；MUST NOT 内嵌机器绝对路径）
_cargo_tmp_root() {
    if command -v cargo_tmp_root >/dev/null 2>&1; then cargo_tmp_root; return; fi
    if [ -n "${ALIOTH_TMP_ROOT:-}" ]; then printf '%s' "${ALIOTH_TMP_ROOT}"; return; fi
    printf '%s/.tmp' "$(dirname "${PROJECT_ROOT:-$(pwd)}")"
}

_CARGO_RUN_LIB_DIR="${BASH_SOURCE[0]%/*}"
if [ -f "${_CARGO_RUN_LIB_DIR}/cargo-target-dirs.sh" ]; then
    # shellcheck source=scripts/lib/cargo-target-dirs.sh
    source "${_CARGO_RUN_LIB_DIR}/cargo-target-dirs.sh"
fi
cargo_select_target() {
    local default_target
    default_target="$(_cargo_tmp_root)/alioth-check"
    if command -v cargo_target_dir >/dev/null 2>&1; then
        default_target="$(cargo_target_dir check)"
    fi
    local target="${CARGO_TARGET_DIR:-${default_target}}"
    if cargo_target_locked "${target}"; then
        local key sess
        key="$(cargo_session_key || true)"
        sess=""
        [ -n "${key}" ] && sess="$(_cargo_tmp_root)/alioth-check-sess-${key}"
        if [ -n "${sess}" ] && ! cargo_target_locked "${sess}"; then
            echo "[cargo-run] ⚠️  ${target} 被其他 cargo 进程占用，改用会话目录 ${sess}" >&2
            target="${sess}"
        else
            local fallback
            fallback="$(_cargo_tmp_root)/alioth-check-$(date +%s)-$$"
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

# ── manifest 根解析（平台仓 = 仓根；ns 仓 = Pre-Proc/{ns} 独立 workspace）───────
# 拆分后 ns 仓（Alioth-NS-*）**没有**仓根 Cargo.toml：其独立 workspace 是
# `Pre-Proc/{ns}/Cargo.toml`（见 build-ns.sh 头注释「日常开发：cd Pre-Proc/{ns}」）。
# 调用方统一 `cd ${PROJECT_ROOT}`，若不在 ns 仓做这一步，`cargo check -p {ns}-service-*`
# 会在仓根报 `could not find Cargo.toml`（2026-09-25 实证）。
# 平台仓有仓根 manifest ⇒ 原语义不变（不进目录）。
cargo_enter_manifest_root() {
    [ -f "${PROJECT_ROOT}/Cargo.toml" ] && return 0
    local f count=0 first=""
    for f in "${PROJECT_ROOT}"/Pre-Proc/*/Cargo.toml; do
        [ -f "${f}" ] || continue
        count=$((count + 1))
        [ -z "${first}" ] && first="${f}"
    done
    [ -n "${first}" ] || return 0
    if [ "${count}" -gt 1 ]; then
        echo "[cargo-run] ⚠️ 本仓有 ${count} 个 ns workspace，默认使用 ${first}（在 ns 多人/多 ns 布局下请显式 cd 后跑 cargo）" >&2
    fi
    cd "$(dirname "${first}")"
}

# ── 守卫执行 ────────────────────────────────────────────────────────
# run_guarded 依赖 src_fingerprint（唯一实现 = scripts/lib/src-fingerprint.sh）；调用方未 source 时自补
# （指纹根仍取调用方 export 的 PROJECT_ROOT：缺省 = 仓库根，调用方按需收窄）。
if ! declare -F src_fingerprint >/dev/null 2>&1 && [ -f "${_CARGO_RUN_LIB_DIR}/src-fingerprint.sh" ]; then
    # shellcheck source=scripts/lib/src-fingerprint.sh
    source "${_CARGO_RUN_LIB_DIR}/src-fingerprint.sh"
fi

# run_guarded <cmd...> — 通用源码漂移守卫（唯一实现；cargo 无关）
#
# 语义：命令前后各取一次源码内容指纹；不一致 ⇒ 首轮结论不可信 ⇒ **重跑一次**并打印两次结论；
#   重跑期仍在变 ⇒ 显式提示「结论仅供参照」；指纹未变 ⇒ 只跑一次（真实失败 MUST NOT 被重跑掩盖）。
# 指纹实现：scripts/lib/src-fingerprint.sh（构建输入闭包的内容流 sha256）；调用方未 source 时本文件自补。
# `set -e` 安全：命令经 `if` 形态执行——cargo-check.sh / test-all.sh 等调用方均带 errexit，
#   裸执行首轮失败会让 shell 在 `rc=$?` 之前退出 ⇒ 重跑判定永不触发（2026-09-27 修正）。
run_guarded() {
    local fp_dirs=()
    # CARGO_RUN_FP_DIRS：指纹根覆盖（空格分隔；测试与受限场景用；缺省 = 整个编译面）
    if [ -n "${CARGO_RUN_FP_DIRS:-}" ]; then
        # shellcheck disable=SC2206
        fp_dirs=(${CARGO_RUN_FP_DIRS})
    fi

    local fp_before fp_after rc=0
    if [ "${#fp_dirs[@]}" -gt 0 ]; then
        fp_before="$(src_fingerprint "${fp_dirs[@]}")"
    else
        fp_before="$(src_fingerprint)"
    fi

    if "$@"; then rc=0; else rc=$?; fi

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
        if "$@"; then rc=0; else rc=$?; fi
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

# cargo_run_guarded <check|test|build|...> [cargo args...]
# cargo 专用形态 = target 选择（三级 fallback）+ 通用守卫 run_guarded
cargo_run_guarded() {
    local sub="$1"
    shift
    cargo_gc_targets
    cargo_select_target
    cargo_ensure_source_config
    cargo_enter_manifest_root
    run_guarded cargo "${sub}" "$@"
}

# ── sccache 接线（唯一实现 = scripts/lib/sccache.sh；source 期即启用，幂等）────────
# 覆盖全部 source 本文件的入口（cargo-check / cargo-test / test-all / check-rust-tests-compile /
# build-ns / alioth-build / namespace-db / sync-db / coverage / dev-meta）：编译单元经
# RUSTC_WRAPPER 走二级缓存，跨 target 目录 / 跨 namespace / 跨 worktree 复用同一缓存项
# （SCCACHE_BASEDIR = 仓库根；判据见 scripts/check/check-sccache-wiring.ts）。
# fail-soft：sccache 缺失或显式停用时只报告，不阻断构建。
if [ -f "${_CARGO_RUN_LIB_DIR}/sccache.sh" ]; then
    # shellcheck source=scripts/lib/sccache.sh
    source "${_CARGO_RUN_LIB_DIR}/sccache.sh"
    sccache_enable_if_available "cargo-run"
fi

# cargo_ensure_arm64_fp16 —— arm64 半精度特性注入（**构建面必需品**，非仅测试）
#
# 背景（2026-09-27 实证，原 E3 环境缺口）：依赖 `gemm-f16`（`gemm-common` 的 inline asm 用
# `fmla v0.8h, ...`）在 **arm64 默认 target** 下编译报
#   `error: instruction requires: fullfp16`（8 处）⇒ **任何** cargo 构建（dev / release / test）
# 都会失败（实测：`mise run meta` 的钉扎产物构建在此中止）。修法 = 注入 `-C target-feature=+fp16`
# （rustc 1.98 的 arm64 特性名；LLVM 旧名 `fullfp16` 未收录 ⇒ 误用会被忽略并告警）。
# 仅在 CPU 支持半精度（`/proc/cpuinfo` 含 `asimdhp`）且用户未显式设 `RUSTFLAGS` 时注入；
# 不支持 FP16 的 arm64 机器**本就无法构建**本项目（gemm-f16 编译即失败）⇒ 注入不额外抬高门槛。
cargo_ensure_arm64_fp16() {
    [ -n "${ALIOTH_RT_SKIP_FP16:-}" ] && return 0
    [ -z "${RUSTFLAGS:-}" ] || return 0
    [ "$(uname -m)" = aarch64 ] || return 0
    grep -qm1 -E '( |^)asimdhp( |$)' /proc/cpuinfo 2>/dev/null || return 0
    export RUSTFLAGS="-C target-feature=+fp16"
}

# 顶层自动注入：本 lib 的**所有**消费者（dev 编排 / 发布构建 / 检查 / 测试）都需要该特性
# （arm64 上 gemm-f16 编译期即要求；不注入则构建失败）
cargo_ensure_arm64_fp16

