//! 审批节点模型解析（fix-avic-approval-node-model）：节点 = 操作、
//! 审批人 = 操作→岗位桥、签署模式 = 操作分类（comments 不承载结构）。
//! 遗留 comments-JSON meta 读路径（parse_node_meta/resolve_assignees/NodeMeta）
//! 已删除——零调用方实证（WZ 引擎经 resolve_node_assign 消费操作模型）。

use common::error::AliothError as ApiError;

/// 签署模式（设计器 FlowNode.mode）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignMode {
    /// 依次签署（默认）：按解析顺序逐个建实例
    #[default]
    Sequential,
    /// 会签：全员建实例，全部通过才推进；任一驳回即节点驳回
    AndSign,
    /// 或签（竞签）：全员建实例，首个终态动作定案，其余取消
    OrSign,
    /// 投票（quorum 决，2026-09-02）：全员建票，approved ≥ quorum 推进并取消余票
    Vote,
}

impl SignMode {
    pub fn parse(s: &str) -> SignMode {
        match s {
            "and_sign" => SignMode::AndSign,
            "or_sign" => SignMode::OrSign,
            "vote" => SignMode::Vote,
            _ => SignMode::Sequential,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            SignMode::Sequential => "sequential",
            SignMode::AndSign => "and_sign",
            SignMode::OrSign => "or_sign",
            SignMode::Vote => "vote",
        }
    }
}

/// 审批节点模型解析（fix-avic-approval-node-model + refactor-flow-node-operation-model）：
/// 节点 = 操作（operation 行），审批人 = 操作→岗位桥（三类动作：
/// review→rr_review / approve→rr_approve / action→rr_post），
/// 签署模式 = 操作分类（comments 不承载结构）。
///
/// - 入参 op_id = operation 节点行 id（advance 调用方统一传操作行；
///   实例侧经 rr_event 桥（ref_left=实例, ref_right=节点事件模板）反查）
/// - 审批人：三类岗位桥 UNION（ref_left=operation → ref_right=岗位）→ 岗位.fk_user
/// - 签署模式：`zc_id_operation.ck_cate-proc_op → zc_id_cate-proc_op.code`
///
/// 模型无接线（桥/分类为空）→ 空审批人 + Sequential（现状语义：仅 admin 可见兜底）。
pub struct NodeAssign {
    pub assignees: Vec<i64>,
    pub sign_mode: SignMode,
}
pub async fn resolve_node_assign<'e, E>(executor: E, op_id: i64) -> Result<NodeAssign, ApiError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    // 单查询双输出：审批人（操作→三类岗位桥 UNION → 岗位.fk_user）+ 签署模式
    // （操作.ck_cate-proc_op → 分类.code）；by-value executor 单次 fetch。
    let rows: Vec<(Option<i64>, Option<String>)> = sqlx::query_as(
        r#"WITH cate_rows AS (
               -- 岗位角色四类（2026-09-03）：rr_approve 行带 ck_cate-role 类别；
               -- review/post 无类别（NULL）。升级/备选桥行不直接产生待办——
               -- 升级由 SLA 超时接管、备选按积压阈值动态并入（见文末 UNION 分支）。
               SELECT r.ref_left, r.ref_right, rc.code AS cate_code
               FROM isahl."zc_id_operation_rr_approve" r
               LEFT JOIN isahl."zc_id_cate-approve_role" rc
                 ON rc.id = r."ck_cate-role" AND rc.deleted_at IS NULL
               WHERE r.deleted_at IS NULL
               UNION ALL
               SELECT ref_left, ref_right, NULL::text AS cate_code
               FROM isahl."zc_id_operation_rr_review" WHERE deleted_at IS NULL
               UNION ALL
               SELECT ref_left, ref_right, NULL::text AS cate_code
               FROM isahl."zc_id_operation_rr_post" WHERE deleted_at IS NULL
           ),
           direct_users AS (
               SELECT DISTINCT pos.fk_user
               FROM cate_rows br
               JOIN isahl."zc_id_subj-position" pos
                 ON pos.id = br.ref_right AND pos.deleted_at IS NULL AND pos.fk_user IS NOT NULL
               WHERE br.ref_left = $1
                 AND (br.cate_code IS NULL OR br.cate_code IN ('ROLE-DIRECT', 'ROLE-DEPUTY'))
           ),
           -- 备选触发（2026-09-03 裁决：直管岗位成员未决审批数 ≥ 阈值时并入备选岗位成员；
           -- 未决口径 = 岗位成员全局在途（岗位负载语义，非单流程限定）；
           -- 阈值固定 10；节点可配 backupThreshold 已由 publish 物化至节点模板
           -- timeline（even-approve.timeline.backupThreshold），resolve 侧模板桥读取待接入）
           backup_pos AS (
               SELECT br.ref_right FROM cate_rows br WHERE br.ref_left = $1 AND br.cate_code = 'ROLE-BACKUP'
           ),
           in_flight AS (
               SELECT count(*) AS n
               FROM isahl."zc_id_oper-approve" i
               WHERE i.fk_operator IN (SELECT fk_user FROM direct_users)
                 AND i.deleted_at IS NULL
                 AND NOT EXISTS (
                     SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                     JOIN isahl."zc_id_stus-approve" st ON st.id = ls.ref_right
                     WHERE ls.ref_left = i.id AND ls.deleted_at IS NULL
                       AND st.code IN ('approved', 'rejected', 'withdrawn', 'cancelled', 'abstained')
                 )
           )
           SELECT pos.fk_user, c.code
           FROM isahl.zc_id_operation o
           JOIN cate_rows brf ON brf.ref_left = o.id
           LEFT JOIN isahl."zc_id_subj-position" pos
             ON pos.id = brf.ref_right AND pos.deleted_at IS NULL
                AND pos.fk_user IS NOT NULL
           LEFT JOIN isahl."zc_id_cate-proc_op" c
             ON c.id = o."ck_cate-proc_op" AND c.deleted_at IS NULL
           WHERE o.id = $1 AND o.deleted_at IS NULL
             AND (brf.cate_code IS NULL OR brf.cate_code IN ('ROLE-DIRECT', 'ROLE-DEPUTY'))
           UNION ALL
           SELECT pos2.fk_user, NULL
           FROM backup_pos bp
           JOIN isahl."zc_id_subj-position" pos2
             ON pos2.id = bp.ref_right AND pos2.deleted_at IS NULL AND pos2.fk_user IS NOT NULL
           CROSS JOIN in_flight
           -- 阈值：固定 10（节点级 backupThreshold 已物化于 even-approve 模板 timeline
           -- （publish timeline.backupThreshold）；resolve 侧经模板桥读取列为引擎下一单元）
           WHERE in_flight.n >= 10
           ORDER BY fk_user"#,
    )
    .bind(op_id)
    .fetch_all(executor)
    .await
    .map_err(|e| ApiError::Database(format!("resolve node assign: {}", e)))?;

    let mut assignees: Vec<i64> = Vec::new();
    let mut mode_code: Option<String> = None;
    for (user, code) in rows {
        if let Some(u) = user {
            assignees.push(u);
        }
        if mode_code.is_none() {
            mode_code = code;
        }
    }

    Ok(NodeAssign {
        assignees,
        sign_mode: mode_code
            .as_deref()
            .map(SignMode::parse)
            .unwrap_or_default(),
    })
}
