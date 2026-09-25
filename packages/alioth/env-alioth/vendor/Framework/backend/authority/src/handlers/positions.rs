//! 审批岗位（position）只读列表 Handler
//!
//! 岗位实体 = `isahl.zc_id_subj-position`（identity-org 组织域维护：建岗挂人）。
//! authority 仅提供只读列表供审批 UI（流程设计器节点岗位 / 转办抄送）选择——
//! 审批岗位类别字典（zc_id_cate-approve_role 直管/代理/升级/备选）不在此端点。
//! NGAC 资源复用 "approval-roles"（审批候选岗位可见性同审批角色）。
//!
//! 任职字段随行返回（`repositories::list_position_options` 落地）：**行粒度 =
//! (真实岗位, 活跃任职账号)**，每行 `{ id, name, fk_user }`，`fk_user` = 该岗位的一名任职账号
//! （岗位标量 `fk_user` ∪ 任职桥 `post_rr_employee` 派生的 `empl-natural`/`empl-agent`.fk_user，
//! 去重、仅活跃；判据单一实现 `common::position_incumbent_accounts_sql!()`）。
//! 无活跃任职账号的岗位不出行（无可解析审批人）。
//!
//! 前端据此按岗位名（notice）去重并在「岗位 → 任职员工」间推导级联筛选
//! （`ApproverSelPair` / `toApproverCascade`）；**只读标量**会把仅经组织管理挂桥任职的
//! 岗位静默排除（2026-09-23 实证：AVIC 13 真实岗位仅 4 个有标量），故行按账号展开为必需。

use actix_web::{web, HttpRequest, HttpResponse};
use sqlx::PgPool;

use crate::ngac::NgacGuard;
use crate::repositories::list_position_options;

/// 注册审批岗位只读路由
pub fn register<G: NgacGuard + 'static>(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/positions").route(web::get().to(list_positions::<G>)));
}

/// GET /positions — 真实岗位的**任职账号行**（每 (真实岗位, 活跃任职账号) 一行
/// `{ id, name, fk_user }`；无活跃任职账号的岗位不出行）
///
/// 岗位可见性判据 = `common::real_position_row!`（仅排除 `_t_='范例'` 编制范例行）；
/// MUST NOT 用类列 NULL 判据——见该宏文档（2026-09-23 审批岗位下拉恒空事故）。
async fn list_positions<G: NgacGuard + 'static>(
    pool: web::Data<PgPool>,
    _req: HttpRequest,
) -> Result<HttpResponse, actix_web::Error> {
    let _guard = G::default();
    let out = list_position_options(pool.get_ref())
        .await
        .map_err(|e| actix_web::error::ErrorInternalServerError(e.to_string()))?;
    Ok(HttpResponse::Ok().json(out))
}
