//! 列类型元数据解析 — information_schema + 进程级缓存
//!
//! 供过滤器（QueryBuilder 按列类型分派 SQL cast）和写入绑定
//! （bind_json 按列类型强转值）共享，避免各自查一次 information_schema。
//! 表结构变更后重启进程即可刷新缓存。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use sqlx::{PgPool, Row};

/// (schema, table) → (column → data_type) 进程级缓存
type ColumnTypeMap = HashMap<String, String>;
type ColumnTypeCache = HashMap<(String, String), ColumnTypeMap>;

static COLUMN_TYPE_CACHE: LazyLock<Mutex<ColumnTypeCache>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn column_type_cache() -> &'static Mutex<ColumnTypeCache> {
    &COLUMN_TYPE_CACHE
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

/// 解析表的列 → data_type 映射（带进程级缓存）。
///
/// `table` 可为 `"isahl"."zc_id_contract"`、`isahl.zc_id_process` 或裸表名。
/// 查询失败时返回空表（调用方按「未知类型」退化为历史行为，不报错）。
pub async fn resolve(pool: &PgPool, table: &str) -> HashMap<String, String> {
    let (schema, table) = split_table_name(table);
    let key = (schema.clone(), table.clone());
    // 缓存命中时直接返回；guard 在块内释放，避免跨 await 持有非 Send 锁
    {
        let cache = column_type_cache()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(map) = cache.get(&key) {
            return map.clone();
        }
    }
    let rows = sqlx::query(
        "SELECT column_name, data_type FROM information_schema.columns \
         WHERE table_schema = $1 AND table_name = $2",
    )
    .bind(&schema)
    .bind(&table)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let map: HashMap<String, String> = rows
        .iter()
        .map(|r| {
            let name: String = r.get("column_name");
            let ty: String = r.get("data_type");
            (name, ty)
        })
        .collect();
    column_type_cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(key, map.clone());
    map
}
