//! 列级 schema 单源 — `Framework/backend/dict/schema/value-domain-columns.tsv`
//!
//! 生成器 = `scripts/gen-value-domain-schema.ts`（源 = 模型元数据 `isahl_meta.meta_fields`
//! ∩ 实表列；随模型版本刷新，与 `crud/fk-index.tsv` 同族处置）；门禁 =
//! `scripts/check/check-value-domain-schema.ts`（重跑逐字节比对，漂移即红）。
//!
//! 消费面（`/dict/tables` payload、写径可写列校验）MUST 只读本模块——
//! 禁第二份列名 / 列中文名硬编码（规约 `value-domain-office :: value-domain-column-schema`）。
//! 产物经 include_str! 编译期绑定：坏行 / 缺头 **fail-loud**（panic），不静默降级。

/// 编译期绑定的列级 schema 产物（format=1；见模块头）。
const ARTIFACT: &str = include_str!("../schema/value-domain-columns.tsv");

/// 单列元数据（产物行投影）。
#[derive(Debug, Clone, Copy)]
pub struct ColumnMeta<'a> {
    /// 物理列名（= `meta_fields.name`）
    pub name: &'a str,
    /// 模型元数据 data_type（`meta_fields.data_type`）
    pub data_type: &'a str,
    /// 列中文名（`meta_fields.title`；模型未填标题 ⇒ 空串）
    pub title: &'a str,
    /// 是否可编辑（族可编辑集内）
    pub editable: bool,
}

/// 单表 schema（列按产物 (table, column) 码元序）。
#[derive(Debug)]
pub struct TableSchema<'a> {
    /// 表名（`isahl` schema 下）
    pub name: &'a str,
    /// 全列（含不可编辑列）
    pub columns: Vec<ColumnMeta<'a>>,
    /// 可编辑列名集（写径校验面）
    pub editable: Vec<&'a str>,
}

/// 解析后的列级 schema。
#[derive(Debug)]
pub struct Schema<'a> {
    /// 产物绑定的模型版本（`# model_version: <v>`）
    pub model_version: &'a str,
    /// 表集（产物行序；(table, column) 码元序分组）
    pub tables: Vec<TableSchema<'a>>,
}

/// 解析产物。
///
/// 坏行 fail-loud（panic）：头缺失 / model_version 缺失 / 字段数 ≠ 5 / editable 非布尔 / 列名为空。
/// 生成器已保证行序；此处只校验形状与取值合法，不重排。
pub fn parse_artifact(text: &str) -> Schema<'_> {
    let mut lines = text.lines();
    let header = lines.next().unwrap_or_default();
    if !header.starts_with("# value-domain-columns") {
        panic!(
            "value-domain-columns 产物头缺失（首行 = {header:?}）；重跑 scripts/gen-value-domain-schema.ts 刷新"
        );
    }
    let version_line = lines.next().unwrap_or_default();
    let model_version = version_line
        .strip_prefix("# model_version: ")
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            panic!("value-domain-columns 产物缺 model_version（行 2 = {version_line:?}）")
        });

    let mut tables: Vec<TableSchema<'_>> = Vec::new();
    for (idx, line) in lines.enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cell: Vec<&str> = line.split('\t').collect();
        let row_no = idx + 3; // 1-based 行号（含两行头）
        if cell.len() != 5 {
            panic!(
                "value-domain-columns 产物坏行（行 {row_no}）：字段数 {} ≠ 5 — {line:?}",
                cell.len()
            );
        }
        let editable = match cell[4] {
            "true" => true,
            "false" => false,
            other => panic!(
                "value-domain-columns 产物坏行（行 {row_no}）：editable = {other:?}（须 true/false）"
            ),
        };
        if cell[1].is_empty() {
            panic!("value-domain-columns 产物坏行（行 {row_no}）：列名为空 — {line:?}");
        }
        let meta = ColumnMeta {
            name: cell[1],
            data_type: cell[2],
            title: cell[3],
            editable,
        };
        if tables.last().map(|t| t.name) != Some(cell[0]) {
            tables.push(TableSchema {
                name: cell[0],
                columns: Vec::new(),
                editable: Vec::new(),
            });
        }
        let table = tables.last_mut().expect("just pushed");
        if editable {
            table.editable.push(cell[1]);
        }
        table.columns.push(meta);
    }
    Schema {
        model_version,
        tables,
    }
}

static SCHEMA: std::sync::OnceLock<Schema<'static>> = std::sync::OnceLock::new();

/// 编译期产物（懒解析一次；坏行 panic）。
fn schema() -> &'static Schema<'static> {
    SCHEMA.get_or_init(|| parse_artifact(ARTIFACT))
}

/// 产物绑定的模型版本（`/dict/tables` 顶层 schemaVersion）。
pub fn model_version() -> &'static str {
    schema().model_version
}

/// 表列集；表不在产物（值域家族外）⇒ None。
pub fn table_columns(table: &str) -> Option<&'static [ColumnMeta<'static>]> {
    schema()
        .tables
        .iter()
        .find(|t| t.name == table)
        .map(|t| t.columns.as_slice())
}

/// 表可编辑列集；表不在产物 ⇒ None（写径据此 fail-loud，见 handler）。
pub fn editable_columns(table: &str) -> Option<&'static [&'static str]> {
    schema()
        .tables
        .iter()
        .find(|t| t.name == table)
        .map(|t| t.editable.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ok_groups_tables_and_editable() {
        let text = "# value-domain-columns  format=1\n# model_version: v1.2.3\nzc_id_cate-x\tnotice\ttext\t名称\ttrue\nzc_id_cate-x\tcode\ttext\t编码\tfalse\nzc_id_tags\thits\tbigint\t\tfalse\n";
        let s = parse_artifact(text);
        assert_eq!(s.model_version, "v1.2.3");
        assert_eq!(s.tables.len(), 2);
        assert_eq!(s.tables[0].name, "zc_id_cate-x");
        assert_eq!(s.tables[0].columns.len(), 2);
        assert_eq!(s.tables[0].editable, vec!["notice"]);
        assert_eq!(s.tables[0].columns[0].title, "名称");
        assert_eq!(s.tables[1].columns[0].title, "");
        assert!(!s.tables[1].columns[0].editable);
        assert!(s.tables[1].editable.is_empty());
    }

    #[test]
    #[should_panic(expected = "字段数")]
    fn parse_rejects_bad_field_count() {
        parse_artifact(
            "# value-domain-columns  format=1\n# model_version: v1\nzc_id_cate-x\tnotice\n",
        );
    }

    #[test]
    #[should_panic(expected = "editable")]
    fn parse_rejects_bad_editable() {
        parse_artifact(
            "# value-domain-columns  format=1\n# model_version: v1\nzc_id_cate-x\tnotice\ttext\t\tyes\n",
        );
    }

    #[test]
    #[should_panic(expected = "model_version")]
    fn parse_rejects_missing_version() {
        parse_artifact("# value-domain-columns  format=1\nzc_id_cate-x\tnotice\ttext\t\ttrue\n");
    }

    #[test]
    fn embedded_artifact_is_parseable_and_covers_families() {
        let version = model_version();
        assert!(
            version.starts_with('v'),
            "model_version 应为 v-prefixed，实得 {version:?}"
        );
        let cols = table_columns("zc_id_cate-warehouse").expect("cate-warehouse 应在产物内");
        assert!(cols.iter().any(|c| c.name == "notice" && c.editable));
        assert!(cols.iter().any(|c| c.name == "id" && !c.editable));
        let editable = editable_columns("zc_id_cate-warehouse").expect("cate-warehouse 在产物内");
        assert!(editable.contains(&"notice"));
        assert!(!editable.contains(&"id"));
        assert!(editable_columns("no-such-table").is_none());
    }
}
