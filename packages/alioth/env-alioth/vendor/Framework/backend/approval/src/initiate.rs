//! 业务侧审批发起助手（add-wz-flow-business-initiation 提取的共享实现）。
//!
//! 范式来源 = contract `transition.rs`（wire-contract-approval-engine）。三个消费方：
//! contract（保留自有实现）、accounts-payable（FLOW-FREIGHT）、OpenActivity（FLOW-DRIVER-ONBOARD）。
//! 新消费方 MUST 用本模块，禁止再复制 DAG 遍历/实例插入为第三、第四份实现。
//! 发起三件事由调用方事务串联：业务状态迁移 → [`create_instance_tx`] → 提交后
//! NGAC 行注册（[`register_created_row_ngac`]，提交后 fail-open）。

use common::error::AliothError as ApiError;
use sqlx::{PgPool, Postgres, Transaction};
/// 流程首审批节点定位：start 节点（图内第一个 operation 行）沿 DAG `next-ops` 逐跳，
/// 返回首个落 `zc_id_oper-approve` 叶表的操作行 id（publish.rs 物化格式）。
pub async fn find_first_approve_node_tx(
    tx: &mut Transaction<'_, Postgres>,
    flow_code: &str,
) -> Result<i64, ApiError> {
    let flow_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM "isahl"."zc_id_process"
           WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow_code)
    .fetch_optional(&mut **tx)
    .await?;
    let flow_id = flow_id
        .ok_or_else(|| ApiError::Internal(format!("{flow_code} 流程未种子（seed 缺失）")))?;

    let start_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT ref_right FROM "isahl"."zc_id_process_rr_operation"
           WHERE ref_left = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1"#,
    )
    .bind(flow_id)
    .fetch_optional(&mut **tx)
    .await?;
    let mut cur =
        start_id.ok_or_else(|| ApiError::Internal(format!("{flow_code} 无 start 节点")))?;

    for _ in 0..20 {
        let is_approve: Option<bool> = sqlx::query_scalar(
            r#"SELECT EXISTS (
                   SELECT 1 FROM "isahl"."zc_id_process_rr_operation" rro
                   JOIN "isahl"."zc_id_oper-approve" oa
                     ON oa.id = rro.ref_right AND oa.deleted_at IS NULL
                   WHERE rro.ref_left = $1 AND rro.ref_right = $2 AND rro.deleted_at IS NULL
               )"#,
        )
        .bind(flow_id)
        .bind(cur)
        .fetch_optional(&mut **tx)
        .await?
        .flatten();
        if is_approve.unwrap_or(false) {
            return Ok(cur);
        }
        let next: Option<serde_json::Value> = sqlx::query_scalar(
            r#"SELECT "next-ops" FROM "isahl"."zc_id_process_rr_operation"
               WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL LIMIT 1"#,
        )
        .bind(flow_id)
        .bind(cur)
        .fetch_optional(&mut **tx)
        .await?
        .flatten();
        let ids: Vec<i64> = next
            .and_then(|v| v.as_array().cloned())
            .map(|a| {
                a.iter()
                    .filter_map(|x| {
                        x.as_i64()
                            .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let Some(next_id) = ids.into_iter().next() else {
            break;
        };
        cur = next_id;
    }
    Err(ApiError::Internal(format!("{flow_code} 无审批节点")))
}

/// 创建审批实例（业务动作同事务桥）：绑定 `flow_code` 首审批节点。
///
/// - 审批人：节点 `operation_rr_approve` 桥 → 岗位 → fk_user（[`crate::node_meta::resolve_node_assign`]）；
///   解析为空 → NULL（admin 全可见兜底）+ warn——禁止回退申请人自审。
/// - 实例↔节点事件关联：`operation_rr_event` 桥（fk_approve 物理列已移除）。
/// - comments：纯文本定位锚（调用方约定域前缀，如 `运输费用审批实例：承运账单 {id}…`），
///   订阅方按文本解析回链（remove-comments-json-embedding 降级契约，非 JSON 嵌入）。
pub async fn create_instance_tx(
    tx: &mut Transaction<'_, Postgres>,
    flow_code: &str,
    node_label_fallback: &str,
    instance_code: &str,
    comments: &str,
    applicant_user_id: i64,
) -> Result<i64, ApiError> {
    let op_id = find_first_approve_node_tx(tx, flow_code).await?;
    let node_name: Option<String> = sqlx::query_scalar(
        r#"SELECT notice FROM "isahl".zc_id_operation
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(op_id)
    .fetch_optional(&mut **tx)
    .await?
    .flatten();

    let node_assign = crate::node_meta::resolve_node_assign(&mut **tx, op_id).await?;
    let operator: Option<i64> = node_assign.assignees.first().copied();
    if operator.is_none() {
        common::telemetry::warn!(
            "approval initiate: flow {} first node {} resolved zero assignees — fk_operator NULL (admin-visible)",
            flow_code,
            op_id
        );
    }

    // 节点事件模板（实例挂模板——rr_event 桥 ref_right；无模板时兜底操作行自身）
    let template_event_id: i64 = sqlx::query_scalar(
        r#"SELECT oe.ref_right FROM "isahl".zc_id_operation_rr_event oe
           WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(op_id)
    .fetch_optional(&mut **tx)
    .await?
    .flatten()
    .unwrap_or(op_id);

    // 本体坐标（§6.12 声明即必须）：JC/FTA/↓_EZ，code→ZUID 经 ontology_binding 解析
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve_conn(tx, ("JC", "FTA", "↓_EZ"))
            .await
            .map_err(ApiError::from_sqlx)?;

    let instance_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_oper-approve"
           (id, notice, code, fk_subject, fk_operator, comments, tpl_id, created_by_id,
            dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id"#,
    )
    .bind(node_name.unwrap_or_else(|| node_label_fallback.to_string()))
    .bind(instance_code)
    .bind(applicant_user_id)
    .bind(operator)
    .bind(comments)
    .bind(op_id)
    .bind(applicant_user_id)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(&mut **tx)
    .await?;

    sqlx::query(
        // id 省略：由列默认 gen_next_uid(267) 决定（勿显式写 zuid——id 同类性规约）
        r#"INSERT INTO "isahl".zc_id_operation_rr_event (ref_left, ref_right, created_by_id)
           VALUES ($1, $2, $3)"#,
    )
    .bind(instance_id)
    .bind(template_event_id)
    .bind(applicant_user_id)
    .execute(&mut **tx)
    .await?;

    Ok(instance_id)
}
/// 行级 NGAC 注册（授权写入**单源** = `common::ngac_grants`；consolidate-ngac-grant-authoring）：
/// 自定义 create 路径必须显式注册行 OA + 创建者 UA 关联，否则 dock approve 的
/// `require_resource_access` 行级判定不可见该实例。失败仅告警不阻断（注册可补，实例已落库）。
pub async fn register_created_row_ngac(
    pool: &PgPool,
    resource_type: &str,
    item_id: i64,
    user_id: i64,
) {
    // 锚点单行（derive-creator-grants-from-lifecycle：创建者授权由派生层判定，不扇出关联）。
    if let Err(e) = common::ngac_grants::ensure_resource_oa(
        pool,
        &common::ngac_grants::ResourceOaSpec {
            o_name: format!("{resource_type}-{item_id}"),
            resource_type,
            resource_id: item_id,
            resource_identifier: None,
            created_by: Some(user_id),
            policy_class_name: None,
        },
    )
    .await
    {
        common::telemetry::warn!("approval NGAC 锚点写入失败（{resource_type} {item_id}）: {e}");
    }
}
