#!/bin/bash
# ego-cli.sh — ego-browser CLI 代际适配（shell 侧；判据正本 = scripts/lib/ego-cli.ts 头注）
#
# 用法：
#   source scripts/lib/ego-cli.sh
#   ego_cli_argv            # → 打印需追加的 argv（stdin 代际为空串；nodejs 代际为 "nodejs"）
#   ego_cli_style           # → nodejs | stdin
#
# 例：ego-browser $(ego_cli_argv) > out 2>&1 <<'EOF' … EOF      # 注意 stdin 代际下展开为空 ⇒ 合法
#
# 判据（与 TS 侧逐条一致）：VV_EGO_CLI_STYLE 覆写 > `ego-browser --help` 文本含 `nodejs` ⇒ nodejs > 否则 stdin。
# help 文本属**纯文本叶节点**，直接按行匹配（非结构化格式解析）。

ego_cli_style() {
  if [ -n "${VV_EGO_CLI_STYLE:-}" ]; then
    printf '%s' "$VV_EGO_CLI_STYLE"
    return 0
  fi
  if command -v ego-browser >/dev/null 2>&1 && ego-browser --help 2>&1 | grep -q 'nodejs'; then
    printf 'nodejs'
  else
    printf 'stdin'
  fi
}

ego_cli_argv() {
  [ "$(ego_cli_style)" = "nodejs" ] && printf 'nodejs' || printf ''
}
