#!/bin/bash
# cargo-check.sh — 在独立 target 目录中执行 cargo check（不抢部署构建/门禁的锁）
#
# 特性（实现内聚在 scripts/lib/cargo-run.sh）:
#   - target 默认 /tmp/alioth-check（rust-clean 保留的热缓存之一）；被活 cargo 进程占用时
#     自动改用会话稳定目录 /tmp/alioth-check-sess-<hash>，仍占用则退为带时间戳的冷目录
#   - 启动时机会式 GC：清理超龄 sess/fallback/gate 临时目录（占锁跳过）
#   - **源码漂移守卫**：编译前后比对源码内容指纹；不一致（编译读到变化中的树，共享
#     checkout 下他人 checkout/stash 会致「符号找不到」类瞬时假错）⇒ 自动重跑一次并打印两次结论
#   - 可用 CARGO_TARGET_DIR 覆盖
#
# 用法:
#   bash scripts/cargo-check.sh -p wz-service-isahl-db
#   bash scripts/cargo-check.sh -p wz-service-isahl-db --all-features
#   bash scripts/cargo-check.sh --workspace
#
# 聚焦测试用同守卫的 scripts/cargo-test.sh（`cargo test` 形态）。

set -euo pipefail
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PROJECT_ROOT

# shellcheck source=scripts/lib/src-fingerprint.sh
source "${PROJECT_ROOT}/scripts/lib/src-fingerprint.sh"
# shellcheck source=scripts/lib/cargo-run.sh
source "${PROJECT_ROOT}/scripts/lib/cargo-run.sh"

cd "${PROJECT_ROOT}"
cargo_run_guarded check "$@"
