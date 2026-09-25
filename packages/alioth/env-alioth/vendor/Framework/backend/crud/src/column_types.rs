//! 列类型元数据解析 — information_schema + 进程级缓存
//!
//! 供过滤器（QueryBuilder 按列类型分派 SQL cast）和写入绑定
//! （bind_json 按列类型强转值）共享，避免各自查一次 information_schema。
//! 表结构变更后重启进程即可刷新缓存。

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use sqlx::{PgPool, Row};

/// 列元数据：`information_schema.columns` 的 `data_type` + USER-DEFINED 列的 UDT 名。
///
/// `udt` 仅在 `data_type = 'USER-DEFINED'`（枚举 / geometry 等）时有值——写径据此
/// 给占位符补 `::"udt"` 强转（文本参数直接写枚举列会报
/// `column "x" is of type status_flag but expression is of type text`）。
#[derive(Debug, Clone)]
pub struct ColumnMeta {
    pub data_type: String,
    pub udt: Option<String>,
}

impl ColumnMeta {
    /// 列类型的 SQL 拼写（供既有按类型分派的调用方使用）
    pub fn data_type(&self) -> &str {
        &self.data_type
    }

    /// USER-DEFINED 列的类型名（枚举/geometry）；其余列 `None`
    pub fn udt(&self) -> Option<&str> {
        self.udt.as_deref()
    }
}

/// (schema, table) → (column → 列元数据) 进程级缓存
pub type ColumnTypeMap = HashMap<String, ColumnMeta>;
type ColumnTypeCache = HashMap<(String, String), Arc<ColumnTypeMap>>;

static COLUMN_TYPE_CACHE: LazyLock<Mutex<ColumnTypeCache>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn column_type_cache() -> &'static Mutex<ColumnTypeCache> {
    &COLUMN_TYPE_CACHE
}

/// SQL 绑定占位符。USER-DEFINED 列（枚举/geometry）附 `::"udt"` 强转——
/// 文本参数直接写枚举列会报 `column "x" is of type status_flag but expression is of type text`。
pub fn placeholder(idx: usize, meta: Option<&ColumnMeta>) -> String {
    match meta.and_then(ColumnMeta::udt) {
        Some(udt) => format!("${}::\"{}\"", idx, udt),
        None => format!("${}", idx),
    }
}

/// 解析 `AliothDbEntity::table_name()`（如 `"isahl"."zc_id_contract"` / `isahl.zc_id_process`）
/// 为 (schema, table) 二元组；无 schema 前缀时默认 `isahl`。
pub fn split_table_name(raw: &str) -> (String, String) {
    let cleaned: String = raw.chars().filter(|c| *c != '"').collect();
    match cleaned.split_once('.') {
        Some((schema, table)) => (schema.to_string(), table.to_string()),
        None => ("isahl".to_string(), cleaned),
    }
}

/// 解析表的列 → data_type 映射（带进程级缓存，返回 `Arc` 共享——调用方不再
/// 逐次深拷贝整表映射，列表请求的 items/count 两段 SQL 复用同一份）。
///
/// `table` 可为 `"isahl"."zc_id_contract"`、`isahl.zc_id_process` 或裸表名。
/// 查询失败时返回空表（调用方按「未知类型」退化为历史行为，不报错）。
pub async fn resolve(pool: &PgPool, table: &str) -> Arc<ColumnTypeMap> {
    let (schema, table) = split_table_name(table);
    let key = (schema.clone(), table.clone());
    // 缓存命中时直接返回；guard 在块内释放，避免跨 await 持有非 Send 锁
    {
        let cache = column_type_cache()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(map) = cache.get(&key) {
            return Arc::clone(map);
        }
    }
    let rows = sqlx::query(
        "SELECT column_name, data_type, udt_name FROM information_schema.columns \
         WHERE table_schema = $1 AND table_name = $2",
    )
    .bind(&schema)
    .bind(&table)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let map: Arc<ColumnTypeMap> = Arc::new(
        rows.iter()
            .map(|r| {
                let name: String = r.get("column_name");
                let data_type: String = r.get("data_type");
                let udt_name: String = r.get("udt_name");
                let udt = (data_type == "USER-DEFINED").then_some(udt_name);
                (name, ColumnMeta { data_type, udt })
            })
            .collect(),
    );
    column_type_cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(key, Arc::clone(&map));
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(data_type: &str, udt: Option<&str>) -> ColumnMeta {
        ColumnMeta {
            data_type: data_type.to_string(),
            udt: udt.map(str::to_string),
        }
    }

    /// USER-DEFINED 列（枚举/geometry）必须带 `::"udt"` 强转——否则文本参数写枚举列报
    /// `column "x" is of type status_flag but expression is of type text`（2026-09-22 实测）。
    #[test]
    fn placeholder_casts_user_defined_columns() {
        assert_eq!(
            placeholder(3, Some(&meta("USER-DEFINED", Some("status_flag")))),
            r#"$3::"status_flag""#
        );
        assert_eq!(
            placeholder(1, Some(&meta("USER-DEFINED", Some("geometry")))),
            r#"$1::"geometry""#
        );
    }

    /// 普通列与未知列（未解析到元数据）保持裸占位符——不引入多余 cast
    #[test]
    fn placeholder_leaves_known_and_unknown_columns_bare() {
        assert_eq!(placeholder(2, Some(&meta("bigint", None))), "$2");
        assert_eq!(placeholder(7, Some(&meta("text", None))), "$7");
        assert_eq!(placeholder(4, None), "$4");
    }

    /// `udt` 仅对 USER-DEFINED 生效（data_type 别的取值即使带 udt_name 也不 cast）
    #[test]
    fn udt_accessor_is_opt_in() {
        let m = meta("USER-DEFINED", Some("status_flag"));
        assert_eq!(m.udt(), Some("status_flag"));
        assert_eq!(m.data_type(), "USER-DEFINED");
        assert_eq!(meta("text", None).udt(), None);
    }
}
