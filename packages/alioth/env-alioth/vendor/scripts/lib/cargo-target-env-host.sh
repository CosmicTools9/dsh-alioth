#!/usr/bin/env bash
# cargo-target-env-host.sh —— 组合根 workspace（Gateway/host）的 mise `[env] _.source` 加载面
#
# 用途固定 `host`（映射唯一事实源 = scripts/lib/cargo-target-dirs.sh），其余逻辑复用
# scripts/lib/cargo-target-env.sh（显式导出 CARGO_TARGET_DIR + sccache 接线）。
#
# 为什么单独成文件：mise 的 `[env]` 字面量在本加载面 source **之后**注入（见
# cargo-target-env.sh 头注，2026-09-16 实证）⇒ 组件无法经自己的 `[env]` 覆盖
# `CARGO_TARGET_PURPOSE`，只能由本加载面把用途钉死为 host（组合根是独立 workspace，
# 用 root workspace 的 `check` 缓存口径会与实际 workspace 不符）。
#
# 用法（Gateway/host/.mise.toml）:
#   [env]
#   _.source = ["../../scripts/lib/cargo-target-env-host.sh", "../../scripts/env/decrypt-env.sh"]

CARGO_TARGET_PURPOSE=host
# shellcheck source=scripts/lib/cargo-target-env.sh
source "${BASH_SOURCE[0]%/*}/cargo-target-env.sh"
