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

# ── 仅允许在 dev/test 库上执行 mutation ────────────────────────────
case "$db_name" in
  aliothstudio_dev|aliothstudio_test|wz|avic_caasec)
    ;;
  aliothstudio|aliothstudio_pre)
    echo "❌ REFUSED: Cannot mutate $db_name — production/pre-release DB (AGENTS.md §DB_TIER_ISOLATION)" >&2
    exit 1
    ;;
  *)
    echo "❌ REFUSED: Unknown database '$db_name'" >&2
    exit 1
    ;;
esac

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
