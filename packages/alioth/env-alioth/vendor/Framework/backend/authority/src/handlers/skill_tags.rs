//! 技能标签 HTTP Handler + 路由注册
//!
//! 泛型参数 `G: NgacGuard` 控制 NGAC defense-in-depth 行为。

use actix_web::{web, HttpRequest, HttpResponse};
use common::data::ListQuery;
use sqlx::PgPool;

use crate::models::{CreateSkillTagRequest, UpdateSkillTagRequest};
use crate::ngac::NgacGuard;
use crate::services::SkillTagService;

/// 注册技能标签相关的全部路由
pub fn register<G: NgacGuard + 'static>(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/skill-tags")
            .route(web::get().to(list_skill_tags::<G>))
            .route(web::post().to(create_skill_tag::<G>)),
    )
    .service(
        web::resource("/skill-tags/{id}")
            .route(web::get().to(get_skill_tag::<G>))
            .route(web::patch().to(update_skill_tag::<G>))
            .route(web::delete().to(delete_skill_tag::<G>)),
    )
    // 员工-技能矩阵（GET /employee-skills）：工程资源技能矩阵/抽屉数据源。
    // 独立字面路径（不与 /skill-tags 冲突）；注册于 employees CRUD 之后亦可
    // （前缀 employees 不同，无参数段遮蔽问题）。
    .service(web::resource("/employee-skills").route(web::get().to(list_employee_skills::<G>)));
}

/// GET /service/authority/employee-skills — 员工技能桥平铺行
///
/// 数据源 isahl."zc_id_relation-employee_r_skill-tags"（ref_left=员工，
/// ref_right=zc_id_tags-skill，lk_proficiency=熟练度 1-4）。矩阵页此前
/// 无后端数据源（employeeSkills 恒空 → 表头 3 技能列、单元格全 '—'）。
#[derive(serde::Serialize)]
struct EmployeeSkillRow {
    #[serde(with = "common::serde_zuid")]
    employee_id: i64,
    #[serde(with = "common::serde_zuid")]
    skill_id: i64,
    proficiency: i64,
}

async fn list_employee_skills<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, actix_web::Error> {
    let rows: Vec<EmployeeSkillRow> = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"SELECT r.ref_left, r.ref_right, r.lk_proficiency
           FROM isahl."zc_id_relation-employee_r_skill-tags" r
           JOIN isahl."zc_id_subj-employee" e ON e.id = r.ref_left AND e.deleted_at IS NULL
           JOIN isahl."zc_id_tags-skill" t ON t.id = r.ref_right AND t.deleted_at IS NULL
           WHERE r.deleted_at IS NULL
           ORDER BY r.ref_left, r.ref_right"#,
    )
    .fetch_all(pool.get_ref())
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .into_iter()
    .map(|(employee_id, skill_id, proficiency)| EmployeeSkillRow {
        employee_id,
        skill_id,
        proficiency,
    })
    .collect();
    let total = rows.len() as i64;
    Ok(
        HttpResponse::Ok().json(common::data::PaginatedResponse::new(
            rows,
            total,
            1,
            total.max(1),
        )),
    )
}

/// GET /api/service/identity/skill-tags
async fn list_skill_tags<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, actix_web::Error> {
    let guard = G::default();
    let visible_ids = guard.visible_ids(&req, "skill-tags");
    SkillTagService::new(pool.get_ref().clone())
        .list_with_rls(&query.into_inner(), visible_ids.as_deref())
        .await
        .map(|r| HttpResponse::Ok().json(r))
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

/// GET /api/service/identity/skill-tags/{id}
async fn get_skill_tag<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = common::context::require_auth(&req)?;
    let id = path.into_inner();
    let guard = G::default();
    guard
        .check_access(pool.get_ref(), user_id, "skill-tags", id, "read")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    SkillTagService::new(pool.get_ref().clone())
        .get(id)
        .await
        .map(|r| match r {
            Some(entity) => HttpResponse::Ok().json(entity),
            None => HttpResponse::NotFound().json(serde_json::json!({
                "error": "not_found",
                "message": format!("SkillTag {} not found", id)
            })),
        })
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

/// POST /api/service/identity/skill-tags
async fn create_skill_tag<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    body: web::Json<CreateSkillTagRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = common::context::require_auth(&req)?;
    let guard = G::default();
    guard
        .check_access(pool.get_ref(), user_id, "skill-tags", 0, "write")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    SkillTagService::new(pool.get_ref().clone())
        .create(body.into_inner(), user_id)
        .await
        .map(|e| HttpResponse::Created().json(e))
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

/// PATCH /api/service/identity/skill-tags/{id}
async fn update_skill_tag<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<UpdateSkillTagRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = common::context::require_auth(&req)?;
    let id = path.into_inner();
    let guard = G::default();
    guard
        .check_access(pool.get_ref(), user_id, "skill-tags", id, "write")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    SkillTagService::new(pool.get_ref().clone())
        .update(id, body.into_inner(), user_id)
        .await
        .map(|r| match r {
            Some(entity) => HttpResponse::Ok().json(entity),
            None => HttpResponse::NotFound().json(serde_json::json!({
                "error": "not_found",
                "message": format!("SkillTag {} not found", id)
            })),
        })
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}

/// DELETE /api/service/identity/skill-tags/{id}
async fn delete_skill_tag<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = common::context::require_auth(&req)?;
    let id = path.into_inner();
    let guard = G::default();
    guard
        .check_access(pool.get_ref(), user_id, "skill-tags", id, "delete")
        .await
        .map_err(actix_web::error::ErrorForbidden)?;
    SkillTagService::new(pool.get_ref().clone())
        .delete(id, user_id)
        .await
        .map(|_| HttpResponse::NoContent().finish())
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))
}
