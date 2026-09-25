#!/usr/bin/env bash
# cargo-target-dirs.sh — cargo 目标目录唯一映射（用途 → 绝对路径）
#
# 为什么单独成 lib：每个 cargo 调用点各自拼目标目录 ⇒ 口径漂移（`.cargo/config.toml` 的
# `target-dir` grep 恒回退根 `target/`）、且 cwd 推导的落点不在命令行上（外部探活工具不可见）。
# 本文件是「用途 → 目录」的**唯一事实源**；`docs/specs/COMPILATION_GUIDE.md`
# §Target 目录竞争隔离 的表与其同步。
#
# 用法（调用方需先设 PROJECT_ROOT，或当前目录为仓库根）:
#   source scripts/lib/cargo-target-dirs.sh
#   export CARGO_TARGET_DIR="$(cargo_target_dir check)"
# 无法 source bash 的上下文（mise run 字符串、TS 工具）用 CLI 面:
#   env CARGO_TARGET_DIR="$(bash scripts/cargo-target.sh check)" cargo build …
#
# 用途表（覆写变量名同步）:
#   check      /tmp/alioth-check          root workspace 唯一构建缓存（组件任务/测试/SSO release/e2e）
#   gate       /tmp/alioth-check-gate     push 门禁（rust-tests-compile）
#   host       /tmp/alioth-host           组合根独立 workspace（Gateway/host：gateway-host 与 ns service 图）
#   meta       <root>/Meta/backend/target Meta 独立 workspace（构建/测试/lint/运行）
#   meta-dev   /tmp/alioth-meta-dev       Meta `dev`/`tui` 热重载任务
#   ontology   /tmp/alioth-ontology       ontology-mapping CLI
#   ns <NAME>  <root>/Deploy/<NS>/target  namespace 部署/开发（NS 经 ns_canon 归一）
#   ns-workspace <NAME>
#              <root>/Pre-Proc/<NS>/target namespace service 独立 workspace（own Cargo.lock）
#
# 说明：`ns` / `host` / `ns-workspace` 是**三个不同 workspace**的槽位——
#   · `ns`          = Gateway/OpenActivity 以 ns feature 编译的**部署/运行**缓存
#                     （组合根 workspace `Gateway/host`，见 scripts/build-ns.sh）
#   · `host`        = 组合根 workspace 的**组件检查/测试**缓存（`Gateway/host` 独立 workspace，
#                     非 ns 定向：组件任务、push 编译门禁、test-all 阶段）
#   · `ns-workspace`= `Pre-Proc/{ns}` 自身 workspace 的编译缓存
#                     （docs/specs/COMPILATION_GUIDE.md §从 namespace workspace 编译）

# shellcheck source=scripts/lib/ns-name.sh
source "${BASH_SOURCE[0]%/*}/ns-name.sh"

# cargo_target_root —— 仓库根（绝对路径）。PROJECT_ROOT 优先（**归一为绝对**——Meta/backend
# 的 [env] 把 PROJECT_ROOT 设为相对值 "../.."，直接拼接会产出随 cwd 变化的相对 target 目录）；
# 未设时由本文件位置推导。
cargo_target_root() {
    if [ -n "${PROJECT_ROOT:-}" ]; then
        (cd "${PROJECT_ROOT}" && pwd)
    else
        (cd "${BASH_SOURCE[0]%/*}/../.." && pwd)
    fi
}

# cargo_target_dir <purpose> [ns] —— 打印目标目录（不创建、不导出）
cargo_target_dir() {
    local purpose="${1:-}" ns="${2:-}" root
    root="$(cargo_target_root)"
    case "${purpose}" in
        check)    printf '%s' "${CARGO_TARGET_DIR_CHECK:-/tmp/alioth-check}" ;;
        gate)     printf '%s' "${CARGO_TARGET_DIR_GATE:-/tmp/alioth-check-gate}" ;;
        host)     printf '%s' "${CARGO_TARGET_DIR_HOST:-/tmp/alioth-host}" ;;
        meta)     printf '%s' "${CARGO_TARGET_DIR_META:-${root}/Meta/backend/target}" ;;
        meta-dev) printf '%s' "${CARGO_TARGET_DIR_META_DEV:-/tmp/alioth-meta-dev}" ;;
        ontology) printf '%s' "${CARGO_TARGET_DIR_ONTOLOGY:-/tmp/alioth-ontology}" ;;
        ns)
            if [ -z "${ns}" ]; then
                echo "cargo_target_dir: 用途 'ns' 需要 namespace 参数" >&2
                return 2
            fi
            printf '%s' "${root}/Deploy/$(ns_canon "${ns}")/target"
            ;;
        ns-workspace)
            if [ -z "${ns}" ]; then
                echo "cargo_target_dir: 用途 'ns-workspace' 需要 namespace 参数" >&2
                return 2
            fi
            printf '%s' "${root}/Pre-Proc/$(ns_canon "${ns}")/target"
            ;;
        *)
            echo "cargo_target_dir: 未知用途 '${purpose}'（可用: check|gate|host|meta|meta-dev|ontology|ns|ns-workspace）" >&2
            return 2
            ;;
    esac
}
