#!/usr/bin/env bash
# repo-role.sh — 「本仓是哪一类仓」与「本 ns 库 DSN」的**唯一实现**（shell 侧）
#
# 为什么需要：拆分后同一套门禁在两类仓运行（平台仓 `AliothMeta` / namespace 仓 `Alioth-NS-*`），
# 而 DB 判定面必须随仓角色分流——ns 仓的 DB 类门禁若沿用平台共享 dev 库缺省值，判定面就是**别的仓的
# 库**（错误判定，劣于诚实 skip）。此处把「角色判据」「ns 名」「本 ns DSN」三件事收敛到单一实现，
# 供 db-url-resolve.sh / db-ready.sh / ns-hook.sh 与 TS 侧（`scripts/lib/repo-role.ts` 经本文件
# 委托）共用；MUST NOT 在消费方各自复制缺省值。
#
# 判据（与 `scripts/pre/repo-profile.ts::NS_REPO_MARKER` 同锚点）：仓根存在 `.alioth/platform.conf`
# ⇒ ns 仓；否则平台仓。
#
# 用法（source 后调用）：
#   repo_role_root [dir]        # 仓根（git toplevel；非 git 仓 ⇒ rc=1）。缺省 $PWD
#   repo_path_physical <path>   # 路径物理化（符号链接被解析；不要求 git 仓）。缺省 $PWD
#   repo_root_physical [dir]    # 仓根的物理形态（= repo_role_root → repo_path_physical）
#   repo_role <root>        # platform | ns
#   repo_ns_name <root>     # ns 名（ns 仓；无法唯一判定 ⇒ rc=1）
#   repo_ns_db_url <root>   # 本 ns dev 层 DSN（`<root>/Deploy/<ns>/.env` 的 DATABASE_URL；缺失 ⇒ rc=1）
#
# 契约：
#   · ns 名判定 MUST 唯一（`Pre-Proc/` 下唯一真实目录，回退 `Deploy/` 下含 `.env` 的唯一目录）；
#     多候选或零候选 ⇒ rc=1（MUST NOT 猜）；
#   · `repo_ns_db_url` MUST NOT 回落其他层级 env（dev 层 = `Deploy/<ns>/.env`，层语义唯一实现 =
#     `scripts/lib/ns-env-file.sh`）；文件缺失或键缺失 ⇒ rc=1，MUST NOT 返回平台共享库缺省值。

# 判据常量（ns 仓标记）——与 TS 侧 `NS_REPO_MARKER` 同值
REPO_ROLE_NS_MARKER=".alioth/platform.conf"

# repo_role_root [dir] —— 仓根（git toplevel）
repo_role_root() {
    local dir="${1:-$PWD}"
    git -C "$dir" rev-parse --show-toplevel 2>/dev/null || return 1
}

# repo_path_physical <path> —— 路径物理化（符号链接被解析；**不要求**是 git 仓）
repo_path_physical() {
    local p="${1:-$PWD}"
    (cd "$p" 2>/dev/null && pwd -P) || return 1
}

# repo_root_physical [dir] —— 仓根的**物理**形态（`repo_role_root` → `repo_path_physical`）
#
# 为什么单独一支：`git -C <path> rev-parse --show-toplevel` 走 `getcwd()`，符号链接被解析；
# 纯字符串前缀比较会被符号链接骗过（误判「在仓内」）。ns 仓经符号链接**消费**平台面
# （`scripts`/`Framework`/…）、平台仓经 `Pre-Proc/{ns}` 看到 ns 内容 ⇒ 一切「按路径写入/格式化」
# 的前置判定 MUST 用物理形态（原实现散在 `scripts/parallel/repo-scope.sh`，2026-10-08 收敛至此）。
repo_root_physical() {
    local root
    root="$(repo_role_root "${1:-$PWD}")" || return 1
    repo_path_physical "$root"
}

# repo_role <root> —— platform | ns
repo_role() {
    local root="${1:-}"
    [[ -n "$root" ]] || return 1
    if [[ -f "${root%/}/${REPO_ROLE_NS_MARKER}" ]]; then
        printf 'ns\n'
    else
        printf 'platform\n'
    fi
}

# repo_ns_name <root> —— ns 名（唯一判定）
repo_ns_name() {
    local root="${1:-}" name="" cands=() d
    [[ -n "$root" ]] || return 1
    root="${root%/}"

    if [[ -d "${root}/Pre-Proc" ]]; then
        for d in "${root}/Pre-Proc"/*/; do
            [[ -d "$d" ]] || continue
            name="$(basename "$d")"
            [[ "$name" == .* ]] && continue
            cands+=("$name")
        done
    fi
    if [[ ${#cands[@]} -ne 1 && -d "${root}/Deploy" ]]; then
        local deploy_cands=()
        for d in "${root}/Deploy"/*/; do
            [[ -d "$d" ]] || continue
            [[ -f "${d}.env" ]] || continue
            deploy_cands+=("$(basename "$d")")
        done
        [[ ${#deploy_cands[@]} -eq 1 ]] && cands=("${deploy_cands[0]}")
    fi
    [[ ${#cands[@]} -eq 1 ]] || return 1
    printf '%s\n' "${cands[0]}"
}

# repo_ns_db_url <root> —— 本 ns dev 层 DSN（读 `<root>/Deploy/<ns>/.env` 的 DATABASE_URL）
repo_ns_db_url() {
    local root="${1:-}" ns env_url line
    [[ -n "$root" ]] || return 1
    root="${root%/}"
    ns="$(repo_ns_name "$root")" || return 1
    local env_file="${root}/Deploy/${ns}/.env"
    [[ -f "$env_file" ]] || return 1
    while IFS= read -r line; do
        case "$line" in
            DATABASE_URL=*)
                env_url="${line#DATABASE_URL=}"
                ;;
        esac
    done <"$env_file"
    [[ -n "$env_url" ]] || return 1
    # 去包裹引号（env 文件两种书写形态都存在）
    env_url="${env_url%\"}"
    env_url="${env_url#\"}"
    env_url="${env_url%\'}"
    env_url="${env_url#\'}"
    case "$env_url" in
        postgres*) printf '%s\n' "$env_url" ;;
        *) return 1 ;;
    esac
}

# 自检：角色判据 + ns 名唯一性 + DSN 读取（合成夹具，无外部依赖）
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    set -uo pipefail
    rc=0
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT

    # 平台仓形态（无 .alioth/platform.conf）
    mkdir -p "${tmp}/platform/Pre-Proc"
    git -C "$tmp/platform" init -q 2>/dev/null
    [[ "$(repo_role "$tmp/platform")" == platform ]] || { echo "自检失败: 平台仓角色"; rc=1; }

    # ns 仓形态
    mkdir -p "${tmp}/ns-a/Pre-Proc/Alioth" "${tmp}/ns-a/Deploy/Alioth" "${tmp}/ns-a/.alioth"
    : >"${tmp}/ns-a/.alioth/platform.conf"
    printf 'DATABASE_URL=postgres://isahl@localhost:5432/alioth\n' >"${tmp}/ns-a/Deploy/Alioth/.env"
    [[ "$(repo_role "$tmp/ns-a")" == ns ]] || { echo "自检失败: ns 仓角色"; rc=1; }
    [[ "$(repo_ns_name "$tmp/ns-a")" == Alioth ]] || { echo "自检失败: ns 名"; rc=1; }
    [[ "$(repo_ns_db_url "$tmp/ns-a")" == "postgres://isahl@localhost:5432/alioth" ]] || { echo "自检失败: ns DSN"; rc=1; }

    # 多候选 ⇒ 拒绝（不得猜）
    mkdir -p "${tmp}/ns-b/Pre-Proc/Alioth" "${tmp}/ns-b/Pre-Proc/WZ" "${tmp}/ns-b/.alioth"
    : >"${tmp}/ns-b/.alioth/platform.conf"
    if repo_ns_name "$tmp/ns-b" >/dev/null 2>&1; then echo "自检失败: 多候选未被拒绝"; rc=1; fi

    # 缺 env ⇒ 不得返回缺省
    mkdir -p "${tmp}/ns-c/Pre-Proc/SE" "${tmp}/ns-c/.alioth"
    : >"${tmp}/ns-c/.alioth/platform.conf"
    if repo_ns_db_url "$tmp/ns-c" >/dev/null 2>&1; then echo "自检失败: 缺 env 未拒绝"; rc=1; fi

    [[ $rc -eq 0 ]] && echo "✓ repo-role.sh 自检通过"
    exit $rc
fi
