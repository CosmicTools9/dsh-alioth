#!/usr/bin/env bash
# file-digest.sh — 文件摘要/大小的可移植原语（BSD 与 GNU userland 双兼容）
#
# 为什么单独成 lib：交付链有两处消费——发布时**记录**摘要（write-release-manifest.sh）与
# 验收时**核对**摘要（verify-release-manifest.sh）。两侧 MUST 用同一算法与同一失败语义：
# 若各自降级（如失败时都写 "missing"），两边会互相「吻合」而通关，把「缺工具」伪装成
# 「一致」（fail-open，正是本原语要消除的形态）。
#
# 真机 macOS userland 的事实（2026-09-20 实测，macOS 27 / arm64）：
#   - `/usr/bin/stat` 无 `-c`（GNU 专属）⇒ 大小须先试 `-f%z`
#   - 摘要工具：GNU 侧 `md5sum`；BSD 侧 `md5 -q`（`md5sum` 仅在 coreutils gnubin 才叫此名）
#   - `sort -z` 不是 POSIX ⇒ 定序只用 `LC_ALL=C sort`
#
# 用法（source 后调用；调用方需自备 set -euo pipefail 策略）:
#   source scripts/lib/file-digest.sh
#   file_size_bytes <file>        # 字节数（文件不存在 → 0）
#   file_md5 <file>               # 32 位十六进制小写（缺工具/读失败 → 非 0 退出）
#   digest_tool_name              # 当前生效的摘要工具名（日志留痕）
#   files_md5_digest <dir>        # 目录内全部文件（递归）的确定性摘要
#
# 退出码（file_md5 / files_md5_digest）：2 路径不存在 / 3 缺摘要工具 / 4 计算失败

# file_size_bytes <file> —— 字节数；文件不存在输出 0
file_size_bytes() {
    local f="${1:-}" n=""
    if [ -f "$f" ]; then
        n="$(stat -f%z "$f" 2>/dev/null || true)"
        [ -n "$n" ] || n="$(stat -c%s "$f" 2>/dev/null || true)"
        [ -n "$n" ] || n="$(wc -c < "$f" 2>/dev/null | tr -d '[:space:]')"
    fi
    printf '%s' "${n:-0}"
}

# digest_tool_name —— 打印当前可用的摘要工具名（md5sum | md5）；都不可用返回 1
digest_tool_name() {
    if command -v md5sum >/dev/null 2>&1; then
        printf 'md5sum'
    elif command -v md5 >/dev/null 2>&1; then
        printf 'md5'
    else
        return 1
    fi
}

# _file_digest_stdin —— stdin 的摘要（内部：与 files_md5_digest 共用工具选择）
_file_digest_stdin() {
    local tool
    tool="$(digest_tool_name)" || return 3
    if [ "$tool" = "md5sum" ]; then
        md5sum | awk '{print $1}'
    else
        md5 -q
    fi
}

# file_md5 <file> —— 单个文件的 md5（十六进制小写）
file_md5() {
    local f="${1:-}" out=""
    [ -f "$f" ] || { echo "file_md5: 文件不存在：${f}" >&2; return 2; }
    local tool
    if ! tool="$(digest_tool_name)"; then
        echo "file_md5: 缺摘要工具（md5sum 或 md5）" >&2
        return 3
    fi
    if [ "$tool" = "md5sum" ]; then
        out="$(md5sum "$f" 2>/dev/null | awk '{print $1}')"
    else
        out="$(md5 -q "$f" 2>/dev/null)"
    fi
    [ -n "$out" ] || { echo "file_md5: 摘要计算失败：${f}" >&2; return 4; }
    printf '%s' "$out"
}

# files_md5_digest <dir> —— 目录内全部文件的确定性摘要。
# 算法：对每个文件取 "<md5>  <相对路径>"（路径内的换行转义为字面 \n），按字节序排序后整体再摘要。
# 遍历用 find -print0 + read -d ''（空格/换行安全的文件名）；定序用 LC_ALL=C sort（POSIX）。
files_md5_digest() {
    local dir="${1:-}" tmp sorted
    [ -d "$dir" ] || { echo "files_md5_digest: 目录不存在：${dir}" >&2; return 2; }
    digest_tool_name >/dev/null || { echo "files_md5_digest: 缺摘要工具（md5sum 或 md5）" >&2; return 3; }

    tmp="$(mktemp "${TMPDIR:-/tmp}/files-md5.XXXXXX")" || return 4
    sorted="${tmp}.sorted"
    local f rel esc h
    while IFS= read -r -d '' f; do
        rel="${f#"${dir}"/}"
        esc="${rel//$'\n'/\\n}"
        h="$(file_md5 "$f")" || { rm -f "$tmp" "$sorted"; return 4; }
        printf '%s  %s\n' "$h" "$esc" >> "$tmp"
    done < <(find "$dir" -type f -print0 2>/dev/null)

    LC_ALL=C sort "$tmp" > "$sorted" || { rm -f "$tmp" "$sorted"; return 4; }
    local digest
    digest="$(_file_digest_stdin < "$sorted")" || { rm -f "$tmp" "$sorted"; return 4; }
    rm -f "$tmp" "$sorted"
    [ -n "$digest" ] || return 4
    printf '%s' "$digest"
}
