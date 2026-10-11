//! NGAC 策略操作动作（wire-nl-assist-into-assistant；写径收敛见 change
//! `consolidate-ngac-write-path`）。
//!
//! 通道：HTTP `POST /api/chat-sessions/{sid}/execute-action`（confirmed 来自用户
//! 点击）→ `GatewayActionHandler::run_action("ngac_strategy_apply")` → 本模块。
//! 提案来源：ai-agent `ngac_strategy_draft` 工具（只出 `needs_confirmation`，零写入）
//! 与 nl-assist 管线（同一操作集）。
//!
//! 门控：`common::permissions::is_admin_user`（与 `require_admin` 同一判据实现）。
//!
//! **不变量（consolidate-ngac-write-path）**：
//! - **写径 = SSO 服务层**：本模块 MUST NOT 直写 `isahl_auth.ngac_*`——全部经
//!   [`gateway_sso::facade::ngac::policy_write`]（同一模块亦服务于 `/api/admin/ngac/*` 端点），
//!   由该层承担「单事务 + 行镜像 + `ngac_policy_audit_log` 审计」，满足活规约
//!   `ngac-admin::nl-assist-assistant-wiring` 的「确认后逐项经 SSO 服务层执行」。
//! - **操作集 = 单一真相源** `ai_agent::tools::ngac_ops::POLICY_OPS`（nl-assist 校验 /
//!   工具层提案校验 / 本模块分支集合三处同源；此前 8 vs 6 分叉）。
//! - **陈旧保护**：`params.snapshot_version` 存在时 MUST 与服务端当前策略图版本一致，
//!   陈旧 MUST 拒绝且零写库（与前端 `ngac-ops.ts` 同判据）。
//! - 授权的 `right` 以**名字**入参，由服务层在事务内解析为 id（未知名字静默丢弃，
//!   与 `grant_rights` 同口径）。

use async_trait::async_trait;
use common::audit::{record_audit_event, Decision};
use gateway_sso::facade::ngac::policy_write as pw;
use serde_json::{json, Value};
use sqlx::PgPool;

use super::apply_protocol::{apply_operations, gate_admin, ApplyCtx, ItemExecutor};

/// 动作入口：admin 门控 → 陈旧校验 → 逐项执行（协议实现单源 = `apply_protocol`）。
pub async fn apply(pool: &PgPool, user_id: i64, params: &Value) -> Result<Value, String> {
    let gate = gate_admin(pool, user_id, "策略操作").await;
    gate.clone()?; // 门控先于陈旧判定（fail-closed：门控失败不触库）
    check_snapshot(pool, params).await?;
    apply_operations(pool, user_id, gate, params, StrategyExecutor).await
}

/// 提案快照陈旧判定（纯函数便于单测）：`Some(v)` 与服务端当前版本不一致 ⇒ 陈旧。
fn snapshot_is_stale(proposed: Option<i64>, current: i64) -> bool {
    matches!(proposed, Some(v) if v != current)
}

/// 陈旧保护：仅在提案携带 `snapshot_version` 时校验（`ngac_strategy_draft` 的提案不带
/// 该键——MUST NOT 让该路径全数失败；nl-assist 提案带）。
pub(crate) async fn check_snapshot(pool: &PgPool, params: &Value) -> Result<(), String> {
    let Some(proposed) = params.get("snapshot_version").and_then(Value::as_i64) else {
        return Ok(());
    };
    let current = gateway_sso::facade::ngac::graph::graph_snapshot(pool)
        .await
        .map_err(|e| format!("策略图快照读取失败: {e}"))?
        .version;
    if snapshot_is_stale(Some(proposed), current) {
        return Err(format!(
            "STALE_PROPOSAL: 提案快照 v{proposed} 已过期（当前 v{current}），请重新生成"
        ));
    }
    Ok(())
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn req_i64(op: &Value, field: &str) -> Result<i64, String> {
    op.get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{field} 必填"))
}

/// 目标 id：`<field>` 优先，兼容 LLM 简写的 `id`。
fn target_id(op: &Value, field: &str) -> Result<i64, String> {
    req_i64(op, field).or_else(|_| req_i64(op, "id"))
}

/// 授权右（**名字**）：键缺席 ⇒ `None`（不改）；存在 ⇒ `Some`（空数组 = 清空）。
fn opt_rights(op: &Value) -> Option<Vec<String>> {
    op.get("actions").map(|_| str_list(op.get("actions")))
}

/// UA/OA id → `(ua_name, resource_type, resource_id)`（只读解析，供图级授权用）。
async fn resolve_resource(
    pool: &PgPool,
    ua_id: i64,
    oa_id: i64,
) -> Result<(String, String, i64), String> {
    sqlx::query_as(
        "SELECT ua.o_name, oa.resource_type, COALESCE(oa.fk_resource, 0) \
         FROM isahl_auth.ngac_user_attribute ua, isahl_auth.ngac_object_attribute oa \
         WHERE ua.id = $1 AND oa.id = $2 AND ua.deleted_at IS NULL AND oa.deleted_at IS NULL",
    )
    .bind(ua_id)
    .bind(oa_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("实体查询失败: {e}"))
}

/// 单项执行体：**全部经 SSO 策略写服务层** + 逐项 `chat_ai.action.execute` 审计。
struct StrategyExecutor;

#[async_trait]
impl ItemExecutor for StrategyExecutor {
    async fn apply_one(&self, op: &Value, ctx: &ApplyCtx<'_>) -> Result<Value, String> {
        let op_type = op.get("op").and_then(Value::as_str).unwrap_or("");
        let actor = pw::Actor::internal(ctx.user_id);
        let result = apply_op(ctx.pool, &actor, op_type, op).await?;
        // 逐项 chat 动作审计（与另两个动作同口径；chat-ai-action-audit）
        let _ = record_audit_event(
            ctx.pool,
            ctx.user_id,
            &ctx.email,
            &format!("ngac-strategy/{op_type}"),
            "chat_ai.action.execute",
            &Decision::Permit,
        )
        .await;
        Ok(result)
    }
}

/// 单操作的领域执行（事务/镜像/审计全部委托服务层）。
async fn apply_op(
    pool: &PgPool,
    actor: &pw::Actor,
    op_type: &str,
    op: &Value,
) -> Result<Value, String> {
    let not_found = |what: &str, id: i64| format!("TARGET_NOT_FOUND: {what} {id}");
    match op_type {
        // ── 关联（association）──
        "create_association" => {
            let ua_id = req_i64(op, "ua_id")?;
            let oa_id = req_i64(op, "oa_id")?;
            let rights = str_list(op.get("actions"));
            let (ua_name, rt, rid) = resolve_resource(pool, ua_id, oa_id).await?;
            let granted = pw::grant_resource_rights(pool, actor, &rt, rid, &ua_name, &rights)
                .await
                .map_err(|e| format!("grant_rights: {e}"))?;
            Ok(json!({
                "op": op_type, "granted": granted, "ua": ua_name,
                "resource": format!("{rt}:{rid}")
            }))
        }
        "update_association" => {
            let id = target_id(op, "association_id")?;
            let changed = pw::update_association(
                pool,
                actor,
                id,
                &pw::AssociationPatch {
                    o_name: op.get("o_name").and_then(Value::as_str).map(str::to_string),
                    rights: opt_rights(op),
                    conditions: op.get("conditions").cloned(),
                },
            )
            .await
            .map_err(|e| format!("update_association: {e}"))?;
            if !changed {
                return Err(not_found("association", id));
            }
            Ok(json!({ "op": op_type, "association_id": id.to_string() }))
        }
        "delete_association" => {
            let id = target_id(op, "association_id")?;
            let changed = pw::delete_association(pool, actor, id)
                .await
                .map_err(|e| format!("delete_association: {e}"))?;
            if !changed {
                return Err(not_found("association", id));
            }
            Ok(json!({ "op": op_type, "association_id": id.to_string() }))
        }
        // ── 禁止（prohibition）──
        "create_prohibition" => {
            let ua_id = req_i64(op, "ua_id")?;
            let oa_id = req_i64(op, "oa_id")?;
            let rights = str_list(op.get("actions"));
            let id = pw::create_prohibition(pool, actor, ua_id, oa_id, &rights)
                .await
                .map_err(|e| format!("create_prohibition: {e}"))?;
            Ok(json!({ "op": op_type, "prohibition_id": id.to_string() }))
        }
        "update_prohibition" => {
            let id = target_id(op, "prohibition_id")?;
            let changed = pw::update_prohibition(
                pool,
                actor,
                id,
                &pw::ProhibitionPatch {
                    rights: opt_rights(op),
                    is_active: op.get("is_active").and_then(Value::as_bool),
                    conditions: op.get("conditions").cloned(),
                },
            )
            .await
            .map_err(|e| format!("update_prohibition: {e}"))?;
            if !changed {
                return Err(not_found("prohibition", id));
            }
            Ok(json!({ "op": op_type, "prohibition_id": id.to_string() }))
        }
        "delete_prohibition" => {
            let id = target_id(op, "prohibition_id")?;
            let changed = pw::delete_prohibition(pool, actor, id)
                .await
                .map_err(|e| format!("delete_prohibition: {e}"))?;
            if !changed {
                return Err(not_found("prohibition", id));
            }
            Ok(json!({ "op": op_type, "prohibition_id": id.to_string() }))
        }
        // ── UA 指派 ──
        "assign_ua" => {
            let user_id = req_i64(op, "user_id")?;
            let ua_id = req_i64(op, "ua_id")?;
            let assigned = pw::assign_user_attribute(pool, actor, user_id, ua_id)
                .await
                .map_err(|e| format!("assign_ua: {e}"))?;
            Ok(json!({
                "op": op_type, "user_id": user_id.to_string(), "ua_id": ua_id.to_string(),
                "already_assigned": !assigned
            }))
        }
        "remove_ua_assignment" => {
            let user_id = req_i64(op, "user_id")?;
            let ua_id = req_i64(op, "ua_id")?;
            let removed = pw::remove_user_attribute(pool, actor, user_id, ua_id)
                .await
                .map_err(|e| format!("remove_ua_assignment: {e}"))?;
            if !removed {
                return Err(format!(
                    "TARGET_NOT_FOUND: user {user_id} 未绑定 UA {ua_id}"
                ));
            }
            Ok(json!({ "op": op_type, "user_id": user_id.to_string(), "ua_id": ua_id.to_string() }))
        }
        other => Err(format!("UNKNOWN_OP: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 陈旧判定纯函数：仅在提案携带版本且不一致时为陈旧。
    #[test]
    fn snapshot_staleness_semantics() {
        assert!(snapshot_is_stale(Some(3), 4), "版本不一致 ⇒ 陈旧");
        assert!(!snapshot_is_stale(Some(4), 4), "版本一致 ⇒ 放行");
        assert!(
            !snapshot_is_stale(None, 4),
            "无版本（draft 工具提案）⇒ 不校验"
        );
    }

    /// 操作集单一来源：本模块分支 = 共享常量（新增分支须同步该常量）。
    #[test]
    fn op_set_matches_shared_constant() {
        let ops = ai_agent::tools::ngac_ops::POLICY_OPS;
        for expected in [
            "create_association",
            "update_association",
            "delete_association",
            "create_prohibition",
            "update_prohibition",
            "delete_prohibition",
            "assign_ua",
            "remove_ua_assignment",
        ] {
            assert!(ops.contains(&expected), "共享常量缺 {expected}");
        }
        assert_eq!(ops.len(), 8, "共享常量条数固定为 8（改动须同批改本测试）");
    }

    /// 权限键语义：缺席 = 不改（None）≠ 空数组 = 清空（Some(vec![])）。
    #[test]
    fn rights_absence_differs_from_empty() {
        assert!(opt_rights(&json!({ "op": "update_association" })).is_none());
        assert_eq!(
            opt_rights(&json!({ "op": "update_association", "actions": [] })),
            Some(Vec::new())
        );
        assert_eq!(
            opt_rights(&json!({ "actions": ["read", "write"] })),
            Some(vec!["read".to_string(), "write".to_string()])
        );
    }

    /// 目标 id：`<field>` 优先，兼容 `id` 简写。
    #[test]
    fn target_id_accepts_field_or_generic_id() {
        assert_eq!(
            target_id(&json!({ "association_id": 7 }), "association_id").unwrap(),
            7
        );
        assert_eq!(target_id(&json!({ "id": 9 }), "association_id").unwrap(), 9);
        assert!(target_id(&json!({}), "association_id").is_err());
    }
}
