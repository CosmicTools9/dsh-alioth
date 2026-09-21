#!/usr/bin/env bash
# =============================================================================
# artifact-trim.sh — 交付面二进制体积收敛原语（openspec change reduce-build-artifact-size）
#
# 为什么存在：交付面二进制此前未剥符号，实测白占 23–25%（Cosmic-Tools 143→107MB、
# meta-backend 75→58MB）；且剥符号会令既有签名失效。本原语把三步固定为**一个不可拆的
# 顺序**，调用方只调它，避免"剥了忘重签 → 内核杀进程（provenance/签名失效）"这类事故：
#
#   ① strip（默认档：去符号，保留 __eh_frame 等展开表）
#   ② xattr -d com.apple.provenance（Sequoia+ 跨卷 cp 语义，未清则可能被内核杀）
#   ③ codesign --force --deep --sign -（ad-hoc 重签，紧跟剥除之后）
#
# 未剥符号的构建输出（`{ns}/target/release/`、`Meta/backend/target/release/`）MUST 保持不变
# ——事故定位以它为准（`atos -o <unstripped> -l <loadaddr>`）。
#
# 用法（source 后调用单函数）：
#   source scripts/lib/artifact-trim.sh
#   trim_release_binary "$TARGET_DIR/$BINARY_NAME"
# 退出码：0 成功；2 缺参数；3 文件不存在
# =============================================================================

# 跨平台文件大小/摘要原语（唯一实现 = scripts/lib/file-digest.sh）
# shellcheck source=scripts/lib/file-digest.sh
source "${BASH_SOURCE[0]%/*}/file-digest.sh"

trim_release_binary() {
  local bin="${1:-}"
  if [ -z "$bin" ]; then
    echo "trim_release_binary: 缺少二进制路径参数" >&2
    return 2
  fi
  if [ ! -f "$bin" ]; then
    echo "trim_release_binary: 文件不存在：$bin" >&2
    return 3
  fi

  local before after
  before=$(file_size_bytes "$bin")

  # ① strip：跨平台可用（macOS strip / GNU binutils strip）
  # 签名失效警告属预期（紧随其后重签）；其他提示上报
  local strip_out
  strip_out=$(strip "$bin" 2>&1) || true
  case "$strip_out" in
    "" | *"invalidate the code signature"*) ;;
    *) echo "   ⚠ artifact-trim: strip 提示：${strip_out}" >&2 ;;
  esac

  # ②③ provenance xattr + ad-hoc 重签：仅 macOS 有这两个语义
  # （Linux 打包不执行；否则 codesign 不存在 → 每次打"签名校验未通过"的假噪音）
  if [ "$(uname -s)" = "Darwin" ]; then
    xattr -d com.apple.provenance "$bin" 2>/dev/null || true
    codesign --force --deep --sign - "$bin" 2>/dev/null || true
  fi

  after=$(file_size_bytes "$bin")
  local pct
  pct=$(awk -v b="$before" -v a="$after" 'BEGIN { printf "%.1f", (b > 0) ? (b - a) * 100.0 / b : 0 }')
  printf "   Trim: %sMB → %sMB（-%s%%）\n" \
    "$((before / 1024 / 1024))" "$((after / 1024 / 1024))" "$pct"

  if [ "$(uname -s)" = "Darwin" ]; then
    if codesign --verify --strict "$bin" >/dev/null 2>&1; then
      printf "   签名: 有效（ad-hoc）\n"
    else
      printf "   ⚠ 签名校验未通过：%s\n" "$bin" >&2
    fi
  fi

  return 0
}
