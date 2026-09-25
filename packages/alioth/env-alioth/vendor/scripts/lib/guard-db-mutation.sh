#!/bin/bash
# DB 修改操作执行门禁
# 在 AGENTS.md 中声明为「Always」：任何 psql ALTER/INSERT/UPDATE/DELETE/DROP/TRUNCATE 前必须调用。
#
# 用法：
#   bash scripts/lib/guard-db-mutation.sh check <db_name>
#     → 检查 DB 名称是否合法（仅 dev/test 可写）
#     → 检查 backup-ddl.sh 是否已运行（备份文件存在）
#   bash scripts/lib/guard-db-mutation.sh backup <db_name>
#     → 执行 backup-ddl.sh 备份当前 schema
#
# 退出码：0=通过, 1=拒绝（输出原因到 stderr）

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

action="${1:-}"
db_name="${2:-}"

if [ -z "$action" ] || [ -z "$db_name" ]; then
  echo "Usage: $0 {check|backup} <db_name>" >&2
  exit 1
fi

# ── 目标库合法性：namespace 库由 ns 集合派生（lib/ns-db-name.sh 单一实现）──
# 为什么不是静态 case 名单：名单会随 ns 集合扩展漂移（曾只有 wz|avic_caasec，导致
# cosmic_tools/se 等库落 `Unknown database` 被误拒——执行面阻塞）。派生面不可得即中止，
# MUST NOT 回退静态名单。
if [[ ! -f "${SCRIPT_DIR}/../db/lib/ns-db-name.sh" ]]; then
    echo "❌ REFUSED: ns↔库名映射库缺失（scripts/db/lib/ns-db-name.sh）——无法判定目标库合法性（fail-closed）" >&2
    exit 1
fi
# shellcheck source=../db/lib/ns-db-name.sh
source "${SCRIPT_DIR}/../db/lib/ns-db-name.sh"

case "$db_name" in
  aliothstudio|aliothstudio_pre)
    echo "❌ REFUSED: Cannot mutate $db_name — production/pre-release DB (AGENTS.md §DB_TIER_ISOLATION)" >&2
    exit 1
    ;;
esac

if [[ "$db_name" != "aliothstudio_dev" && "$db_name" != "aliothstudio_test" ]]; then
    if ! ns_db_is_known "$db_name"; then
        echo "❌ REFUSED: Unknown database '$db_name'（非 dev/test，亦不属 ns 集合）" >&2
        exit 1
    fi
fi

BACKUP_DIR="${PROJECT_ROOT}/Backup/ad-hoc"

if [ "$action" = "backup" ]; then
  mkdir -p "$BACKUP_DIR"
  timestamp=$(date +%Y%m%d-%H%M%S)
  dump_file="${BACKUP_DIR}/aliothstudio_${db_name}-${timestamp}.dump"
  echo "📦 Backing up ${db_name} schema to ${dump_file}..."
  pg_dump -d "$db_name" --schema-only -f "$dump_file" 2>/dev/null
  echo "✅ Backup written: ${dump_file}"
  echo "${timestamp}" > "${BACKUP_DIR}/.last-backup"
  exit 0
fi

if [ "$action" = "check" ]; then
  # ── 检查方向正确性：DB 是真相源，视图/代码应适配 DB ────────────
  echo "🔍 [guard-db-mutation] Target: ${db_name}"
  echo "🔍 [guard-db-mutation] Rule: DB is schema truth source → mutate code/views to match DB, not vice versa"

  # ── 检查备份是否存在 ──────────────────────────────────────────────
  last_backup_file="${BACKUP_DIR}/.last-backup"
  if [ -f "$last_backup_file" ]; then
    last_ts=$(cat "$last_backup_file")
    echo "✅ [guard-db-mutation] Last backup: ${last_ts}"
  else
    echo "❌ REFUSED: No backup found. Run: bash $0 backup ${db_name}" >&2
    exit 1
  fi

  echo "✅ [guard-db-mutation] Gate passed"
  exit 0
fi

echo "Unknown action: $action" >&2
exit 1
