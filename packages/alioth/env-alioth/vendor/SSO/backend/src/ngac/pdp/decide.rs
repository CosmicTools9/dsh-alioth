use actix_web::{web, HttpRequest, HttpResponse};
use sqlx::PgPool;

use super::*;

use crate::ngac::pip::{Pip, PostgresPip};

/// 旧格式单条决策（`POST /api/ngac/pdp/check`）——委托 `decide_access`（唯一语义源）。
///
/// 旧实现内联了首中即定遍历（无 admin 兜底、无 bootstrap Permit）；现收敛为同源
/// 委托，deny-overrides / admin 遍历后兜底 / bootstrap Permit / conditions 全部继承。
pub async fn check_access(
    req_http: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<PdpCheckRequest>,
) -> HttpResponse {
    let req = body.into_inner();
    let decision = decide_access(pool.get_ref(), req.user_id, &req.resource, &req.action).await;
    let permitted = decision == Decision::Permit;
    let reason = match decision {
        Decision::Deny => "Access explicitly denied by prohibition".to_string(),
        Decision::Permit => "Access granted".to_string(),
        Decision::NotApplicable => "No matching access right found".to_string(),
    };
    audit_decision(
        pool.clone(),
        req.user_id,
        &req.resource,
        &req.action,
        decision,
        &req_http,
    );

    HttpResponse::Ok().json(PdpCheckResponse { permitted, reason })
}

/// 批量决策——逐项委托 `decide_access`（唯一语义源）。
///
/// 旧实现内联了第二套遍历/合并（无 admin 兜底、无 bootstrap Permit），与
/// `decide_access` 漂移；现收敛为同源委托，deny-overrides / admin 遍历后兜底 /
/// bootstrap Permit / conditions 求值全部继承。
pub async fn check_access_batch(
    pool: web::Data<PgPool>,
    body: web::Json<PdpCheckBatchRequest>,
) -> HttpResponse {
    let req = body.into_inner();

    let mut results = Vec::with_capacity(req.checks.len());
    for check in req.checks {
        let decision =
            decide_access(pool.get_ref(), req.user_id, &check.resource, &check.action).await;
        let permitted = decision == Decision::Permit;
        let reason = match decision {
            Decision::Deny => "Access explicitly denied by prohibition".to_string(),
            Decision::Permit => "Access granted".to_string(),
            Decision::NotApplicable => "No matching access right found".to_string(),
        };
        results.push(PdpCheckResponse { permitted, reason });
    }

    HttpResponse::Ok().json(PdpCheckBatchResponse { results })
}

/// 资源级决策核心（含 **bootstrap Permit 兜底**）。
///
/// 被 `ngac_decide` HTTP handler 与 `ngac_pep` 中间件共用，确保 PEP 不复用裸
/// `Pdp::check_access`（其默认 `NotApplicable` 即 deny），从而避免无策略时锁死 SSO 管理员。
///
/// 语义（与 `ngac_decide` 一致）：仅 `Decision::Permit` 视为放行；`Deny` / `NotApplicable` 视为拒绝。
/// 解析失败、DB 错误或策略加载失败均 **fail-closed** 返回 `Decision::Deny`。
pub(crate) async fn decide_access(
    pool: &PgPool,
    user_id: i64,
    resource: &str,
    action: &str,
) -> Decision {
    let resource_parts: Vec<&str> = resource.split(':').collect();
    if resource_parts.len() != 2 {
        log::warn!(
            "decide_access: invalid resource format '{}', expected 'type:id'",
            resource
        );
        return Decision::Deny;
    }
    let (resource_type, resource_id_str) = (resource_parts[0], resource_parts[1]);
    let fk_resource = match resource_id_str.parse::<i64>() {
        Ok(id) => id,
        Err(_) => {
            log::warn!("decide_access: invalid resource id '{}'", resource_id_str);
            return Decision::Deny;
        }
    };

    let pip = PostgresPip::new(pool.clone());
    let pdp = Pdp::global();
    if let Err(e) = pdp.ensure_policy_loaded(&pip).await {
        log::error!("decide_access: failed to load policy from database: {}", e);
        return Decision::Deny;
    }

    // 关键：bootrap 阶段（首次部署未配置任何策略）默认放行，避免首个管理员被锁在 NGAC 之外。
    if !pdp.policy_graph().has_associations() {
        log::info!(
            "decide_access: bootstrap phase — no NGAC associations, permitting user={} resource={}",
            user_id,
            resource
        );
        return Decision::Permit;
    }

    let user_attrs = match pip.get_all_user_attributes_with_inheritance(user_id).await {
        Ok(attrs) => attrs,
        Err(e) => {
            log::error!("decide_access: failed to get user attributes: {}", e);
            return Decision::Deny;
        }
    };
    let object_attrs = match pip
        .get_all_object_attributes_with_inheritance(resource_type, fk_resource)
        .await
    {
        Ok(attrs) => attrs,
        Err(e) => {
            log::error!("decide_access: failed to get object attributes: {}", e);
            return Decision::Deny;
        }
    };

    // Admin 治理豁免改为**遍历后兜底**（fix-ngac-decision-consistency）：仅在
    // 全对遍历无匹配（NotApplicable）时 Permit——admin 绕过的是「无策略」而非
    // 显式 prohibition；prohibition 对 admin 同样生效（彻底 deny-overrides，
    // decide/explain/matrix 三方一致）。与 list.rs admin 全量可见（§6.2）保持
    // 既有分工：列表可见性豁免不变，单条决策 prohibition 优先。
    let is_admin = user_attrs.iter().any(|a| a.o_name == "admin");

    let ctx = ConditionContext {
        now: Utc::now(),
        user_ua_names: user_attrs
            .iter()
            .map(|a| a.o_name.clone())
            .collect::<Vec<_>>(),
        oa_closure_names: object_attrs
            .iter()
            .map(|a| a.o_name.clone())
            .collect::<Vec<_>>(),
    };
    // deny-overrides 全局合并（fix-ngac-decision-consistency）：遍历全部 (UA, OA) 对——
    // 任一对 Deny 即终态（prohibition 优先于一切 association，可早停）；Permit 记录后
    // 继续（必须排除其余对上的 Deny）；全不适用为 NotApplicable。遍历顺序不再影响结果。
    let mut saw_permit = false;
    for user_attr in &user_attrs {
        for obj_attr in &object_attrs {
            match pdp.check_access(user_attr.id, obj_attr.id, action, &ctx) {
                Decision::Deny => return Decision::Deny,
                Decision::Permit => saw_permit = true,
                Decision::NotApplicable => {}
            }
        }
    }
    if saw_permit || is_admin {
        // admin 遍历后兜底：仅无匹配（NotApplicable）时豁免 Permit
        Decision::Permit
    } else {
        Decision::NotApplicable
    }
}

/// 可达性解释：参与决策的属性节点。
#[derive(Debug, Clone, Serialize)]
pub struct ExplainNode {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    pub resource_type: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_resource: Option<i64>,
}

/// 可达性解释：一条参与求值的策略（含命中状态与条件求值结果）。
#[derive(Debug, Clone, Serialize)]
pub struct ExplainStep {
    #[serde(with = "common::serde_zuid")]
    pub user_attribute_id: i64,
    pub user_attribute: String,
    #[serde(with = "common::serde_zuid")]
    pub object_attribute_id: i64,
    pub object_attribute: String,
    pub rule_type: String,
    pub kind: String,
    pub access_rights: Vec<String>,
    pub conditions: Option<serde_json::Value>,
    pub conditions_met: bool,
    pub matched: bool,
}

/// 可达性解释响应（"为什么能/不能"）。
#[derive(Debug, Clone, Serialize)]
pub struct ExplainResponse {
    pub permitted: bool,
    /// "permit" | "deny" | "not_applicable"
    pub outcome: String,
    pub reason: String,
    pub user_attributes: Vec<ExplainNode>,
    pub object_attributes: Vec<ExplainNode>,
    pub steps: Vec<ExplainStep>,
}

/// 决策路径解释：与 `decide_access` 共用 `Pdp::evaluate_pair` 与相同遍历顺序，
/// 因此 outcome 与真实决策必然一致（不允许前端复制一套授权语义）。
pub(crate) async fn explain_access(
    pool: &PgPool,
    user_id: i64,
    resource: &str,
    action: &str,
) -> ExplainResponse {
    let deny = |reason: &str| ExplainResponse {
        permitted: false,
        outcome: "deny".to_string(),
        reason: reason.to_string(),
        user_attributes: vec![],
        object_attributes: vec![],
        steps: vec![],
    };

    let resource_parts: Vec<&str> = resource.split(':').collect();
    if resource_parts.len() != 2 {
        return deny("Invalid resource format. Expected 'type:id'");
    }
    let (resource_type, resource_id_str) = (resource_parts[0], resource_parts[1]);
    let fk_resource = match resource_id_str.parse::<i64>() {
        Ok(id) => id,
        Err(_) => return deny("Invalid resource ID format. Expected integer."),
    };

    let pip = PostgresPip::new(pool.clone());
    let pdp = Pdp::global();
    if let Err(e) = pdp.ensure_policy_loaded(&pip).await {
        log::error!("explain_access: failed to load policy from database: {}", e);
        return deny("Failed to load policy");
    }

    if !pdp.policy_graph().has_associations() {
        return ExplainResponse {
            permitted: true,
            outcome: "permit".to_string(),
            reason: "Bootstrap phase — no policies configured, all access permitted".to_string(),
            user_attributes: vec![],
            object_attributes: vec![],
            steps: vec![],
        };
    }

    let user_attrs = match pip.get_all_user_attributes_with_inheritance(user_id).await {
        Ok(attrs) => attrs,
        Err(e) => {
            log::error!("explain_access: failed to get user attributes: {}", e);
            return deny("Failed to load user attributes");
        }
    };
    let object_attrs = match pip
        .get_all_object_attributes_with_inheritance(resource_type, fk_resource)
        .await
    {
        Ok(attrs) => attrs,
        Err(e) => {
            log::error!("explain_access: failed to get object attributes: {}", e);
            return deny("Failed to load object attributes");
        }
    };

    let ctx = ConditionContext {
        now: Utc::now(),
        user_ua_names: user_attrs
            .iter()
            .map(|a| a.o_name.clone())
            .collect::<Vec<_>>(),
        oa_closure_names: object_attrs
            .iter()
            .map(|a| a.o_name.clone())
            .collect::<Vec<_>>(),
    };
    let mut steps: Vec<ExplainStep> = Vec::new();
    // 与 decide_access 同一 deny-overrides 合并语义（任一 Deny → deny；否则任一
    // Permit → permit）。explain 不早停——全对求值以记录完备 steps（解释"为什么"
    // 需要呈现被 deny 盖住的 allow 边与被 allow 引出的全部候选）。
    let mut saw_deny = false;
    let mut saw_permit = false;
    for ua in &user_attrs {
        for oa in &object_attrs {
            let (decision, rules) = pdp.evaluate_pair(ua.id, oa.id, action, &ctx);
            for r in rules {
                steps.push(ExplainStep {
                    user_attribute_id: ua.id,
                    user_attribute: ua.o_name.clone(),
                    object_attribute_id: oa.id,
                    object_attribute: oa.o_name.clone(),
                    rule_type: r.rule_type,
                    kind: r.kind,
                    access_rights: r.access_rights,
                    conditions: r.conditions,
                    conditions_met: r.conditions_met,
                    matched: r.matched,
                });
            }
            match decision {
                Decision::Deny => saw_deny = true,
                Decision::Permit => saw_permit = true,
                Decision::NotApplicable => {}
            }
        }
    }
    let outcome = if saw_deny {
        "deny"
    } else if saw_permit {
        "permit"
    } else {
        "not_applicable"
    };
    // Admin 治理豁免兜底（与 decide_access 同语义：仅 not_applicable 时放行，
    // 显式 prohibition 对 admin 同样生效——deny-overrides 三方一致）。
    // 放在遍历后：steps 完备记录（含 matched allow/deny 边）供解释。
    let is_admin = user_attrs.iter().any(|a| a.o_name == "admin");
    let (permitted, outcome, reason) = if outcome == "not_applicable" && is_admin {
        (
            true,
            "permit",
            "Admin attribute exemption — full access".to_string(),
        )
    } else {
        (
            outcome == "permit",
            outcome,
            match outcome {
                "permit" => "Access granted by matched association".to_string(),
                "deny" => "Access denied by matched prohibition".to_string(),
                _ => "No matching policy".to_string(),
            },
        )
    };
    ExplainResponse {
        permitted,
        outcome: outcome.to_string(),
        reason,
        user_attributes: user_attrs
            .iter()
            .map(|a| ExplainNode {
                id: a.id,
                o_name: a.o_name.clone(),
                resource_type: None,
                fk_resource: None,
            })
            .collect(),
        object_attributes: object_attrs
            .iter()
            .map(|a| ExplainNode {
                id: a.id,
                o_name: a.o_name.clone(),
                resource_type: Some(a.resource_type.clone()),
                fk_resource: a.fk_resource,
            })
            .collect(),
        steps,
    }
}

pub async fn ngac_decide_explain(
    req_http: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<PdpCheckRequest>,
) -> HttpResponse {
    // 安全：explain 返回策略图路径（属性名/规则/条件），属敏感信息，
    // 仅超级管理员可查询——复用 admin::handlers::require_admin（同一决策语义）。
    let auth_state = match req_http.app_data::<web::Data<crate::auth::AuthState>>() {
        Some(state) => state,
        None => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "AuthState not configured"}));
        }
    };
    let _admin_id = match crate::admin::handlers::require_admin(
        &req_http,
        pool.get_ref(),
        auth_state.get_ref(),
    )
    .await
    {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let req = body.into_inner();
    let resp = explain_access(pool.get_ref(), req.user_id, &req.resource, &req.action).await;
    audit_decision(
        pool.clone(),
        req.user_id,
        &req.resource,
        &req.action,
        if resp.permitted {
            Decision::Permit
        } else {
            Decision::Deny
        },
        &req_http,
    );
    HttpResponse::Ok().json(resp)
}

/// 本人 explain 请求体（add-ngac-self-access-review D1）：不含 user_id——
/// 决策主体恒取 JWT `sub`。
#[derive(Debug, Deserialize)]
pub struct SelfExplainRequest {
    pub resource: String,
    pub action: String,
}

/// `POST /api/ngac/decide/explain/me` — 本人作用域决策解释
/// （add-ngac-self-access-review D1）。
///
/// 与 admin `decide/explain` 同一 `explain_access` 实现（同源同遍历，
/// outcome 与真实决策必然一致）；user_id 强制 JWT 本人，MUST NOT 接受参数注入。
/// 泄露面仅本人 UA 名与本人请求资源的 OA 名/规则（见 design D1 评估）。
pub async fn ngac_decide_explain_self(
    req_http: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<crate::auth::AuthState>,
    body: web::Json<SelfExplainRequest>,
) -> HttpResponse {
    let claims = match crate::auth::jwt::validate_access_token(
        &req_http,
        &state.verification_keys(),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            log::warn!("ngac_decide_explain_self: token validation failed: {}", e);
            return HttpResponse::Unauthorized().json(serde_json::json!({
                "error": "Invalid or missing authentication token"
            }));
        }
    };
    let user_id: i64 = match claims.sub.parse() {
        Ok(id) => id,
        Err(_) => {
            return HttpResponse::Unauthorized().json(serde_json::json!({
                "error": "Invalid token subject"
            }));
        }
    };

    let req = body.into_inner();
    let resp = explain_access(pool.get_ref(), user_id, &req.resource, &req.action).await;
    audit_decision(
        pool.clone(),
        user_id,
        &req.resource,
        &req.action,
        if resp.permitted {
            Decision::Permit
        } else {
            Decision::Deny
        },
        &req_http,
    );
    HttpResponse::Ok().json(resp)
}

pub async fn ngac_decide(
    req_http: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<ngac_contract::PdpCheckRequest>,
) -> HttpResponse {
    let req = body.into_inner();
    let decision = decide_access(pool.get_ref(), req.user_id, &req.resource, &req.action).await;
    let permitted = decision == Decision::Permit;
    let reason = match decision {
        Decision::Deny => "Access explicitly denied by prohibition".to_string(),
        Decision::Permit => "Access granted".to_string(),
        Decision::NotApplicable => "No matching access right found".to_string(),
    };

    // 将决策异步写入审计事件表（fire-and-forget），打通 EPP 管道。
    audit_decision(
        pool.clone(),
        req.user_id,
        &req.resource,
        &req.action,
        decision,
        &req_http,
    );

    HttpResponse::Ok().json(ngac_contract::PdpCheckResponse { permitted, reason })
}
