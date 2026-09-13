#!/bin/bash
# cargo 依赖源择源回退（多国内镜像）
#
# 背景：cargo 原生只支持**单一** source replacement（无 failover）；用户级
# `~/.cargo/config.toml` 把 crates-io 指向 `mirror.isahl.com`——该域名在不同网络
# 状态下会解析到不可达内网 IP（如 192.168.10.253），cargo 于是对每个 crate 反复
# 6s 超时重试（表现为构建长时间卡在 `Downloading … crates`，dev 栈起不来）。
#
# 本脚本按优先级探测候选镜像可达性，把**第一个可达源**写入项目级
# `.cargo/config.toml`（`.cargo/` 已在 .gitignore → 仅本机生效；回退 = 删除该文件）。
#
# 用法：
#   source scripts/lib/cargo-mirror-fallback.sh   # dev 脚本内引用（幂等；失败不阻断）
#   bash   scripts/lib/cargo-mirror-fallback.sh   # 手动择源（0=已选中 / 1=全部不可达）
#
# 环境开关：CARGO_MIRROR_PROBE_TIMEOUT（默认 4s）、CARGO_MIRROR_SKIP=1（跳过探测）

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

cargo_mirror_verify() {
    local root="$1" manifest="${2:-}"
    [ "${CARGO_MIRROR_SKIP_VERIFY:-0}" = "1" ] && return 0
    [ -n "$manifest" ] && [ -f "$manifest" ] || return 0
    # 以 cargo 自身验收（curl 探针不足以判定：实测 isahl 镜像 curl 200 但 cargo
    # 因本机 DNS 缓存残留解析到不可达内网 IP——只有 cargo 取源成功才算可用）
    ( cd "$root" && timeout "${CARGO_MIRROR_VERIFY_TIMEOUT:-30}" cargo fetch --quiet --manifest-path "$manifest" ) >/dev/null 2>&1
}

# 下载吞吐预检：index 探针 200 ≠ 能下载。实测某镜像**单流**可取（565KB/s）但
# cargo **并发批量**取源时停摆（`failed to transfer more than 10 bytes in 30s`）。
# 故此处按 cargo 行为建模：并发拉 N 个真实 crate 文件，全部在期限内成功才判可用。
# 阈值/超时：CARGO_MIRROR_DL_PROBES（默认 6）、CARGO_MIRROR_DL_TIMEOUT（默认 8s）。
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
    [ "${ok:-0}" -ge "${CARGO_MIRROR_DL_PROBES:-6}" ]
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
    local ordered=("${first[@]}" "${rest[@]}")
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
            {
                echo "# 由 scripts/lib/cargo-mirror-fallback.sh 生成（多镜像择源）——仅本机生效（.cargo/ 已 gitignore）。"
                echo "# 选中源：${name}（探测 ${probe} → 200）"
                echo "# 回退：删除本文件即恢复用户级 ~/.cargo/config.toml 配置。"
                echo ""
                echo "[source.crates-io]"
                echo "replace-with = \"${name}\""
                echo ""
                echo "[source.${name}]"
                echo "registry = \"${registry}\""
            } > "$cfg"
            if cargo_mirror_verify "$root" "$verify_manifest"; then
                printf '%s' "$name" > "${root}/.cargo/mirror.selected"
                echo "[cargo-mirror] 已选依赖源：${name} → ${registry}（cargo 取源实测通过）"
                return 0
            fi
            echo "[cargo-mirror] ⚠️ ${name} curl 探针 200 但 cargo 取源失败（如 DNS 指向不可达地址）——尝试下一个候选" >&2
        fi
    done
    echo "[cargo-mirror] ⚠️ 候选镜像均不可用（共 ${#CARGO_MIRROR_CANDIDATES[@]} 个；已回退用户级配置）" >&2
    rm -f "$cfg"
    return 1
}

# 直接执行 → 退出码反映择源结果；被 source → 择源失败不阻断调用方
if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
    cargo_mirror_select
    exit $?
else
    cargo_mirror_select || true
fi
