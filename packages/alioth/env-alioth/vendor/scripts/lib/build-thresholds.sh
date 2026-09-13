#!/bin/bash
# build-thresholds.sh — 巨单体构建触发线检测（仅二进制大小）
#
# 背景：meta-backend 为巨单体（2026-09-03 基线：255MB debug 二进制、
# 17.7MB __eh_frame、lld 下重链 76s）。拆分方案（Cargo feature 功能域裁剪）
# 的启动触发线：二进制 > 500MB。本文件把触发线固化为硬失败门禁，跨过即
# fail 并提示拆分。
#
# 增量重链时间线已移除（2026-09-03 用户裁决）：构建耗时受 rebase/依赖变更
# 影响波动过大（523s 超时即为误报），不构成可靠的拆分信号。
#
# 用法（source 后调用）：
#   source scripts/lib/build-thresholds.sh
#   <构建命令>
#   check_binary_size_threshold "$BIN"

THRESHOLD_SIZE_BYTES=$((500 * 1024 * 1024))   # 500MB

check_binary_size_threshold() {
  local bin="$1"
  [ -f "$bin" ] || return 0

  local size
  size=$(stat -f%z "$bin" 2>/dev/null || stat -c%s "$bin")

  if [ "$size" -gt "$THRESHOLD_SIZE_BYTES" ]; then
    echo "" >&2
    echo "❌ 构建触发线已跨过：二进制大小超限" >&2
    echo "   产物: $bin" >&2
    echo "   大小: $((size / 1024 / 1024))MB > 500MB" >&2
    echo "   处置: 巨单体需拆分——按 Cargo feature 功能域裁剪方案执行" >&2
    exit 1
  fi
}
