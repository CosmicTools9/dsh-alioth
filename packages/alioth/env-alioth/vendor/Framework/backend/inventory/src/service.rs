//! 库存统计查询服务——物化视图 `isahl.mv_inventory` 分页读取 + 货/储元名称解析
//!
//! 语义（用户裁定）：库存 = 货（production）在储元（storage）中的时空切片数量统计。
//! mv_inventory 为容量行物化视图（`zc_id_production_rr_storage`：qty/capacity 经标量表解析）。

use crate::models::{InventoryBalanceSummary, NameResolver, RefKind};
use common::AliothError as ApiError;
use crud::{ListQuery, PaginatedResponse};
use sqlx::PgPool;
use std::sync::Arc;

/// 库存统计服务（通用；refs 解析由 namespace 壳注入）
#[derive(Clone)]
pub struct InventoryService {
    pool: PgPool,
    resolver: Arc<dyn NameResolver>,
}

impl InventoryService {
    pub fn new(pool: PgPool, resolver: Arc<dyn NameResolver>) -> Self {
        Self { pool, resolver }
    }

    /// 分页查询库存余额（时空切片数量统计）
    ///
    /// - 过滤：`production_id` / `storage_id`（可选）
    /// - 排序：`qty` 降序（默认）/ `production_id` 升序
    pub async fn list(
        &self,
        query: &ListQuery,
        production_id: Option<i64>,
        storage_id: Option<i64>,
    ) -> Result<PaginatedResponse<InventoryBalanceSummary>, ApiError> {
        let page = query.page.max(1);
        let page_size = query.page_size.clamp(1, 100);

        let mut sql = String::from(
            "SELECT id, production_id, storage_id, qty, capacity, unit \
             FROM isahl.mv_inventory WHERE 1=1",
        );
        let mut binds: Vec<i64> = Vec::new();
        if let Some(p) = production_id {
            sql.push_str(" AND production_id = $");
            sql.push_str(&(binds.len() + 1).to_string());
            binds.push(p);
        }
        if let Some(s) = storage_id {
            sql.push_str(" AND storage_id = $");
            sql.push_str(&(binds.len() + 1).to_string());
            binds.push(s);
        }
        let order = match query.sort_field.as_deref() {
            Some("production") => "production_id ASC",
            _ => "qty DESC",
        };
        sql.push_str(&format!(
            " ORDER BY {order} LIMIT {} OFFSET {}",
            page_size,
            (page - 1) * page_size
        ));

        // 动态 SQL（排序/分页数值拼接，过滤值全部走 binds）——显式 AssertSqlSafe 审计
        let mut q = sqlx::query_as::<
            _,
            (
                i64,
                i64,
                i64,
                rust_decimal::Decimal,
                rust_decimal::Decimal,
                Option<i64>,
            ),
        >(sqlx::AssertSqlSafe(sql));
        for b in binds {
            q = q.bind(b);
        }
        let rows = q.fetch_all(&self.pool).await.map_err(ApiError::from)?;

        // 汇总数量（总行数）
        let total: i64 = sqlx::query_scalar("SELECT count(*) FROM isahl.mv_inventory")
            .fetch_one(&self.pool)
            .await
            .map_err(ApiError::from)?;

        // 货/储元名称批量解析（namespace 注入）
        let production_ids: Vec<i64> = rows.iter().map(|r| r.1).collect();
        let storage_ids: Vec<i64> = rows.iter().map(|r| r.2).collect();
        let names = self
            .resolver
            .resolve(&self.pool, RefKind::Material, &production_ids)
            .await;
        let place_names = self
            .resolver
            .resolve(&self.pool, RefKind::Place, &storage_ids)
            .await;

        // 时空切片时点：PG 无系统级物化视图刷新时间（pg_matviews 无 refreshtime 列），
        // 本期返回 None；P1 可由刷新脚本维护刷新台账表。
        let refreshed_at: Option<chrono::DateTime<chrono::Utc>> = None;

        let items = rows
            .into_iter()
            .map(
                |(id, production_id, storage_id, qty, capacity, unit)| InventoryBalanceSummary {
                    id,
                    production_id,
                    production_name: names
                        .get(&RefKind::Material)
                        .and_then(|m| m.get(&production_id))
                        .cloned(),
                    storage_id,
                    storage_name: place_names
                        .get(&RefKind::Place)
                        .and_then(|m| m.get(&storage_id))
                        .cloned(),
                    qty,
                    capacity,
                    unit,
                    refreshed_at,
                },
            )
            .collect();

        Ok(PaginatedResponse {
            items,
            total,
            page,
            page_size,
        })
    }
}
