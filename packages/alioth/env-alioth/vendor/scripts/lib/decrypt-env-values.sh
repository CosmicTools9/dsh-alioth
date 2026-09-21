#!/usr/bin/env bash
# decrypt-env-values.sh — env 文件 enc: 值解密（CBC 体系，.encryption_key）
#
# 单一实现来源：decrypt-env.sh（mise _.source 链）与原 dev-gateway.sh / Deploy start.sh
# 的「set -a source .env」裸源缺口（2026-09-13 dmn-assist 收口实证：进程拿到 enc: 密文
# 原文 → LLM 401）。本 lib 供三处共用。
#
# 密钥解析优先级：ENCRYPTION_KEY_PATH → 项目根 .encryption_key（含 .git 目录）→ 向上最近
# .encryption_key。无密钥时保留原始值不报错（mise 链既有语义）。
#
# 用法：source 本文件后调用 `decrypt_env_file <env 文件路径>`——对文件内每个
# KEY=VALUE：enc: 前缀 AES-256-CBC 解密后 export 明文；明文原样 export（覆盖语义）。

# 密钥查找（含 .git 的项目根优先，否则向上取最近）
dev_find_encryption_key() {
    local dir="$PWD"
    local root_dir=""
    local nearest_key=""
    while [[ "$dir" != "/" ]]; do
        if [[ -f "$dir/.encryption_key" ]]; then
            nearest_key="$dir/.encryption_key"
        fi
        if [[ -d "$dir/.git" ]]; then
            root_dir="$dir"
            break
        fi
        dir="$(dirname "$dir")"
    done
    if [[ -n "$root_dir" ]] && [[ -f "$root_dir/.encryption_key" ]]; then
        echo "$root_dir/.encryption_key"
        return
    fi
    echo "$nearest_key"
}

# decrypt_env_file <env_file>：逐行处理 KEY=VALUE（enc: CBC 解密 / 明文直传）
decrypt_env_file() {
    local env_file="$1"
    [[ -f "$env_file" ]] || return 0

    local key_file="${ENCRYPTION_KEY_PATH:-}"
    if [[ -z "$key_file" ]] || [[ ! -f "$key_file" ]]; then
        key_file=$(dev_find_encryption_key)
    fi
    if [[ -z "$key_file" ]] || [[ ! -f "$key_file" ]]; then
        # 无密钥时保留原始值（enc: 密文原样），不报错
        return 0
    fi

    local key_hex
    key_hex=$(openssl enc -base64 -d -in "$key_file" | xxd -p -c 64)

    local line key value iv ciphertext plaintext
    while IFS= read -r line || [[ -n "$line" ]]; do
        [[ "$line" =~ ^[[:space:]]*# ]] && continue
        [[ -z "$line" ]] && continue
        if [[ "$line" == *=* ]]; then
            key="${line%%=*}"
            value="${line#*=}"
            value="${value#\"}"
            value="${value%\"}"
            value="${value#\'}"
            value="${value%\'}"

            if [[ "$value" =~ ^enc: ]]; then
                iv="${value#enc:}"
                iv="${iv%%:*}"
                ciphertext="${value#enc:*:}"
                # 密钥失配时两种坏产出一律拒绝导出：
                # ① padding 校验失败：openssl 非零退出但可能已吐部分明文（原 `|| true` 会照单全收）；
                # ② padding 恰好合法（~1/256）：输出二进制垃圾——下游 cargo build-script
                #   env::vars() 遇非 UTF-8 环境变量直接 panic（2026-09-16 实证：.encryption_key
                #   重建后 LLM_API_KEY 解出 32 字节垃圾，meta-backend 全量构建崩于 libm）。
                if ! plaintext=$(openssl enc -aes-256-cbc -d -a -A -nosalt -K "$key_hex" -iv "$iv" <<< "$ciphertext" 2>/dev/null); then
                    printf 'decrypt-env: %s 解密失败（密钥失配?），已跳过\n' "$key" >&2
                    continue
                fi
                if [[ -n "$plaintext" ]]; then
                    if printf '%s' "$plaintext" | iconv -f UTF-8 -t UTF-8 >/dev/null 2>&1; then
                        printf -v "$key" "%s" "$plaintext"
                        export "$key"
                    else
                        printf 'decrypt-env: %s 解密产出非 UTF-8（密钥失配?），已跳过\n' "$key" >&2
                    fi
                fi
            else
                # 明文直传（覆盖语义：后层文件/显式明文压过 enc: 密文）
                printf -v "$key" "%s" "$value"
                export "$key"
            fi
        fi
    done < "$env_file"
}
