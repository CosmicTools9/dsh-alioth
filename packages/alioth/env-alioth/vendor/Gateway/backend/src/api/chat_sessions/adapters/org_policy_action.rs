//! NGAC 权限规范动作（route-ngac-policy-authoring-through-ai-assist 2.2）。
//!
//! 通道：HTTP `POST /api/chat-sessions/{sid}/execute-action`（confirmed 来自用户
//! 点击）→ `GatewayActionHandler::run_action("ngac_org_policy_apply")` → 本模块。
//! 提案来源：ai-agent `ngac_policy_draft` 工具（只出 `needs_confirmation`，零写入）。
//!
//! 门控：`common::permissions::is_admin_user`（与 `require_admin` 同一判据实现；
//! ADR D-051「admin 门控等价保留」）。
//! 写入：SSO org-policy 服务层（`gateway_sso::facade::ngac::org_policy`）——Rhai 强校验
//! （`validate_org_policy_expression`）、状态机（编辑门/顺序迁移）、同事务审计
//! 等价保留。逐项执行、失败即停并明示已完成项（不自动回滚；跨项原子性不在本
//! 通道承诺——逐项审计可追溯）。

use gateway_sso::facade::ngac::org_policy::{
    create_org_policy_class, create_org_policy_label, create_org_policy_rule,
    delete_org_policy_label, delete_org_policy_rule, update_org_policy_class,
    update_org_policy_label, update_org_policy_rule, CreateOrgPolicyClass, CreateOrgPolicyLabel,
    CreateOrgPolicyRule, UpdateOrgPolicyClass, UpdateOrgPolicyLabel, UpdateOrgPolicyRule,
};
use serde_json::{json, Value};
use sqlx::PgPool;

use async_trait::async_trait;

use super::apply_protocol::{apply_operations, gate_admin, ApplyCtx, ItemExecutor};

/// 动作入口：admin 门控 + 逐项执行（协议实现单源 = `apply_protocol`）。
pub async fn apply(pool: &PgPool, user_id: i64, params: &Value) -> Result<Value, String> {
    let gate = gate_admin(pool, user_id, "权限规范写入").await;
    apply_operations(pool, user_id, gate, params, OrgPolicyExecutor).await
}

/// 形状校验后的单操作（服务端 parse 单源；纯函数便于单测）。
#[derive(Debug)]
enum ParsedOp {
    UpsertClass(CreateOrgPolicyClass),
    PatchClass {
        class_id: i64,
        input: UpdateOrgPolicyClass,
    },
    CreateRule(CreateOrgPolicyRule),
    PatchRule {
        rule_id: i64,
        input: UpdateOrgPolicyRule,
    },
    DeleteRule {
        rule_id: i64,
    },
    CreateLabel(CreateOrgPolicyLabel),
    PatchLabel {
        code: String,
        input: UpdateOrgPolicyLabel,
    },
    DeleteLabel {
        code: String,
    },
}

/// 解析 + 形状复校验（不信任上游；与工具层同判据，服务端 fail-closed）。
fn parse_operation(op: &Value) -> Result<(String, ParsedOp), String> {
    let o = op
        .as_object()
        .ok_or_else(|| "operation 必须为 JSON 对象".to_string())?;
    let name = o
        .get("op")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "operation 缺少 op 字段".to_string())?
        .to_string();
    let s = |f: &str| -> Option<String> { o.get(f).and_then(|v| v.as_str()).map(str::to_string) };
    let req_id = |f: &str| -> Result<i64, String> {
        o.get(f)
            .and_then(|v| v.as_i64())
            .ok_or_else(|| format!("{name}: 缺少整数字段 {f}"))
    };
    let opt_i32 = |f: &str| -> Result<Option<i32>, String> {
        match o.get(f).and_then(|v| v.as_i64()) {
            Some(v) => Ok(Some(
                i32::try_from(v).map_err(|_| format!("{name}: {f} 超出 i32 范围"))?,
            )),
            None => Ok(None),
        }
    };
    let actions_opt = |f: &str| -> Option<Vec<String>> {
        o.get(f).and_then(|v| v.as_array()).map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
    };
    // 表达式字段：空串/缺省 = 未设置；非字符串拒绝（标识符级校验在 SSO 服务层）
    let expr = |f: &str| -> Result<Option<Value>, String> {
        match o.get(f) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(t)) if t.trim().is_empty() => Ok(None),
            Some(Value::String(t)) => Ok(Some(Value::String(t.clone()))),
            Some(_) => Err(format!("{name}: {f} MUST 为 Rhai 源码字符串")),
        }
    };

    let parsed = match name.as_str() {
        "upsert_class" => ParsedOp::UpsertClass(CreateOrgPolicyClass {
            code: s("code").ok_or_else(|| format!("{name}: 缺少 code"))?,
            notice: s("notice").ok_or_else(|| format!("{name}: 缺少 notice"))?,
            scope: o.get("scope").cloned(),
            ua_template: o.get("ua_template").cloned(),
            label_code: s("label_code"),
            prohibition_template: expr("prohibition_template")?,
            audit_required: o
                .get("audit_required")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            effective_from: None,
            effective_until: None,
            comments: s("comments"),
        }),
        "patch_class" => ParsedOp::PatchClass {
            class_id: req_id("class_id")?,
            input: UpdateOrgPolicyClass {
                notice: s("notice"),
                scope: o.get("scope").cloned(),
                ua_template: o.get("ua_template").cloned(),
                label_code: s("label_code"),
                prohibition_template: expr("prohibition_template")?,
                audit_required: o.get("audit_required").and_then(|v| v.as_bool()),
                effective_from: None,
                effective_until: None,
                comments: s("comments"),
            },
        },
        "create_rule" => ParsedOp::CreateRule(CreateOrgPolicyRule {
            policy_class_id: req_id("policy_class_id")?,
            subject_code: s("subject_code").ok_or_else(|| format!("{name}: 缺少 subject_code"))?,
            resource_type: s("resource_type")
                .ok_or_else(|| format!("{name}: 缺少 resource_type"))?,
            actions: actions_opt("actions")
                .ok_or_else(|| format!("{name}: 缺少数组字段 actions"))?,
            condition: expr("condition")?,
            obligation: expr("obligation")?,
            label_code: s("label_code"),
        }),
        "patch_rule" => ParsedOp::PatchRule {
            rule_id: req_id("rule_id")?,
            input: UpdateOrgPolicyRule {
                subject_code: s("subject_code"),
                resource_type: s("resource_type"),
                actions: actions_opt("actions"),
                condition: expr("condition")?,
                obligation: expr("obligation")?,
                label_code: s("label_code"),
            },
        },
        "delete_rule" => ParsedOp::DeleteRule {
            rule_id: req_id("rule_id")?,
        },
        "create_label" => ParsedOp::CreateLabel(CreateOrgPolicyLabel {
            code: s("code").ok_or_else(|| format!("{name}: 缺少 code"))?,
            rank: opt_i32("rank")?.ok_or_else(|| format!("{name}: 缺少整数字段 rank"))?,
            domain: s("domain").unwrap_or_else(|| "security".to_string()),
            notice: s("notice").ok_or_else(|| format!("{name}: 缺少 notice"))?,
        }),
        "patch_label" => ParsedOp::PatchLabel {
            code: s("code").ok_or_else(|| format!("{name}: 缺少 code"))?,
            input: UpdateOrgPolicyLabel {
                rank: opt_i32("rank")?,
                domain: s("domain"),
                notice: s("notice"),
            },
        },
        "delete_label" => ParsedOp::DeleteLabel {
            code: s("code").ok_or_else(|| format!("{name}: 缺少 code"))?,
        },
        other => {
            return Err(format!(
                "未知操作 '{other}'（白名单：upsert_class/patch_class/create_rule/patch_rule/delete_rule/create_label/patch_label/delete_label）"
            ));
        }
    };
    Ok((name, parsed))
}

/// 单项执行体（解析 → SSO 服务层；错误文本保留服务层可行动语义）。
struct OrgPolicyExecutor;

#[async_trait]
impl ItemExecutor for OrgPolicyExecutor {
    async fn apply_one(&self, op: &Value, ctx: &ApplyCtx<'_>) -> Result<Value, String> {
        let pool = ctx.pool;
        let uid = ctx.user_id;
        let email = ctx.email.as_str();
        let (name, parsed) = parse_operation(op)?;
        let summary = op
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or(&name)
            .to_string();
        match parsed {
            ParsedOp::UpsertClass(input) => {
                let row = create_org_policy_class(pool, uid, email, input)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(
                    json!({ "op": name, "id": row.id, "code": row.code, "state": row.state, "summary": summary }),
                )
            }
            ParsedOp::PatchClass { class_id, input } => {
                let row = update_org_policy_class(pool, uid, email, class_id, input)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(
                    json!({ "op": name, "id": row.id, "code": row.code, "state": row.state, "summary": summary }),
                )
            }
            ParsedOp::CreateRule(input) => {
                let row = create_org_policy_rule(pool, uid, email, input)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(
                    json!({ "op": name, "id": row.id, "subject_code": row.subject_code, "summary": summary }),
                )
            }
            ParsedOp::PatchRule { rule_id, input } => {
                let row = update_org_policy_rule(pool, uid, email, rule_id, input)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(json!({ "op": name, "id": row.id, "summary": summary }))
            }
            ParsedOp::DeleteRule { rule_id } => {
                let row = delete_org_policy_rule(pool, uid, email, rule_id)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(json!({ "op": name, "id": row.id, "summary": summary }))
            }
            ParsedOp::CreateLabel(input) => {
                let row = create_org_policy_label(pool, uid, email, input)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(json!({ "op": name, "code": row.code, "rank": row.rank, "summary": summary }))
            }
            ParsedOp::PatchLabel { code, input } => {
                let row = update_org_policy_label(pool, uid, email, &code, input)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(json!({ "op": name, "code": row.code, "summary": summary }))
            }
            ParsedOp::DeleteLabel { code } => {
                let row = delete_org_policy_label(pool, uid, email, &code)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(json!({ "op": name, "code": row.code, "summary": summary }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_upsert_class_carries_rhai_expression() {
        let (name, parsed) = parse_operation(&json!({
            "op": "upsert_class", "code": "c1", "notice": "n",
            "scope": {"resource_types": ["invoice"]},
            "prohibition_template": "action == \"export\" && !permitted"
        }))
        .unwrap();
        assert_eq!(name, "upsert_class");
        match parsed {
            ParsedOp::UpsertClass(input) => {
                assert_eq!(input.code, "c1");
                assert_eq!(
                    input.prohibition_template,
                    Some(json!("action == \"export\" && !permitted"))
                );
                assert!(input.audit_required);
            }
            _ => panic!("op 解析类型不符"),
        }
    }

    #[test]
    fn parse_rejects_structured_expression_placeholder() {
        let err = parse_operation(&json!({
            "op": "create_rule", "policy_class_id": 7, "subject_code": "s",
            "resource_type": "r", "actions": ["read"], "condition": {"and": []}
        }))
        .unwrap_err();
        assert!(err.contains("Rhai 源码字符串"), "{err}");
    }

    #[test]
    fn parse_rejects_unknown_op_and_missing_fields() {
        assert!(parse_operation(&json!({"op": "drop_table"}))
            .unwrap_err()
            .contains("白名单"));
        assert!(parse_operation(&json!({"op": "patch_class"}))
            .unwrap_err()
            .contains("class_id"));
        assert!(
            parse_operation(&json!({"op": "create_label", "code": "L1", "rank": 1}))
                .unwrap_err()
                .contains("notice")
        );
    }

    #[test]
    fn parse_empty_string_expression_means_unset() {
        let (_, parsed) = parse_operation(&json!({
            "op": "patch_rule", "rule_id": 3, "condition": "   "
        }))
        .unwrap();
        match parsed {
            ParsedOp::PatchRule { rule_id, input } => {
                assert_eq!(rule_id, 3);
                assert!(input.condition.is_none());
            }
            _ => panic!("op 解析类型不符"),
        }
    }
}
