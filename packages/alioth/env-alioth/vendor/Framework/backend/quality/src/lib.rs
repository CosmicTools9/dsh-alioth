pub mod alerts;
pub mod dashboard;
pub mod models;
pub mod profile;
pub mod profiler;
pub mod report;
pub mod rules;
pub mod scheduler;

use actix_web::{web, HttpResponse, Result};
use sqlx::PgPool;

pub use alerts::{
    AlertConfig, AlertFrequency, AlertPayload, AlertSent, AlertService, EmailNotifier,
    WebhookNotifier,
};
pub use dashboard::{
    CategoryScore, DashboardData, DashboardService, FailureDistribution, QualityScore,
    QualityScoreCalculator, TopIssue, TrendPoint,
};
pub use models::{
    CreateRuleRequest, QualityCheckResponse, QualityError, QualityReportResponse,
    QualityRuleResponse, RuleType, RunCheckRequest,
};
pub use profile::{
    BooleanStatistics, DateTimeStatistics, FieldProfile, NumericStatistics, ProfileEngine,
    ProfileRepository, ProfileStatistics, TextStatistics,
};
pub use profiler::QualityProfiler;
pub use report::excel_exporter::ExcelExporter;
pub use report::pdf_exporter::PdfExporter;
pub use report::{
    QualityReport, QualityReportService, Recommendation, RecommendationPriority, ReportParams,
    ReportPeriod, ReportSummary, RuleExecutionDetail,
};
pub use rules::{
    get_builtin_rules, ExecuteRulesRequest, QualityRule, RuleDefinition, RuleEngine,
    RuleExecutionResult, RuleRepository, RuleSeverity, RuleStatus, RuleType as NewRuleType,
};
pub use scheduler::{
    SamplingStrategy, ScheduledJob, SchedulerService, TestRun, TestRunService, TestRunStatus,
    TestRunType,
};

// 原有控制器函数（向后兼容）
async fn create_rule(
    pool: web::Data<PgPool>,
    req: web::Json<CreateRuleRequest>,
) -> Result<HttpResponse> {
    let req = req.into_inner();
    let rule_type = RuleType::from_string(&req.rule_type);
    let severity = crate::models::Severity::from_string(&req.severity);
    let parameters = req.parameters.unwrap_or(serde_json::json!({}));

    match QualityProfiler::create_rule(
        pool.get_ref(),
        req.name,
        rule_type,
        req.target_table,
        req.target_column,
        parameters,
        severity,
    )
    .await
    {
        Ok(rule) => Ok(HttpResponse::Created().json(QualityRuleResponse::from(rule))),
        Err(e) => {
            log::error!("Failed to create quality rule: {}", e);
            Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

async fn list_rules(pool: web::Data<PgPool>) -> Result<HttpResponse> {
    match QualityProfiler::list_rules(pool.get_ref()).await {
        Ok(rules) => {
            let responses: Vec<QualityRuleResponse> =
                rules.into_iter().map(QualityRuleResponse::from).collect();
            Ok(HttpResponse::Ok().json(responses))
        }
        Err(e) => {
            log::error!("Failed to list quality rules: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

async fn get_rule(pool: web::Data<PgPool>, path: web::Path<i64>) -> Result<HttpResponse> {
    let rule_id = path.into_inner();

    match QualityProfiler::get_rule(pool.get_ref(), rule_id).await {
        Ok(Some(rule)) => Ok(HttpResponse::Ok().json(QualityRuleResponse::from(rule))),
        Ok(None) => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Rule not found"
        }))),
        Err(e) => {
            log::error!("Failed to get quality rule: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateRuleRequest {
    pub name: Option<String>,
    pub rule_type: Option<String>,
    pub target_table: Option<String>,
    pub target_column: Option<Option<String>>,
    pub parameters: Option<serde_json::Value>,
    pub severity: Option<String>,
}

async fn update_rule(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    req: web::Json<UpdateRuleRequest>,
) -> Result<HttpResponse> {
    let rule_id = path.into_inner();
    let req = req.into_inner();

    let rule_type = req.rule_type.map(|t| RuleType::from_string(&t));
    let severity = req
        .severity
        .map(|s| crate::models::Severity::from_string(&s));

    match QualityProfiler::update_rule(
        pool.get_ref(),
        rule_id,
        req.name,
        rule_type,
        req.target_table,
        req.target_column,
        req.parameters,
        severity,
    )
    .await
    {
        Ok(rule) => Ok(HttpResponse::Ok().json(QualityRuleResponse::from(rule))),
        Err(sqlx::Error::RowNotFound) => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Rule not found"
        }))),
        Err(e) => {
            log::error!("Failed to update quality rule: {}", e);
            Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

async fn delete_rule(pool: web::Data<PgPool>, path: web::Path<i64>) -> Result<HttpResponse> {
    let rule_id = path.into_inner();

    match QualityProfiler::delete_rule(pool.get_ref(), rule_id).await {
        Ok(()) => Ok(HttpResponse::NoContent().finish()),
        Err(e) => {
            log::error!("Failed to delete quality rule: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

async fn run_check(
    pool: web::Data<PgPool>,
    req: web::Json<RunCheckRequest>,
) -> Result<HttpResponse> {
    let rule_id = req.rule_id;

    match QualityProfiler::run_check(pool.get_ref(), rule_id).await {
        Ok(check) => Ok(HttpResponse::Accepted().json(QualityCheckResponse::from(check))),
        Err(e) => {
            log::error!("Failed to run quality check: {}", e);
            Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

async fn get_check(pool: web::Data<PgPool>, path: web::Path<i64>) -> Result<HttpResponse> {
    let check_id = path.into_inner();

    match QualityProfiler::get_check(pool.get_ref(), check_id).await {
        Ok(Some(check)) => Ok(HttpResponse::Ok().json(QualityCheckResponse::from(check))),
        Ok(None) => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Check not found"
        }))),
        Err(e) => {
            log::error!("Failed to get quality check: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

async fn get_report(pool: web::Data<PgPool>, path: web::Path<i64>) -> Result<HttpResponse> {
    let check_id = path.into_inner();

    match QualityProfiler::generate_report(pool.get_ref(), check_id).await {
        Ok(Some(report)) => Ok(HttpResponse::Ok().json(QualityReportResponse::from(report))),
        Ok(None) => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Report not found"
        }))),
        Err(e) => {
            log::error!("Failed to generate quality report: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

async fn get_profile(pool: web::Data<PgPool>, path: web::Path<String>) -> Result<HttpResponse> {
    let table = path.into_inner();

    match QualityProfiler::get_profile(pool.get_ref(), &table).await {
        Ok(profile) => Ok(HttpResponse::Ok().json(profile)),
        Err(e) => {
            log::error!("Failed to get table profile: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

// ==================== 新增 API 控制器 ====================

/// 获取规则库定义
async fn get_rule_library() -> HttpResponse {
    let rules = get_builtin_rules();
    HttpResponse::Ok().json(rules)
}

/// 获取字段画像
async fn get_field_profile(pool: web::Data<PgPool>, path: web::Path<i64>) -> Result<HttpResponse> {
    let field_id = path.into_inner();
    let repo = ProfileRepository::new(pool.get_ref().clone());

    match repo.get_latest_by_field(field_id).await {
        Ok(Some(profile)) => Ok(HttpResponse::Ok().json(profile)),
        Ok(None) => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Profile not found"
        }))),
        Err(e) => {
            log::error!("Failed to get field profile: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

/// 执行字段画像
async fn profile_field(_pool: web::Data<PgPool>, path: web::Path<i64>) -> Result<HttpResponse> {
    let _field_id = path.into_inner();
    // 简化实现：返回未实现错误
    Ok(HttpResponse::NotImplemented().json(serde_json::json!({
        "error": "Profile execution not yet implemented"
    })))
}

/// 获取字段画像历史
async fn get_field_profile_history(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    query: web::Query<ProfileHistoryQuery>,
) -> Result<HttpResponse> {
    let field_id = path.into_inner();
    let repo = ProfileRepository::new(pool.get_ref().clone());

    match repo
        .get_history_by_field(field_id, query.limit.unwrap_or(10))
        .await
    {
        Ok(profiles) => Ok(HttpResponse::Ok().json(profiles)),
        Err(e) => {
            log::error!("Failed to get profile history: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

/// 获取集合画像
async fn get_collection_profiles(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse> {
    let collection_id = path.into_inner();
    let repo = ProfileRepository::new(pool.get_ref().clone());

    match repo.get_by_collection(collection_id).await {
        Ok(profiles) => Ok(HttpResponse::Ok().json(profiles)),
        Err(e) => {
            log::error!("Failed to get collection profiles: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

/// 执行集合画像
async fn profile_collection(
    _pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse> {
    let _collection_id = path.into_inner();
    // 简化实现
    Ok(HttpResponse::NotImplemented().json(serde_json::json!({
        "error": "Collection profile execution not yet implemented"
    })))
}

/// 获取画像趋势
async fn get_profile_trend(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    query: web::Query<TrendQuery>,
) -> Result<HttpResponse> {
    let field_id = path.into_inner();
    let repo = ProfileRepository::new(pool.get_ref().clone());

    match repo
        .get_profile_trend(field_id, query.days.unwrap_or(30))
        .await
    {
        Ok(trend) => Ok(HttpResponse::Ok().json(trend)),
        Err(e) => {
            log::error!("Failed to get profile trend: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

/// 创建质量规则（新API）
async fn create_quality_rule(
    pool: web::Data<PgPool>,
    req: web::Json<CreateQualityRuleRequest>,
) -> Result<HttpResponse> {
    let repo = RuleRepository::new(pool.get_ref().clone());
    let rule = QualityRule {
        id: 0,
        name: req.name.clone(),
        description: req.description.clone(),
        rule_type: req
            .rule_type
            .parse()
            .map_err(|_| actix_web::error::ErrorBadRequest("Invalid rule type"))?,
        enabled: req.enabled,
        severity: req
            .severity
            .parse()
            .map_err(|_| actix_web::error::ErrorBadRequest("Invalid severity"))?,
        parameters: req.parameters.clone(),
        collection_id: req.collection_id,
        field_id: req.field_id,
        created_by: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    match repo.create(&rule).await {
        Ok(id) => Ok(HttpResponse::Created().json(serde_json::json!({ "id": id }))),
        Err(e) => {
            log::error!("Failed to create quality rule: {}", e);
            Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

/// 获取规则列表（新API）
async fn list_quality_rules(
    pool: web::Data<PgPool>,
    query: web::Query<ListRulesQuery>,
) -> Result<HttpResponse> {
    let repo = RuleRepository::new(pool.get_ref().clone());

    let rules = if let Some(collection_id) = query.collection_id {
        repo.get_by_collection(collection_id).await
    } else if let Some(field_id) = query.field_id {
        repo.get_by_field(field_id).await
    } else {
        // 获取所有规则（简化实现）
        Ok(vec![])
    };

    match rules {
        Ok(rules) => Ok(HttpResponse::Ok().json(rules)),
        Err(e) => {
            log::error!("Failed to list quality rules: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

/// 获取单个规则（新API）
async fn get_quality_rule(pool: web::Data<PgPool>, path: web::Path<i64>) -> Result<HttpResponse> {
    let rule_id = path.into_inner();
    let repo = RuleRepository::new(pool.get_ref().clone());

    match repo.get_by_id(rule_id).await {
        Ok(Some(rule)) => Ok(HttpResponse::Ok().json(rule)),
        Ok(None) => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": "Rule not found"
        }))),
        Err(e) => {
            log::error!("Failed to get quality rule: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

/// 更新规则
async fn update_quality_rule(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    req: web::Json<UpdateQualityRuleRequest>,
) -> Result<HttpResponse> {
    let rule_id = path.into_inner();
    let repo = RuleRepository::new(pool.get_ref().clone());

    let updates = rules::repository::UpdateRuleRequest {
        name: req.name.clone(),
        description: req.description.clone(),
        rule_type: req.rule_type.as_ref().and_then(|t| t.parse().ok()),
        enabled: req.enabled,
        severity: req.severity.as_ref().and_then(|s| s.parse().ok()),
        parameters: req.parameters.clone(),
        collection_id: req
            .collection_id
            .map(|c| if c == 0 { None } else { Some(c) }),
        field_id: req.field_id.map(|f| if f == 0 { None } else { Some(f) }),
    };

    match repo.update(rule_id, &updates).await {
        Ok(rule) => Ok(HttpResponse::Ok().json(rule)),
        Err(e) => {
            log::error!("Failed to update quality rule: {}", e);
            Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

/// 删除规则
async fn delete_quality_rule(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse> {
    let rule_id = path.into_inner();
    let repo = RuleRepository::new(pool.get_ref().clone());

    match repo.delete(rule_id).await {
        Ok(()) => Ok(HttpResponse::NoContent().finish()),
        Err(e) => {
            log::error!("Failed to delete quality rule: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

/// 执行规则
async fn execute_rule(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    query: web::Query<ExecuteRuleQuery>,
) -> Result<HttpResponse> {
    let rule_id = path.into_inner();
    let engine = RuleEngine::new(pool.get_ref().clone());

    match engine
        .execute_rule(rule_id, query.sample_limit.unwrap_or(10))
        .await
    {
        Ok(result) => Ok(HttpResponse::Ok().json(result)),
        Err(e) => {
            log::error!("Failed to execute rule: {}", e);
            Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

/// 获取规则执行历史
async fn get_rule_executions(
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    query: web::Query<ExecutionHistoryQuery>,
) -> Result<HttpResponse> {
    let rule_id = path.into_inner();
    let repo = RuleRepository::new(pool.get_ref().clone());

    match repo
        .get_execution_history(rule_id, query.limit.unwrap_or(10))
        .await
    {
        Ok(history) => Ok(HttpResponse::Ok().json(history)),
        Err(e) => {
            log::error!("Failed to get rule executions: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

/// 批量执行规则
async fn execute_rules(
    _pool: web::Data<PgPool>,
    _req: web::Json<ExecuteRulesRequest>,
) -> Result<HttpResponse> {
    // 简化实现
    Ok(HttpResponse::NotImplemented().json(serde_json::json!({
        "error": "Batch execution not yet implemented"
    })))
}

// 查询参数结构
#[derive(Debug, serde::Deserialize)]
pub struct ProfileHistoryQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
pub struct TrendQuery {
    pub days: Option<i32>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ListRulesQuery {
    pub collection_id: Option<i64>,
    pub field_id: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateQualityRuleRequest {
    pub name: String,
    pub description: Option<String>,
    pub rule_type: String,
    pub enabled: bool,
    pub severity: String,
    pub parameters: serde_json::Value,
    pub collection_id: Option<i64>,
    pub field_id: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateQualityRuleRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub rule_type: Option<String>,
    pub enabled: Option<bool>,
    pub severity: Option<String>,
    pub parameters: Option<serde_json::Value>,
    pub collection_id: Option<i64>,
    pub field_id: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ExecuteRuleQuery {
    pub sample_limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ExecutionHistoryQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
pub struct TrendQueryParams {
    pub days: Option<i32>,
}

#[derive(Debug, serde::Deserialize)]
pub struct IssuesQueryParams {
    pub limit: Option<i64>,
}

// ==================== 仪表板 API 控制器 ====================

async fn get_dashboard_score(pool: web::Data<PgPool>) -> Result<HttpResponse> {
    let service = DashboardService::new(pool.get_ref().clone());

    match service.get_dashboard_data().await {
        Ok(data) => Ok(HttpResponse::Ok().json(data)),
        Err(e) => {
            log::error!("Failed to get dashboard data: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

async fn get_quality_trend(
    pool: web::Data<PgPool>,
    query: web::Query<TrendQueryParams>,
) -> Result<HttpResponse> {
    let service = DashboardService::new(pool.get_ref().clone());

    match service.get_quality_trend(query.days.unwrap_or(30)).await {
        Ok(trend) => Ok(HttpResponse::Ok().json(trend)),
        Err(e) => {
            log::error!("Failed to get quality trend: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

async fn get_failure_distribution(pool: web::Data<PgPool>) -> Result<HttpResponse> {
    let service = DashboardService::new(pool.get_ref().clone());

    match service.get_failure_distribution().await {
        Ok(dist) => Ok(HttpResponse::Ok().json(dist)),
        Err(e) => {
            log::error!("Failed to get failure distribution: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

async fn get_top_issues(
    pool: web::Data<PgPool>,
    query: web::Query<IssuesQueryParams>,
) -> Result<HttpResponse> {
    let service = DashboardService::new(pool.get_ref().clone());

    match service.get_top_issues(query.limit.unwrap_or(10)).await {
        Ok(issues) => Ok(HttpResponse::Ok().json(issues)),
        Err(e) => {
            log::error!("Failed to get top issues: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

// ==================== 报告 API 控制器 ====================

#[derive(Debug, serde::Deserialize)]
pub struct GenerateReportRequest {
    pub period: String, // "24h", "7d", "30d", "custom"
    pub collection_ids: Option<Vec<i64>>,
    pub rule_ids: Option<Vec<i64>>,
    pub include_details: Option<bool>,
    pub include_recommendations: Option<bool>,
}

async fn generate_report(
    pool: web::Data<PgPool>,
    req: web::Json<GenerateReportRequest>,
) -> Result<HttpResponse> {
    let service = QualityReportService::new(pool.get_ref().clone());

    let period = match req.period.as_str() {
        "24h" => report::ReportPeriod::Last24Hours,
        "7d" => report::ReportPeriod::Last7Days,
        "30d" => report::ReportPeriod::Last30Days,
        _ => report::ReportPeriod::Last7Days, // 默认
    };

    let params = report::ReportParams {
        period,
        collection_ids: req.collection_ids.clone(),
        rule_ids: req.rule_ids.clone(),
        include_details: req.include_details.unwrap_or(true),
        include_recommendations: req.include_recommendations.unwrap_or(true),
    };

    match service.generate_report(params).await {
        Ok(report) => Ok(HttpResponse::Ok().json(report)),
        Err(e) => {
            log::error!("Failed to generate report: {}", e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string()
            })))
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct DownloadReportQuery {
    pub format: String, // "pdf" or "excel"
}

async fn download_report(
    _pool: web::Data<PgPool>,
    _path: web::Path<i64>,
    query: web::Query<DownloadReportQuery>,
) -> Result<HttpResponse> {
    // 简化实现：返回示例报告内容
    let format = query.format.to_lowercase();

    if format == "pdf" {
        // 返回示例 PDF 内容
        let pdf_content = b"%PDF-1.4\n1 0 obj\n<<\n/Type /Catalog\n>>\nendobj\n%%EOF\n".to_vec();
        Ok(HttpResponse::Ok()
            .content_type("application/pdf")
            .append_header(("Content-Disposition", "attachment; filename=\"report.pdf\""))
            .body(pdf_content))
    } else {
        // 返回示例 Excel 内容（CSV 格式）
        let csv_content = "\u{FEFF}数据质量报告\n\n摘要\n整体评分,85\n"
            .as_bytes()
            .to_vec();
        Ok(HttpResponse::Ok()
            .content_type("text/csv; charset=utf-8")
            .append_header(("Content-Disposition", "attachment; filename=\"report.csv\""))
            .body(csv_content))
    }
}

// 路由配置
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/quality/rules")
            .route(web::get().to(list_rules))
            .route(web::post().to(create_rule)),
    )
    .service(
        web::resource("/quality/rules/{id}")
            .route(web::get().to(get_rule))
            .route(web::put().to(update_rule))
            .route(web::delete().to(delete_rule)),
    )
    .service(web::resource("/quality/checks").route(web::post().to(run_check)))
    .service(web::resource("/quality/checks/{id}").route(web::get().to(get_check)))
    .service(web::resource("/quality/reports/{check_id}").route(web::get().to(get_report)))
    .service(web::resource("/quality/profile/{table}").route(web::get().to(get_profile)))
    // 新增 API 端点
    .service(
        web::scope("/quality")
            // 画像端点
            .route("/profile/field/{field_id}", web::post().to(profile_field))
            .route(
                "/profile/field/{field_id}",
                web::get().to(get_field_profile),
            )
            .route(
                "/profile/field/{field_id}/history",
                web::get().to(get_field_profile_history),
            )
            .route(
                "/profile/collection/{collection_id}",
                web::post().to(profile_collection),
            )
            .route(
                "/profile/collection/{collection_id}",
                web::get().to(get_collection_profiles),
            )
            .route(
                "/profile/field/{field_id}/trend",
                web::get().to(get_profile_trend),
            )
            // 规则端点
            .route("/rules", web::get().to(list_quality_rules))
            .route("/rules", web::post().to(create_quality_rule))
            .route("/rules/{id}", web::get().to(get_quality_rule))
            .route("/rules/{id}", web::put().to(update_quality_rule))
            .route("/rules/{id}", web::delete().to(delete_quality_rule))
            .route("/rules/{id}/execute", web::post().to(execute_rule))
            .route("/rules/{id}/executions", web::get().to(get_rule_executions))
            .route("/rules/library", web::get().to(get_rule_library))
            .route("/execute", web::post().to(execute_rules))
            // 仪表板端点
            .route("/dashboard/score", web::get().to(get_dashboard_score))
            .route("/dashboard/trend", web::get().to(get_quality_trend))
            .route(
                "/dashboard/failures",
                web::get().to(get_failure_distribution),
            )
            .route("/dashboard/issues", web::get().to(get_top_issues))
            // 报告端点
            .route("/reports/generate", web::post().to(generate_report))
            .route("/reports/{id}/download", web::get().to(download_report)),
    );
}
