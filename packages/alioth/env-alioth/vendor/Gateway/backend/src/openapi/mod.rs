//! OpenAPI 文档端点（openapi-external-access）
//!
//! - `GET /api/openapi.json` — OAS3 文档（须认证：L1 API Key / 服务令牌 /
//!   自然人令牌；无免认证公开端点）。文档从 `Pre-Proc/{ns}/openapi.json`
//!   静态读取（由 `scripts/openapi-generate.ts` 生成）。
//! - `GET /api/openapi/` — Swagger UI playground（须认证，同文档端点）。
//!   前端内嵌 swagger-ui（CDN），`url` 指向 `/api/openapi.json`，
//!   授权用 `Authorization: Bearer`（服务令牌自动从 localStorage 读取）。
//!
//! 安全：两个端点均在 `/api` scope 内，经 NgacEnforcer PEP 认证；
//! scope 校验对文档端点豁免（`read:openapi` 由 openapi.json 元数据标注，
//! 但 PEP 不做强制——文档为元数据，不涉业务数据）。

use actix_web::{web, HttpRequest, HttpResponse};

pub mod idempotency;
pub mod metering;
pub mod outbound_admin;

/// 返回 OAS3 文档 JSON。
pub async fn get_openapi_json(_req: HttpRequest) -> HttpResponse {
    let namespace = std::env::var("NAMESPACE").unwrap_or_default();
    // resolve_preproc_path 同款探测：Deploy/{ns} → Pre-Proc/{ns} → 相对探测
    let base = resolve_openapi_base(&namespace);
    let path = std::path::Path::new(&base).join("openapi.json");
    match std::fs::read_to_string(&path) {
        Ok(doc) => HttpResponse::Ok()
            .content_type("application/json")
            .body(doc),
        Err(e) => {
            log::error!("openapi.json not found at {}: {}", path.display(), e);
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "OPENAPI_NOT_FOUND",
                "message": "Run `bun scripts/openapi-generate.ts --ns <namespace>` first",
            }))
        }
    }
}

/// 返回 Swagger UI playground（HTML）。
pub async fn get_swagger_ui(_req: HttpRequest) -> HttpResponse {
    let html = r#"<!DOCTYPE html>
<html lang="zh">
<head>
  <meta charset="UTF-8">
  <title>AliothStudio OpenAPI Playground</title>
  <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css">
  <style>
    body { margin: 0; font-family: -apple-system, sans-serif; }
    #topbar {
      padding: 10px 20px; background: #1a1a2e; color: #fff;
      display: flex; align-items: center; gap: 12px;
    }
    #topbar h1 { font-size: 16px; margin: 0; flex: 1; }
    #topbar input { padding: 6px 10px; border-radius: 4px; border: none; width: 320px; }
    #topbar button { padding: 6px 14px; border: none; border-radius: 4px; cursor: pointer; }
    .hint { padding: 6px 20px; background: #fffbe6; font-size: 12px; color: #614700; }
  </style>
</head>
<body>
  <div id="topbar">
    <h1>AliothStudio OpenAPI Playground</h1>
    <input id="token" type="password" placeholder="Bearer token（服务令牌 / API Key 兑换）" />
    <button onclick="applyToken()">应用 Token</button>
  </div>
  <div class="hint">
    获取令牌：<code>POST /api/auth/token</code>（client_credentials，body:
    <code>grant_type=client_credentials&amp;client_id=...&amp;client_secret=...</code>）
    或 <code>POST /api/auth/authenticate</code>（Authorization: Bearer ak_xxx）。
    令牌输入后点「应用 Token」，即可调试下方端点；scope 不足返回 403。
  </div>
  <div id="swagger-ui"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
  <script>
    function applyToken() {
      const t = document.getElementById('token').value.trim();
      localStorage.setItem('openapi_token', t);
      location.reload();
    }
    const token = localStorage.getItem('openapi_token') || '';
    if (token) { document.getElementById('token').value = token; }
    window.ui = SwaggerUIBundle({
      url: '/api/openapi.json',
      dom_id: '#swagger-ui',
      deepLinking: true,
      presets: [SwaggerUIBundle.presets.apis],
      requestInterceptor: (req) => {
        if (token) {
          req.headers['Authorization'] = 'Bearer ' + token;
        }
        return req;
      },
    });
  </script>
</body>
</html>"#;
    HttpResponse::Ok().content_type("text/html").body(html)
}

/// 探测 openapi.json 基础目录（与 main.rs resolve_preproc_path 同优先级）。
fn resolve_openapi_base(namespace: &str) -> String {
    if let Ok(dep) = std::env::var("DEPLOY_PATH") {
        if !dep.is_empty() {
            return format!("{}/{}", dep.trim_end_matches('/'), namespace);
        }
    }
    if let Ok(pp) = std::env::var("PREPROC_APPS_PATH") {
        if !pp.is_empty() {
            return format!("{}/{}", pp.trim_end_matches('/'), namespace);
        }
    }
    // 自动探测：Pre-Proc/{ns}
    for cand in ["Pre-Proc", "../Pre-Proc", "../../Pre-Proc"] {
        let p = format!("{}/{}", cand, namespace);
        if std::path::Path::new(&p).exists() {
            return p;
        }
    }
    format!("Pre-Proc/{}", namespace)
}

/// 注册 OpenAPI 路由（在 /api scope 内，经 PEP 认证）。
///
/// 授权模型（refactor-openapi-admin-ngac-pdp）：PEP 统一决策——
/// usage 系列 → `openapi_analytics:0`，scope-catalog / outbound → `openapi_admin:0`
/// （map_resource 特判 + 迁移 029 OA 种子）；handler 内不再做任何 admin 检查。
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
/// OA 由迁移 031 seed，目录 MUST 与 PEP 推导并集一致）的并集，
/// 再与 `api_clients.scopes` 现有授权值并集；每项含授权 client 数与来源。
/// 授权：PEP → `openapi_admin:0`。
pub async fn get_scope_catalog(_req: HttpRequest, pool: web::Data<sqlx::PgPool>) -> HttpResponse {
    // 派生：registry 注册的资源类型 × 4 action
    const ACTIONS: [&str; 4] = ["read", "create", "update", "delete"];
    // OpenAPI 产品 CRUD 实体（/api/service/openapi/{entity}，管理面）：
    // PEP map_resource service 分支按 entity 段推导 {entity}:0，scope 推导
    // 同实体名 {action}:{entity}（seed-openapi-product-oa，迁移 031 OA）。
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
        "client_id": client_id,
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
                "client_id": cid,
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
                "client_id": cid,
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
    }
}
