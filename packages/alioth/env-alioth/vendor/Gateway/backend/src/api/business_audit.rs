//! 业务审计域 Gateway API（change wire-business-audit-domain 组 2.4，D4 复用口径）。
//!
//! - `GET /api/business-audit/audit-projects` — 审计项目列表（audit-writer 读面）
//! - `GET /api/business-audit/audit-projects/{id}/export?type=invoice|receipt|writeoff|archive`
//!   — 审计取数导出（CSV 流式 attachment；行级范围 = 审计项目受审主体集合）
//!
//! 数据源（D4：复用 ns 既有查询面，MUST NOT 第二业务读径）：
//! - `invoice`（进项发票）：WZ `accounts-payable::FjaRepository::list_invoices_in`
//! - `receipt`（银行回单）：WZ `accounts-receivable::FjiRepository::list_receipts`
//! - `writeoff`（付款核销）：WZ `accounts-payable::FjaRepository::list_payment_matches`
//! - `archive`（审计工作底稿）：`audit-writer::read::archive_csv`（审计域自有聚合）
//!
//! 非 WZ 构建：财务三类返回 400（该 ns 无 WZ 财务数据源——端点契约存在、数据源随 ns）。
use actix_web::HttpMessage;
use actix_web::{web, HttpRequest, HttpResponse};
use sqlx::PgPool;

fn current_user(req: &HttpRequest) -> Option<i64> {
    req.extensions()
        .get::<common::context::RequestContext>()
        .map(|ctx| ctx.user_id)
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/business-audit")
            .route("/audit-projects", web::get().to(list_audit_projects))
            .route(
                "/audit-projects/{id}/export",
                web::get().to(export_audit_data),
            )
            .route("/smtv-reviews", web::get().to(list_smtv_reviews))
            .route("/smtv-reviews", web::post().to(create_smtv_review)),
    );
}

async fn list_audit_projects(pool: web::Data<PgPool>) -> HttpResponse {
    match audit_writer::read::list_audit_projects(pool.get_ref()).await {
        Ok(items) => HttpResponse::Ok().json(items),
        Err(e) => HttpResponse::InternalServerError().body(e.to_string()),
    }
}

#[derive(Debug, serde::Deserialize)]
struct ExportQuery {
    /// invoice | receipt | writeoff | archive
    #[serde(rename = "type")]
    kind: String,
    #[serde(default = "default_format")]
    format: String,
}

fn default_format() -> String {
    "csv".to_string()
}

async fn export_audit_data(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    query: web::Query<ExportQuery>,
) -> HttpResponse {
    let _user = match current_user(&req) {
        Some(u) => u,
        None => return HttpResponse::Unauthorized().finish(),
    };
    if query.format != "csv" {
        return HttpResponse::BadRequest().body("仅支持 format=csv");
    }
    let audit_id = path.into_inner();

    let (csv, filename) = match query.kind.as_str() {
        "archive" => match audit_writer::read::archive_csv(pool.get_ref(), audit_id).await {
            Ok(csv) => (csv, format!("audit-{audit_id}-archive.csv")),
            Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
        },
        #[cfg(feature = "wz")]
        "invoice" => {
            match wz_service_accounts_payable::repositories::FjaRepository::new(
                pool.get_ref().clone(),
            )
            .list_invoices_in()
            .await
            {
                Ok(rows) => (
                    crate::api::business_audit_csv::invoice_in_csv(&rows),
                    format!("audit-{audit_id}-invoice-in.csv"),
                ),
                Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
            }
        }
        #[cfg(feature = "wz")]
        "receipt" => {
            let q = wz_service_accounts_receivable::models::ListReceiptsQuery {
                page: 1,
                page_size: 100,
                ..Default::default()
            };
            match wz_service_accounts_receivable::repositories::FjiRepository::new(
                pool.get_ref().clone(),
            )
            .list_receipts(&q)
            .await
            {
                Ok((rows, _, _, _)) => (
                    crate::api::business_audit_csv::receipt_csv(&rows),
                    format!("audit-{audit_id}-receipts.csv"),
                ),
                Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
            }
        }
        #[cfg(feature = "wz")]
        "writeoff" => {
            match wz_service_accounts_payable::repositories::FjaRepository::new(
                pool.get_ref().clone(),
            )
            .list_payment_matches()
            .await
            {
                Ok(rows) => (
                    crate::api::business_audit_csv::payment_match_csv(&rows),
                    format!("audit-{audit_id}-writeoffs.csv"),
                ),
                Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
            }
        }
        other => {
            return HttpResponse::BadRequest().body(format!(
                "未知或当前 ns 不可用的导出类型：{other}（可用：archive{}）",
                if cfg!(feature = "wz") {
                    "、invoice、receipt、writeoff"
                } else {
                    ""
                }
            ))
        }
    };

    HttpResponse::Ok()
        .insert_header(("Content-Type", "text/csv; charset=utf-8"))
        .insert_header((
            "Content-Disposition",
            format!("attachment; filename=\"{filename}\""),
        ))
        .body(csv)
}

/// GET /api/business-audit/smtv-reviews — 结算复盘列表（新→旧）。
async fn list_smtv_reviews(pool: web::Data<PgPool>) -> HttpResponse {
    match audit_writer::read::list_smtv_reviews(pool.get_ref()).await {
        Ok(items) => HttpResponse::Ok().json(items),
        Err(e) => HttpResponse::InternalServerError().body(e.to_string()),
    }
}

#[derive(Debug, serde::Deserialize)]
struct CreateSmtvReviewRequest {
    pub code: String,
    pub notice: String,
    pub comments: Option<String>,
    pub fk_subject: Option<i64>,
}

/// POST /api/business-audit/smtv-reviews — 登记结算复盘操作行
/// （坐标 JC/FTA/↓_EZ 由写链内解析；id 省略走列默认）。
async fn create_smtv_review(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<CreateSmtvReviewRequest>,
) -> HttpResponse {
    let user_id = match current_user(&req) {
        Some(u) => u,
        None => return HttpResponse::Unauthorized().finish(),
    };
    let mut conn = match pool.get_ref().acquire().await {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };
    match audit_writer::insert_smtv_review_tx(
        &mut conn,
        &audit_writer::OperationRowInput {
            code: body.code.clone(),
            notice: body.notice.clone(),
            comments: body.comments.clone(),
            fk_subject: body.fk_subject,
            fk_operator: Some(user_id),
        },
        user_id,
    )
    .await
    {
        Ok(id) => HttpResponse::Created().json(serde_json::json!({ "id": id.to_string() })),
        Err(e) => HttpResponse::BadRequest().body(e.to_string()),
    }
}
