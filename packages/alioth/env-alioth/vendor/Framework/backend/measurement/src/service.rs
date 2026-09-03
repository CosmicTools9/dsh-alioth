//! 计量 Service 层（共享内核）——业务语义 DTO 组装
//!
//! 单位详情/列表项自动填充 multiplier（相对同量纲 base 单位的换算率）。
//! handler 与内部消费者共用本模块；ns 壳不再提供本地 service。

use common::data::{ListQuery, PaginatedResponse};
use common::error::AliothError;
use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::PgPool;

use crate::biz_models::MeasurementUnit;
use crate::biz_repositories::MeasurementUnitRepository;
use crud::AliothRepository;

/// 统一单位响应项（multiplier 组装注入——相对同量纲 base 的换算率）
#[derive(Debug, Clone, Serialize)]
pub struct MeasurementUnitResp {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub code: String,
    pub symbol: String,
    pub dimension: String,
    pub system: String,
    pub base: bool,
    pub multiplier: Decimal,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// 统一单位详情（multiplier 可为 None——无换算率）
#[derive(Debug, Clone, Serialize)]
pub struct MeasurementUnitDetail {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub code: Option<String>,
    pub symbol: Option<String>,
    pub dimension: Option<String>,
    pub system: Option<String>,
    pub base: Option<bool>,
    pub multiplier: Option<Decimal>,
}

/// 查询单位换算率：left 单位 到同量纲 base 单位 的 multiply。
async fn find_mult_to_base(pool: &PgPool, unit_id: i64) -> Result<Option<Decimal>, AliothError> {
    let mark: Option<Decimal> = sqlx::query_scalar(
        r#"SELECT r.multiply
           FROM isahl.zc_id_rate r
           JOIN isahl.zc_id_unit u ON u.id = r.ck_left
           JOIN isahl.zc_id_unit base ON base.id = r.ck_right AND base.base = true
           WHERE r.ck_left = $1
             AND base.system = u.system
             AND r.deleted_at IS NULL
           LIMIT 1"#,
    )
    .bind(unit_id)
    .fetch_optional(pool)
    .await
    .map_err(AliothError::from)?;
    Ok(mark)
}

#[derive(Clone)]
pub struct MeasurementService {
    pool: PgPool,
}

impl MeasurementService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 将 biz 实体 `MeasurementUnit` 组装为统一响应项，自动填充相对 base 的 multiplier。
    /// base 单位本身没有换算率时，multiplier 回退为 1。
    pub async fn to_list_item(
        &self,
        unit: MeasurementUnit,
    ) -> Result<MeasurementUnitResp, AliothError> {
        let multiplier = find_mult_to_base(&self.pool, unit.id)
            .await?
            .unwrap_or(Decimal::ONE);
        Ok(MeasurementUnitResp {
            id: unit.id,
            name: unit.name,
            code: unit.code.unwrap_or_default(),
            symbol: unit.symbol.unwrap_or_default(),
            dimension: unit.dimension.unwrap_or_default(),
            system: unit.system.unwrap_or_default(),
            base: unit.base.unwrap_or(false),
            multiplier,
            created_at: unit.created_at,
            updated_at: unit.updated_at,
            deleted_at: unit.deleted_at,
        })
    }

    /// 分页列出单位列表项，每条记录带 multiplier（RLS 读：visible_ids/authorized_columns 可选）。
    pub async fn list_units(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<PaginatedResponse<MeasurementUnitResp>, AliothError> {
        let repo = MeasurementUnitRepository::from(self.pool.clone());
        let page = repo
            .list_with_rls(query, visible_ids, authorized_columns)
            .await?;
        let mut items = Vec::with_capacity(page.items.len());
        for unit in page.items {
            items.push(self.to_list_item(unit).await?);
        }
        Ok(PaginatedResponse {
            items,
            total: page.total,
            page: page.page,
            page_size: page.page_size,
        })
    }

    /// 将 biz 实体 `MeasurementUnit` 组装为业务语义详情。
    /// multiplier 字段通过查询 `zc_id_rate` 中 left→base 的换算率填充。
    pub async fn to_unit_detail(
        &self,
        unit: MeasurementUnit,
    ) -> Result<MeasurementUnitDetail, AliothError> {
        let multiplier = find_mult_to_base(&self.pool, unit.id).await?;
        Ok(MeasurementUnitDetail {
            id: unit.id,
            name: unit.name,
            code: unit.code,
            symbol: unit.symbol,
            dimension: unit.dimension,
            system: unit.system,
            base: unit.base,
            multiplier,
        })
    }
}
