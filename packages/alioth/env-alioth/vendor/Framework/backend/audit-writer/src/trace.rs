//! 正向追溯读径（change add-recall-support 组 3.1，零 DDL）。
//!
//! 给定批次或设备标识，返回经 `bom-outbound`/`deta-trade_order`/`sto-voucher` 族的全部流向行。
//! NGAC 行级授权 = 调用方注入 `visible_ids`（Gateway PEP `X-Visible-Ids` 口径）；
//! 30s 服务端超时（[READONLY] DB 探查口径——杀客户端不取消服务端语句）。

use common::AliothError;
use sqlx::PgPool;

/// 正向追溯结果行。
#[derive(Debug, serde::Serialize)]
pub struct TraceRow {
    pub id: String,
    pub table: String,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// 追溯锚点（二选一）。
#[derive(Debug)]
pub enum TraceAnchor {
    /// 批次 id（`zc_id_tags-batch`）
    Batch(i64),
    /// 设备清单 id（`zc_id_bom-equipment`）
    Equipment(i64),
}

/// 正向追溯（只读查询；[READONLY]——调用方 MUST 用只读用户或确保无写径）。
///
/// 查询面复用既有引用列（`tk_batch_no` → `tags-batch`；`fk_obj-storage` → 储元/设备）——
/// 零 DDL、零新业务读径（本查询 = 审计/召回域自有聚合面，非第二业务读径）。
pub async fn trace_forward(
    pool: &PgPool,
    anchor: &TraceAnchor,
    visible_ids: Option<&[i64]>,
) -> Result<Vec<TraceRow>, AliothError> {
    let (batch_id, equip_id) = match anchor {
        TraceAnchor::Batch(id) => (Some(*id), None),
        TraceAnchor::Equipment(id) => (None, Some(*id)),
    };

    // 流向三族：出库/订单明细/库存凭证
    let rows = sqlx::query_as::<
        _,
        (
            i64,
            String,
            Option<String>,
            Option<String>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"SELECT id, 'bom-outbound', code, notice, created_at
           FROM "isahl"."zc_id_bom-outbound"
           WHERE tk_batch_no = $1 AND deleted_at IS NULL
           UNION ALL
           SELECT d.id, 'deta-trade_order', d.code, d.notice, d.created_at
           FROM "isahl"."zc_id_deta-trade_order" d
           WHERE d.tk_batch_no = $1 AND d.deleted_at IS NULL
           UNION ALL
           SELECT v.id, 'sto-voucher', v.code, v.notice, v.created_at
           FROM "isahl"."zc_id_stat-sto-voucher" v
           WHERE v."fk_obj-storage" = $2 AND v.deleted_at IS NULL
           ORDER BY created_at DESC"#,
    )
    .bind(batch_id)
    .bind(equip_id)
    .fetch_all(pool)
    .await
    .map_err(AliothError::from_sqlx)?;

    let items: Vec<TraceRow> = rows
        .into_iter()
        .filter(|(id, ..)| visible_ids.map(|ids| ids.contains(id)).unwrap_or(true))
        .map(|(id, table, code, notice, created_at)| TraceRow {
            id: id.to_string(),
            table,
            code,
            notice,
            created_at,
        })
        .collect();
    Ok(items)
}
