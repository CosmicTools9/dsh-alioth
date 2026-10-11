//! OpenAPI 计费聚合（change `add-openapi-usage-billing`）
//!
//! 规约：`openspec/specs/openapi-external-access :: usage-billing-aggregation`。
//!
//! `GET /api/admin/api-billing?month=YYYY-MM`（admin 面门控，与 `api-reconcile` 同族）：
//! - 用量源 = `isahl_auth.api_usage`（窗口 `[month_start, next_month_start)`）；
//! - 产品/价格源 = 产品行 `projection`（前缀匹配 `/api/service/` 之后的 route）
//!   × `qk_price` → `isahl."zc_id_scal-price".mark`（**唯一价源**）；
//! - 金额/单价以**字符串**承载（numeric 精度，MUST NOT f64）；
//! - 未定价（无产品行命中或标量价缺失）⇒ `unit_price: null` 且不计入总额，
//!   计入 `unpriced_calls`（MUST NOT 以 0 冒充「免费」）；
//! - 月份早于保留期 ⇒ 400 `WINDOW_OUT_OF_RETENTION`（判定复用 `subscriptions` 既有实现）。
//!
//! **计价文法单一实现 = `openapi_billing` crate**（change `add-openapi-payment-channel`）：
//! 本模块只做 admin 面 HTTP 装配；SQL/窗口解析全部经共享 crate（发起收款端同源消费同一份
//! 计价 SQL，杜绝「报表金额 ≠ 收款金额」的静默分歧）。

use actix_web::{web, HttpRequest, HttpResponse};
use sqlx::PgPool;

use super::subscriptions::{reconcile_earliest_month, window_out_of_retention};
use crate::auth::AuthState;

/// GET /api/admin/api-billing?month=YYYY-MM —— 用量计费聚合（只读）。
pub async fn usage_billing(
    _req: HttpRequest,
    query: web::Query<std::collections::HashMap<String, String>>,
    pool: web::Data<PgPool>,
    _state: web::Data<AuthState>,
) -> HttpResponse {
    let now = chrono::Utc::now();
    let window = match openapi_billing::month_window(query.get("month").map(String::as_str), now) {
        Ok(w) => w,
        Err(e) => {
            return HttpResponse::BadRequest().json(serde_json::json!({ "error": e.to_string() }))
        }
    };
    // 窗口下界 MUST 与对账导出同源（MUST NOT 复刻判定）
    let earliest_month = reconcile_earliest_month(now);
    if window_out_of_retention(window.start, &earliest_month) {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "WINDOW_OUT_OF_RETENTION",
            "message": format!("api_usage 保留期受限，最早可查月份为 {}", earliest_month),
            "earliest_month": earliest_month,
        }));
    }
    let (start, end) = (window.start_dt(), window.end_dt());

    let products_face = openapi_billing::products_face_present(pool.get_ref()).await;

    let mut items: Vec<serde_json::Value> = Vec::new();
    let mut total_amount = "0".to_string();
    let mut priced_calls: i64 = 0;
    let mut unpriced_calls: i64 = 0;
    let mut currencies: Vec<String> = Vec::new();

    if products_face {
        match openapi_billing::month_lines(pool.get_ref(), start, end, None).await {
            Ok(lines) => {
                for line in lines {
                    if let Some(c) = line.currency.as_ref() {
                        if !currencies.iter().any(|x| x == c) {
                            currencies.push(c.clone());
                        }
                    }
                    let calls = line.calls;
                    if line.unit_price.is_none() {
                        unpriced_calls += calls;
                    } else {
                        priced_calls += calls;
                    }
                    items.push(serde_json::json!({
                        "client_id": line.client_id,
                        "product_code": line.product_code,
                        "product_name": line.product_name,
                        "projection": line.projection,
                        "calls": calls,
                        "unit_price": line.unit_price,
                        "amount": line.amount,
                        "currency": line.currency,
                    }));
                }
            }
            Err(e) => {
                log::error!("api-billing detail query failed: {}", e);
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "error": "BILLING_QUERY_FAILED",
                }));
            }
        }

        if let Ok(totals) = openapi_billing::month_totals(pool.get_ref(), start, end, None).await {
            total_amount = totals.total_amount;
        }
        if let Ok(n) = openapi_billing::month_unmatched_calls(pool.get_ref(), start, end).await {
            unpriced_calls += n;
        }
    } else {
        // 无产品面（未启用产品管理的 ns）⇒ 全部调用视为未定价（MUST NOT 500）
        unpriced_calls = openapi_billing::month_total_usage_calls(pool.get_ref(), start, end)
            .await
            .unwrap_or(0);
    }

    let currency = match currencies.len() {
        0 => None,
        1 => Some(currencies.remove(0)),
        _ => Some("mixed".to_string()),
    };

    HttpResponse::Ok().json(serde_json::json!({
        "month": window.month,
        "currency": currency,
        "items": items,
        "totals": {
            "total_calls": priced_calls + unpriced_calls,
            "priced_calls": priced_calls,
            "unpriced_calls": unpriced_calls,
            "total_amount": total_amount,
        },
        // 同 route 命中多产品 ⇒ 逐产品分行（不合并，避免重复计价）
        "overlap_policy": "per-product",
    }))
}
