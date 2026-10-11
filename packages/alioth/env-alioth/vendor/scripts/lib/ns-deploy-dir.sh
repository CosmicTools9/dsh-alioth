#!/usr/bin/env bash
# ns-deploy-dir.sh —— 平台仓 `Deploy/{ns}` 的归属解析（单一实现）
#
# 拆分后（2026-09-24 `Alioth-NS-*` 迁出）**`Deploy/{ns}` 归 ns 仓**（Apps/apps.json/seed/.env/
# bin/target 全在其内），而平台的构建与启动脚本一律以 `${PROJECT_ROOT}/Deploy/{ns}` 为落点
# （`build-ns.sh` 的 `--target-dir`、`dev-gateway.sh`、`namespace-ports.sh` 的 `.env` 扫描）。
# 平台仓若无该目录，构建/启动会在仓内**另建一份空的** `Deploy/{ns}`：apps.json / Apps 缺位、
# `.env` 缺位 ⇒ 启动即失配（2026-09-25 实证：宿主树被清理后平台仓无法起 WZ dev）。
#
# 契约：
#   ① `Deploy/{ns}` 已存在（实体目录或符号链接）⇒ 不动（幂等）；
#   ② 不存在、且同级 `../Alioth-NS-<ns>/Deploy/<ns>` 存在 ⇒ 建立**相对符号链接**消费 ns 仓正本；
#   ③ 两者皆无（平台内置 ns / 无对应 ns 仓布局）⇒ 回落为仓内实体目录。
#
# 用法（在 `PROJECT_ROOT` 已导出的入口脚本内）：
#   source scripts/lib/ns-deploy-dir.sh
#   ns_deploy_dir_ensure WZ
ns_deploy_dir_ensure() {
    local ns="${1:-}"
    [ -n "${ns}" ] || { echo "[ns-deploy] 缺 namespace 参数" >&2; return 2; }
    local root="${PROJECT_ROOT:?PROJECT_ROOT 未设置（调用方需先 export）}"
    local target="${root}/Deploy/${ns}"
    [ -e "${target}" ] && return 0
    local repo="${root}/../Alioth-NS-${ns}"
    if [ -d "${repo}/Deploy/${ns}" ]; then
        ln -sfn "../../$(basename "${repo}")/Deploy/${ns}" "${target}"
        echo "[ns-deploy] Deploy/${ns} → $(readlink "${target}")（消费 ns 仓正本；单一事实源 = ns 仓）" >&2
        return 0
    fi
    mkdir -p "${target}"
    echo "[ns-deploy] Deploy/${ns} 无同名 ns 仓，回落为仓内实体目录" >&2
    return 0
}

# ── 该 ns 的落点（日志 / 运行态）──────────────────────────────────────────────
# 契约：namespace 的一切**运行态产物**（开发日志、PID、启动锁）随 `Deploy/{ns}` 落在 ns 侧，
# 与「Deploy/{ns} 归 ns 仓」同一条归属规则（2026-09-25 用户裁定：
# 「NS-Log 都归属各个 NS 的路径」）。MUST NOT 落到平台仓根下的 `logs/dev` / `.runtime/gateway`——
# 那会让同一 ns 的源码在本仓、日志/PID 在平台仓，跨仓排障与清理都失真。
#
# ns_deploy_dir_path <ns>            → Deploy/{ns} 绝对路径（按需建立，见 ns_deploy_dir_ensure）
# ns_log_dir <ns> [<sub>]            → Deploy/{ns}/logs[/<sub>]（开发日志根；<sub> 供 OA 等子面）
# ns_runtime_dir <ns>                → Deploy/{ns}/.runtime（PID / 启动锁）
ns_deploy_dir_path() {
    local ns="${1:-}"
    [ -n "${ns}" ] || { echo "[ns-deploy] 缺 namespace 参数" >&2; return 2; }
    ns_deploy_dir_ensure "${ns}" >/dev/null
    printf '%s/Deploy/%s\n' "${PROJECT_ROOT:?PROJECT_ROOT 未设置（调用方需先 export）}" "${ns}"
}

ns_log_dir() {
    local d
    d="$(ns_deploy_dir_path "${1:-}")/logs" || return $?
    [ -n "${2:-}" ] && d="${d}/${2}"
    printf '%s\n' "${d}"
}

ns_runtime_dir() {
    printf '%s/.runtime\n' "$(ns_deploy_dir_path "${1:-}")"
}
