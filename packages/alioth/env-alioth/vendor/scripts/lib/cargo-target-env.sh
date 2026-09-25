#!/usr/bin/env bash
# cargo-target-env.sh —— mise `[env] _.source` 加载面：导出 CARGO_TARGET_DIR（默认用途 check）
#
# 用途取值见 scripts/lib/cargo-target-dirs.sh（check = root workspace 唯一构建缓存）。
# 目的：组件 .mise.toml 不重复目标目录字面量，且任意 cargo 调用点（含临时/未登记的命令）
# 都带显式目标目录——MUST NOT 依赖 cwd 默认推导（根 target/）。
#
# 适用面：**root workspace 成员的组件**（Framework / SSO / Gateway）。Meta/backend 与
# Deploy/{ns} 的 cwd 默认目录本就是其 canonical target（独立 workspace），不需要本兜底面。
#
# 用途覆写：经 shell 环境变量传入（`CARGO_TARGET_PURPOSE=ontology mise run ...`）。
# MUST NOT 用 .mise.toml 的 [env] 字面量设置——mise 先执行 `_.source` 再注入 [env] 字面量
# （2026-09-16 实证：Meta 的 [env] CARGO_TARGET_PURPOSE="meta" 对源脚本不可见，落到默认 check）。
#
# 用法（组件 .mise.toml）:
#   [env]
#   _.source = ["../../scripts/lib/cargo-target-env.sh", "../../scripts/env/decrypt-env.sh"]

CARGO_TARGET_PURPOSE="${CARGO_TARGET_PURPOSE:-check}"
_cargo_target_env_dir="${BASH_SOURCE[0]%/*}"
if [ -f "${_cargo_target_env_dir}/cargo-target-dirs.sh" ]; then
    # shellcheck source=scripts/lib/cargo-target-dirs.sh
    source "${_cargo_target_env_dir}/cargo-target-dirs.sh"
    if command -v cargo_target_dir >/dev/null 2>&1; then
        CARGO_TARGET_DIR="$(cargo_target_dir "${CARGO_TARGET_PURPOSE}" "${CARGO_TARGET_PURPOSE_NS:-}")"
        export CARGO_TARGET_DIR
        mkdir -p "${CARGO_TARGET_DIR}"
    fi
fi

# ── sccache 接线（唯一实现 = scripts/lib/sccache.sh；source 期即启用，幂等）────────
# 覆盖组件 `.mise.toml` 的任务面（Framework / SSO / Gateway 的 setup/dev/test/lint …）：
# 任务字符串里的 cargo 调用由此获得 RUSTC_WRAPPER + 缓存面环境。fail-soft，不阻断。
if [ -f "${_cargo_target_env_dir}/sccache.sh" ]; then
    # shellcheck source=scripts/lib/sccache.sh
    source "${_cargo_target_env_dir}/sccache.sh"
    sccache_enable_if_available "mise ${CARGO_TARGET_PURPOSE}"
fi
unset _cargo_target_env_dir
