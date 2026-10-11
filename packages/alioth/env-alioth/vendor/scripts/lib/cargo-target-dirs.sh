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
#   check      <仓父目录>/.tmp/alioth-check          root workspace 唯一构建缓存（组件任务/测试/SSO release/e2e）
#   gate       <仓父目录>/.tmp/alioth-check-gate     push 门禁（rust-tests-compile）
#   host       <仓父目录>/.tmp/alioth-host           组合根独立 workspace（Gateway/host：gateway-host 与 ns service 图）
#   meta       <root>/Meta/backend/target Meta 独立 workspace（构建/测试/lint/运行）
#   meta-dev   <仓父目录>/.tmp/alioth-meta-dev       Meta `dev`/`tui` 热重载任务
#   ontology   <仓父目录>/.tmp/alioth-ontology       ontology-mapping CLI
#   ns <NAME>  <仓父目录>/.tmp/alioth-ns-deps  ns 部署/开发（**共享依赖层**：全 ns 同目录，
#              使同源依赖图只编译一份；`CARGO_TARGET_DIR_NS` 覆写可回到 per-ns 隔离；NS 参数保留以零改调用点）
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

# cargo_tmp_root —— 临时编译产物根（**跨平台统一**；可 `ALIOTH_TMP_ROOT` 覆写）。
#
# 为什么不用 `/tmp`：macOS 的 `/tmp` 是 `/private/tmp` 且默认 tmpfs（内存-backed、容量=内存 50%），
# Linux 亦然；两平台布局/容量语义不同 ⇒ 引擎缓存落入根盘目录 `<仓父目录>/.tmp` 才稳定
# （不占内存、不受 tmpfs 上限约束、路径跨平台一致）。2026-09-28 用户裁决「临时编译路径全部转入
# ~/WorkSpace/.tmp」；本函数是其唯一实现（落点由 `cargo_target_dir` 使用）。
cargo_tmp_root() {
    if [ -n "${ALIOTH_TMP_ROOT:-}" ]; then
        printf '%s' "${ALIOTH_TMP_ROOT}"
    else
        # 仓父目录（本机 = ~/WorkSpace）下的 `.tmp`；仓移动/异机同样成立（MUST NOT 内嵌机器绝对路径）
        printf '%s/.tmp' "$(dirname "$(cargo_target_root)")"
    fi
}

# cargo_cache_shard —— 共享用途缓存的分片键（**默认关闭**：2026-10-04 用户裁决改为共享）。
#
# 沿革（两轮用户裁决，方向相反，面相同）：
#   · 2026-09-30：`check|gate|host|meta-dev|ontology` 五个用途原先全仓共用一个目录 ⇒ 多 ns 仓并发
#     构建/推送门禁在同一 target 目录上串行排队（cargo 文件锁），跨 ns 互相拖慢 ⇒ **按 ns 分片**
#     （代价 = 磁盘↑，换跨 ns 并行）。
#   · 2026-10-04：实测该面已达 **43 GB**（`alioth-check` 18 GB + `alioth-check-gate` 25 GB），且
#     每新增一个 ns 再 +10~25 GB ⇒ 用户裁决**改为共享**（磁盘优先），接受「跨 ns 构建经 cargo 锁
#     串行化」与「sccache 已吸收重复编译」的代价。**本条取代 2026-09-30 的分片口径**。
# 现状语义：默认**不分片**（共享单目录）；`ALIOTH_CACHE_SHARD=1` 显式开启按 ns 分片（需要跨 ns
# 并行的场合，如多 ns 同时跑完整门禁）；`CARGO_TARGET_DIR_*` 显式覆写仍最高优先。
# 判据 = `scripts/check/check-ns-build-target-single-source.ts`（ns 档位：共享用途输出 MUST 无 `-<ns>` 后缀；
# 置 `ALIOTH_CACHE_SHARD=1` 时 MUST 恢复后缀）。
cargo_cache_shard() {
    if [ "${ALIOTH_CACHE_SHARD:-0}" != "1" ]; then return 0; fi
    local base
    base="$(basename "$(cargo_target_root)")"
    case "${base}" in
        Alioth-NS-*) printf -- '-%s' "$(printf '%s' "${base#Alioth-NS-}" | tr '[:upper:]' '[:lower:]')" ;;
        *) ;;
    esac
}
# 注：`ns-workspace` 天然按 ns 分目录；`ns` 用途与 `check|gate|host|meta-dev|ontology` 用途
# **自 2026-10-04 起统一为共享**（前者：全 ns 共享依赖层，见 `cargo_target_dir` 的 `ns` 分支；
# 后者：默认不分片）——两轮裁决最终收敛到「磁盘优先」，代价（跨 ns 构建经 cargo 锁串行）由
# sccache（跨 target 复用编译单元）与「低频发布窗口 + stage-lock」承担。
# 需要跨 ns 并行的场合：`ALIOTH_CACHE_SHARD=1` 恢复分片（显式、留痕）。
# （原 `alioth-ns-frontend` overlay 缓存已于 2026-10-02 随 overlay 退役删除——ns 前端改由平台
#  workspace 成员供给，依赖落平台 store。）

# cargo_target_dir <purpose> [ns] —— 打印目标目录（不创建、不导出）
cargo_target_dir() {
    local purpose="${1:-}" ns="${2:-}" root shard
    root="$(cargo_target_root)"
    shard="$(cargo_cache_shard)"
    case "${purpose}" in
        check)    printf '%s' "${CARGO_TARGET_DIR_CHECK:-$(cargo_tmp_root)/alioth-check${shard}}" ;;
        gate)     printf '%s' "${CARGO_TARGET_DIR_GATE:-$(cargo_tmp_root)/alioth-check-gate${shard}}" ;;
        host)     printf '%s' "${CARGO_TARGET_DIR_HOST:-$(cargo_tmp_root)/alioth-host${shard}}" ;;
        meta)     printf '%s' "${CARGO_TARGET_DIR_META:-${root}/Meta/backend/target}" ;;
        meta-dev) printf '%s' "${CARGO_TARGET_DIR_META_DEV:-$(cargo_tmp_root)/alioth-meta-dev${shard}}" ;;
        ontology) printf '%s' "${CARGO_TARGET_DIR_ONTOLOGY:-$(cargo_tmp_root)/alioth-ontology${shard}}" ;;
        ns)
            # 共享依赖层（2026-10-04 用户裁决：重开 ADR D-026 作用域），使同源依赖图
            # （Framework/Gateway/SSO/vendor + 第三方 crate）在全 ns 间只编译一份。
            # · ns 参数保留：零改调用点，且用于覆写提示；共享层本身不随 ns 变化。
            # · 按需回到 per-ns 隔离（D-026 的隔离能力不消失，只从默认改为按需）：
            #     CARGO_TARGET_DIR_NS="<root>/Deploy/<ns>/target"
            # · 本用途**有意不参与 cargo_cache_shard**（共享即目的；分片会抵消收益）。
            if [ -z "${ns}" ]; then
                echo "cargo_target_dir: 用途 'ns' 需要 namespace 参数" >&2
                return 2
            fi
            printf '%s' "${CARGO_TARGET_DIR_NS:-$(cargo_tmp_root)/alioth-ns-deps}"
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
