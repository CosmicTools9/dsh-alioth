#!/bin/bash
# generate-dicts.sh — 从 AliothStudio 真库导出校验快照三件套（fk_index 模式：
# 静态字典由 DB 派生，随代码分发）。覆盖:
#   skill-alioth/src/data/coordinates.json   (scene/factor/function 坐标码)
#   skill-alioth/src/data/fk-index.json      (物理引用索引 [table, field, target, local_key])
#   skill-alioth/src/data/physical-tables.json (isahl 物理表 [table, parent] + 根族公共列)
# 用法: bash scripts/generate-dicts.sh [DATABASE_URL]
set -euo pipefail
DB_URL="${1:-postgres://isahl@localhost/aliothstudio_dev}"
DATA_DIR="packages/alioth/skill-alioth/src/data"
mkdir -p "$DATA_DIR"

echo "== coordinates.json =="
psql "$DB_URL" -t -A -c "
SELECT json_build_object(
  '\$schema', 'https://dsh-alioth.local/schemas/coordinates-dict.json',
  'description', 'Alioth coordinate dictionaries (isahl.zc_id_scene/factor/function codes). Exported from the AliothStudio dev database; regenerate with this script.',
  'scene', (SELECT json_agg(code ORDER BY code) FROM isahl.zc_id_scene WHERE deleted_at IS NULL),
  'factor', (SELECT json_agg(code ORDER BY code) FROM isahl.zc_id_factor WHERE deleted_at IS NULL),
  'function', (SELECT json_agg(code ORDER BY code) FROM isahl.zc_id_function WHERE deleted_at IS NULL)
)" > "$DATA_DIR/coordinates.json"

echo "== fk-index.json =="
psql "$DB_URL" -t -A -c "
SELECT json_build_object(
  '\$schema', 'https://dsh-alioth.local/schemas/fk-index.json',
  'description', 'Physical FK reference index [table, field, target, local_key] derived from isahl_meta.meta_fields reference_config (AliothStudio generate-fk-index.ts pattern).',
  'refs', (SELECT json_agg(json_build_array(fk_collection, name, config->'reference_config'->>'target_table', config->'reference_config'->>'local_key') ORDER BY fk_collection, name)
           FROM isahl_meta.meta_fields
           WHERE config ? 'reference_config'
             AND config->'reference_config'->>'local_key' IS NOT NULL
             AND config->'reference_config'->>'local_key' != '')
)" > "$DATA_DIR/fk-index.json"

echo "== physical-tables.json =="
psql "$DB_URL" -t -A -c "
SELECT json_build_object(
  '\$schema', 'https://dsh-alioth.local/schemas/physical-tables.json',
  'description', 'isahl physical table index [table, parent] + root-family common columns, exported from the AliothStudio dev DB (fk_index pattern).',
  'root_columns', (SELECT json_agg(a.attname ORDER BY a.attname)
                   FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
                   JOIN pg_attribute a ON a.attrelid=c.oid
                   WHERE n.nspname='isahl' AND c.relkind='r' AND a.attnum>0 AND NOT a.attisdropped
                     AND NOT EXISTS (SELECT 1 FROM pg_inherits i WHERE i.inhrelid=c.oid)),
  'tables', (SELECT json_agg(json_build_array(c.relname, COALESCE(p.relname,'')) ORDER BY c.relname)
             FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
             LEFT JOIN pg_inherits i ON i.inhrelid=c.oid
             LEFT JOIN pg_class p ON p.oid=i.inhparent
             WHERE n.nspname='isahl' AND c.relkind='r')
)" > "$DATA_DIR/physical-tables.json"

python3 - "$DATA_DIR" <<'EOF'
import json, os, sys
data = sys.argv[1]
total = 0
for name in ("coordinates.json", "fk-index.json", "physical-tables.json"):
    d = json.load(open(os.path.join(data, name)))
    size = os.path.getsize(os.path.join(data, name))
    print(f"  {name}: {size} bytes")
EOF
echo "done"
