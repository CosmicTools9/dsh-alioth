#!/usr/bin/env bash
# portable-fs.sh — 路径/权限位的可移植原语（BSD 与 GNU userland 双兼容）
#
# 为什么单独成 lib（2026-09-27 实测，macOS 27 / arm64 vs Linux coreutils）：
#   · BSD `/usr/bin/realpath` **无 `-m`**（`-m` 是 GNU 专属：允许路径不存在并做词法归一）
#     ⇒ 直接 `realpath -m <不存在路径>` 在 macOS 上以 `realpath: illegal option -- m` 退出 1；
#       在 `set -e` 脚本里等于**整条命令链中止**（实测：scripts/runtime/control-token.sh init
#       在 macOS 上 rc=1、载体永不生成 ⇒ 16 项 test 红）。
#   · BSD `/usr/bin/stat` **无 `-c`**（GNU 专属）⇒ 权限位须退化到 `stat -f '%Lp'`。
#   字节数/摘要请用 `file-digest.sh`（`file_size_bytes` / `file_sha256` / `file_sha256_stdin`）——
#   同族原语已在那里实现，MUST NOT 在此重复。
#
# 机器判据：`bun scripts/check/check-shell-portability.ts` 判据 3（GNU-only 工具写法）；
#   本文件与 `file-digest.sh` 是**原语实现面**，在判据 3 的豁免清单内（降级分支必须写出 GNU 形态）。
#
# 用法（source 后调用）:
#   source scripts/lib/portable-fs.sh
#   pfs_abs_path <path>   # 物理化绝对路径（展开符号链接）；路径**可以不存在**（`realpath -m` 语义）
#   pfs_mode <path>       # 八进制权限位（如 `600`、`700`）；路径不存在/不可读 → 非 0 退出、无输出
#
# 退出码：pfs_abs_path 恒 0（最坏回退为原样拼接的绝对路径）；pfs_mode 1 = 不可判定。

# pfs_abs_path <path> —— 物理化绝对路径（GNU `realpath -m` 语义的 BSD 退化实现）
pfs_abs_path() {
    local p="${1:-}" out="" dir base
    if [ -z "$p" ]; then
        printf '%s' "$PWD"
        return 0
    fi
    # ① GNU：`-m` 允许不存在路径并做词法归一
    if out="$(realpath -m "$p" 2>/dev/null)"; then
        printf '%s' "$out"
        return 0
    fi
    # ② BSD 退化：父目录 physical 化后拼接 ⇒ 已存在路径的符号链接同样被展开
    dir="$(dirname -- "$p")"
    base="$(basename -- "$p")"
    if [ -d "$dir" ]; then
        (cd -- "$dir" && printf '%s/%s' "$(pwd -P)" "$base")
        return 0
    fi
    # ③ 父目录不存在：已绝对者原样（词法归一不是守卫判据所需）；相对者挂到 PWD 下
    case "$p" in
        /*) printf '%s' "$p" ;;
        *) printf '%s/%s' "$(pwd -P)" "$p" ;;
    esac
}

# pfs_mode <path> —— 八进制权限位（GNU `stat -c '%a'` / BSD `stat -f '%Lp'`）
pfs_mode() {
    local p="${1:-}" m=""
    [ -n "$p" ] && [ -e "$p" ] || return 1
    m="$(stat -c '%a' "$p" 2>/dev/null || true)"
    [ -n "$m" ] || m="$(stat -f '%Lp' "$p" 2>/dev/null || true)"
    [ -n "$m" ] || return 1
    printf '%s' "$m"
}
