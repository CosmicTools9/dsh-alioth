//! DocumentIngester — 嵌套 JSON 文档的 DFS 级联创建引擎
//!
//! 将前端传入的嵌套 JSON 文档自动拆分为多个本体对象，处理关联创建。
//!
//! # 协议
//!
//! 前端 JSON 中使用 `_table` 字段声明每个嵌套对象的目标表：
//!
//! ```json
//! {
//!   "_table": "zc_id_stat-trade_order",
//!   "notice": "采购订单",
//!   "fk_customer": { "id": 12345 },           // 已有记录，直接关联
//!   "fk_warehouse": { "_table": "zc_id_stor-plc-warehouse",
//!                     "notice": "新仓库" },     // 需先创建
//!   "items": [
//!     { "_table": "zc_id_deta-trade_order",
//!       "fk_product": { "id": 67890 },
//!       "qk_qty": 10 }
//!   ]
//! }
//! ```
//!
//! # 算法
//!
//! 1. 迭代 DFS 遍历 JSON 树，使用显式栈避免 async fn 递归限制
//! 2. 对每个嵌套对象：
//!    - 有 `"id"` → 提取为 FK 值（已有记录）
//!    - 无 `"id"` → 创建子对象，取返回的 id 作为 FK 值
//! 3. 将嵌套对象替换为标量 FK 值后创建父对象
//! 4. 全部在单个事务中执行

use crate::schema_repository::SchemaRepository;
use common::AliothError;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};

/// 文档级联创建引擎。
pub struct DocumentIngester {
    repo: SchemaRepository,
}

impl DocumentIngester {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repo: SchemaRepository::new(pool),
        }
    }

    /// 主入口：创建根文档及其所有嵌套子对象。
    pub async fn ingest(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        root_table: &str,
        doc: Value,
        user_id: i64,
    ) -> Result<i64, AliothError> {
        // 1. 解析整个文档——所有嵌套对象被替换为 FK 标量（或原地创建）
        let resolved = self.flatten_document(tx, doc, user_id).await?;

        // 2. 创建根对象
        let root_id = self
            .repo
            .create_tx(tx, root_table, resolved, user_id)
            .await?;

        Ok(root_id)
    }

    /// 扁平化整个 JSON 文档：迭代 DFS 处理所有嵌套对象。
    ///
    /// 返回处理后的根 Value，其中所有嵌套对象已被替换为 FK id（Number）或原样保留。
    async fn flatten_document(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        doc: Value,
        user_id: i64,
    ) -> Result<Value, AliothError> {
        match doc {
            Value::Object(map) => {
                let resolved = self.flatten_object(tx, map, user_id).await?;
                Ok(Value::Object(resolved))
            }
            other => Ok(other),
        }
    }

    /// 扁平化一个对象：处理所有字段，将嵌套对象创建/关联后替换为 FK。
    async fn flatten_object(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        mut obj: serde_json::Map<String, Value>,
        user_id: i64,
    ) -> Result<serde_json::Map<String, Value>, AliothError> {
        let keys: Vec<String> = obj.keys().cloned().collect();
        let mut resolved = serde_json::Map::new();

        for key in keys {
            let value = obj.remove(&key).unwrap();
            match value {
                // 嵌套对象：使用 Box::pin 处理（打破 async 递归）
                Value::Object(inner) => {
                    let r = self.flatten_object_boxed(tx, inner, user_id).await?;
                    resolved.insert(key, r);
                }
                // 数组：逐元素处理（元素本身可能是嵌套对象）
                Value::Array(items) => {
                    let mut processed = Vec::with_capacity(items.len());
                    for item in items {
                        match item {
                            Value::Object(inner) => {
                                let r = self.flatten_object_boxed(tx, inner, user_id).await?;
                                processed.push(r);
                            }
                            other => processed.push(other),
                        }
                    }
                    resolved.insert(key, Value::Array(processed));
                }
                // 标量 → 原样保留
                other => {
                    resolved.insert(key, other);
                }
            }
        }

        Ok(resolved)
    }

    /// 扁平化一个对象字段，返回替换后的标量 FK 值。
    /// 使用 Box::pin 打破间接 async 递归（flatten_object → flatten_object_boxed → flatten_object）。
    fn flatten_object_boxed<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        mut inner: serde_json::Map<String, Value>,
        user_id: i64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, AliothError>> + Send + 'a>>
    {
        Box::pin(async move {
            // 有 "id" → 直接关联已有记录
            if let Some(id_val) = inner.get("id") {
                if let Some(id) = id_val.as_i64() {
                    return Ok(Value::from(id));
                }
            }

            // 无 "id" → 需要创建. 必须有 _table
            let table = match inner.remove("_table") {
                Some(Value::String(t)) => t,
                Some(_) => return Err(AliothError::BadRequest("`_table` must be a string".into())),
                None => {
                    return Err(AliothError::BadRequest(
                        "nested object must have either `id` or `_table`".into(),
                    ))
                }
            };

            // 扁平化当前对象的字段，然后创建
            let flat_fields = self.flatten_object(tx, inner, user_id).await?;
            let new_id = self
                .repo
                .create_tx(tx, &table, Value::Object(flat_fields), user_id)
                .await?;
            Ok(Value::from(new_id))
        })
    }
}

// ─── 便捷函数 ──────────────────────────────────────────────────────

/// 创建 DocumentIngester 并执行一次 ingest（开/关事务的快捷入口）。
pub async fn ingest_document(
    pool: &PgPool,
    root_table: &str,
    doc: Value,
    user_id: i64,
) -> Result<i64, AliothError> {
    let ingester = DocumentIngester::new(pool.clone());

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| AliothError::Database(e.to_string()))?;

    let result = ingester.ingest(&mut tx, root_table, doc, user_id).await;

    match &result {
        Ok(_) => {
            tx.commit()
                .await
                .map_err(|e| AliothError::Database(e.to_string()))?;
        }
        Err(_) => {
            let _ = tx.rollback().await;
        }
    }

    result
}
