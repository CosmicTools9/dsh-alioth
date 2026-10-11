use actix_web::{web, HttpRequest, HttpResponse};
use sqlx::PgPool;
use std::collections::HashMap;

use chrono::Utc;

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
    // tighten-pdp-decision-surface A：调用者主体必须等于决策主体
    if let Err(resp) = super::enforce_decision_subject(&req_http, req.user_id) {
        return resp;
    }
    let decision = decide_access(pool.get_ref(), req.user_id, &req.resource, &req.action).await;
    let permitted = decision == Decision::Permit;
    let reason = match decision {
        Decision::Deny => "Access explicitly denied by prohibition or label ceiling".to_string(),
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

    HttpResponse::Ok().json(PdpCheckResponse {
        permitted,
        reason,
        obligations: Vec::new(),
    })
}

/// 批量决策——逐项委托 `decide_access`（唯一语义源）。
///
/// 旧实现内联了第二套遍历/合并（无 admin 兜底、无 bootstrap Permit），与
/// `decide_access` 漂移；现收敛为同源委托，deny-overrides / admin 遍历后兜底 /
/// bootstrap Permit / conditions 求值全部继承。
pub async fn check_access_batch(
    req_http: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<PdpCheckBatchRequest>,
) -> HttpResponse {
    let req = body.into_inner();
    // tighten-pdp-decision-surface A：调用者主体必须等于决策主体
    if let Err(resp) = super::enforce_decision_subject(&req_http, req.user_id) {
        return resp;
    }

    let mut results = Vec::with_capacity(req.checks.len());
    for check in req.checks {
        let decision =
            decide_access(pool.get_ref(), req.user_id, &check.resource, &check.action).await;
        let permitted = decision == Decision::Permit;
        let reason = match decision {
            Decision::Deny => {
                "Access explicitly denied by prohibition or label ceiling".to_string()
            }
            Decision::Permit => "Access granted".to_string(),
            Decision::NotApplicable => "No matching access right found".to_string(),
        };
        results.push(PdpCheckResponse {
            permitted,
            reason,
            obligations: Vec::new(),
        });
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

    // 决策唯一实现 = `ngac_host::Evaluator`（openspec extract-ngac-decision-host）：
    // 输入装配（密级上限 → bootstrap → UA/OA 闭包 → creator 派生）+ 合并 + 决策序
    // 全量委托；`decide_detailed` 一并返回装配真值（义务上下文单源，MUST NOT 二次查询）。
    // 失败（DB / 策略加载）⇒ fail-closed `Deny`（与旧行为一致）。
    let (decision, inputs) = match ngac_host::Evaluator::global()
        .decide_detailed(pool, user_id, resource_type, fk_resource, action)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            log::error!("decide_access: NGAC decide failed: {}", e);
            return Decision::Deny;
        }
    };
    // pre 义务判决参与（NGAC_SPEC §2.6 / add-ngac-pre-obligations）：仅终态 Permit 时叠加，
    // 命中保留算子 `deny` 的义务 ⇒ Deny；零 `phase='pre'` 行 ⇒ EXISTS 短路零成本；fail-safe。
    if decision == Decision::Permit
        && crate::ngac::obligation::evaluate_pre_obligation_denies(
            pool,
            &crate::ngac::obligation::ObligationEvalContext {
                user_id,
                resource,
                action,
                decision: "Permit",
                subject_attributes: &inputs.ua_names,
                object_attributes: &inputs.oa_names,
            },
        )
        .await
    {
        log::info!(
            "decide_access: pre-obligation deny — user={} resource={}",
            user_id,
            resource
        );
        return Decision::Deny;
    }
    decision
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

    // 分级（密级）上限（NGAC_SPEC §2.7）：与 `decide_access` 同位置（**bootstrap 短路之前**）同判据，
    // 否则 explain 会在真实决策 Deny 时报可授权。
    match ngac_host::label_clearance_denies(pool, resource_type, fk_resource, user_id).await {
        Ok(true) => {
            return deny("Denied by label ceiling: subject clearance below resource label");
        }
        Ok(false) => {}
        Err(e) => log::warn!("explain_access: label ceiling unavailable, skipping: {e}"),
    }

    let pip = PostgresPip::new(pool.clone());
    let pdp = Pdp::global();
    if let Err(e) = pdp.ensure_policy_loaded(&pip).await {
        log::error!("explain_access: failed to load policy from database: {}", e);
        return deny("Failed to load policy");
    }

    if pdp.policy_graph().bootstrap_empty() {
        return ExplainResponse {
            permitted: true,
            outcome: "permit".to_string(),
            reason: "Bootstrap phase — no policies configured, all access permitted".to_string(),
            user_attributes: vec![],
            object_attributes: vec![],
            steps: vec![],
        };
    }

    let user_attrs = match pip
        .get_resource_scoped_user_attributes(user_id, resource_type, fk_resource)
        .await
    {
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
        // 切片 B：`explain` 与真决策**同源消费**（同参数 `user_id`/`resource`/`action`）——
        // 否则「explain 说 Permit 而真决策 Deny」的 introspect 分叉（§2.7 同族要求）。
        rhai_eval: Some(ngac_host::rhai_condition_evaluator(
            user_id,
            resource.to_string(),
            action.to_string(),
        )),
    };
    // 合并走唯一实现（`evaluate_merge_in`，NGAC_SPEC §2.5）——明细含全部参与求值的
    // (UA, OA) 对与规则轨迹（不早停），steps 由明细生成；outcome 与真实决策必然一致。
    let is_admin =
        ngac_host::attr_names_include_admin(user_attrs.iter().map(|a| a.o_name.as_str()));
    let pg = pdp.policy_graph();
    let ua_ids: Vec<i64> = user_attrs.iter().map(|a| a.id).collect();
    let oa_ids: Vec<i64> = object_attrs.iter().map(|a| a.id).collect();
    let (merged, outcomes) = pdp.evaluate_merge_in(&pg, &ua_ids, &oa_ids, action, &ctx, is_admin);
    let ua_name: HashMap<i64, &str> = user_attrs
        .iter()
        .map(|a| (a.id, a.o_name.as_str()))
        .collect();
    let oa_by_id: HashMap<i64, &crate::ngac::pip::NgacObjectAttribute> =
        object_attrs.iter().map(|a| (a.id, a)).collect();
    let mut steps: Vec<ExplainStep> = Vec::new();
    for pair in outcomes {
        let Some(oa) = oa_by_id.get(&pair.oa_id) else {
            continue;
        };
        for r in pair.rules {
            steps.push(ExplainStep {
                user_attribute_id: pair.ua_id,
                user_attribute: ua_name
                    .get(&pair.ua_id)
                    .copied()
                    .unwrap_or_default()
                    .to_string(),
                object_attribute_id: pair.oa_id,
                object_attribute: oa.o_name.clone(),
                rule_type: r.rule_type,
                kind: r.kind,
                access_rights: r.access_rights,
                conditions: r.conditions,
                conditions_met: r.conditions_met,
                matched: r.matched,
            });
        }
    }
    // 创建者授权派生（derive-creator-grants-from-lifecycle）：与 decide_access 同序——仅翻转
    // NotApplicable ⇒ Permit（Deny 不可覆盖）；explain 语义 = reason 标注 creator-derived
    //（无物理关联行可引，ADDED 规约）。
    let creator_derived = merged == Decision::NotApplicable
        && ngac_host::creator_derived_permits(pool, user_id, fk_resource, action)
            .await
            .unwrap_or(false);
    // Admin 治理豁免 = 遍历后兜底（仅 NotApplicable 时放行）——已在 evaluate_merge_in 内
    // 与 decide_access 同源；此处仅把决策映射为 explain 的 outcome/reason 文案。
    let admin_exempted = merged == Decision::Permit && is_admin && !steps.iter().any(|s| s.matched);
    // pre 义务判决参与（NGAC_SPEC §2.6 / add-ngac-pre-obligations）：与 decide_access 同口径——
    // 终态放行（Permit，或 creator 派生翻转的 NotApplicable）时叠加；命中保留算子 `deny` ⇒ Deny。
    let effective_permit =
        merged == Decision::Permit || (merged == Decision::NotApplicable && creator_derived);
    let subj_names: Vec<String> = user_attrs.iter().map(|a| a.o_name.clone()).collect();
    let oa_names: Vec<String> = object_attrs.iter().map(|a| a.o_name.clone()).collect();
    let pre_denied = effective_permit
        && crate::ngac::obligation::evaluate_pre_obligation_denies(
            pool,
            &crate::ngac::obligation::ObligationEvalContext {
                user_id,
                resource,
                action,
                decision: "Permit",
                subject_attributes: &subj_names,
                object_attributes: &oa_names,
            },
        )
        .await;
    let (permitted, outcome, reason) = match merged {
        Decision::Permit if admin_exempted => (
            true,
            "permit",
            "Admin attribute exemption — full access".to_string(),
        ),
        Decision::Permit => (
            true,
            "permit",
            "Access granted by matched association".to_string(),
        ),
        Decision::Deny => (
            false,
            "deny",
            "Access denied by matched prohibition".to_string(),
        ),
        Decision::NotApplicable if creator_derived => (
            true,
            "permit",
            "Creator-derived grant (derived at decision time)".to_string(),
        ),
        Decision::NotApplicable => (false, "not_applicable", "No matching policy".to_string()),
    };
    // pre 义务可否决：覆盖上面算出的放行结果（与 decide_access 一致）。
    let (permitted, outcome, reason) = if pre_denied {
        (false, "deny", "Access denied by pre-obligation".to_string())
    } else {
        (permitted, outcome, reason)
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
    // tighten-pdp-decision-surface A：调用者主体必须等于决策主体
    if let Err(resp) = super::enforce_decision_subject(&req_http, req.user_id) {
        return resp;
    }
    let decision = decide_access(pool.get_ref(), req.user_id, &req.resource, &req.action).await;
    let permitted = decision == Decision::Permit;
    let reason = match decision {
        Decision::Deny => "Access explicitly denied by prohibition or label ceiling".to_string(),
        Decision::Permit => "Access granted".to_string(),
        Decision::NotApplicable => "No matching access right found".to_string(),
    };

    // 义务随判决下发（wire-obligations-into-pep-audit / §2.6）：零配置面短路（EXISTS
    // 单查）；有配置才构建求值上下文（UA 名集 = PIP 同源解析、OA 名集 = 闭包单源）——
    // PEP 审计行据此落 `obligations_triggered`，与 SSO 决策审计同源对称。
    let obligations = super::evaluate_response_obligations(
        pool.get_ref(),
        req.user_id,
        &req.resource,
        &req.action,
        &decision,
    )
    .await;

    // 将决策异步写入审计事件表（fire-and-forget），打通 EPP 管道。
    audit_decision(
        pool.clone(),
        req.user_id,
        &req.resource,
        &req.action,
        decision,
        &req_http,
    );

    HttpResponse::Ok().json(ngac_contract::PdpCheckResponse {
        permitted,
        reason,
        obligations,
    })
}
