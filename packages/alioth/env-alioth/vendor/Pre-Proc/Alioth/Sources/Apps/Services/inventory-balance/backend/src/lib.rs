//! Alioth namespace inventory-balance service — 薄封装层
//!
//! 库存统计通用逻辑已沉淀 Framework/backend/inventory（用户裁定：
//! 「库存 = 货在储元中的时空切片数量统计」语义同构通用）。
//! 本模块仅：
//! 1. 注册 namespace 路由前缀 `/service/inventory-balance`；
//! 2. 注入 **货/储元名称解析**（目标表因 namespace 而异）：
//!    - 货（Material）→ `isahl."zc_id_production"`（生产/物料实体）
//!    - 储元（Place）→ `isahl."zc_id_stor-container"`（库位/容器实体）
//!
//! 路由委托：`/service/inventory-balance/balances` → inventory::configure_routes。

use actix_web::web;
use inventory::models::{NameResolver, RefKind, RefNames};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;

/// 货/储元名称查询静态 SQL（表名编译期固化；ns 二表为闭式集合）
const MATERIAL_NAMES_SQL: &str =
    r#"SELECT id, notice FROM isahl."zc_id_production" WHERE id = ANY($1) AND deleted_at IS NULL"#;
const PLACE_NAMES_SQL: &str = r#"SELECT id, notice FROM isahl."zc_id_stor-container" WHERE id = ANY($1) AND deleted_at IS NULL"#;

/// Alioth 货/储元名称解析（业务名 = notice 列；表名 ns 硬编码，非用户输入）
#[derive(Clone, Default)]
pub struct AliothInventoryNameResolver;

#[async_trait::async_trait]
impl NameResolver for AliothInventoryNameResolver {
    async fn resolve(&self, pool: &PgPool, kind: RefKind, ids: &[i64]) -> RefNames {
        let mut names = RefNames::new();
        let sql = match kind {
            RefKind::Material => MATERIAL_NAMES_SQL,
            RefKind::Place => PLACE_NAMES_SQL,
        };
        let resolved = if ids.is_empty() {
            HashMap::new()
        } else {
            sqlx::query_as::<_, (i64, String)>(sql)
                .bind(ids)
                .fetch_all(pool)
                .await
                .unwrap_or_default()
                .into_iter()
                .collect()
        };
        names.insert(kind, resolved);
        names
    }
}

/// 注册 namespace 路由（Gateway service_registry 调用）
pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/service/inventory-balance")
            .app_data(web::Data::new(
                Arc::new(AliothInventoryNameResolver) as Arc<dyn NameResolver>
            ))
            .configure(inventory::configure_routes),
    );
}
