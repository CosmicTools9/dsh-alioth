//! 审批完成事件共享判定（add-wz-flow-business-initiation 提取）。
//!
//! 来源 = contract events.rs（fix-wz-flow-end-node-contract-bridge 的 `*-END` 语义判定）。
//! 消费方：contract / accounts-payable / isahl-db 的 ApprovalCompleted 订阅。
//! 新订阅方 MUST 用本模块，禁止再复制 DAG 终态遍历。

use sqlx::PgPool;

/// 终态判定：节点事件在流程 DAG 中 next-ops 为空或下一节点为结束节点。
///
/// DAG 边 ref_right = 操作行 id（refactor-flow-node-operation-model）——节点事件
/// （even-approve 模板）先经 rr_event 桥反查操作行（tpl_id IS NULL 排除实例行）；
/// 无操作行时兜底直接用事件 id（兼容旧事件键边）。
pub async fn is_final_approval(pool: &PgPool, node_event_id: i64) -> bool {
    let node_key: i64 = sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT oe.ref_left FROM isahl.zc_id_operation_rr_event oe
           JOIN isahl.zc_id_operation o ON o.id = oe.ref_left AND o.tpl_id IS NULL
           WHERE oe.ref_right = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(node_event_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten()
    .unwrap_or(node_event_id);
    let row: Option<(i64, serde_json::Value)> = sqlx::query_as(
        r#"SELECT rro.ref_left, rro."next-ops"
           FROM isahl."zc_id_process_rr_operation" rro
           WHERE rro.ref_right = $1 AND rro.deleted_at IS NULL
           LIMIT 1"#,
    )
    .bind(node_key)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let Some((flow_id, next_ops)) = row else {
        return true; // 无 DAG 记录 → 视为终态（防御）
    };
    let ids: Vec<i64> = next_ops
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| {
                    v.as_i64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return true;
    }
    let next_type: Option<String> = sqlx::query_scalar(
        r#"SELECT rro.code FROM isahl."zc_id_process_rr_operation" rro
           WHERE rro.ref_left = $1 AND rro.ref_right = ANY($2) AND rro.deleted_at IS NULL
           LIMIT 1"#,
    )
    .bind(flow_id)
    .bind(&ids)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    // 指向结束节点或目标节点已删 → 终态
    match next_type.as_deref() {
        None => true,
        Some(code) => is_end_code(code),
    }
}

/// 结束节点 code 语义判定（fix-wz-flow-end-node-contract-bridge）：
/// 取 `-` 分隔末段、大小写不敏感比较 `end`——`end`/`END`/`NODE-END`/任意 `*-END` 均为终态；
/// 末段**整体**比较（非子串匹配），`trend`/`NODE-GM`/空串不为终态。
///
/// 实测（2026-09-15，wz 库 FLOW-CONTRACT）：物化 DAG 边 code = `NODE-END`——
/// 字面量 `== "end"` 使终审判非终态、业务实体永卡 pending。
pub fn is_end_code(code: &str) -> bool {
    matches!(code.rsplit('-').next(), Some(seg) if seg.eq_ignore_ascii_case("end"))
}

#[cfg(test)]
mod tests {
    use super::is_end_code;

    /// 正例：字面形态 + 前缀形态（dev/生产物化）+ 大小写变体
    #[test]
    fn end_code_forms_are_final() {
        for code in [
            "end",
            "END",
            "End",
            "NODE-END",
            "node-end",
            "FLOW-CONTRACT-END",
        ] {
            assert!(is_end_code(code), "{code} 必须判为结束节点");
        }
    }

    /// 反例：末段整体比较（非子串匹配）+ 普通节点 + 空串
    #[test]
    fn non_end_codes_are_not_final() {
        for code in [
            "trend",
            "weekend",
            "NODE-GM",
            "NODE-LEGAL",
            "START",
            "",
            "end-node",
        ] {
            assert!(!is_end_code(code), "{code} 不得判为结束节点");
        }
    }
}
