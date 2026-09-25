#!/usr/bin/env bash
# =============================================================================
# tool-probe.sh — 「已安装」判据的唯一真相源（= 可用，而非 PATH 命中）
#
# 消费方（禁止各写一份）:
#   - scripts/env/verify-env.sh          （check_lsp 的 ✓/✗ 判定）
#   - scripts/setup/install-dev-tools.sh （install_tool 的「已安装，跳过」判定）
#
# 为什么不能只看 `command -v`（2026-09-19 实证）:
#   - rustup 的 proxy shim `~/.cargo/bin/rust-analyzer` 在对应组件未安装时恒命中 PATH，
#     实跑报 `error: Unknown binary 'rust-analyzer' in official toolchain '1.98-…'`；
#     只看命中 ⇒ verify-env 判 ✓（假绿），install_tool 恒跳过 ⇒ 组件永远装不上（不收敛）。
#   - npm 包式 LSP 可能包在而运行时崩（实测 sql-language-server 报
#     `ERR_PACKAGE_PATH_NOT_EXPORTED`），`npm ls -g` 通过 ≠ 命令可用。
#
# 判据（按工具形态三选一，调用方显式声明）:
#   version —— 执行 `<cmd> --version`：rc 为 0 且输出非空
#   argv    —— `--version` 不是通用接口的工具，自定义版本参数（如 lsof -v；实测 lsof 的
#              `--version` rc=1、`-v` rc=0）⇒ 执行 `<cmd> <args…>` 判 rc 与输出
#   pkg     —— 不支持 `--version` 的 npm 包式 LSP（实测 vscode-{json,html,css}-language-server
#              的 --version/--help 均 rc=1 且打印 usage 报错）⇒ 判据退为「npm 全局包含该包」
# =============================================================================

# =============================================================================
# 判据环境对齐（唯一实现 = scripts/env/mise-sync-path.sh）——**在做出任何判定之前**
#
# 为什么放在库里：非交互 shell（hook / 门禁 / agent / 裸 `bash script.sh`）默认拿不到 mise
# 管理的工具（install dirs / shims 不在 PATH），于是 `tool_usable` 把**已装**判成**未装**
# （2026-09-22 实证：`check-env-health.sh --ci` 在裸 shell 报 12 个「未安装」，其中
# typescript-language-server / pyright / vscode-* 全在 mise 的 node 安装目录里）。
# 每个消费方各自记得对齐 = 必然漏；故对齐点收敛到本库。
# 只重算 PATH（不写文件、不装工具、不碰 DB）；幂等；CWD 原样恢复。
# =============================================================================
if command -v mise >/dev/null 2>&1; then
    _tp_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
    if [ -f "${_tp_root}/scripts/env/mise-sync-path.sh" ]; then
        _tp_pwd="${PWD}"
        cd "${_tp_root}" 2>/dev/null && { . "${_tp_root}/scripts/env/mise-sync-path.sh" || true; }
        cd "${_tp_pwd}" 2>/dev/null || true
        unset _tp_pwd
    fi
    unset _tp_root
fi

# probe_version <cmd> [args...] → 0 = rc 0 且输出非空
probe_version() {
    local out rc=0
    out="$("$@" 2>&1)" || rc=$?
    [ "$rc" -eq 0 ] && [ -n "$out" ]
}

# probe_pkg <pkg> → 0 = npm 全局包存在
probe_pkg() {
    command -v npm >/dev/null 2>&1 || return 1
    npm ls -g --depth=0 "$1" >/dev/null 2>&1
}

# tool_usable <cmd> [kind] [kind_arg]
#   kind: version（缺省）| argv（kind_arg = 版本参数串，可含空格）| pkg（此时 <cmd> 即包名，kind_arg 可省）
tool_usable() {
    local cmd="$1" kind="${2:-version}" arg="${3:-}"
    command -v "$cmd" >/dev/null 2>&1 || return 1
    case "$kind" in
        pkg)     probe_pkg "${arg:-$cmd}" ;;
        version) probe_version "$cmd" --version ;;
        # shellcheck disable=SC2086  # 有意按空格拆分 kind_arg（`argv` 形态声明多个参数）
        argv)    probe_version "$cmd" $arg ;;
        *)       return 1 ;;
    esac
}
