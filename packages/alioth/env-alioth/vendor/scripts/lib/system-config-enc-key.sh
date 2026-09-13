#!/usr/bin/env bash
# ── 路径定位（不依赖 CWD） ─────────────────────────────
# 注意：本文件会被 source 进调用方 shell，禁止覆盖调用方的
# SCRIPT_DIR/PROJECT_ROOT 等通用变量——一律使用 SCEK_ 前缀。
# 布局约定：本文件位于 <root>/scripts/lib/，故 root = ../..（仓库与 Deploy/{ns} 包均满足）。
SCEK_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCEK_PROJECT_ROOT="$(cd "${SCEK_SCRIPT_DIR}/../.." && pwd)"
# system-config-enc-key.sh — SYSTEM_CONFIG_ENC_KEY 初始化（自包含）
#
# 所有 namespace 共享同一主密钥，通过统一路径存取。
# 本文件自包含，不依赖任何其他库，可被任何脚本 source。
#
# 调用约定（优先级）：
#   1. SYSTEM_CONFIG_ENC_KEY 已设置 → 跳过
#   2. SYSTEM_CONFIG_ENC_KEY_FILE 已设置 → 读该路径
#   3. 默认 → ${repo_root}/.runtime/gateway/system_config_enc_key
#
# 写入使用原子操作（umask 077 + tmp + mv），确保并发安全。
#
# 用法（在 caller 脚本中）：
#   source "${PROJECT_ROOT}/scripts/lib/system-config-enc-key.sh"
#   ensure_system_config_enc_key "$PROJECT_ROOT"

# 统一日志输出（不依赖外部函数）
sce_info() { printf '[system-config-enc-key] %s\n' "$*"; }
sce_warn() { printf '[system-config-enc-key] WARN: %s\n' "$*" >&2; }

ensure_system_config_enc_key() {
    local repo_root="$1"
    if [ -z "$repo_root" ]; then
        repo_root="$SCEK_PROJECT_ROOT"
    fi

    # 已设置 → 跳过
    if [ -n "${SYSTEM_CONFIG_ENC_KEY:-}" ]; then
        return 0
    fi

    # 自愈密钥路径（主机无关）：优先显式注入文件，否则本机默认路径。
    # 每个主机在自己的 repo_root 下生成/读取密钥，不跨机器共享、不硬编码绝对路径。
    local local_key_file="${repo_root}/.runtime/gateway/system_config_enc_key"
    local enc_key_file="${local_key_file}"
    if [ -n "${SYSTEM_CONFIG_ENC_KEY_FILE:-}" ] && [ -f "$SYSTEM_CONFIG_ENC_KEY_FILE" ]; then
        enc_key_file="$SYSTEM_CONFIG_ENC_KEY_FILE"
    fi

    # 生成 / 读取 key 文件（原子写入，仅在本机可达路径上 mkdir）
    if [ ! -f "$enc_key_file" ]; then
        if command -v openssl &>/dev/null; then
            local enc_key_dir
            enc_key_dir="$(dirname "$enc_key_file")"
            if ! mkdir -p "$enc_key_dir" 2>/dev/null; then
                # 防御：注入路径不可达（如 .env 硬编码了其它机器绝对路径）→ 回退本机路径
                sce_warn "无法创建 enc_key 目录（${enc_key_dir}），回退本机路径 ${local_key_file}"
                enc_key_file="$local_key_file"
                mkdir -p "$(dirname "$enc_key_file")" 2>/dev/null || true
            fi
            local tmp_file="${enc_key_file}.tmp.$$"
            if (umask 077 && openssl rand -base64 32 > "$tmp_file" && mv "$tmp_file" "$enc_key_file") 2>/dev/null; then
                sce_info "SYSTEM_CONFIG_ENC_KEY generated: $enc_key_file"
            else
                rm -f "$tmp_file" 2>/dev/null
                sce_warn "openssl failed, system-config credentials will not be encrypted"
                return 0
            fi
        else
            sce_warn "openssl not found, system-config credentials will not be encrypted"
            return 0
        fi
    fi

    if [ -f "$enc_key_file" ]; then
        SYSTEM_CONFIG_ENC_KEY="$(cat "$enc_key_file")"
        export SYSTEM_CONFIG_ENC_KEY
        export SYSTEM_CONFIG_ENC_KEY_FILE="$enc_key_file"
    fi
}

# ── Deploy namespace 专用版（支持注入 + namespace-local fallback）──
# 4 个 Deploy/{ns}/scripts/start.sh 使用此函数。
# 优先用 SYSTEM_CONFIG_ENC_KEY_FILE 注入（共享 key），否则本地生成。
ensure_system_config_enc_key_for_deploy() {
    local ns_runtime_dir="$1"  # $PROJECT_ROOT/.runtime/gateway
    if [ -z "$ns_runtime_dir" ]; then
        ns_runtime_dir="${SCEK_PROJECT_ROOT}/.runtime/gateway"
    fi

    if [ -n "${SYSTEM_CONFIG_ENC_KEY:-}" ]; then
        return 0
    fi

    # 优先用注入的共享 key 文件
    if [ -n "${SYSTEM_CONFIG_ENC_KEY_FILE:-}" ] && [ -f "$SYSTEM_CONFIG_ENC_KEY_FILE" ]; then
        SYSTEM_CONFIG_ENC_KEY="$(cat "$SYSTEM_CONFIG_ENC_KEY_FILE")"
        export SYSTEM_CONFIG_ENC_KEY
        return 0
    fi

    # fallback: namespace-local key（保持 Deploy/{ns}/ 可独立分发）
    local enc_key_file="${ns_runtime_dir}/system_config_enc_key"
    if [ ! -f "$enc_key_file" ]; then
        if command -v openssl &>/dev/null; then
            if ! mkdir -p "$(dirname "$enc_key_file")" 2>/dev/null; then
                sce_warn "无法创建 enc_key 目录（$(dirname "$enc_key_file")），跳过加密初始化"
                return 0
            fi
            local tmp_file="${enc_key_file}.tmp.$$"
            if (umask 077 && openssl rand -base64 32 > "$tmp_file" && mv "$tmp_file" "$enc_key_file") 2>/dev/null; then
                sce_info "SYSTEM_CONFIG_ENC_KEY generated (namespace-local): $enc_key_file"
            else
                rm -f "$tmp_file" 2>/dev/null
                sce_warn "openssl failed, namespace-local SYSTEM_CONFIG_ENC_KEY not generated"
                return 0
            fi
        else
            sce_warn "openssl not found, namespace-local SYSTEM_CONFIG_ENC_KEY not generated"
            return 0
        fi
    fi

    if [ -f "$enc_key_file" ]; then
        SYSTEM_CONFIG_ENC_KEY="$(cat "$enc_key_file")"
        export SYSTEM_CONFIG_ENC_KEY
        export SYSTEM_CONFIG_ENC_KEY_FILE="$enc_key_file"
    fi
}
