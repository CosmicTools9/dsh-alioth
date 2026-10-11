//! NGAC 图分析面（openspec change `establish-age-graph-analysis-plane` 任务 3.x/5.2；
//! 规约 `openspec/specs/ngac-graph-analysis`）。
//!
//! 只读分析端点族（挂 `/api/admin/ngac/analysis/*`，`require_admin` 门控；不写任何
//! 策略数据——dry-run 在 `ngac-decision` 克隆图上求值）+ SoD 冲突对管理面
//! （`/api/admin/ngac/sod-pairs*`，写径含审计）。
//!
//! - `POST /analysis/sod`：SoD 冲突检测（关系单源：`effective_ua_ids` 双侧 EXISTS）
//! - `POST /analysis/reachability`：OA+action → 可达 (user, UA) 枚举（可达性语义，**不产 Permit/Deny**）
//! - `POST /analysis/explain-paths`：explain 决策的图路径证据（关系单源直读，恒不降级）
//! - `POST /analysis/dry-run`：候选策略补丁 → 克隆图前后受影响清单（零写库）
//!
//! **零 Cypher（2026-10-10 起）**：本模块无任何 AGE Cypher 调用面（NGAC 侧图投影已随
//! change `retire-ngac-age-graph-projection` 退役）⇒ `NGAC_SPEC §11.3` 登记的上游缺陷族
//! 在本面**无暴露**；AGE 读面仅存于知识域（经 `common::age` 三态准入）。
//!
//! **fail-visible**：分析响应统一附 `projection`（`DriftReport`）。契约字段形状保留
//! （语义 = 关系单源当前态：恒就绪 / 零漂移 / `stale=false`），不以陈旧结论形态静默返回。

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

use crate::admin::handlers::require_admin;
use crate::auth::AuthState;
use ngac_decision::{PolicyGraph, PolicyPatch};
use ngac_host::oa_closure_rows;

use crate::ngac::freshness::DriftReport;
use crate::ngac::pdp::Pdp;

// ── 公共面 ───────────────────────────────────────────────────────────────────

/// 数据新鲜度面（响应统一附带）。NGAC 侧图投影已退役
/// （change `retire-ngac-age-graph-projection`）：分析面直接读**关系单源**
/// （`ngac_association` / `ngac_prohibition` / `effective_ua_ids`）⇒ 恒为当前值，
/// 无投影可漂移。字段形态保持不变（前端契约稳定），恒报 ready + 零漂移。
fn projection_health() -> DriftReport {
    DriftReport::fresh()
}

/// admin 门控（`require_admin` 唯一判据；失败直接回错误响应）。
async fn guard_admin(req: &HttpRequest, pool: &PgPool) -> Result<i64, HttpResponse> {
    let state = req
        .app_data::<web::Data<AuthState>>()
        .cloned()
        .expect("AuthState in app data");
    require_admin(req, pool, &state).await
}

// ── SoD 冲突对管理面（任务 3.2：配置 CRUD + 审计）────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SodPair {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    #[serde(with = "common::serde_zuid")]
    pub fk_ua_a: i64,
    #[serde(with = "common::serde_zuid")]
    pub fk_ua_b: i64,
    pub is_active: bool,
}

#[derive(Debug, Deserialize)]
pub struct SodPairCreateRequest {
    o_name: String,
    #[serde(with = "common::serde_zuid")]
    fk_ua_a: i64,
    #[serde(with = "common::serde_zuid")]
    fk_ua_b: i64,
}

/// GET /api/admin/ngac/sod-pairs —— 冲突对清单（含 UA 名）。
pub async fn list_sod_pairs(req: HttpRequest, pool: web::Data<PgPool>) -> HttpResponse {
    if let Err(resp) = guard_admin(&req, pool.get_ref()).await {
        return resp;
    }
    let rows: Vec<(i64, String, i64, i64, bool, String, String)> = match sqlx::query_as(
        "SELECT p.id, p.o_name, p.fk_ua_a, p.fk_ua_b, p.is_active, a.o_name, b.o_name \
         FROM isahl_auth.ngac_sod_pair p \
         JOIN isahl_auth.ngac_user_attribute a ON a.id = p.fk_ua_a \
         JOIN isahl_auth.ngac_user_attribute b ON b.id = p.fk_ua_b \
         WHERE p.deleted_at IS NULL ORDER BY p.id",
    )
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": format!("sod pairs: {e}") }))
        }
    };
    HttpResponse::Ok().json(serde_json::json!({
        "pairs": rows.into_iter().map(|(id, o_name, a, b, is_active, a_name, b_name)| serde_json::json!({
            "id": id, "o_name": o_name, "fk_ua_a": a, "fk_ua_b": b,
            "is_active": is_active, "ua_a_name": a_name, "ua_b_name": b_name,
        })).collect::<Vec<_>>(),
    }))
}

/// POST /api/admin/ngac/sod-pairs —— 新建冲突对（同对无序去重；审计随 admin 写链）。
pub async fn create_sod_pair(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<SodPairCreateRequest>,
) -> HttpResponse {
    let admin_id = match guard_admin(&req, pool.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let b = body.into_inner();
    if b.o_name.trim().is_empty() || b.fk_ua_a == b.fk_ua_b {
        return HttpResponse::BadRequest().json(serde_json::json!({ "error": "INVALID_PAIR" }));
    }
    // 无序对 (a,b)/(b,a) 视为同一冲突对——重复 ⇒ 409
    let dup: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_sod_pair WHERE deleted_at IS NULL \
         AND ((fk_ua_a = $1 AND fk_ua_b = $2) OR (fk_ua_a = $2 AND fk_ua_b = $1))",
    )
    .bind(b.fk_ua_a)
    .bind(b.fk_ua_b)
    .fetch_optional(pool.get_ref())
    .await
    .unwrap_or(None);
    if dup.is_some() {
        return HttpResponse::Conflict().json(serde_json::json!({ "error": "DUPLICATE_PAIR" }));
    }
    let valid: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.ngac_user_attribute \
         WHERE id IN ($1, $2) AND deleted_at IS NULL",
    )
    .bind(b.fk_ua_a)
    .bind(b.fk_ua_b)
    .fetch_one(pool.get_ref())
    .await
    .unwrap_or(0);
    if valid != 2 {
        return HttpResponse::BadRequest().json(serde_json::json!({ "error": "UA_NOT_FOUND" }));
    }
    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": format!("tx: {e}") }))
        }
    };
    let id: i64 = match sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_sod_pair (o_name, fk_ua_a, fk_ua_b, created_by_id) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(&b.o_name)
    .bind(b.fk_ua_a)
    .bind(b.fk_ua_b)
    .bind(admin_id)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": format!("insert: {e}") }))
        }
    };
    // 策略审计（同事务；entity_type 扩面 = SoD 冲突对，规约 ngac-graph-analysis）
    let audit = crate::ngac::audit_writer::AuditRecord {
        action: "insert",
        entity_type: "ngac_sod_pair",
        entity_id: id,
        old_values: None,
        new_values: Some(serde_json::json!({
            "o_name": b.o_name, "fk_ua_a": b.fk_ua_a, "fk_ua_b": b.fk_ua_b,
        })),
        actor: admin_id,
        session_id: None,
        ip_address: None,
    };
    if let Err(e) = crate::ngac::audit_writer::write_audit_tx(&mut tx, &audit).await {
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({ "error": format!("audit: {e}") }));
    }
    if let Err(e) = tx.commit().await {
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({ "error": format!("commit: {e}") }));
    }
    HttpResponse::Created().json(serde_json::json!({ "id": id.to_string() }))
}

/// DELETE /api/admin/ngac/sod-pairs/{id} —— 软删冲突对（同事务审计）。
pub async fn delete_sod_pair(req: HttpRequest, pool: web::Data<PgPool>) -> HttpResponse {
    let admin_id = match guard_admin(&req, pool.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let id: i64 = match req.match_info().get("id").and_then(|v| v.parse().ok()) {
        Some(v) => v,
        None => {
            return HttpResponse::BadRequest().json(serde_json::json!({ "error": "INVALID_ID" }))
        }
    };
    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": format!("tx: {e}") }))
        }
    };
    let affected = match sqlx::query(
        "UPDATE isahl_auth.ngac_sod_pair SET deleted_at = NOW(), updated_by_id = $2 \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(admin_id)
    .execute(&mut *tx)
    .await
    {
        Ok(r) => r.rows_affected(),
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": format!("delete: {e}") }))
        }
    };
    if affected == 0 {
        return HttpResponse::NotFound().json(serde_json::json!({ "error": "NOT_FOUND" }));
    }
    let audit = crate::ngac::audit_writer::AuditRecord {
        action: "delete",
        entity_type: "ngac_sod_pair",
        entity_id: id,
        old_values: None,
        new_values: None,
        actor: admin_id,
        session_id: None,
        ip_address: None,
    };
    if let Err(e) = crate::ngac::audit_writer::write_audit_tx(&mut tx, &audit).await {
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({ "error": format!("audit: {e}") }));
    }
    if let Err(e) = tx.commit().await {
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({ "error": format!("commit: {e}") }));
    }
    HttpResponse::Ok().json(serde_json::json!({ "id": id.to_string(), "deleted": true }))
}

// ── SoD 冲突检测（任务 3.2）──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct SodHit {
    pair_id: i64,
    pair_name: String,
    ua_a: i64,
    ua_a_name: String,
    ua_b: i64,
    ua_b_name: String,
    user_id: i64,
    user_email: String,
}

#[derive(Debug, Serialize)]
struct SodResponse {
    hits: Vec<SodHit>,
    /// 恒 `"sql"`（关系单源直读；NGAC 侧图投影已退役，无降级面）。
    mode: &'static str,
    projection: DriftReport,
    stale: bool,
}

/// SoD 冲突检测：关系单源判定（`effective_ua_ids` 双侧 EXISTS 命中同一用户；NGAC 侧图投影已退役）；
/// 语义 = 只读关注信号（MUST NOT 产生任何阻断或自动写策略动作）。
pub async fn sod_analysis(req: HttpRequest, pool: web::Data<PgPool>) -> HttpResponse {
    if let Err(resp) = guard_admin(&req, pool.get_ref()).await {
        return resp;
    }
    let projection = projection_health();

    // NGAC 侧图投影已退役（retire-ngac-age-graph-projection）：SoD 冲突检测直接走
    // **关系单源**——同一用户有效 UA 闭包（直派∪认知∪委托∪祖先）内冲突对两侧命中。
    type SodRow = (i64, String, i64, i64, String, String, i64, String);
    let rows: Vec<SodRow> = match sqlx::query_as(
        "SELECT p.id, p.o_name, p.fk_ua_a, p.fk_ua_b, a.o_name, b.o_name, u.id, \
                COALESCE((SELECT e.email FROM isahl_auth.auth_user_emails e \
                          WHERE e.fk_user = u.id AND e.deleted_at IS NULL \
                          ORDER BY e.is_primary DESC LIMIT 1), u.username) AS email \
         FROM isahl_auth.ngac_sod_pair p \
         JOIN isahl_auth.ngac_user_attribute a ON a.id = p.fk_ua_a \
         JOIN isahl_auth.ngac_user_attribute b ON b.id = p.fk_ua_b \
         CROSS JOIN isahl_auth.auth_users u \
         WHERE p.is_active AND p.deleted_at IS NULL \
           AND EXISTS (SELECT 1 FROM isahl_auth.effective_ua_ids(u.id) e WHERE e.ua_id = p.fk_ua_a) \
           AND EXISTS (SELECT 1 FROM isahl_auth.effective_ua_ids(u.id) e WHERE e.ua_id = p.fk_ua_b)",
    )
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": format!("sod pairs: {e}") }))
        }
    };
    let hits: Vec<SodHit> = rows
        .into_iter()
        .map(
            |(pair_id, pair_name, ua_a, ua_b, a_name, b_name, user_id, user_email)| SodHit {
                pair_id,
                pair_name,
                ua_a,
                ua_a_name: a_name,
                ua_b,
                ua_b_name: b_name,
                user_id,
                user_email,
            },
        )
        .collect();

    let stale = projection.stale();
    HttpResponse::Ok().json(SodResponse {
        hits,
        mode: "sql",
        projection,
        stale,
    })
}

// ── 可达性枚举（任务 3.1）────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ReachabilityRequest {
    resource_type: String,
    /// 缺省 0 = 集合级
    #[serde(default)]
    fk_resource: i64,
    action: String,
}

#[derive(Debug, Serialize)]
struct ReachableUser {
    user_id: i64,
    user_email: String,
    /// 命中授权边的 UA 名集（闭包内，可能多个）
    via_ua_names: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ReachabilityResponse {
    /// 可达性语义：闭包 UA 经 association 到达目标 OA 闭包且授予 action——
    /// **不等于 Permit**（prohibition/密级/条件不在本判定面；逐请求决策走 PDP）。
    reachable: Vec<ReachableUser>,
    mode: &'static str,
    projection: DriftReport,
    stale: bool,
}

/// 可达性枚举：OA 闭包（`ngac_org::oa_closure_rows` 单源）× 内存策略图 association 边
/// × 用户闭包（`effective_ua_ids` 单源 SQL）。主径 = 内存图 + SQL（图数据为关系单源的
/// 内存派生，与决策热路径同源）；路径证据归 `explain-paths` 端点（同族关系单源枚举）。
pub async fn reachability(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<ReachabilityRequest>,
) -> HttpResponse {
    let body = body.into_inner();
    if let Err(resp) = guard_admin(&req, pool.get_ref()).await {
        return resp;
    }
    let projection = projection_health();

    let pip = crate::ngac::pip::PostgresPip::new(pool.get_ref().clone());
    let pdp = Pdp::global();
    if let Err(e) = pdp.ensure_policy_loaded(&pip).await {
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({ "error": format!("policy load: {e}") }));
    }
    let pg = pdp.policy_graph();
    let oa_rows = match oa_closure_rows(pool.get_ref(), &body.resource_type, body.fk_resource).await
    {
        Ok(r) => r,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": format!("oa closure: {e}") }))
        }
    };
    let oa_ids: Vec<i64> = oa_rows.iter().map(|r| r.0).collect();

    // 授予该 action 的 (UA → OA 闭包) 边集（内存图，与决策热路径同一数据面）——
    // 反向枚举唯一实现 = ngac-decision 索引原语（门禁 P6：宿主机 MUST NOT 直接迭代策略图）。
    let indexed = ngac_decision::IndexedPolicyGraph::from(&pg);
    let ua_ids: Vec<i64> = indexed.uas_granting(&body.action, &oa_ids);

    let reachable = if ua_ids.is_empty() {
        Vec::new()
    } else {
        let rows: Vec<(i64, String, String)> = match sqlx::query_as(
            "SELECT DISTINCT u.id, \
                    COALESCE((SELECT e.email FROM isahl_auth.auth_user_emails e \
                              WHERE e.fk_user = u.id AND e.deleted_at IS NULL \
                              ORDER BY e.is_primary DESC LIMIT 1), u.username) AS email, ua.o_name \
             FROM isahl_auth.auth_users u \
             CROSS JOIN isahl_auth.ngac_user_attribute ua \
             WHERE ua.id = ANY($1) \
               AND EXISTS (SELECT 1 FROM isahl_auth.effective_ua_ids(u.id) e WHERE e.ua_id = ua.id)",
        )
        .bind(&ua_ids)
        .fetch_all(pool.get_ref())
        .await
        {
            Ok(r) => r,
            Err(e) => {
                return HttpResponse::InternalServerError()
                    .json(serde_json::json!({ "error": format!("user closure: {e}") }))
            }
        };
        let mut by_user: Vec<ReachableUser> = Vec::new();
        for (uid, email, ua_name) in rows {
            match by_user.iter_mut().find(|r| r.user_id == uid) {
                Some(entry) => entry.via_ua_names.push(ua_name),
                None => by_user.push(ReachableUser {
                    user_id: uid,
                    user_email: email,
                    via_ua_names: vec![ua_name],
                }),
            }
        }
        by_user
    };

    let stale = projection.stale();
    HttpResponse::Ok().json(ReachabilityResponse {
        reachable,
        mode: "graph+sql",
        projection,
        stale,
    })
}

// ── explain 路径证据（任务 3.3）──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ExplainPathsRequest {
    pub resource: String,
    /// 契约字段（与 explain 请求同构）：当前路径证据按 OA 闭包枚举、不按 action
    /// 过滤——保留入参兼容 explain 调用方直接复用请求体。
    pub action: String,
}

#[derive(Debug, Serialize)]
pub struct ExplainPathsResponse {
    /// 图路径证据：UA -[ASSOCIATION|PROHIBITION]-> OA（属性：边类/权利/两端名）。
    pub paths: Vec<Value>,
    /// 降级 ⇒ true（explain 决策本身不受影响——决策走 crate，路径仅证据补充）。
    pub paths_degraded: bool,
    pub projection: DriftReport,
    pub stale: bool,
}

/// explain-paths 错误分类（HTTP 400 / 500 与工具面共用）。
pub enum ExplainPathsError {
    BadRequest(String),
    Internal(String),
}

/// 路径证据核心（**HTTP 端点与 chat 工具共用**，只读）。
pub async fn explain_paths_evidence(
    pool: &PgPool,
    body: &ExplainPathsRequest,
) -> Result<ExplainPathsResponse, ExplainPathsError> {
    let projection = projection_health();

    let resource_parts: Vec<&str> = body.resource.split(':').collect();
    if resource_parts.len() != 2 {
        return Err(ExplainPathsError::BadRequest(
            "INVALID_RESOURCE".to_string(),
        ));
    }
    let (rt, id) = (resource_parts[0], resource_parts[1]);
    let fk_resource: i64 = match id.parse() {
        Ok(v) => v,
        Err(_) => {
            return Err(ExplainPathsError::BadRequest(
                "INVALID_RESOURCE_ID".to_string(),
            ))
        }
    };
    let oa_rows = match oa_closure_rows(pool, rt, fk_resource).await {
        Ok(r) => r,
        Err(e) => return Err(ExplainPathsError::Internal(format!("oa closure: {e}"))),
    };
    let oa_ids: Vec<i64> = oa_rows.iter().map(|r| r.0).collect();

    // NGAC 侧图投影已退役（retire-ngac-age-graph-projection）：路径证据直读**关系单源**
    // （`ngac_association` + `ngac_prohibition`），与决策同一真相，恒不降级。
    type PathRow = (i64, String, String, Option<String>, i64, String);
    let rows: Vec<PathRow> = sqlx::query_as(
        "SELECT DISTINCT ua.id, COALESCE(ua.o_name, ''), 'ASSOCIATION'::text, \
                (SELECT string_agg(r.o_name, ',' ORDER BY r.o_name) \
                   FROM isahl_auth.ngac_access_right r \
                  WHERE r.id = ANY(a.ak_access_rights)), oa.id, COALESCE(oa.o_name, '') \
           FROM isahl_auth.ngac_association a \
           JOIN isahl_auth.ngac_user_attribute ua ON ua.id = a.fk_user_attribute AND ua.deleted_at IS NULL \
           JOIN isahl_auth.ngac_object_attribute oa ON oa.id = a.fk_object_attribute AND oa.deleted_at IS NULL \
          WHERE a.deleted_at IS NULL AND oa.id = ANY($1) \
         UNION \
         SELECT DISTINCT ua.id, COALESCE(ua.o_name, ''), 'PROHIBITION'::text, \
                (SELECT string_agg(r.o_name, ',' ORDER BY r.o_name) \
                   FROM isahl_auth.ngac_access_right r \
                  WHERE r.id = ANY(COALESCE(p.ak_access_rights, ARRAY[]::bigint[]))), oa.id, COALESCE(oa.o_name, '') \
           FROM isahl_auth.ngac_prohibition p \
           JOIN isahl_auth.ngac_user_attribute ua ON ua.id = p.fk_user_attribute AND ua.deleted_at IS NULL \
           JOIN isahl_auth.ngac_object_attribute oa ON oa.id = p.fk_object_attribute AND oa.deleted_at IS NULL \
          WHERE p.deleted_at IS NULL AND p.is_active AND oa.id = ANY($1)",
    )
    .bind(&oa_ids)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let paths: Vec<Value> = rows
        .into_iter()
        .map(|(ua_id, ua_name, kind, rights, oa_id, oa_name)| {
            serde_json::json!({
                "uaId": ua_id.to_string(),
                "uaName": ua_name,
                "kind": kind,
                "rights": rights,
                "oaId": oa_id.to_string(),
                "oaName": oa_name,
            })
        })
        .collect();

    let stale = projection.stale();
    Ok(ExplainPathsResponse {
        paths,
        paths_degraded: false,
        projection,
        stale,
    })
}

/// explain 的路径证据：关系单源只读枚举（association + prohibition 的 UA→OA 边；
/// `oa_ids` 为参数化常量列表）。NGAC 侧图投影已退役 ⇒ 恒不降级。
pub async fn explain_paths(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<ExplainPathsRequest>,
) -> HttpResponse {
    if let Err(resp) = guard_admin(&req, pool.get_ref()).await {
        return resp;
    }
    match explain_paths_evidence(pool.get_ref(), &body.into_inner()).await {
        Ok(resp) => HttpResponse::Ok().json(resp),
        Err(ExplainPathsError::BadRequest(e)) => {
            HttpResponse::BadRequest().json(serde_json::json!({ "error": e }))
        }
        Err(ExplainPathsError::Internal(e)) => {
            HttpResponse::InternalServerError().json(serde_json::json!({ "error": e }))
        }
    }
}

// ── dry-run 仿真（任务 5.2）──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DryRunPatch {
    pub kind: String,
    pub remove: bool,
    #[serde(default)]
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(default)]
    #[serde(with = "common::serde_zuid")]
    pub fk_user_attribute: i64,
    #[serde(default)]
    #[serde(with = "common::serde_zuid")]
    pub fk_object_attribute: i64,
    #[serde(default)]
    pub rights: Vec<String>,
    #[serde(default)]
    pub is_active: bool,
}

#[derive(Debug, Deserialize)]
pub struct DryRunCheck {
    #[serde(with = "common::serde_zuid")]
    pub user_id: i64,
    pub resource_type: String,
    #[serde(default)]
    #[serde(with = "common::serde_zuid")]
    pub fk_resource: i64,
    pub action: String,
}

#[derive(Debug, Deserialize)]
pub struct DryRunRequest {
    pub patches: Vec<DryRunPatch>,
    pub checks: Vec<DryRunCheck>,
}

#[derive(Debug, Serialize)]
pub struct DryRunCheckResult {
    #[serde(with = "common::serde_zuid")]
    pub user_id: i64,
    pub resource_type: String,
    #[serde(with = "common::serde_zuid")]
    pub fk_resource: i64,
    pub action: String,
    pub before: &'static str,
    pub after: &'static str,
    pub flipped: bool,
}

#[derive(Debug, Serialize)]
pub struct DryRunResponse {
    pub results: Vec<DryRunCheckResult>,
    /// 克隆图求值——关系表零写入（writes 恒 0）；真实生效 = 确认后事务写 + 版本 bump。
    pub writes: usize,
    pub projection: DriftReport,
    pub stale: bool,
}

/// dry-run 核心（**HTTP 端点与 chat 工具共用**；零写库）。
///
/// 语义见 [`dry_run`]：装载当前快照 → `ngac_decision::apply_patches` 克隆 → 前后
/// `evaluate_merge_in` 对比。求值器与真决策同源注入（§2.4 v3）。
pub enum DryRunError {
    /// 请求非法（未知 right 名 / 补丁形态）⇒ 调用方回 400。
    BadRequest(String),
    /// 服务端失败（策略装载 / 闭包查询）⇒ 调用方回 500。
    Internal(String),
}

pub async fn dry_run_impact(
    pool: &PgPool,
    body: &DryRunRequest,
) -> Result<DryRunResponse, DryRunError> {
    let projection = projection_health();

    let pip = crate::ngac::pip::PostgresPip::new(pool.clone());
    let pdp = Pdp::global();
    if let Err(e) = pdp.ensure_policy_loaded(&pip).await {
        return Err(DryRunError::Internal(format!("policy load: {e}")));
    }
    let pg = pdp.policy_graph();

    // 补丁翻译：right 名 → id（经图索引）；未知 right 名/形态 ⇒ 400 fail-closed
    let mut patches: Vec<PolicyPatch> = Vec::new();
    for p in &body.patches {
        let mut right_ids = Vec::new();
        for r in &p.rights {
            match pg.access_right_id(r) {
                Some(id) => right_ids.push(id),
                None => {
                    return Err(DryRunError::BadRequest(format!(
                        "UNKNOWN_ACCESS_RIGHT: {r}"
                    )))
                }
            }
        }
        let patch = match (p.remove, p.kind.as_str()) {
            (true, "association") => PolicyPatch::RemoveAssociation(p.id),
            (true, "prohibition") => PolicyPatch::RemoveProhibition(p.id),
            (true, _) => return Err(DryRunError::BadRequest("INVALID_PATCH_KIND".to_string())),
            (false, "association") => PolicyPatch::AddAssociation(ngac_decision::Association {
                id: p.id,
                fk_user_attribute: p.fk_user_attribute,
                fk_object_attribute: p.fk_object_attribute,
                ak_access_rights: right_ids,
                conditions: None,
            }),
            (false, "prohibition") => PolicyPatch::AddProhibition(ngac_decision::Prohibition {
                id: p.id,
                o_name: String::new(),
                fk_user_attribute: p.fk_user_attribute,
                fk_object_attribute: p.fk_object_attribute,
                ak_access_rights: right_ids,
                is_active: p.is_active,
                conditions: None,
            }),
            (false, _) => return Err(DryRunError::BadRequest("INVALID_PATCH_KIND".to_string())),
        };
        patches.push(patch);
    }
    let patched = ngac_decision::apply_patches(&pg, &patches);

    let mut results = Vec::new();
    for c in &body.checks {
        let (ua_rows, oa_rows) =
            match load_check_context(pool, c.user_id, &c.resource_type, c.fk_resource).await {
                Ok(v) => v,
                Err(e) => return Err(DryRunError::Internal(format!("closure: {e}"))),
            };
        let ctx = ngac_decision::ConditionContext {
            now: chrono::Utc::now(),
            user_ua_names: ua_rows.iter().map(|r| r.1.clone()).collect(),
            oa_closure_names: oa_rows.iter().map(|r| r.1.clone()).collect(),
            // 切片 B：dry-run 与真决策**同源消费**（候选补丁在克隆图上求值，条件求值器同一实现）
            // ——否则「dry-run 说放行、真决策拒绝」的等价性破裂。
            rhai_eval: Some(ngac_host::rhai_condition_evaluator(
                c.user_id,
                format!("{}:{}", c.resource_type, c.fk_resource),
                c.action.clone(),
            )),
        };
        let ua_ids: Vec<i64> = ua_rows.iter().map(|r| r.0).collect();
        let oa_ids: Vec<i64> = oa_rows.iter().map(|r| r.0).collect();
        let is_admin = ngac_host::attr_names_include_admin(ua_rows.iter().map(|r| r.1.as_str()));
        let before = evaluate_label(&pg, &ua_ids, &oa_ids, &c.action, &ctx, is_admin);
        let after = evaluate_label(&patched, &ua_ids, &oa_ids, &c.action, &ctx, is_admin);
        results.push(DryRunCheckResult {
            user_id: c.user_id,
            resource_type: c.resource_type.clone(),
            fk_resource: c.fk_resource,
            action: c.action.clone(),
            before,
            after,
            flipped: before != after,
        });
    }

    let stale = projection.stale();
    Ok(DryRunResponse {
        results,
        writes: 0,
        projection,
        stale,
    })
}

/// 候选策略补丁 dry-run：装载当前快照 → `ngac_decision::apply_patches` 克隆 →
/// 前后 `evaluate_merge_in`（同一 crate 决策语义）对比。**零写库**。
pub async fn dry_run(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<DryRunRequest>,
) -> HttpResponse {
    if let Err(resp) = guard_admin(&req, pool.get_ref()).await {
        return resp;
    }
    match dry_run_impact(pool.get_ref(), &body.into_inner()).await {
        Ok(resp) => HttpResponse::Ok().json(resp),
        Err(DryRunError::BadRequest(e)) => {
            HttpResponse::BadRequest().json(serde_json::json!({ "error": e }))
        }
        Err(DryRunError::Internal(e)) => {
            HttpResponse::InternalServerError().json(serde_json::json!({ "error": e }))
        }
    }
}

/// dry-run 检查项输入：UA 闭包行（`effective_ua_ids` 单源 + 祖先展开）+ OA 闭包行（单源）。
async fn load_check_context(
    pool: &PgPool,
    user_id: i64,
    resource_type: &str,
    fk_resource: i64,
) -> sqlx::Result<(Vec<(i64, String)>, Vec<(i64, String)>)> {
    let ua_rows: Vec<(i64, String)> = sqlx::query_as(
        "WITH RECURSIVE user_attrs AS ( \
             SELECT ua_id, 0 AS depth FROM isahl_auth.effective_ua_ids($1) \
             UNION ALL \
             SELECT unnest(ua.ancestor_ids)::BIGINT, d.depth + 1 \
             FROM isahl_auth.ngac_user_attribute ua \
             JOIN user_attrs d ON ua.id = d.ua_id \
             WHERE d.depth < 10 AND ua.deleted_at IS NULL) \
         SELECT DISTINCT u.ua_id, x.o_name FROM user_attrs u \
         JOIN isahl_auth.ngac_user_attribute x ON x.id = u.ua_id AND x.deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    let oa_rows = oa_closure_rows(pool, resource_type, fk_resource).await?;
    Ok((ua_rows, oa_rows))
}

/// 决策标签（dry-run 报告面）：合并语义 = crate 唯一实现；NotApplicable 按
/// 调用方 fail-closed 呈现为 "deny"（与 PEP 侧一致），供前后对比。
fn evaluate_label(
    pg: &PolicyGraph,
    ua: &[i64],
    oa: &[i64],
    action: &str,
    ctx: &ngac_decision::ConditionContext,
    is_admin: bool,
) -> &'static str {
    let (d, _) = ngac_decision::evaluate_merge_in(pg, ua, oa, action, ctx, is_admin);
    match d {
        ngac_decision::Decision::Permit => "permit",
        ngac_decision::Decision::Deny | ngac_decision::Decision::NotApplicable => "deny",
    }
}

// ── 路由 ─────────────────────────────────────────────────────────────────────

pub fn configure_admin_routes(cfg: &mut web::ServiceConfig) {
    // 与 access_request/binding_request 同形态：在 admin::configure 的 cfg 上注册
    // `/ngac/*` 路由（调用方包 `/api/admin` scope ⇒ 最终路径 `/api/admin/ngac/*`）。
    cfg.route("/ngac/analysis/sod", web::post().to(sod_analysis))
        .route("/ngac/analysis/reachability", web::post().to(reachability))
        .route(
            "/ngac/analysis/explain-paths",
            web::post().to(explain_paths),
        )
        .route("/ngac/analysis/dry-run", web::post().to(dry_run))
        .route("/ngac/sod-pairs", web::get().to(list_sod_pairs))
        .route("/ngac/sod-pairs", web::post().to(create_sod_pair))
        .route("/ngac/sod-pairs/{id}", web::delete().to(delete_sod_pair));
}
