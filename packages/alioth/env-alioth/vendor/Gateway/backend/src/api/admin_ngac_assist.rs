//! POST /api/admin/ngac/nl-assist — 自然语言 → NGAC 策略操作提案
//! （refactor-ngac-admin-nl-graph D1/D2/D6）。
//!
//! **提案-确认（proposal-only）**：本端点 MUST NOT 执行任何策略写操作——
//! LLM 输出仅生成结构化提案；执行由前端确认后经既有 `/api/admin/ngac/*`
//! CRUD 端点完成（服务端校验 + 策略审计照常）。
//!
//! 快照获取 = 进程内函数复用（design D2）：`gateway_sso::ngac::graph::graph_snapshot`
//! 与 `GET /api/admin/ngac/graph` 同一实现，零 Gateway→SSO 出站调用。
//!
//! 安全：admin 门控复用 `gateway_sso::admin::handlers::require_admin`（同语义唯一实现）；
//! 提案校验 fail-closed——操作白名单 + 全部引用命中快照，非法项标 `invalid`
//! 由前端禁用执行；LLM 不可用/输出不可解析 → 4xx 拒绝（不降级为自由文本建议）。
#![cfg(feature = "sso")]

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use gateway_sso::ngac::graph::{self, GraphSnapshot};

use crate::api::chat_sessions::adapters::db_llm_config::DbLlmConfigAdapter;
use crate::api::chat_sessions::ports::LlmConfigPort;

/// NL 操作白名单（design D5）：边级 + 指派级；不含 UA/OA 节点结构与层级编辑。
const OP_WHITELIST: &[&str] = &[
    "create_association",
    "update_association",
    "delete_association",
    "create_prohibition",
    "update_prohibition",
    "delete_prohibition",
    "assign_ua",
    "remove_ua_assignment",
];

/// 多轮历史截断（最近 N 轮进 prompt）。
const HISTORY_CAP: usize = 6;
/// 用户目录注入上限（LLM 解析 assign_ua 目标用）。
const USERS_CAP: i64 = 200;
/// 实例级 OA 每类型注入上限（上下文压缩：集合级全量、实例级抽样）。
const INSTANCES_PER_TYPE_CAP: usize = 10;

#[derive(Debug, Deserialize)]
pub struct NlHistoryTurn {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct NlAssistRequest {
    pub message: String,
    #[serde(default)]
    pub history: Vec<NlHistoryTurn>,
}

/// LLM 原始操作（serde 宽松解析，字段化校验在 `validate_operations`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NlOperation {
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ua_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oa_id: Option<i64>,
    /// update/delete 目标规则 id（association/prohibition）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<i64>,
    /// assign_ua / remove_ua_assignment 目标用户。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<i64>,
    /// access_rights（名称数组）/ conditions / expires_at。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// 校验失败原因（服务端填充；非 None → 前端禁用该项执行）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid_reason: Option<String>,
}

/// 助手图聚焦指令（refactor-ngac-policy-graph-focus D4）：只读展示指令，
/// 不经操作白名单执行链；id/resource_type 命中快照才透传（fail-closed）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NlFocus {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ua_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub oa_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
}

impl NlFocus {
    fn is_empty(&self) -> bool {
        self.ua_ids.is_empty() && self.oa_ids.is_empty() && self.resource_type.is_none()
    }
}

#[derive(Debug, Serialize)]
pub struct NlAssistResponse {
    pub reply: String,
    pub operations: Vec<NlOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<NlFocus>,
    pub snapshot_version: i64,
    pub warnings: Vec<String>,
}

/// LLM 提案的宽松信封（reply/operations 必备；多余字段忽略）。
#[derive(Debug, Deserialize)]
struct LlmEnvelope {
    reply: Option<String>,
    #[serde(default)]
    operations: Vec<NlOperation>,
    #[serde(default)]
    focus: Option<NlFocus>,
    #[serde(default)]
    warnings: Vec<String>,
}

const SYSTEM_PROMPT: &str = r#"你是 NGAC 权限策略助手。管理员会用自然语言描述授权意图；你的任务是把它解析为结构化操作提案。

严格规则：
1. 只输出一个 JSON 对象，禁止 markdown 代码块、禁止任何 JSON 以外的文本。
2. JSON 结构：{"reply": string, "operations": [...], "focus": {...} | null, "warnings": [string, ...]}
   - reply：面向管理员的中文说明（提案摘要或澄清问题）。
   - operations：操作数组，可为空（意图不明时必须为空并在 reply 中追问）。
   - focus：图聚焦指令（只读展示，用于查询/巡检类诉求）：{"ua_ids": [number, ...], "oa_ids": [number, ...], "resource_type": string}，
     全部字段可省略；管理员问「谁对 X 有权限 / X 有什么冲突 / 看某资源域」这类问题时 operations 必须为空、focus 指向相关实体。
   - warnings：需要注意的风险提示（可空数组）。
3. 每个操作对象字段：
   {"op": "...", "ua_id": number, "oa_id": number, "rule_id": number, "user_id": number,
    "params": {"access_rights": ["read", ...], "conditions": {...}, "expires_at": "ISO8601"},
    "summary": "一句话中文摘要"}
   op ∈ create_association | update_association | delete_association |
             create_prohibition | update_prohibition | delete_prohibition |
             assign_ua | remove_ua_assignment
4. ua_id / oa_id / rule_id / user_id 以及 focus 的 ua_ids / oa_ids / resource_type 只能使用「策略图快照」中出现的 id 与类型，禁止编造。
5. access_rights 只能使用快照 access_rights 目录中的名称。
6. assign_ua / remove_ua_assignment 需要 user_id（用户目录）+ ua_id；
   create/update association/prohibition 需要 ua_id + oa_id；
   update/delete_*_association、update/delete_*_prohibition 需要 rule_id（既有规则 id）。
7. conditions 仅允许 not_before/not_after（ISO8601 时间窗）；不确定就不填。
8. 歧义（同名、不存在的实体、意图不清）→ operations 与 focus 均置空，reply 中列出澄清问题。"#;

/// 从 LLM 输出提取 JSON 文本（容忍围栏/前后噪声；失败返回 None → fail-closed）。
fn extract_json(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim();
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end > start {
        Some(&trimmed[start..=end])
    } else {
        None
    }
}

/// 构建注入 LLM 的紧凑快照上下文（集合级 OA 全量、实例级每类型抽样）。
fn build_snapshot_context(snap: &GraphSnapshot, users: &[(i64, Option<String>)]) -> String {
    use std::fmt::Write;
    let mut ctx = String::with_capacity(8 * 1024);
    let _ = writeln!(ctx, "== 策略版本: {} ==", snap.version);

    let _ = writeln!(ctx, "== 用户属性 UA（id | 名称 | 来源 | 持有者数）==");
    for ua in &snap.user_attributes {
        let src = if ua.derived_from.as_deref() == Some("cognition") {
            "本体认知派生"
        } else {
            "直接指派"
        };
        let _ = writeln!(
            ctx,
            "{} | {} | {} | holders={}",
            ua.id, ua.o_name, src, ua.holder_count
        );
    }

    let _ = writeln!(
        ctx,
        "== 对象属性 OA（按 resource_type 分组；集合级全量，实例级抽样）=="
    );
    let mut types: Vec<&str> = snap
        .object_attributes
        .iter()
        .filter_map(|o| o.resource_type.as_deref())
        .collect();
    types.sort_unstable();
    types.dedup();
    for rt in types {
        let oas: Vec<&gateway_sso::ngac::graph::GraphObjectAttribute> = snap
            .object_attributes
            .iter()
            .filter(|o| o.resource_type.as_deref() == Some(rt))
            .collect();
        let collections: Vec<_> = oas.iter().filter(|o| o.fk_resource == Some(0)).collect();
        let instances: Vec<_> = oas.iter().filter(|o| o.fk_resource != Some(0)).collect();
        let _ = writeln!(ctx, "[{rt}] 实例总数 {}", instances.len());
        for oa in collections {
            let _ = writeln!(
                ctx,
                "  集合 {} | {} (o_name={})",
                oa.id, oa.display_name, oa.o_name
            );
        }
        for oa in instances.iter().take(INSTANCES_PER_TYPE_CAP) {
            let _ = writeln!(
                ctx,
                "  实例 {} | {} (fk_resource={})",
                oa.id,
                oa.display_name,
                oa.fk_resource.unwrap_or(0)
            );
        }
    }

    let _ = writeln!(ctx, "== 既有关联 association（允许）==");
    for a in &snap.associations {
        let _ = writeln!(
            ctx,
            "A#{} | ua={} ({}) -> oa={} ({}) | rights=[{}]",
            a.id,
            a.user_attribute.as_deref().unwrap_or("?"),
            a.fk_user_attribute
                .map(|i| i.to_string())
                .unwrap_or_else(|| "?".into()),
            a.object_attribute.as_deref().unwrap_or("?"),
            a.fk_object_attribute
                .map(|i| i.to_string())
                .unwrap_or_else(|| "?".into()),
            a.access_rights.join(",")
        );
    }
    let _ = writeln!(ctx, "== 既有禁止 prohibition（拒绝）==");
    for p in &snap.prohibitions {
        let _ = writeln!(
            ctx,
            "P#{} | ua={} ({}) -> oa={} ({}) | rights=[{}] active={}",
            p.id,
            p.user_attribute.as_deref().unwrap_or("?"),
            p.fk_user_attribute
                .map(|i| i.to_string())
                .unwrap_or_else(|| "?".into()),
            p.object_attribute.as_deref().unwrap_or("?"),
            p.fk_object_attribute
                .map(|i| i.to_string())
                .unwrap_or_else(|| "?".into()),
            p.access_rights.join(","),
            p.is_active
        );
    }

    let _ = writeln!(
        ctx,
        "== access_rights 目录 == {}",
        snap.access_rights
            .iter()
            .map(|r| r.o_name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let _ = writeln!(ctx, "== 用户目录（id | username，用于 assign_ua）==");
    for (id, username) in users {
        let _ = writeln!(
            ctx,
            "{} | {}",
            id,
            username.as_deref().unwrap_or("(无用户名)")
        );
    }
    ctx
}

/// 操作校验（design D6，fail-closed）：白名单 + 引用命中快照。
/// 非法操作保留在列表中标 `invalid_reason`（前端禁用执行并展示原因）。
pub fn validate_operations(
    mut ops: Vec<NlOperation>,
    snap: &GraphSnapshot,
    users: &[(i64, Option<String>)],
) -> (Vec<NlOperation>, Vec<String>) {
    let mut warnings = Vec::new();
    let ua_ids: std::collections::HashSet<i64> =
        snap.user_attributes.iter().map(|u| u.id).collect();
    let oa_ids: std::collections::HashSet<i64> =
        snap.object_attributes.iter().map(|o| o.id).collect();
    let assoc_ids: std::collections::HashSet<i64> =
        snap.associations.iter().map(|a| a.id).collect();
    let prohib_ids: std::collections::HashSet<i64> =
        snap.prohibitions.iter().map(|p| p.id).collect();
    let user_ids: std::collections::HashSet<i64> = users.iter().map(|(id, _)| *id).collect();
    let right_names: std::collections::HashSet<&str> = snap
        .access_rights
        .iter()
        .map(|r| r.o_name.as_str())
        .collect();

    for op in ops.iter_mut() {
        let mut reason: Option<String> = None;
        if !OP_WHITELIST.contains(&op.op.as_str()) {
            reason = Some(format!("操作 {} 不在白名单内", op.op));
        }
        let need_ua = op.op.starts_with("create_")
            || op.op.starts_with("update_")
            || op.op == "assign_ua"
            || op.op == "remove_ua_assignment";
        if reason.is_none() && need_ua {
            match op.ua_id {
                Some(id) if ua_ids.contains(&id) => {}
                Some(id) => reason = Some(format!("ua_id {} 不在快照中", id)),
                None => reason = Some("缺少 ua_id".to_string()),
            }
        }
        let is_assoc = op.op.contains("association");
        let is_prohib = op.op.contains("prohibition");
        if reason.is_none() && (is_assoc || is_prohib) {
            match op.oa_id {
                Some(id) if oa_ids.contains(&id) => {}
                Some(id) => reason = Some(format!("oa_id {} 不在快照中", id)),
                None if op.op.starts_with("create_") => reason = Some("缺少 oa_id".to_string()),
                None => {}
            }
        }
        if reason.is_none() && (op.op.starts_with("update_") || op.op.starts_with("delete_")) {
            let ids = if is_assoc { &assoc_ids } else { &prohib_ids };
            match op.rule_id {
                Some(id) if ids.contains(&id) => {}
                Some(id) => reason = Some(format!("rule_id {} 不在对应规则集内", id)),
                None => reason = Some("缺少 rule_id".to_string()),
            }
        }
        if reason.is_none() && (op.op == "assign_ua" || op.op == "remove_ua_assignment") {
            match op.user_id {
                Some(id) if user_ids.contains(&id) => {}
                Some(id) => reason = Some(format!("user_id {} 不在用户目录内", id)),
                None => reason = Some("缺少 user_id".to_string()),
            }
        }
        // params 校验：access_rights ⊆ 目录名；conditions 键 ⊆ 白名单
        if reason.is_none() {
            if let Some(params) = op.params.as_ref() {
                if let Some(rights) = params.get("access_rights").and_then(|v| v.as_array()) {
                    for r in rights {
                        if !r.as_str().map(|n| right_names.contains(n)).unwrap_or(false) {
                            reason = Some(format!("access_right {:?} 不在目录中", r));
                            break;
                        }
                    }
                }
                if reason.is_none() {
                    if let Some(cond) = params.get("conditions").and_then(|v| v.as_object()) {
                        let ok_keys = ["not_before", "not_after", "user_attr_in", "object_attr_in"];
                        if cond.keys().any(|k| !ok_keys.contains(&k.as_str())) {
                            reason = Some("conditions 含非法键".to_string());
                        }
                    }
                }
                if reason.is_none() && op.op == "assign_ua" {
                    if let Some(exp) = params.get("expires_at").and_then(|v| v.as_str()) {
                        if chrono::DateTime::parse_from_rfc3339(exp).is_err() {
                            reason = Some("expires_at 非 ISO8601".to_string());
                        }
                    }
                }
            } else if op.op.starts_with("create_") {
                reason = Some("缺少 params（access_rights 等）".to_string());
            }
        }
        if reason.is_some() {
            if let Some(r) = &reason {
                warnings.push(format!("操作 {} 被拒绝：{}", op.op, r));
            }
        }
        op.invalid_reason = reason;
    }
    (ops, warnings)
}

/// focus 校验（refactor-ngac-policy-graph-focus D4，fail-closed）：
/// 未知 ua_id/oa_id/resource_type 剔除并告警；剔除后全空 → None（不透传空指令）。
pub fn validate_focus(
    focus: Option<NlFocus>,
    snap: &GraphSnapshot,
) -> (Option<NlFocus>, Vec<String>) {
    let Some(mut f) = focus else {
        return (None, Vec::new());
    };
    if f.is_empty() {
        return (None, Vec::new());
    }
    let mut warnings = Vec::new();
    let ua_ids: std::collections::HashSet<i64> =
        snap.user_attributes.iter().map(|u| u.id).collect();
    let before = f.ua_ids.len();
    f.ua_ids.retain(|id| ua_ids.contains(id));
    if f.ua_ids.len() != before {
        warnings.push(format!(
            "focus 剔除了 {} 个不在快照中的 ua_id",
            before - f.ua_ids.len()
        ));
    }
    let oa_ids: std::collections::HashSet<i64> =
        snap.object_attributes.iter().map(|o| o.id).collect();
    let before = f.oa_ids.len();
    f.oa_ids.retain(|id| oa_ids.contains(id));
    if f.oa_ids.len() != before {
        warnings.push(format!(
            "focus 剔除了 {} 个不在快照中的 oa_id",
            before - f.oa_ids.len()
        ));
    }
    if let Some(rt) = f.resource_type.take() {
        let known = snap
            .object_attributes
            .iter()
            .any(|o| o.resource_type.as_deref() == Some(rt.as_str()));
        if known {
            f.resource_type = Some(rt);
        } else {
            warnings.push(format!("focus resource_type {} 不在快照中，已剔除", rt));
        }
    }
    if f.is_empty() {
        warnings.push("focus 剔除后为空，已忽略".to_string());
        return (None, warnings);
    }
    (Some(f), warnings)
}

/// 注册 nl-assist 路由（`#[cfg(feature = "sso")]`）。
///
/// 路径前缀 MUST 为 `/api/admin/ngac-assist`（refactor-ngac-admin-nl-graph
/// 冒烟实证 2026-08-27）：actix scope 前缀匹配独占无回落——若挂在
/// `/api/admin/ngac`，会遮蔽 SSO admin scope 的同前缀全部端点（audit-log /
/// impact-preview / associations / prohibitions / pip 等 20+ 条 → 404）。
/// `ngac-assist` 与 SSO 全部 scope 前缀（/api/auth /api/ngac /api/audit
/// /api/admin）无重叠，注册顺序前后皆可（main.rs 保持 protected_routes 之前）。
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api/admin/ngac-assist")
            .wrap(gateway_sso::auth::middleware::NgacPep::new())
            .route("/nl-assist", web::post().to(nl_assist)),
    );
}

/// POST /api/admin/ngac/nl-assist
async fn nl_assist(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<gateway_sso::AuthState>,
    body: web::Json<NlAssistRequest>,
) -> HttpResponse {
    // admin 门控：与既有 /api/admin/* 同语义（require_admin 唯一实现）
    let _admin_id =
        match gateway_sso::admin::handlers::require_admin(&req, pool.get_ref(), state.get_ref())
            .await
        {
            Ok(id) => id,
            Err(resp) => return resp,
        };

    let message = body.message.trim().to_string();
    if message.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({"error": "message 不能为空"}));
    }
    if message.chars().count() > 2000 {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"error": "message 过长（上限 2000 字符）"}));
    }

    // 快照（进程内唯一实现；失败 fail-closed，不返回部分上下文）
    let snap = match graph::graph_snapshot(pool.get_ref()).await {
        Ok(s) => s,
        Err(e) => {
            log::error!("nl_assist: graph snapshot failed: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "策略图快照构建失败"}));
        }
    };

    // 用户目录（assign_ua 目标解析用）
    let users: Vec<(i64, Option<String>)> = match sqlx::query_as(
        "SELECT id, username FROM isahl_auth.auth_users WHERE is_active = true ORDER BY id LIMIT $1",
    )
    .bind(USERS_CAP)
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            log::error!("nl_assist: user directory failed: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "用户目录查询失败"}));
        }
    };

    // LLM 装配（复用 chat 基础设施；不可用 fail-closed）
    let llm = match DbLlmConfigAdapter::new(pool.get_ref().clone())
        .load_service()
        .await
    {
        Ok(s) => s,
        Err(e) => {
            log::error!("nl_assist: LLM unavailable: {}", e);
            return HttpResponse::ServiceUnavailable()
                .json(serde_json::json!({"error": "LLM 服务不可用，提案生成失败（fail-closed）"}));
        }
    };

    // prompt：快照 + 多轮历史（截断）+ 当前诉求
    let mut prompt = String::with_capacity(16 * 1024);
    prompt.push_str("== 策略图快照 ==\n");
    prompt.push_str(&build_snapshot_context(&snap, &users));
    let start = body.history.len().saturating_sub(HISTORY_CAP);
    let history = &body.history[start..];
    if !history.is_empty() {
        prompt.push_str("\n== 对话历史（由旧到新）==\n");
        for turn in history {
            let role = if turn.role == "user" {
                "管理员"
            } else {
                "助手"
            };
            let _ = writeln_fn(&mut prompt, format_args!("{}: {}", role, turn.content));
        }
    }
    let _ = writeln_fn(&mut prompt, format_args!("\n== 管理员诉求 ==\n{}", message));
    let _ = writeln_fn(&mut prompt, format_args!("\n请输出 JSON 提案。"));

    let raw = match llm
        .generate_with_system_preamble(
            SYSTEM_PROMPT,
            &prompt,
            Some(0.2),
            Some(4096),
            None,
            None,
            None,
        )
        .await
    {
        Ok(text) => text,
        Err(e) => {
            log::error!("nl_assist: LLM call failed: {}", e);
            return HttpResponse::BadGateway()
                .json(serde_json::json!({"error": "LLM 调用失败（fail-closed）"}));
        }
    };

    // 解析 + 校验（fail-closed：不可解析 → 拒绝，不降级为自由文本）
    let json_text = match extract_json(&raw) {
        Some(t) => t,
        None => {
            log::warn!("nl_assist: LLM output not JSON (len={})", raw.len());
            return HttpResponse::BadGateway().json(serde_json::json!({
                "error": "LLM 输出不可解析为 JSON（fail-closed），请重述诉求"
            }));
        }
    };
    let envelope: LlmEnvelope = match serde_json::from_str(json_text) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("nl_assist: LLM JSON invalid: {}", e);
            return HttpResponse::BadGateway().json(serde_json::json!({
                "error": "LLM 输出 JSON 结构非法（fail-closed），请重述诉求"
            }));
        }
    };

    let (operations, mut warnings) = validate_operations(envelope.operations, &snap, &users);
    let (focus, focus_warnings) = validate_focus(envelope.focus, &snap);
    warnings.extend(focus_warnings);
    warnings.extend(envelope.warnings);

    HttpResponse::Ok().json(NlAssistResponse {
        reply: envelope.reply.unwrap_or_default(),
        operations,
        focus,
        snapshot_version: snap.version,
        warnings,
    })
}

/// `std::fmt::Write::writeln` 的薄封装（避免函数内 use 冲突）。
fn writeln_fn(buf: &mut String, args: std::fmt::Arguments<'_>) -> std::fmt::Result {
    use std::fmt::Write;
    buf.write_fmt(args)?;
    buf.push('\n');
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_sso::ngac::graph::{
        GraphAccessRight, GraphAssociation, GraphObjectAttribute, GraphPolicyClass,
        GraphProhibition, GraphUserAttribute,
    };

    fn snap_fixture() -> GraphSnapshot {
        let ua = GraphUserAttribute {
            id: 1,
            o_name: "operator".to_string(),
            fk_policy_class: None,
            ancestor_ids: vec![],
            derived_from: None,
            description: None,
            holder_count: 2,
            holders: vec!["alice".to_string()],
        };
        let oa = GraphObjectAttribute {
            id: 10,
            o_name: "engineers-collection".to_string(),
            fk_policy_class: None,
            resource_type: Some("engineers".to_string()),
            fk_resource: Some(0),
            resource_identifier: None,
            display_name: "工程师集合".to_string(),
            preview: None,
            module_name: None,
            module_route: None,
            namespace: None,
            ancestor_ids: vec![],
        };
        let assoc = GraphAssociation {
            id: 100,
            o_name: None,
            fk_user_attribute: Some(1),
            user_attribute: Some("operator".to_string()),
            fk_object_attribute: Some(10),
            object_attribute: Some("engineers-collection".to_string()),
            resource_type: Some("engineers".to_string()),
            ak_access_rights: vec![1],
            access_rights: vec!["read".to_string()],
            fk_policy_class: None,
            conditions: None,
        };
        GraphSnapshot {
            version: 7,
            policy_classes: vec![GraphPolicyClass {
                id: 1,
                o_name: "default".to_string(),
                description: None,
                is_active: Some(true),
            }],
            user_attributes: vec![ua],
            object_attributes: vec![oa],
            associations: vec![assoc],
            prohibitions: vec![GraphProhibition {
                id: 200,
                o_name: None,
                fk_user_attribute: Some(1),
                user_attribute: Some("operator".to_string()),
                fk_object_attribute: Some(10),
                object_attribute: Some("engineers-collection".to_string()),
                resource_type: Some("engineers".to_string()),
                ak_access_rights: vec![1],
                access_rights: vec!["delete".to_string()],
                is_active: true,
                conditions: None,
            }],
            access_rights: vec![
                GraphAccessRight {
                    id: 1,
                    o_name: "read".to_string(),
                    description: None,
                    applicable_types: None,
                },
                GraphAccessRight {
                    id: 2,
                    o_name: "create".to_string(),
                    description: None,
                    applicable_types: None,
                },
            ],
        }
    }

    fn op(
        op: &str,
        ua: Option<i64>,
        oa: Option<i64>,
        params: Option<serde_json::Value>,
    ) -> NlOperation {
        NlOperation {
            op: op.to_string(),
            ua_id: ua,
            oa_id: oa,
            rule_id: None,
            user_id: None,
            params,
            summary: None,
            invalid_reason: None,
        }
    }

    fn users_fixture() -> Vec<(i64, Option<String>)> {
        vec![
            (50, Some("alice".to_string())),
            (51, Some("bob".to_string())),
        ]
    }

    #[test]
    fn whitelist_rejects_structural_ops() {
        let snap = snap_fixture();
        let mut o = op("create_user_attribute", Some(1), None, None);
        o.rule_id = None;
        let (ops, warns) = validate_operations(vec![o], &snap, &users_fixture());
        assert!(
            ops[0].invalid_reason.is_some(),
            "白名单外 op 必须标 invalid"
        );
        assert!(!warns.is_empty());
    }

    #[test]
    fn valid_create_association_passes() {
        let snap = snap_fixture();
        let params = serde_json::json!({"access_rights": ["read", "create"]});
        let o = op("create_association", Some(1), Some(10), Some(params));
        let (ops, warns) = validate_operations(vec![o], &snap, &users_fixture());
        assert!(
            ops[0].invalid_reason.is_none(),
            "合法提案应通过：{:?}",
            ops[0].invalid_reason
        );
        assert!(warns.is_empty());
    }

    #[test]
    fn dangling_reference_marks_invalid() {
        let snap = snap_fixture();
        let params = serde_json::json!({"access_rights": ["read"]});
        let o = op("create_association", Some(999), Some(10), Some(params));
        let (ops, _) = validate_operations(vec![o], &snap, &users_fixture());
        assert!(ops[0]
            .invalid_reason
            .as_deref()
            .unwrap_or("")
            .contains("999"));
    }

    #[test]
    fn unknown_access_right_marks_invalid() {
        let snap = snap_fixture();
        let params = serde_json::json!({"access_rights": ["drop_table"]});
        let o = op("create_association", Some(1), Some(10), Some(params));
        let (ops, _) = validate_operations(vec![o], &snap, &users_fixture());
        assert!(ops[0].invalid_reason.is_some());
    }

    #[test]
    fn update_requires_rule_id_in_matching_set() {
        let snap = snap_fixture();
        // rule_id=200 是 prohibition，用于 association update 应 invalid
        let mut o = op("update_association", Some(1), Some(10), None);
        o.rule_id = Some(200);
        let (ops, _) = validate_operations(vec![o], &snap, &users_fixture());
        assert!(
            ops[0].invalid_reason.is_some(),
            "跨规则集 rule_id 应 invalid"
        );

        let mut o2 = op("update_association", Some(1), Some(10), None);
        o2.rule_id = Some(100);
        let (ops2, _) = validate_operations(vec![o2], &snap, &users_fixture());
        assert!(ops2[0].invalid_reason.is_none());
    }

    #[test]
    fn assign_ua_requires_known_user() {
        let snap = snap_fixture();
        let mut o = op("assign_ua", Some(1), None, Some(serde_json::json!({})));
        o.user_id = Some(50);
        let (ops, _) = validate_operations(vec![o], &snap, &users_fixture());
        assert!(ops[0].invalid_reason.is_none());

        let mut o2 = op("assign_ua", Some(1), None, Some(serde_json::json!({})));
        o2.user_id = Some(999);
        let (ops2, _) = validate_operations(vec![o2], &snap, &users_fixture());
        assert!(ops2[0].invalid_reason.is_some());
    }

    #[test]
    fn bad_expires_at_marks_invalid() {
        let snap = snap_fixture();
        let mut o = op(
            "assign_ua",
            Some(1),
            None,
            Some(serde_json::json!({"expires_at": "not-a-date"})),
        );
        o.user_id = Some(50);
        let (ops, _) = validate_operations(vec![o], &snap, &users_fixture());
        assert!(ops[0].invalid_reason.is_some());
    }

    #[test]
    fn extract_json_strips_fences_and_noise() {
        assert_eq!(extract_json("```json\n{\"a\":1}\n```"), Some("{\"a\":1}"));
        assert_eq!(
            extract_json("前置噪声 {\"a\":1} 尾部噪声"),
            Some("{\"a\":1}")
        );
        assert_eq!(extract_json("完全没有大括号"), None);
        assert_eq!(extract_json(""), None);
    }

    #[test]
    fn focus_valid_passthrough() {
        let snap = snap_fixture();
        let f = NlFocus {
            ua_ids: vec![1],
            oa_ids: vec![10],
            resource_type: Some("engineers".to_string()),
        };
        let (out, warns) = validate_focus(Some(f), &snap);
        assert!(warns.is_empty());
        let f = out.expect("全命中 focus 必须透传");
        assert_eq!(f.ua_ids, vec![1]);
        assert_eq!(f.oa_ids, vec![10]);
        assert_eq!(f.resource_type.as_deref(), Some("engineers"));
    }

    #[test]
    fn focus_drops_unknown_ids() {
        let snap = snap_fixture();
        let f = NlFocus {
            ua_ids: vec![1, 999],
            oa_ids: vec![888],
            resource_type: None,
        };
        let (out, warns) = validate_focus(Some(f), &snap);
        assert_eq!(warns.len(), 2, "ua/oa 各一条剔除告警");
        let f = out.expect("ua_id=1 命中，focus 不整体丢弃");
        assert_eq!(f.ua_ids, vec![1]);
        assert!(f.oa_ids.is_empty());
    }

    #[test]
    fn focus_unknown_resource_type_dropped() {
        let snap = snap_fixture();
        let f = NlFocus {
            ua_ids: vec![],
            oa_ids: vec![10],
            resource_type: Some("nope".to_string()),
        };
        let (out, warns) = validate_focus(Some(f), &snap);
        assert!(warns.iter().any(|w| w.contains("resource_type")));
        let f = out.unwrap();
        assert!(f.resource_type.is_none());
        assert_eq!(f.oa_ids, vec![10]);
    }

    #[test]
    fn focus_empty_normalized_none() {
        let snap = snap_fixture();
        let (out, warns) = validate_focus(None, &snap);
        assert!(out.is_none() && warns.is_empty());

        let f = NlFocus::default();
        let (out, warns) = validate_focus(Some(f), &snap);
        assert!(out.is_none() && warns.is_empty());

        // 全部未知 → 剔除后为空 → None + 告警（不透传空指令）
        let f = NlFocus {
            ua_ids: vec![777],
            oa_ids: vec![],
            resource_type: None,
        };
        let (out, warns) = validate_focus(Some(f), &snap);
        assert!(out.is_none());
        assert!(!warns.is_empty());
    }
}
