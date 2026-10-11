//! OpenAPI 文档端点（openapi-external-access）
//!
//! - `GET /api/openapi.json` — OAS3 文档（须认证：L1 API Key / 服务令牌 /
//!   自然人令牌；无免认证公开端点）。文档从 **ns 基目录**下的 `openapi.json`
//!   静态读取（基目录判定单源 = `crate::paths::resolve_ns_base_dir`，与 host
//!   `load_extensions` 同函数；产物由 `scripts/openapi-generate.ts` 生成，
//!   生成逻辑单源 `scripts/lib/openapi-doc.ts`，发布包内路径 = `Deploy/{ns}/openapi.json`）。
//! - `GET /api/openapi/` — 内嵌 **离线 playground**（须认证，同文档端点）。
//!   单文件 HTML 经 `include_str!` 编译进二进制（零 CDN、零外网依赖）；
//!   前端 `fetch /api/openapi.json` 按服务段分组渲染，Bearer 令牌取自 localStorage，
//!   支持 try-it 同源调用。
//!
//! 安全：两个端点均在 `/api` scope 内，经 NgacEnforcer PEP 认证；
//! scope 校验对文档端点豁免（`read:openapi` 由 openapi.json 元数据标注，
//! 但 PEP 不做强制——文档为元数据，不涉业务数据）。

use actix_web::{web, HttpRequest, HttpResponse};

/// 离线 playground（单文件；无外部资源引用——门禁/规约见 openapi-external-access
/// :: oas3-doc-offline-playground）。
const PLAYGROUND_HTML: &str = include_str!("playground.html");

pub mod governance;
pub mod idempotency;
pub mod metering;
pub mod outbound_admin;
pub mod payment;
pub mod projection;

/// 过滤结果缓存：(产物 mtime 毫秒, 投影指纹, 过滤后 body)——避免每请求重解析大产物。
static FILTERED_DOC_CACHE: std::sync::LazyLock<std::sync::RwLock<Option<(u64, String, String)>>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(None));

/// 返回 OAS3 文档 JSON（按产品投影集裁剪；`info.description` 追加生效口径行）。
///
/// 规约：`openapi-external-access :: product-projection-wiring`——投影集为空 ⇒ 全量候选
/// （文档与接线前逐字一致）；判定单源 = `projection::ProductProjection`（与 PEP 同源消费）。
pub async fn get_openapi_json(_req: HttpRequest, pool: web::Data<sqlx::PgPool>) -> HttpResponse {
    let namespace = std::env::var("NAMESPACE").unwrap_or_default();
    // ns 基目录判定单源 = paths::resolve_ns_base_dir（DEPLOY_PATH 即 base、不再二次拼接
    // namespace；与 host load_extensions 同函数）——修复旧实现 DEPLOY_PATH 双拼
    // （产出 Deploy/{ns}/{ns}/openapi.json ⇒ 生产形态恒 404）。
    let base = crate::paths::resolve_ns_base_dir(&namespace);
    let path = std::path::Path::new(&base).join("openapi.json");
    let doc_text = match std::fs::read_to_string(&path) {
        Ok(doc) => doc,
        Err(e) => {
            log::error!("openapi.json not found at {}: {}", path.display(), e);
            return HttpResponse::NotFound().json(serde_json::json!({
                "error": "OPENAPI_NOT_FOUND",
                "message": "Run `bun scripts/openapi-generate.ts --ns <namespace>` first",
            }));
        }
    };

    let mtime_ms = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let projection = projection::global_cache().get(pool.get_ref()).await;
    let fingerprint = format!(
        "{}|{}",
        projection.is_full(),
        projection.prefixes().join(",")
    );
    if let Ok(guard) = FILTERED_DOC_CACHE.read() {
        if let Some((cached_mtime, cached_fp, body)) = guard.as_ref() {
            if *cached_mtime == mtime_ms && cached_fp == &fingerprint {
                return HttpResponse::Ok()
                    .content_type("application/json")
                    .body(body.clone());
            }
        }
    }

    let mut doc: serde_json::Value = match serde_json::from_str(&doc_text) {
        Ok(v) => v,
        Err(e) => {
            // 产物异常：原样返回（文档端点 MUST NOT 因裁剪面失败而不可用）
            log::error!("openapi.json parse failed at {}: {}", path.display(), e);
            return HttpResponse::Ok()
                .content_type("application/json")
                .body(doc_text);
        }
    };
    if !projection.is_full() {
        let visible = projection.filter_doc(&mut doc);
        log::debug!(
            "openapi.json: product projection applied ({} visible paths, {} prefixes)",
            visible,
            projection.prefix_count()
        );
    }
    append_status_line(&mut doc, &projection.status_line());
    let body = serde_json::to_string(&doc).unwrap_or(doc_text);
    if let Ok(mut guard) = FILTERED_DOC_CACHE.write() {
        *guard = Some((mtime_ms, fingerprint, body.clone()));
    }
    HttpResponse::Ok()
        .content_type("application/json")
        .body(body)
}

/// 在 `info.description` 追加生效口径行（MUST 声明；幂等——重复追加前先剥离旧行）。
fn append_status_line(doc: &mut serde_json::Value, line: &str) {
    let Some(info) = doc.get_mut("info").and_then(|v| v.as_object_mut()) else {
        return;
    };
    let current = info
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let base = current
        .lines()
        .filter(|l| !l.starts_with("产品投影") && !l.starts_with("无产品投影"))
        .collect::<Vec<_>>()
        .join("\n");
    let merged = if base.trim().is_empty() {
        line.to_string()
    } else {
        format!("{}\n{}", base.trim_end(), line)
    };
    info.insert("description".to_string(), serde_json::Value::String(merged));
}

/// 产品投影状态（管理面只读视图；PDP 资源 `openapi_admin:0`）——
/// 文档/授权面生效口径的唯一权威来源（reuses `projection` 缓存，零额外查询）。
pub async fn get_projection_status(pool: web::Data<sqlx::PgPool>) -> HttpResponse {
    let projection = projection::global_cache().get(pool.get_ref()).await;
    HttpResponse::Ok().json(serde_json::json!({
        "full": projection.is_full(),
        "prefixes": projection.prefixes(),
        "status_line": projection.status_line(),
    }))
}

/// 返回离线 playground（HTML；单文件内嵌，MUST NOT 引用外部 CDN）。
pub async fn get_swagger_ui(_req: HttpRequest) -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html")
        .body(PLAYGROUND_HTML)
}

/// 注册 OpenAPI 路由（在 /api scope 内，经 PEP 认证）。
///
/// 授权模型（refactor-openapi-admin-ngac-pdp）：PEP 统一决策——
/// usage 系列 → `openapi_analytics:0`，scope-catalog / outbound → `openapi_admin:0`
/// （map_resource 特判 + OA 种子 seed-ngac-07-openapi-admin-analytics-oas.sql）；
/// handler 内不再做任何 admin 检查。
pub fn configure_routes(cfg: &mut web::ServiceConfig, pool: sqlx::PgPool) {
    cfg.route("/openapi.json", web::get().to(get_openapi_json))
        .route("/openapi/", web::get().to(get_swagger_ui))
        .route("/openapi", web::get().to(get_swagger_ui))
        .route("/openapi/usage", web::get().to(get_usage_report))
        .route(
            "/openapi/usage/outbound",
            web::get().to(get_outbound_usage_report),
        )
        .route("/openapi/scope-catalog", web::get().to(get_scope_catalog))
        .route(
            "/openapi/projection-status",
            web::get().to(get_projection_status),
        )
        .route(
            "/openapi/provider-catalog",
            web::get().to(get_provider_catalog),
        )
        // 发起收款（change add-openapi-payment-channel）：金额取自计费聚合单源
        .route(
            "/openapi/billing/pay",
            web::post().to(payment::post_billing_pay),
        )
        // 到账回调（第二阶段）：公开路径 + 服务侧验签补偿（无 JWT；见 host 公开路径白名单）
        .route(
            "/openapi/billing/notify",
            web::post().to(payment::post_billing_notify),
        )
        .service(web::scope("/openapi").configure(outbound_admin::register))
        .app_data(web::Data::new(pool));
}

/// GET /api/openapi/usage/outbound?days=7
///
/// 出向用量聚合：按 provider × interface 分组输出窗口内调用量、错误数、
/// 平均/P95 延迟。数据来自 `isahl_auth.outbound_call_log`
/// （status 为写者统一语义串 'ok'/'error'，非 HTTP 状态码）。
/// 授权：PEP → `openapi_analytics:0`。
pub async fn get_outbound_usage_report(
    _req: HttpRequest,
    query: web::Query<std::collections::HashMap<String, String>>,
    pool: web::Data<sqlx::PgPool>,
) -> HttpResponse {
    let days: i32 = query
        .get("days")
        .and_then(|d| d.parse().ok())
        .unwrap_or(7)
        .clamp(1, 90);

    let rows: Vec<(String, String, i64, i64, f64, i64)> = match sqlx::query_as(
        r#"
        SELECT provider,
               interface,
               COUNT(*)::bigint AS total,
               COUNT(*) FILTER (WHERE status <> 'ok')::bigint AS errors,
               COALESCE(AVG(latency_ms), 0)::float8 AS avg_ms,
               COALESCE(PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY latency_ms), 0)::bigint AS p95_ms
        FROM isahl_auth.outbound_call_log
        WHERE requested_at >= NOW() - make_interval(days => $1)
        GROUP BY provider, interface
        ORDER BY provider, interface
        "#,
    )
    .bind(days)
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(r) => r,
        Err(e) => {
            log::error!("openapi outbound usage query failed: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Outbound usage query failed"
            }));
        }
    };

    let list: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(provider, interface, total, errors, avg_ms, p95_ms)| {
            serde_json::json!({
                "provider": provider,
                "interface": interface,
                "total": total,
                "errors": errors,
                "avg_ms": avg_ms,
                "p95_ms": p95_ms,
            })
        })
        .collect();
    HttpResponse::Ok().json(serde_json::json!({ "list": list, "days": days }))
}

/// GET /api/openapi/scope-catalog
///
/// scope 目录：resource registry 派生的 `{action}:{resource_type}`
/// （action ∈ read/create/update/delete，与 PEP 判定格式一致）与
/// OpenAPI 产品资源类型（configs/sales/purchases/mades —— registry 未注册，
/// 但 PEP map_resource service 分支按 entity 段推导 `{action}:{entity}`，
/// OA 由 seed-ngac-08-openapi-product-oas.sql 播种，目录 MUST 与 PEP 推导并集一致）的并集，
/// 再与 `api_clients.scopes` 现有授权值并集；每项含授权 client 数与来源。
/// 授权：PEP → `openapi_admin:0`。
pub async fn get_scope_catalog(_req: HttpRequest, pool: web::Data<sqlx::PgPool>) -> HttpResponse {
    // 派生：registry 注册的资源类型 × 4 action
    const ACTIONS: [&str; 4] = ["read", "create", "update", "delete"];
    // OpenAPI 产品 CRUD 实体（/api/service/openapi/{entity}，管理面）：
    // PEP map_resource service 分支按 entity 段推导 {entity}:0，scope 推导
    // 同实体名 {action}:{entity}（seed-ngac-08-openapi-product-oas.sql OA）。
    const OPENAPI_PRODUCT_ENTITIES: [&str; 4] = ["configs", "sales", "purchases", "mades"];
    let registry = ngac_contract::ResourceRegistry::new().with_alioth_defaults();
    let mut derived: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for def in registry.list_types() {
        for action in ACTIONS {
            derived.insert(
                format!("{}:{}", action, def.type_name),
                def.type_name.clone(),
            );
        }
    }
    for entity in OPENAPI_PRODUCT_ENTITIES {
        for action in ACTIONS {
            derived.insert(format!("{}:{}", action, entity), entity.to_string());
        }
    }

    // 授权统计：api_clients.scopes 展开
    let grants: Vec<(String, i64)> = match sqlx::query_as(
        r#"
        SELECT s AS scope, COUNT(*)::bigint AS client_count
        FROM (
            SELECT unnest(scopes) AS s
            FROM isahl_auth.api_clients
            WHERE deleted_at IS NULL
        ) t
        GROUP BY s
        "#,
    )
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(r) => r,
        Err(e) => {
            log::error!("openapi scope catalog query failed: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Scope catalog query failed"
            }));
        }
    };
    let grant_count: std::collections::HashMap<String, i64> = grants.into_iter().collect();

    // 并集
    let mut all: std::collections::BTreeSet<String> = derived.keys().cloned().collect();
    all.extend(grant_count.keys().cloned());

    let scopes: Vec<serde_json::Value> = all
        .into_iter()
        .map(|scope| {
            serde_json::json!({
                "scope": scope,
                "resource_type": derived.get(&scope),
                "client_count": grant_count.get(&scope).copied().unwrap_or(0),
            })
        })
        .collect();
    HttpResponse::Ok().json(serde_json::json!({ "scopes": scopes }))
}

/// GET /api/openapi/provider-catalog
///
/// provider 目录：**已装注册表**的只读视图（`outbound_client::provider`）。唯一数据源 =
/// 适配器面装载结果（`build.rs` 编译指令驱动的 `install`）——MUST NOT 在平台侧另建 provider
/// 清单（配置文件 / 表 / 常量表）。
///
/// `installed` 与 `providers.is_empty()` 是两个**独立事实**：「装载未执行」（平台模板档空装配）
/// 与「本构建零适配器」MUST 可区分——后者恒 `installed=true`。授权：PEP → `openapi_admin:0`。
///
/// 规约：`openspec/specs/openapi-external-access :: outbound-provider-catalog-and-membership-check`。
pub async fn get_provider_catalog(_req: HttpRequest) -> HttpResponse {
    let installed = outbound_client::provider::installed();
    let providers: Vec<&'static str> = outbound_client::provider::registry()
        .map(|r| r.ids())
        .unwrap_or_default();
    HttpResponse::Ok().json(serde_json::json!({
        "installed": installed,
        "providers": providers,
    }))
}

/// GET /api/openapi/usage?client_id=...&days=7
///
/// SLA/用量报告：聚合指定 client（或全部）在窗口内的调用量、错误率、P95 延迟。
/// 数据来自 `api_usage` 计量流水。NGAC 授权：调用者须持有 admin 属性
/// GET /api/openapi/usage?client_id=...&days=7
///
/// SLA/用量报告：聚合指定 client（或全部）在窗口内的调用量、错误率、P95 延迟。
/// 数据来自 `api_usage` 计量流水。授权：PEP → `openapi_analytics:0`。
pub async fn get_usage_report(
    _req: HttpRequest,
    query: web::Query<std::collections::HashMap<String, String>>,
    pool: web::Data<sqlx::PgPool>,
) -> HttpResponse {
    let client_id = query.get("client_id").cloned();
    let days: i32 = query
        .get("days")
        .and_then(|d| d.parse().ok())
        .unwrap_or(7)
        .clamp(1, 90);

    // 聚合：按日分组统计（调用量/错误/平均延迟/计数）
    let rows: Vec<(String, i64, i64, f64, i64)> = sqlx::query_as(
        r#"
        SELECT to_char(date_trunc('day', u.requested_at), 'YYYY-MM-DD') AS day,
               COUNT(*)::bigint AS total,
               COUNT(*) FILTER (WHERE u.status >= 400)::bigint AS errors,
               COALESCE(AVG(u.latency_ms), 0)::float8 AS avg_latency_ms,
               PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY u.latency_ms)::bigint AS p95_ms
        FROM isahl_auth.api_usage u
        WHERE u.requested_at >= NOW() - ($1 * INTERVAL '1 day')
          AND ($2::text IS NULL OR u.fk_subscription = (
              SELECT s.id FROM isahl_auth.api_subscriptions s
              JOIN isahl_auth.api_clients c ON c.id = s.fk_client
              WHERE c.client_id = $2 AND s.deleted_at IS NULL LIMIT 1
          ))
        GROUP BY day
        ORDER BY day
        "#,
    )
    .bind(days)
    .bind(&client_id)
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_default();

    // 汇总
    let total: i64 = rows.iter().map(|r| r.1).sum();
    let errors: i64 = rows.iter().map(|r| r.2).sum();
    let avg_p95: i64 = rows
        .iter()
        .filter(|r| r.4 > 0)
        .map(|r| r.4)
        .max()
        .unwrap_or(0);

    // 路由热点：Top 10（route+method 聚合，extract-openapi-analytics D1）
    let route_rows: Vec<(String, String, i64, i64, f64, i64)> = sqlx::query_as(
        r#"
        SELECT u.route, u.method,
               COUNT(*)::bigint AS total,
               COUNT(*) FILTER (WHERE u.status >= 400)::bigint AS errors,
               COALESCE(AVG(u.latency_ms), 0)::float8 AS avg_latency_ms,
               COALESCE(PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY u.latency_ms), 0)::bigint AS p95_ms
        FROM isahl_auth.api_usage u
        WHERE u.requested_at >= NOW() - ($1 * INTERVAL '1 day')
          AND ($2::text IS NULL OR u.fk_subscription = (
              SELECT s.id FROM isahl_auth.api_subscriptions s
              JOIN isahl_auth.api_clients c ON c.id = s.fk_client
              WHERE c.client_id = $2 AND s.deleted_at IS NULL LIMIT 1
          ))
        GROUP BY u.route, u.method
        ORDER BY total DESC
        LIMIT 10
        "#,
    )
    .bind(days)
    .bind(&client_id)
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_default();

    // 配额消耗：逐 active subscription（日历日/月窗口，与 metering.rs 配额判定同语义，D6）
    let quota_rows: Vec<(String, String, i64, i64, i64, i64)> = sqlx::query_as(
        r#"
        SELECT c.client_id, p.code, p.quota_daily, p.quota_monthly,
               (SELECT COUNT(*) FROM isahl_auth.api_usage u
                 WHERE u.fk_subscription = s.id
                   AND u.requested_at >= date_trunc('day', NOW()))::bigint AS used_today,
               (SELECT COUNT(*) FROM isahl_auth.api_usage u
                 WHERE u.fk_subscription = s.id
                   AND u.requested_at >= date_trunc('month', NOW()))::bigint AS used_month
        FROM isahl_auth.api_subscriptions s
        JOIN isahl_auth.api_clients c ON c.id = s.fk_client AND c.deleted_at IS NULL
        JOIN isahl_auth.api_plans p ON p.id = s.fk_plan AND p.deleted_at IS NULL
        WHERE s.deleted_at IS NULL AND s.status = 'active'
          AND (s.expires_at IS NULL OR s.expires_at > NOW())
          AND ($1::text IS NULL OR c.client_id = $1)
        ORDER BY c.client_id, s.id DESC
        "#,
    )
    .bind(&client_id)
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_default();

    // SLA 达成：5xx 口径实际可用性 + P95 对照 plan 承诺（D2）
    let sla_rows: Vec<(String, String, f64, i32, i64, i64, i64)> = sqlx::query_as(
        r#"
        SELECT c.client_id, p.code, p.sla_availability::float8, p.sla_p95_ms,
               COUNT(u.id)::bigint AS total,
               COUNT(u.id) FILTER (WHERE u.status >= 500)::bigint AS server_errors,
               COALESCE(PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY u.latency_ms), 0)::bigint AS actual_p95
        FROM isahl_auth.api_subscriptions s
        JOIN isahl_auth.api_clients c ON c.id = s.fk_client AND c.deleted_at IS NULL
        JOIN isahl_auth.api_plans p ON p.id = s.fk_plan AND p.deleted_at IS NULL
        LEFT JOIN isahl_auth.api_usage u
          ON u.fk_subscription = s.id
          AND u.requested_at >= NOW() - ($1 * INTERVAL '1 day')
        WHERE s.deleted_at IS NULL AND s.status = 'active'
          AND (s.expires_at IS NULL OR s.expires_at > NOW())
          AND ($2::text IS NULL OR c.client_id = $2)
        GROUP BY c.client_id, p.code, p.sla_availability, p.sla_p95_ms
        ORDER BY c.client_id, MAX(s.id) DESC
        "#,
    )
    .bind(days)
    .bind(&client_id)
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_default();

    HttpResponse::Ok().json(serde_json::json!({
        "client_id": client_id, // id-json-ok: OAuth client_id 为字符串标识（非 64bit 整数 id）
        "days": days,
        "total_requests": total,
        "error_requests": errors,
        "error_rate": if total > 0 { errors as f64 / total as f64 } else { 0.0 },
        "p95_latency_ms": avg_p95,
        "daily": rows.into_iter().map(|(day, total, errors, avg, p95)| {
            serde_json::json!({
                "day": day,
                "total": total,
                "errors": errors,
                "avg_latency_ms": avg,
                "p95_latency_ms": p95,
            })
        }).collect::<Vec<_>>(),
        "routes": route_rows.into_iter().map(|(route, method, total, errors, avg, p95)| {
            serde_json::json!({
                "route": route,
                "method": method,
                "total": total,
                "errors": errors,
                "avg_latency_ms": avg,
                "p95_latency_ms": p95,
            })
        }).collect::<Vec<_>>(),
        "quotas": quota_rows.into_iter().map(|(cid, plan, qd, qm, used_d, used_m)| {
            serde_json::json!({
                "client_id": cid.to_string(),
                "plan_code": plan,
                "quota_daily": qd,
                "quota_monthly": qm,
                "used_today": used_d,
                "used_month": used_m,
                "daily_pct": quota_pct(used_d, qd),
                "monthly_pct": quota_pct(used_m, qm),
            })
        }).collect::<Vec<_>>(),
        "sla": sla_rows.into_iter().map(|(cid, plan, sla_avail, sla_p95, total, srv_err, actual_p95)| {
            let actual_avail = actual_availability(total, srv_err);
            serde_json::json!({
                "client_id": cid.to_string(),
                "plan_code": plan,
                "sla_availability": sla_avail,
                "sla_p95_ms": sla_p95,
                "actual_availability": actual_avail,
                "actual_p95_ms": actual_p95,
                "attained": sla_attained(sla_avail, sla_p95, total, srv_err, actual_p95),
            })
        }).collect::<Vec<_>>(),
    }))
}

/// 配额消耗比：quota=0（不限）→ None（null）；否则 used/quota。
/// （extract-openapi-analytics spec: unlimited-quota-is-null）
fn quota_pct(used: i64, quota: i64) -> Option<f64> {
    if quota > 0 {
        Some(used as f64 / quota as f64)
    } else {
        None
    }
}

/// 实际可用性（5xx 口径，D2）：窗口内无流量 → None（不判定）。
fn actual_availability(total: i64, server_errors: i64) -> Option<f64> {
    if total > 0 {
        Some(1.0 - server_errors as f64 / total as f64)
    } else {
        None
    }
}

/// SLA 达成判定：无流量 → None；sla_p95_ms<=0 表示无 P95 承诺，仅看可用性。
/// （extract-openapi-analytics spec: no-traffic-not-judged）
fn sla_attained(
    sla_avail: f64,
    sla_p95_ms: i32,
    total: i64,
    server_errors: i64,
    actual_p95: i64,
) -> Option<bool> {
    actual_availability(total, server_errors)
        .map(|a| a >= sla_avail && (sla_p95_ms <= 0 || actual_p95 <= sla_p95_ms as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_pct_unlimited_is_null() {
        assert_eq!(quota_pct(123, 0), None);
    }

    #[test]
    fn quota_pct_ratio() {
        assert_eq!(quota_pct(850, 1000), Some(0.85));
        assert_eq!(quota_pct(1200, 1000), Some(1.2)); // 超配允许 >1（预警展示用）
    }

    #[test]
    fn availability_5xx_only() {
        // 4xx 不计失效：total=100、server_errors(5xx)=2 → 0.98
        assert_eq!(actual_availability(100, 2), Some(0.98));
        assert_eq!(actual_availability(0, 0), None);
    }

    #[test]
    fn attained_requires_availability_and_p95() {
        // 可用性达标但 P95 超标 → 未达成
        assert_eq!(sla_attained(0.99, 800, 1000, 1, 900), Some(false));
        // 双达标
        assert_eq!(sla_attained(0.99, 800, 1000, 1, 500), Some(true));
        // 无 P95 承诺（0）→ 仅看可用性
        assert_eq!(sla_attained(0.99, 0, 1000, 1, 99999), Some(true));
        // 可用性不达标
        assert_eq!(sla_attained(0.999, 800, 10000, 20, 500), Some(false));
        // 无流量 → 不判定
        assert_eq!(sla_attained(0.999, 800, 0, 0, 0), None);
    }
}

#[cfg(test)]
mod route_registration_tests {
    use super::*;
    use actix_web::{test, App};

    /// 路由注册完整性：openapi 全家端点必须可路由（不得 404）。
    /// handler 在 lazy pool 下会 403/500——只要不是 404 即证明路由存在。
    #[actix_web::test]
    async fn openapi_routes_are_registered() {
        let pool =
            sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent").expect("lazy pool");
        let app = test::init_service(
            App::new()
                .service(web::scope("/api").configure(|cfg| configure_routes(cfg, pool.clone()))),
        )
        .await;
        for uri in [
            "/api/openapi",
            "/api/openapi/usage?days=7",
            "/api/openapi/usage/outbound?days=7",
            "/api/openapi/scope-catalog",
            "/api/openapi/provider-catalog",
        ] {
            let req = test::TestRequest::get().uri(uri).to_request();
            let resp = test::call_service(&app, req).await;
            assert_ne!(
                resp.status().as_u16(),
                404,
                "{} 应被路由（非 404），实际 {}",
                uri,
                resp.status()
            );
        }
        // POST 端点（发起收款）：路由存在即非 404（无注册行时为 503，方法不匹配为 405）
        let req = test::TestRequest::post()
            .uri("/api/openapi/billing/pay")
            .set_json(serde_json::json!({ "client_id": "c1", "month": "2026-01" }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_ne!(
            resp.status().as_u16(),
            404,
            "/api/openapi/billing/pay 应被路由（非 404），实际 {}",
            resp.status()
        );

        // POST 端点（到账回调，公开路径）：路由存在即非 404（无签名头时为 400，无注册行时为 503）
        let req = test::TestRequest::post()
            .uri("/api/openapi/billing/notify")
            .set_json(serde_json::json!({}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_ne!(
            resp.status().as_u16(),
            404,
            "/api/openapi/billing/notify 应被路由（非 404），实际 {}",
            resp.status()
        );
    }
}
