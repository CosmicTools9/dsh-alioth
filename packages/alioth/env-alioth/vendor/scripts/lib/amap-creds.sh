#!/usr/bin/env bash
# amap-creds.sh — AMAP key/secret 的 **DB 承载读取**（构建期注入 process env）
#
# 规约正本（change `move-env-secrets-to-db` D1 + `openspec/specs/amap-env-config`）：
#   承载 = `isahl."zc_id_prot-env_config"` 行（`code = 'integration:amap'`，`settings` 承载
#   `key`/`secret`；**按环境隔离** = 各环境各自库里的行）。构建脚本读 DB → 注入 process env →
#   vite 消费（`Gateway/frontend/vite.config.ts` 保留 env 入参通道，源码零硬编码兜底）。
#   与 `approval:auto-approve` 同族同形（`Framework/backend/common/src/platform_config.rs` 的运行期读取器）。
#
# 明文承载的理由：AMAP key/secret 是**注入浏览器**的 JSAPI 值（`window.__ALIOTH_AMAP_KEY__/SECRET__`，
# 见 `Framework/frontend/components/src/map/loadAMap.ts` 的 JSAPI 类型约束）——对最终用户公开可见，
# 落 `settings`；`enc_fields`（AES-256-GCM + `SYSTEM_CONFIG_ENC_KEY`）留给真正的服务端机密。
#
# 契约（fail-soft，构建 MUST NOT 因此失败）：
#   · 已显式注入（CI/离线构建入参）→ 原值优先，不读 DB；
#   · DB 不可达 / psql 缺失 / 行缺失或软删 / 字段为空 → 不设置变量并返回 0
#     （前端渲染「配置 Key 后显示地图」占位，见 `ClickableMap`）;
#   · 幂等，可重复调用；不打印凭据值（只打印「有/无」与来源库掩码）。
#
# 用法：
#   source "${PROJECT_ROOT}/scripts/lib/amap-creds.sh"
#   load_amap_creds_from_db [db_url] [ns]
#     db_url 省略 → $ALIOTH_AMAP_DB_URL → $DATABASE_URL → `db-url-resolve.sh`（组件 .mise.toml / mise env）
#     ns     省略 → 不按 namespace 解析（DB URL 三级链之外无第四级回退——MUST NOT 猜库）

# 配置行 code（`zc_id_prot-env_config.code`）；`settings` 键名固定 key/secret。
AMAP_CREDS_CODE="integration:amap"

# ── 路径定位（source 型；AMC_ 前缀避免覆盖调用方变量） ─────
amap_creds_lib_dir() { cd "$(dirname "${BASH_SOURCE[0]}")" && pwd; }

# amap_creds_log <msg> —— 免依赖日志（调用方可能有自己的 log）
amap_creds_log() { printf '[amap-creds] %s\n' "$*" >&2; }

# amap_creds_mask_db <url> —— 掩码连接串里的口令（日志安全）
amap_creds_mask_db() { printf '%s' "$1" | sed -E 's#(//[^:/@]*):[^@/]*@#\1:***@#'; }

# amap_creds_resolve_db_url [db_url] [ns] —— DB URL 解析（不猜库：解析不到就空）
amap_creds_resolve_db_url() {
    local explicit="${1:-}" ns="${2:-}" lib_dir url
    if [ -n "$explicit" ]; then printf '%s' "$explicit"; return 0; fi
    if [ -n "${ALIOTH_AMAP_DB_URL:-}" ]; then printf '%s' "${ALIOTH_AMAP_DB_URL}"; return 0; fi
    if [ -n "${DATABASE_URL:-}" ]; then printf '%s' "${DATABASE_URL}"; return 0; fi
    lib_dir="$(amap_creds_lib_dir)"
    if [ -f "${lib_dir}/db-url-resolve.sh" ]; then
        # shellcheck source=scripts/lib/db-url-resolve.sh
        source "${lib_dir}/db-url-resolve.sh"
        url="$(db_url_resolve 2>/dev/null || true)"
        if [ -n "$url" ]; then printf '%s' "$url"; return 0; fi
    fi
    [ -n "$ns" ] && amap_creds_log "未解析到 ${ns} 的 DB URL（DATABASE_URL / .mise.toml 均无）——跳过 DB 取值"
    return 0
}

# load_amap_creds_from_db [db_url] [ns]
# 读 DB 行 → export ALIOTH_AMAP_KEY / ALIOTH_AMAP_SECRET（未取到 → 保持未设置/空值）。
load_amap_creds_from_db() {
    local explicit_url="${1:-}" ns="${2:-}" url out key secret

    # 显式注入优先（迁移期、CI/离线构建、单测桩）
    if [ -n "${ALIOTH_AMAP_KEY:-}" ] && [ -n "${ALIOTH_AMAP_SECRET:-}" ]; then
        return 0
    fi
    command -v psql >/dev/null 2>&1 || { amap_creds_log "psql 不可用——跳过 DB 取值（前端注入空值）"; return 0; }

    url="$(amap_creds_resolve_db_url "$explicit_url" "$ns")"
    if [ -z "$url" ]; then
        return 0
    fi

    # 只读单行查询（fail-soft：任何 psql 失败 → 空值，MUST NOT 中断构建）
    out="$(PGCONNECT_TIMEOUT=5 psql "$url" -X -v ON_ERROR_STOP=1 -tA -F "$(printf '\037')" -c "
        SELECT COALESCE(settings->>'key', ''), COALESCE(settings->>'secret', '')
          FROM isahl.\"zc_id_prot-env_config\"
         WHERE code = '${AMAP_CREDS_CODE}' AND deleted_at IS NULL
         ORDER BY id DESC
         LIMIT 1" 2>/dev/null)" || out=""

    if [ -z "$out" ]; then
        amap_creds_log "库 $(amap_creds_mask_db "$url") 无 '${AMAP_CREDS_CODE}' 配置行——注入空值（地图显示占位提示）"
        return 0
    fi
    key="${out%%"$(printf '\037')"*}"
    secret="${out#*"$(printf '\037')"}"

    if [ -z "$key" ] && [ -z "$secret" ]; then
        amap_creds_log "'${AMAP_CREDS_CODE}' 行的 settings.key/secret 均为空——注入空值（地图显示占位提示）"
        return 0
    fi
    [ -n "$key" ] && export ALIOTH_AMAP_KEY="$key"
    [ -n "$secret" ] && export ALIOTH_AMAP_SECRET="$secret"
    amap_creds_log "已从库 $(amap_creds_mask_db "$url") 注入 AMAP 凭据（key ${#key}B / secret ${#secret}B）"
    return 0
}

# 自检：掩码（无 DB 依赖）。`bash scripts/lib/amap-creds.sh --self-test`
if [ "${1:-}" = "--self-test" ]; then
    rc=0
    masked="$(amap_creds_mask_db 'postgres://user:secret@localhost:5432/db')"
    if [ "$masked" != 'postgres://user:***@localhost:5432/db' ]; then
        printf 'FAIL 掩码：%s\n' "$masked" >&2; rc=1
    fi
    unmasked="$(amap_creds_mask_db 'postgres://isahl@localhost:5432/db')"
    if [ "$unmasked" != 'postgres://isahl@localhost:5432/db' ]; then
        printf 'FAIL 无口令 URL 不应改动：%s\n' "$unmasked" >&2; rc=1
    fi
    [ "$rc" -eq 0 ] && printf 'OK amap-creds 自检通过\n'
    exit "$rc"
fi
