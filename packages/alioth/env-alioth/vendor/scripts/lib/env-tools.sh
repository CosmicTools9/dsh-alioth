#!/usr/bin/env bash
# =============================================================================
# env-tools.sh — 环境工具**清单与口径**的唯一真相源
#
# 与 scripts/lib/tool-probe.sh 分工：
#   tool-probe.sh = 「已安装」怎么判（可用性语义）
#   env-tools.sh  = 要判**哪些**工具、各自必需性/版本下限/安装入口/缺失提示
#
# 消费方（禁止各写一份清单）:
#   - scripts/setup/install-dev-tools.sh  安装（install_fn + 跳过判据）
#   - scripts/check/check-env-health.sh   体检（缺失/过旧）
#   - scripts/env/verify-env.sh           验证（缺失/过旧 + 版本下限）
#
# 字段（`|` 分隔，顺序固定；字段内禁止出现 `|`）:
#   family|name|cmd|probe_kind|probe_arg|critical|min_version|install_fn|hint
#     family       tool = 环境 CLI；lsp = LSP server
#     name         展示名
#     cmd          PATH 上的命令名；probe_kind=pkg 时为 npm 全局包名
#     probe_kind   version | argv | pkg（语义见 scripts/lib/tool-probe.sh）
#     probe_arg    argv 的版本参数串；pkg 的包名；version 留空
#     critical     true = 缺失即失败；false = 缺失只降级告警（**三处口径必须一致**）
#     min_version  非空时比对版本下限（major.minor 粒度）；它**不是安装源**——
#                  版本真相源仍是根 `.mise.toml` `[tools]` / rustup / brew
#     install_fn   install-dev-tools.sh 内的安装函数名；`-` = 无自动安装路径
#     hint         缺失时的安装提示
#
# 新增/调整环境工具 MUST 只改本文件（改完三处消费方自动生效）。
# =============================================================================

env_tools_registry() {
    cat <<'ENV_TOOLS_REGISTRY'
tool|Git|git|version||true|2.30|install_git|brew install git（macOS）或 apt-get install git（Linux）
tool|mise|mise|version||true|2026.9.6|install_mise|curl https://mise.jdx.dev/install.sh
tool|Node.js|node|version||true|24.21|install_nodejs|mise install（根 .mise.toml [tools] pin）或 nvm install
tool|pnpm|pnpm|version||true|12.6.0|install_pnpm|mise install pnpm（pin 见根 .mise.toml）
tool|bun|bun|version||true|1.4.2|install_bun|brew install bun
tool|Rust/Cargo|cargo|version||true|1.98|install_rust|rustup update
tool|Python 3|python3|version||true|3.14|install_python3|brew install python3（macOS）或 apt-get install python3（Linux）
tool|psql|psql|version||true|18.6|install_pg_isready|brew install postgresql@18（macOS）或 apt-get install postgresql-client（Linux）
tool|pg_isready|pg_isready|version||true||install_pg_isready|brew install postgresql（macOS）或 apt-get install postgresql-client（Linux）
tool|jq|jq|version||true||install_jq|brew install jq（macOS）或 apt-get install jq（Linux）
tool|unzip|unzip|argv|-v|true||install_unzip|brew install unzip（macOS）或 apt-get install unzip（Linux）
tool|yq|yq|version||true|4.53.3|install_yq|brew install yq（macOS）或 apt-get install yq（Linux）
tool|GNU coreutils (timeout)|timeout|version||true||install_coreutils|bash scripts/setup/install-dev-tools.sh（coreutils + 软链 ~/.local/bin）
tool|openspec|openspec|version||true|1.13.1|install_openspec|mise install npm:@fission-ai/openspec（pin 见根 .mise.toml）
tool|codegraph|codegraph|version||false|0.9.6|install_codegraph|bash scripts/codegraph/setup-codegraph.sh --install
tool|ast-grep|ast-grep|version||false||install_ast_grep|npm install -g @ast-grep/cli 或 brew install ast-grep / cargo install ast-grep
tool|lsof|lsof|argv|-v|false||install_lsof|brew install lsof（macOS）；Linux 随 util-linux/基础包
lsp|rust-analyzer|rust-analyzer|version||true||install_rust_analyzer|rustup component add rust-analyzer
lsp|TypeScript LSP|typescript-language-server|version||true||install_typescript_lsp|npm install -g typescript-language-server
lsp|Pyright|pyright|version||true||install_pyright|npm install -g pyright
lsp|marksman|marksman|version||true||install_marksman|brew install marksman（macOS）或 cargo install marksman
lsp|JSON LSP|vscode-json-language-server|pkg|vscode-langservers-extracted|true||install_json_lsp|npm install -g vscode-langservers-extracted
lsp|HTML LSP|vscode-html-language-server|pkg|vscode-langservers-extracted|true||install_html_lsp|npm install -g vscode-langservers-extracted
lsp|CSS LSP|vscode-css-language-server|pkg|vscode-langservers-extracted|true||install_css_lsp|npm install -g vscode-langservers-extracted
lsp|Bash LSP|bash-language-server|version||false||install_bash_lsp|npm install -g bash-language-server
lsp|YAML LSP|yaml-language-server|version||false||install_yaml_lsp|npm install -g yaml-language-server
lsp|Taplo (TOML)|taplo|version||false||install_taplo|brew install taplo（macOS）或 cargo install taplo-cli
lsp|Postgres LSP|postgrestools|version||false|0.25|install_postgrestools|npm install -g @postgrestools/postgrestools
ENV_TOOLS_REGISTRY
}

# env_tools_foreach <callback> [family]
#   对注册表每行调用 <callback> family name cmd probe_kind probe_arg critical min_version install_fn hint
#   回调在**当前 shell**执行（进程替换而非管道）⇒ 回调内的计数器/数组对调用方可见。
env_tools_foreach() {
    local callback="$1"
    local family_filter="${2:-}"
    local family name cmd kind arg critical min_version install_fn hint

    while IFS='|' read -r family name cmd kind arg critical min_version install_fn hint; do
        case "$family" in
            ''|'#'*) continue ;;
        esac
        if [ -n "$family_filter" ] && [ "$family" != "$family_filter" ]; then
            continue
        fi
        "$callback" "$family" "$name" "$cmd" "$kind" "$arg" "$critical" "$min_version" "$install_fn" "$hint"
    done < <(env_tools_registry)
}

# env_tools_foreach_name <callback> <cmd...>
#   只对 cmd 命中 <cmd...> 的注册表条目调用回调（按名挑单个工具用——避免第二份清单）
env_tools_foreach_name() {
    local callback="$1"
    shift
    local family name cmd kind arg critical min_version install_fn hint want

    while IFS='|' read -r family name cmd kind arg critical min_version install_fn hint; do
        case "$family" in
            ''|'#'*) continue ;;
        esac
        for want in "$@"; do
            if [ "$cmd" = "$want" ]; then
                "$callback" "$family" "$name" "$cmd" "$kind" "$arg" "$critical" "$min_version" "$install_fn" "$hint"
            fi
        done
    done < <(env_tools_registry)
}

# env_tool_ver_string <cmd> <probe_kind> <probe_arg>
#   取用于展示的版本串（pkg 形态没有 `--version`，返回包名标记）；取不到返回空串。
env_tool_ver_string() {
    local cmd="$1" kind="$2" arg="${3:-}"
    case "$kind" in
        pkg)  echo "npm 全局包 ${arg}" ;;
        # argv 形态的版本输出常以标题行开头（如 lsof 的 "lsof version information:"）
        # ⇒ 取首个带 x.y 版本号的行并去前导空白
        argv) "$cmd" $arg 2>&1 | grep -m1 -E '[0-9]+\.[0-9]+' | sed 's/^[[:space:]]*//' || true ;;
        *)    "$cmd" --version 2>&1 | head -1 || true ;;
    esac
}
