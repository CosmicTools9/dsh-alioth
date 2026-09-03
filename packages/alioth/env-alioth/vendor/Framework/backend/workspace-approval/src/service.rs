//! 审批状态切换服务
//!
//! 基于 zc_id_stus-approve + zc_id_lifecycle_r_primary-status 的两个动作：
//! 1. approve → 将状态设为"approved"
//! 2. reject → 将状态设为"rejected"
//!
//! D2（消除双真相源）：execute 在同一事务内写入 zc_id_deta-opinion，
//! 使 overview 的 LATERAL JOIN 状态判断有单一真相源。

use sqlx::PgPool;

use crate::hook::ApprovalHook;
use crate::models::{
    ApprovalActionResponse, ApprovalActor, APPROVAL_NOTICE_APPROVED, APPROVAL_NOTICE_REJECTED,
};

pub struct ApprovalService;

impl ApprovalService {
    /// 执行审批动作
    ///
    /// `actor` 为操作者信息（含 user_id 与可选意见），用于：
    /// 1. 鉴权（fk_operator 非 NULL 时必须匹配 user_id）
    /// 2. 意见落库（写入 zc_id_deta-opinion）
    ///
    /// `hook` 为可选的审批后回调。hook 执行失败不影响审批状态——它已经写入 DB。
    /// hook 实现方负责自行记录错误。
    pub async fn execute(
        pool: &PgPool,
        approval_id: i64,
        status_code: &str,
        actor: Option<ApprovalActor>,
        hook: Option<&dyn ApprovalHook>,
    ) -> ApprovalActionResponse {
        // T1.2 鉴权：查询 fk_operator，非 NULL 且 != user_id → 403
        if let Some(a) = &actor {
            let fk_operator: Option<i64> = sqlx::query_scalar(
                r#"SELECT fk_operator FROM isahl."zc_id_oper-approve"
                   WHERE id = $1 AND deleted_at IS NULL"#,
            )
            .bind(approval_id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

            if let Some(op) = fk_operator {
                if op != a.user_id {
                    return ApprovalActionResponse::fail("APPROVAL_NOT_OPERATOR");
                }
            }
        }

        // 1. 查找目标状态 ID
        let status_id: Option<i64> = match sqlx::query_scalar::<_, Option<i64>>(
            r#"SELECT id FROM isahl."zc_id_stus-approve" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
        )
        .bind(status_code)
        .fetch_one(pool)
        .await
        {
            Ok(Some(id)) => Some(id),
            _ => None,
        };

        let status_id = match status_id {
            Some(id) => id,
            None => {
                let label = match status_code {
                    "approved" => "已通过",
                    "rejected" => "已拒绝",
                    _ => status_code,
                };
                match sqlx::query_scalar::<_, i64>(
                    r#"INSERT INTO isahl."zc_id_stus-approve" (id, code, notice)
                       VALUES (isahl.gen_next_zuid(), $1, $2) RETURNING id"#,
                )
                .bind(status_code)
                .bind(label)
                .fetch_one(pool)
                .await
                {
                    Ok(id) => id,
                    Err(e) => {
                        return ApprovalActionResponse::fail(format!("创建审批状态失败: {}", e))
                    }
                }
            }
        };

        // 2. 意见通知文本
        let notice = match status_code {
            "approved" => APPROVAL_NOTICE_APPROVED,
            "rejected" => APPROVAL_NOTICE_REJECTED,
            _ => status_code,
        };

        // 3. D2 同事务：生命周期主状态更新 + 意见落库（意见失败 → 整体回滚，消除状态双真相源窗口）
        let mut tx = match pool.begin().await {
            Ok(t) => t,
            Err(e) => return ApprovalActionResponse::fail(format!("开启事务失败: {}", e)),
        };
        // 三态查询（活跃行原地 UPDATE / 软删行 restore / 无行 INSERT）；
        // ref_left 全表唯一——COUNT 活跃行判 exists 遇软删行会 INSERT 撞约束。
        // 查询失败与迁移写同级：回滚，不允许误记为 INSERT 初始
        let row = match crud::audit_outbox::fetch_primary_status_row_tx(&mut tx, approval_id).await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.rollback().await;
                return ApprovalActionResponse::fail(format!("读取当前状态失败: {}", e));
            }
        };
        let old_status = row.filter(|(_, active)| *active).map(|(s, _)| s);

        let r = match row {
            Some((_, true)) => {
                sqlx::query(
                    r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
                       SET ref_right = $1, updated_at = NOW()
                       WHERE ref_left = $2 AND deleted_at IS NULL"#,
                )
                .bind(status_id)
                .bind(approval_id)
                .execute(&mut *tx)
                .await
            }
            Some((_, false)) => {
                sqlx::query(
                    r#"UPDATE isahl."zc_id_lifecycle_r_primary-status"
                       SET ref_right = $1, deleted_at = NULL, updated_at = NOW()
                       WHERE ref_left = $2 AND deleted_at IS NOT NULL"#,
                )
                .bind(status_id)
                .bind(approval_id)
                .execute(&mut *tx)
                .await
            }
            None => sqlx::query(
                r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (id, ref_left, ref_right)
                       VALUES (isahl.gen_next_zuid(), $1, $2)"#,
            )
            .bind(approval_id)
            .bind(status_id)
            .execute(&mut *tx)
            .await,
        };

        if let Err(e) = r {
            let _ = tx.rollback().await;
            return ApprovalActionResponse::fail(format!("更新审批状态失败: {}", e));
        }

        // ADR D-010：主状态变更审计与迁移同事务（严格零丢失；失败整体回滚，
        // 与意见落库同级——审计是状态双真相源防线的一部分）
        let audit_user = actor.as_ref().map(|a| a.user_id);
        if let Err(e) = crud::audit_outbox::audit_primary_status_tx(
            &mut tx,
            approval_id,
            old_status,
            status_id,
            audit_user,
        )
        .await
        {
            let _ = tx.rollback().await;
            return ApprovalActionResponse::fail(format!("审计入队失败: {}", e));
        }

        // 4. 意见落库（同事务）——overview 用 LATERAL JOIN 读此表判断状态
        //    时间锚：qk_date → zc_id_scal-date 当日行（flow-process-continuity 规约）
        if let Some(a) = &actor {
            let opinion_text = a.opinion.as_deref().unwrap_or("");
            let date_anchor = match common::scalar::ScalarService::new(pool.clone())
                .find_or_create_date_tx(&mut tx, &chrono::Utc::now().format("%Y-%m-%d").to_string())
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.rollback().await;
                    return ApprovalActionResponse::fail(format!("写入审批意见失败: {}", e));
                }
            };
            if let Err(e) = sqlx::query(
                r#"INSERT INTO isahl."zc_id_deta-opinion"
                   (id, notice, opinion, fk_list, fk_biller, qk_date, created_at, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, NOW(), $6)"#,
            )
            .bind(notice)
            .bind(opinion_text)
            .bind(approval_id)
            .bind(a.user_id)
            .bind(date_anchor)
            .bind(a.user_id)
            .execute(&mut *tx)
            .await
            {
                let _ = tx.rollback().await;
                return ApprovalActionResponse::fail(format!("写入审批意见失败: {}", e));
            }
        }

        if let Err(e) = tx.commit().await {
            return ApprovalActionResponse::fail(format!("提交审批事务失败: {}", e));
        }

        let msg = match status_code {
            "approved" => "已通过",
            "rejected" => "已拒绝",
            _ => "审批状态已更新",
        };

        // Post-approval hook (best-effort, does not roll back the committed transaction)
        if let Some(h) = hook {
            h.on_approval(pool, approval_id, status_code).await;
        }

        ApprovalActionResponse::ok(msg)
    }
}
