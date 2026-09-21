#!/bin/bash
# cargo 依赖源择源回退（多国内镜像）
#
# 背景：cargo 原生只支持**单一** source replacement（无 failover）；用户级
# `~/.cargo/config.toml` 把 crates-io 指向 `mirror.isahl.com`——该域名在不同网络
# 状态下会解析到不可达内网 IP（如 192.168.10.253），cargo 于是对每个 crate 反复
# 6s 超时重试（表现为构建长时间卡在 `Downloading … crates`，dev 栈起不来）。
#
# 本脚本按优先级探测候选镜像可达性，并把**经 cargo 实测取源通过**的第一个候选写入项目级
# `.cargo/config.toml`（`.cargo/` 已在 .gitignore → 仅本机生效；回退 = 删除该文件）。
# ⚠️ 可达性以 **cargo 自身**为准，curl 探针只是快筛：本机实测 curl 可达 LAN 镜像而 rustup 的
# cargo 被即拒（`Couldn't connect … after 0 ms`，二进制级放行差异）⇒ 仅凭 curl 会误选不可用源。
# 故候选先写临时文件、用 `cargo --config <tmp>` 验收，**通过才顶替**真实配置；全候选失败时
# **保留既有配置**（绝不先毁后写），只有从未有过项目配置时才落回用户级。
#
# 用法：
#   source scripts/lib/cargo-mirror-fallback.sh   # dev 脚本内引用（幂等；失败不阻断）
#   bash   scripts/lib/cargo-mirror-fallback.sh   # 手动择源（0=已选中 / 1=全部不可达）
#
# 环境开关：CARGO_MIRROR_PROBE_TIMEOUT（默认 4s）、CARGO_MIRROR_SKIP=1（跳过探测）、
#   CARGO_MIRROR_SKIP_VERIFY=1（跳过 cargo 验收）、CARGO_MIRROR_VERIFY_TIMEOUT（默认 180s/工作区）、
#   CARGO_MIRROR_VERIFY_MANIFEST（默认 SSO/backend/Cargo.toml）、
#   CARGO_MIRROR_VERIFY_MANIFEST_EXTRA（默认 Meta/backend/Cargo.toml）、
#   CARGO_MIRROR_DL_MIN_OK（默认 1）、CARGO_MIRROR_DL_TIMEOUT（默认 8s）

# 候选顺序 = 优先级：① 之前成功的内网/官方域名（可达时最快，实测 0.07s）→
# ② 国内公网镜像（实测：aliyun 0.04s / rsproxy 0.13s / ustc 0.15s / tuna 0.76s；
#    tencent crates.io-index 路径 404 无稀疏索引 → 不纳入）。
# 语义：按顺序探测，取第一个可达源；不可达即自动落到下一个。
#
# ⚠️ isahl 条目 MUST 沿用用户级 `~/.cargo/config.toml` 的源名 `mirror`：
# cargo 不允许「同一注册表 URL 被两个不同源名定义」（实测报
# `source 'x' defines source registry 'y', but that source is already defined`）——
# 同名 + 同 URL 才被视为同一源，可被项目级正常覆盖。
CARGO_MIRROR_CANDIDATES=(
    "mirror|sparse+http://mirror.isahl.com:6464/crates/|http://mirror.isahl.com:6464/crates/config.json"
    "aliyun-sparse|sparse+https://mirrors.aliyun.com/crates.io-index/|https://mirrors.aliyun.com/crates.io-index/config.json"
    "rsproxy-sparse|sparse+https://rsproxy.cn/index/|https://rsproxy.cn/index/config.json"
    "ustc-sparse|sparse+https://mirrors.ustc.edu.cn/crates.io-index/|https://mirrors.ustc.edu.cn/crates.io-index/config.json"
    "tuna-sparse|sparse+https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/|https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/config.json"
)

# GNU `timeout` 是**外部依赖**（唯一供给方 = scripts/setup/install-dev-tools.sh 的
# `install_coreutils`：macOS 装 coreutils 并把 `timeout` 软链到 ~/.local/bin，Linux 属基础包）。
# 脚本/子 shell 内不存在时直呼 `timeout` 会以 127 立即返回、**cargo 从不执行** ⇒ 验收把
# **所有**候选误判为「取源失败」（2026-09-20 实证：cargo 垫片记录为空 = cargo 从未执行；
# `timeout` 只作为 OMP bash 工具的 builtin 存在，三处系统二进制路径皆无）。故此处显式告警
# 并跳过验收，绝不静默误判；缺失由 scripts/env/verify-env.sh 健康检查与安装器收敛。
cargo_mirror_verify() {
    local root="$1" manifest="${2:-}" candidate_cfg="${3:-}"
    [ "${CARGO_MIRROR_SKIP_VERIFY:-0}" = "1" ] && return 0
    [ -n "$manifest" ] && [ -f "$manifest" ] || return 0
    if ! command -v timeout >/dev/null 2>&1; then
        echo "[cargo-mirror] ⚠️ 缺 GNU timeout ⇒ 跳过 cargo 取源验收（安装：bash scripts/setup/install-dev-tools.sh）" >&2
        return 0
    fi
    # 以 cargo 自身验收（curl 探针不足以判定：实测 isahl 镜像 curl 200 但 cargo
    # 因本机 DNS 缓存残留解析到不可达内网 IP——只有 cargo 取源成功才算可用）。
    # 预算 180s/工作区：实测冷缓存取源 SSO/backend 需 151s，原定 30s 会把**可用源**判死。
    # `candidate_cfg` 存在时用 `cargo --config <file>` **在不触碰真实配置**的前提下验收
    # ——CLI `--config` 优先级最高，故候选源覆盖项目/用户级 `replace-with`。
    # **两个工作区都须通过**：只验 SSO 会放过「能服务 SSO 但缺 Meta 依赖」的源——实测
    # `Meta/backend` 需要 `candle-core`，某镜像缺该 crate 时 SSO 验收通过、Meta 编译失败
    # （2026-09-20 push 门禁即由此失败），故默认追加 `CARGO_MIRROR_VERIFY_MANIFEST_EXTRA`。
    local use_cfg=() m
    [ -n "$candidate_cfg" ] && [ -f "$candidate_cfg" ] && use_cfg=(--config "$candidate_cfg")
    for m in "$manifest" "${CARGO_MIRROR_VERIFY_MANIFEST_EXTRA:-${root}/Meta/backend/Cargo.toml}"; do
        [ -f "$m" ] || continue
        ( cd "$root" && timeout "${CARGO_MIRROR_VERIFY_TIMEOUT:-180}" cargo ${use_cfg[@]+"${use_cfg[@]}"} fetch --quiet --manifest-path "$m" ) >/dev/null 2>&1 || return 1
    done
    return 0
}

# 下载吞吐预检：index 探针 200 ≠ 能下载。实测某镜像**单流**可取（565KB/s）但
# cargo **并发批量**取源时停摆（`failed to transfer more than 10 bytes in 30s`）。
# 故此处按 cargo 行为建模：并发拉 N 个真实 crate 文件做**快筛**——至少一个成功即放行，交给
# 下方 cargo 实测验收（权威判据）；全失败才跳过该候选、省掉 verify 预算。
# 阈值/超时：CARGO_MIRROR_DL_MIN_OK（默认 1）、CARGO_MIRROR_DL_TIMEOUT（默认 8s）。
cargo_mirror_throughput_ok() {
    local registry="$1" dl base tmpd ok i
    base="${registry#sparse+}"
    base="${base%/}"
    dl="$(curl -s -m 5 "${base}/config.json" 2>/dev/null | jq -r '.dl // empty' 2>/dev/null)"
    [ -n "$dl" ] || return 1
    local probes=(serde/1.0.0 libc/0.2.0 quote/1.0.0 proc-macro2/1.0.0 unicode-ident/1.0.0 itoa/1.0.0)
    tmpd="$(mktemp -d)"
    for i in "${!probes[@]}"; do
        (
            if curl -s -o /dev/null -m "${CARGO_MIRROR_DL_TIMEOUT:-8}" "${dl%/}/${probes[$i]}/download" 2>/dev/null; then
                : > "$tmpd/$i.ok"
            fi
        ) &
    done
    wait
    ok="$(ls "$tmpd" 2>/dev/null | wc -l | tr -d ' ')"
    rm -rf "$tmpd"
    # 判据：**至少一个**探针下载成功 ⇒ 该源能下载 ⇒ 交给下方 cargo 实测验收（权威判据）。
    # 要求 N/N 全成会在链路抖动时（6 路并发 + 8s 预算）把**可用源**误判为不可用——2026-09-20 实测：
    # ustc/aliyun/tuna 索引均 200、手工 `cargo fetch` 成功，探针却因个别超时被整体跳过，
    # 择源遂报「候选镜像均不可用」。全失败（ok=0）才是真的不可达，直接跳过省掉 verify 预算。
    [ "${ok:-0}" -ge "${CARGO_MIRROR_DL_MIN_OK:-1}" ]
}

cargo_mirror_select() {
    local root="${PROJECT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
    local cfg="${root}/.cargo/config.toml"
    local tmo="${CARGO_MIRROR_PROBE_TIMEOUT:-4}"
    local verify_manifest="${CARGO_MIRROR_VERIFY_MANIFEST:-${root}/SSO/backend/Cargo.toml}"
    [ "${CARGO_MIRROR_SKIP:-0}" = "1" ] && return 0
    # 上次成功源优先（粘性）：稳态下首个候选即命中，启动近秒级；
    # 该源失效时其 cargo 验收会失败 → 自动回到声明的优先级顺序。
    local prefer="" entry name registry probe code first=() rest=()
    [ -f "${root}/.cargo/mirror.selected" ] && prefer="$(cat "${root}/.cargo/mirror.selected" 2>/dev/null)"
    for entry in "${CARGO_MIRROR_CANDIDATES[@]}"; do
        if [ -n "$prefer" ] && [ "${entry%%|*}" = "$prefer" ]; then
            first+=("$entry")
        else
            rest+=("$entry")
        fi
    done
    # bash 3.2 + set -u：空数组的 "${arr[@]}" 会报 unbound variable（无粘性选择时
    # first 为空即触发）——用 ${arr[@]+"${arr[@]}"} 惯用法兜住。
    local ordered=("${first[@]+"${first[@]}"}" "${rest[@]+"${rest[@]}"}")
    for entry in "${ordered[@]}"; do
        name="${entry%%|*}"
        registry="$(printf '%s' "$entry" | cut -d'|' -f2)"
        probe="${entry##*|}"
        code="$(curl -s -o /dev/null -m "$tmo" -w '%{http_code}' "$probe" 2>/dev/null)"
        if [ "$code" = "200" ]; then
            # 吞吐预检（快失败）：index 可探 ≠ 能批量下载（实测停摆于 30s 内 0 字节）
            if ! cargo_mirror_throughput_ok "$registry"; then
                echo "[cargo-mirror] ⚠️ ${name} index 探针 200 但下载吞吐不足（<${CARGO_MIRROR_MIN_KBPS:-50}KB/s 或超时）——跳过，尝试下一个候选" >&2
                continue
            fi
            mkdir -p "${root}/.cargo"
            local cand="${root}/.cargo/.config.candidate.$$"
            # 候选写**临时文件**（本文件会被其他会话/工具的 cargo 并发读取；非原子写入会让读者
            # 拿到半截 TOML ⇒ cargo 报 `could not find a configured source …` 而秒级失败）
            {
                echo "# 由 scripts/lib/cargo-mirror-fallback.sh 生成（多镜像择源）——仅本机生效（.cargo/ 已 gitignore）。"
                echo "# 选中源：${name}（探测 ${probe} → 200；经 cargo --config 实测取源通过后落盘）"
                echo "# 回退：删除本文件即恢复用户级 ~/.cargo/config.toml 配置。"
                echo ""
                echo "[source.crates-io]"
                echo "replace-with = \"${name}\""
                echo ""
                echo "[source.${name}]"
                echo "registry = \"${registry}\""
            } > "$cand"
            # **验收通过才顶替真实配置**。原流程「先写真实文件再验收」有两个后果：
            # ① 候选失败时顶掉了**既有可用配置**；② 全候选失败时 `rm -f "$cfg"` 删除配置并
            # 回退到用户级——本机用户级同样指向不可达镜像（探针用 curl 判可达、真正取源的是
            # cargo，两者放行面不同）⇒ 之后任何依赖下载都会挂起。改用 `cargo --config <cand>`
            # 验收：不触碰真实配置，成功才 `mv`（原子）落盘。
            if cargo_mirror_verify "$root" "$verify_manifest" "$cand"; then
                mv -f "$cand" "$cfg"
                printf '%s' "$name" > "${root}/.cargo/mirror.selected"
                echo "[cargo-mirror] 已选依赖源：${name} → ${registry}（cargo 取源实测通过）"
                return 0
            fi
            rm -f "$cand"
            echo "[cargo-mirror] ⚠️ ${name} curl 探针 200 但 cargo 取源失败（curl 可达 ≠ cargo 可达）——尝试下一个候选" >&2
        fi
    done
    if [ -f "$cfg" ]; then
        echo "[cargo-mirror] ⚠️ 候选镜像均不可用（共 ${#CARGO_MIRROR_CANDIDATES[@]} 个）——**保留既有项目配置，未改动**" >&2
    else
        echo "[cargo-mirror] ⚠️ 候选镜像均不可用（共 ${#CARGO_MIRROR_CANDIDATES[@]} 个；无既有项目配置 ⇒ 将落到用户级 ~/.cargo/config.toml）" >&2
    fi
    return 1
}

# 直接执行 → 退出码反映择源结果；被 source → 择源失败不阻断调用方
if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
    cargo_mirror_select
    exit $?
else
    cargo_mirror_select || true
fi
