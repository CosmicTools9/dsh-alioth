#!/usr/bin/env bash
# =============================================================================
# zip-extract.sh — zip 解压**唯一实现**（source 面 + CLI 面）
#
# 为什么必须统一：上游 bun release 资产是 **zip**（无 .tar.gz 变体）。调用点曾写成
#     unzip -q "$asset" 2>/dev/null || tar -xf "$asset"
# 两个缺陷：
#   ① Linux 无 unzip 时回退 `tar`，而 **GNU tar 不能读 zip**（macOS 的 /usr/bin/tar 是
#      bsdtar，恰好能读）⇒「本机过、目标机挂」的结构性假绿；
#   ② `2>/dev/null` 吞掉 unzip 的真实报错，只剩 tar 的
#      `This does not look like a tar archive` ⇒ 真实原因（缺解压器）被伪装成「归档损坏」。
# 2026-09-22 实证：mise node 镜像修好后，bun 步骤即以该形态整段失败。
#
# 通道：unzip（基础包）→ bsdtar（libarchive，能读 zip）→ busybox unzip applet（零安装）→ fail-loud（给装法，不猜）。
# `unzip` 属环境工具注册表（scripts/lib/env-tools.sh）登记项 ⇒ 缺失可被检测并自愈。
#
# 用法:
#   source scripts/lib/zip-extract.sh && extract_zip <archive.zip> <dest_dir>
#   bash scripts/lib/zip-extract.sh <archive.zip> <dest_dir>
#   bash scripts/lib/zip-extract.sh --self-test
#
# 测试注入点：ZIP_EXTRACT_TOOL=unzip|bsdtar|busybox 强制单一通道（自检覆盖「目标机无 unzip」面；默认 auto）。
# =============================================================================

extract_zip() {
    local archive="$1" dest="$2" tool="${ZIP_EXTRACT_TOOL:-auto}"

    [ -f "$archive" ] || { echo "❌ extract_zip: 归档不存在: ${archive}" >&2; return 1; }
    if [ ! -d "$dest" ]; then
        mkdir -p "$dest" || { echo "❌ extract_zip: 无法创建目标目录: ${dest}" >&2; return 1; }
    fi

    if [ "$tool" = "unzip" ] || [ "$tool" = "auto" ]; then
        if command -v unzip >/dev/null 2>&1; then
            unzip -q -o "$archive" -d "$dest" && return 0
            echo "❌ extract_zip: unzip 解压失败（归档可能损坏）: ${archive}" >&2
            return 1
        fi
        if [ "$tool" = "unzip" ]; then
            echo "❌ extract_zip: ZIP_EXTRACT_TOOL=unzip 但 unzip 不可用" >&2
            return 1
        fi
    fi

    if [ "$tool" = "bsdtar" ] || [ "$tool" = "auto" ]; then
        if command -v bsdtar >/dev/null 2>&1; then
            bsdtar -xf "$archive" -C "$dest" && return 0
            echo "❌ extract_zip: bsdtar 解压失败（归档可能损坏）: ${archive}" >&2
            return 1
        fi
        if [ "$tool" = "bsdtar" ]; then
            echo "❌ extract_zip: ZIP_EXTRACT_TOOL=bsdtar 但 bsdtar 不可用" >&2
            return 1
        fi
    fi

    if [ "$tool" = "busybox" ] || [ "$tool" = "auto" ]; then
        # busybox 的 unzip applet：精简 Linux 常自带（**零安装**路径）——2026-09-22 实证：
        # 目标机 unzip/bsdtar 均缺时装 unzip 需 apt+sudo（不可用）或 brew，而 busybox 就地可用。
        # 存在性 MUST 按 applet 探测（装了 busybox ≠ 有 unzip applet）。
        if command -v busybox >/dev/null 2>&1 && busybox --list 2>/dev/null | grep -qx unzip; then
            # busybox unzip 无 -d（保持 CWD 语义）⇒ 子 shell 内切目录，不影响调用方
            (cd "$dest" && busybox unzip -o "$archive" >/dev/null 2>&1) && return 0
            echo "❌ extract_zip: busybox unzip 解压失败（归档可能损坏）: ${archive}" >&2
            return 1
        fi
        if [ "$tool" = "busybox" ]; then
            echo "❌ extract_zip: ZIP_EXTRACT_TOOL=busybox 但 busybox unzip applet 不可用" >&2
            return 1
        fi
    fi

    echo "❌ extract_zip: 无可用 zip 解压器（需 unzip / bsdtar / busybox unzip applet）: ${archive}" >&2
    echo "   装法: macOS → brew install unzip；Debian/Ubuntu → sudo apt-get install -y unzip；RHEL → sudo dnf install -y unzip" >&2
    return 1
}

# ── 自检（bash scripts/lib/zip-extract.sh --self-test）──────────────────────
# 覆盖：默认通道 / 强制单一通道（unzip 缺位时的真实故障面）/ 目标目录自动创建。
_zip_extract_self_test() {
    local tmp maker="" forced rc=0
    tmp="$(mktemp -d)" || { echo "❌ zip-extract 自检: mktemp 失败"; return 1; }
    mkdir -p "${tmp}/src/bun-linux-x64"
    printf '#!/bin/sh\necho stub\n' > "${tmp}/src/bun-linux-x64/bun"

    if command -v zip >/dev/null 2>&1; then
        (cd "${tmp}/src" && zip -qr "${tmp}/fixture.zip" .) && maker="zip"
    elif command -v bsdtar >/dev/null 2>&1; then
        (cd "${tmp}/src" && bsdtar -a -cf "${tmp}/fixture.zip" .) && maker="bsdtar"
    fi
    if [ -z "$maker" ] || [ ! -f "${tmp}/fixture.zip" ]; then
        echo "⏭️  zip-extract 自检跳过：无 zip 构造器（zip / bsdtar 皆缺）"
        rm -rf "$tmp"
        return 0
    fi

    if extract_zip "${tmp}/fixture.zip" "${tmp}/out" && [ -f "${tmp}/out/bun-linux-x64/bun" ]; then
        echo "  ✅ 默认通道解压成功（构造器=${maker}）"
    else
        echo "  ❌ 默认通道解压失败（构造器=${maker}）"
        rc=1
    fi

    for forced in unzip bsdtar busybox; do
        rm -rf "${tmp}/out-${forced}"
        if [ "$forced" = "busybox" ]; then
            if ! { command -v busybox >/dev/null 2>&1 && busybox --list 2>/dev/null | grep -qx unzip; }; then
                echo "  ⏭️  强制通道 busybox 跳过（本机无 busybox unzip applet）"
                continue
            fi
        elif ! command -v "$forced" >/dev/null 2>&1; then
            echo "  ⏭️  强制通道 ${forced} 跳过（本机无 ${forced}）"
            continue
        fi
        if ZIP_EXTRACT_TOOL="$forced" extract_zip "${tmp}/fixture.zip" "${tmp}/out-${forced}" \
            && [ -f "${tmp}/out-${forced}/bun-linux-x64/bun" ]; then
            echo "  ✅ 强制通道 ${forced} 解压成功"
        else
            echo "  ❌ 强制通道 ${forced} 解压失败"
            rc=1
        fi
    done

    rm -rf "$tmp"
    if [ "$rc" = 0 ]; then
        echo "✅ zip-extract 自检通过"
    else
        echo "❌ zip-extract 自检失败"
    fi
    return "$rc"
}

# ── CLI 面 ──────────────────────────────────────────────────────────────────
if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
    case "${1:-}" in
        --self-test)
            _zip_extract_self_test
            exit $?
            ;;
        ''|-h|--help)
            echo "用法: bash scripts/lib/zip-extract.sh <archive.zip> <dest_dir>"
            echo "      bash scripts/lib/zip-extract.sh --self-test"
            exit 0
            ;;
        *)
            extract_zip "$@"
            exit $?
            ;;
    esac
fi
