//! 审批岗位（position）只读列表 Handler
//!
//! 岗位实体 = `isahl.zc_id_subj-position`（identity-org 组织域维护：建岗挂人）。
//! authority 仅提供只读列表供审批 UI（流程设计器节点岗位 / 转办抄送）选择——
//! 审批岗位类别字典（zc_id_cate-approve_role 直管/代理/升级/备选）不在此端点。
//! NGAC 资源复用 "approval-roles"（审批候选岗位可见性同审批角色）。
//!
//! `fk_user`（任职人 auth user）随行返回：前端据此按岗位名（notice）分组并在
//! 岗位→任职员工 间推导任职关系（员工 fk_user 命中岗位名下任职记录即任职人），
//! 支撑设计器「审批岗位+审批员工」级联筛选（add-approver-position-employee-cascade）。

use actix_web::{web, HttpRequest, HttpResponse};
use serde::Serialize;
use sqlx::PgPool;

use crate::ngac::NgacGuard;

/// 注册审批岗位只读路由
pub fn register<G: NgacGuard + 'static>(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/positions").route(web::get().to(list_positions::<G>)));
}

/// 岗位选项 DTO（id/fk_user 串化——zuid 超 2^53，禁止数值化）
#[derive(Serialize)]
struct PositionOption {
    #[serde(with = "common::serde_zuid")]
    id: i64,
    name: String,
    /// 任职人 auth user（未挂人岗位为 null；级联任职推导仅用非空行）
    #[serde(with = "common::serde_zuid::opt")]
    fk_user: Option<i64>,
}

/// GET /positions — 全部活跃岗位（含未挂人岗位；发布时直管缺位由代理兜底）
async fn list_positions<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    _req: HttpRequest,
) -> Result<HttpResponse, actix_web::Error> {
    let _guard = G::default();
    let rows: Vec<(i64, String, Option<i64>)> = sqlx::query_as(
        r#"SELECT id, notice AS name, fk_user FROM isahl."zc_id_subj-position"
           WHERE deleted_at IS NULL AND _f_ IS NULL
           ORDER BY notice, id"#,
    )
    .fetch_all(pool.get_ref())
    .await
    .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))?;
    let out: Vec<PositionOption> = rows
        .into_iter()
        .map(|(id, name, fk_user)| PositionOption { id, name, fk_user })
        .collect();
    Ok(HttpResponse::Ok().json(out))
}
