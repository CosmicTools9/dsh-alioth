//! 审批自动触发订阅者（fix-flow-designer-runtime-chain 遗留项①）
//!
//! 消费 crud 层发布的 `EntityCreated` 领域事件：业务实体（三域叶表行）创建后，
//! 若其范畴绑定已发布流程，自动 initiate（实体桥绑定 + 首链实例）。
//! 无绑定/未发布/非三域叶表 → 静默跳过。错误只 log 不崩溃（订阅循环永活）。

use common::error::AliothError as ApiError;
use common::event_bus::{DomainEvent, DomainEventBus};
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;

use crate::advance::initiate_flow;
use crate::context_meta::context_fields_of;

#[derive(Debug, Deserialize)]
struct EntityCreatedPayload {
    entity_table: String,
    /// 业务实体行 id（zuid；crud 发布端 JSON 字符串化，防 2^53 精度截断）
    #[serde(with = "common::serde_zuid")]
    entity_id: i64,
    #[allow(dead_code)]
    created_by: Option<i64>,
}

/// 范畴绑定已发布范例流程（双范畴契约）：
/// - legacy：scope-definition 行直接落业务叶表 → 范畴行 tableoid == entity_table；
/// - 新契约：flow-context 范例行落域父表（zc_id_event/zc_id_task/zc_id_even-approve）
///   → entity_table 的域 == 范畴行的域（经 context_family_table 归一比对）。
async fn bound_published_flow(pool: &PgPool, entity_table: &str) -> Result<Option<i64>, ApiError> {
    let candidates: Vec<(i64, String, String)> = sqlx::query_as(
        r#"SELECT p.id, replace(c.tableoid::regclass::text, '"', ''), c._t_
           FROM isahl.zc_id_process p
           JOIN isahl."zc_id_proc-context" c
             ON c.id = p.fk_context AND c.deleted_at IS NULL
           WHERE p.deleted_at IS NULL
             AND p._f_ = '实现' AND p._t_ = '范例'
             AND EXISTS (
                 SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                 JOIN isahl."zc_id_stus-process" s ON s.id = ls.ref_right
                 WHERE ls.ref_left = p.id AND ls.deleted_at IS NULL AND s.code = 'published'
             )
           ORDER BY CASE WHEN p._f_ = '实现'
                            AND (p._t_ = '范例' OR p._t_ IS NULL)
                        THEN 0 ELSE 1 END,
                    p.updated_at DESC"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;

    let entity_domain = crate::context_domain::domain_of_leaf(entity_table).unwrap_or("");
    let entity_family = crate::context_domain::context_family_table(entity_domain);
    for (flow_id, ctx_leaf, ctx_t) in candidates {
        let matched = match ctx_t.as_str() {
            "scope-definition" => ctx_leaf == entity_table,
            "flow-context" => entity_family == Some(ctx_leaf.as_str()),
            _ => false,
        };
        if matched {
            return Ok(Some(flow_id));
        }
    }
    Ok(None)
}

/// 单实体自动发起（订阅事件处理 + 可直接测试的纯函数路径）
pub async fn maybe_auto_initiate(
    pool: &PgPool,
    entity_table: &str,
    entity_id: i64,
    created_by: i64,
    bus: Option<&Arc<dyn DomainEventBus>>,
) -> Result<(), ApiError> {
    // 1. 非三域叶表（context_meta 白名单外）→ 跳过
    if context_fields_of(entity_table).is_none() {
        return Ok(());
    }
    // 2. 无绑定已发布流程 → 跳过
    let Some(flow_id) = bound_published_flow(pool, entity_table).await? else {
        return Ok(());
    };
    // 3. 物化执行行（实现·实例，tpl_id → 范例）并以之为链根自动发起
    let summary = format!("流程自动发起：实体 {entity_table}#{entity_id}");
    let execution_id =
        crate::advance::materialize_flow_execution(pool, flow_id, created_by, &summary).await?;
    common::telemetry::info!(
        "auto-initiate: entity {entity_table}#{entity_id} → flow {flow_id} exec {execution_id}"
    );
    initiate_flow(
        pool,
        flow_id,
        created_by,
        entity_table,
        entity_id,
        Some(execution_id),
        bus,
    )
    .await?;
    Ok(())
}

async fn handle_event(pool: &PgPool, event: DomainEvent) {
    if event.event_type != "EntityCreated" {
        return;
    }
    let Ok(payload) = serde_json::from_value::<EntityCreatedPayload>(event.payload.clone()) else {
        common::telemetry::warn!(
            "auto-initiate: EntityCreated payload 解析失败: {:?}",
            event.payload
        );
        return;
    };
    let user = payload.created_by.unwrap_or(1);
    if let Err(e) =
        maybe_auto_initiate(pool, &payload.entity_table, payload.entity_id, user, None).await
    {
        common::telemetry::warn!(
            "auto-initiate failed for {}#{}: {}",
            payload.entity_table,
            payload.entity_id,
            e
        );
    }
}

/// 订阅 EntityCreated（Gateway 装配：event_bus 就绪后 spawn）
pub fn subscribe_auto_initiate(bus: Arc<dyn DomainEventBus>, pool: PgPool) {
    actix_web::rt::spawn(async move {
        let mut subscriber = match bus.subscribe("EntityCreated").await {
            Ok(s) => s,
            Err(e) => {
                common::telemetry::error!("auto-initiate: subscribe EntityCreated failed: {}", e);
                return;
            }
        };
        loop {
            match subscriber.recv().await {
                Ok(evt) => handle_event(&pool, evt).await,
                Err(_) => continue,
            }
        }
    });
}
