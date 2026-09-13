//! 部门岗位 Handler — org-wz 综合管理模块
//!
//! 端点（挂载在 /service/isahl-db）：
//! - 部门 CRUD:   `GET/POST /departments`、`GET/PUT/DELETE /departments/{id}`
//! - 岗位 CRUD:   `GET/POST /positions`、`GET/PUT/DELETE /positions/{id}`
//! - 岗位编制范例: `POST /positions/templates`、`POST /positions/templates/{id}/instantiate`、`DELETE /positions/templates/{id}`
//! - 部门↔岗位：   `GET /departments/{id}/positions`、`POST /departments/{id}/positions`、`DELETE /departments/{id}/positions/{relId}`
//!
//! 零 DDL：复用现有叶表族
//! - `isahl.zc_id_orga-department`       — 部门（notice=名称, code=编码, comments=备注）
//! - `isahl.zc_id_subj-position`         — 岗位（notice=名称, code, comments, fk_user, fk_parent, ck_category）
//! - `isahl.zc_id_subj-org_rr_position`  — 部门↔岗位分配（ref_left=部门, ref_right=岗位）
//!
//! D-2a 岗位 tpl 双态（设计/实现分层，tpl_id 同表关联铁律）：
//! - 编制范例行（模板）= `POST /positions/templates` 建：tpl_id=NULL +
//!   `_f_='设计' AND _t_='范例'`（类写入契约 §4.3.3 形态 2 显式字面量对）——
//!   与既有真实岗位行（类列 NULL：legacy 直建 + 实例行）以 `_f_ IS NULL` 判别；
//! - 实例行 = `instantiate_position_template` 建：tpl_id=范例 id、
//!   ck_category 继承范例类别、notice=范例名(+序号)；实例即真实岗位。
//! - 类别校验（B-1 align-cognition-ua-category 同源约束）：ck_category 必须指向
//!   `zc_id_category` **基表行**（tableoid 过滤），子族字典（zc_id_cate-position 等）
//!   不派生 `position:{类别code}` UA——岗位读径统一 `_f_ IS NULL` 排除范例行。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};

// ═══════════════════════════════════════════════════════════
// DTO — 部门
// ═══════════════════════════════════════════════════════════

/// 部门列表/详情 DTO（camelCase，L2 语义）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepartmentDto {
    #[serde(with = "common::serde_zuid")]
    pub(crate) id: i64,
    /// notice → name
    pub(crate) name: String,
    pub(crate) code: String,
    pub(crate) comments: String,
    /// 父部门 id（org_rr_subordinate 桥派生；根部门为 null）
    #[serde(with = "common::serde_zuid::opt")]
    pub(crate) parent_id: Option<i64>,
}

/// POST /departments 请求体
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDepartmentRequest {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) code: String,
    #[serde(default)]
    pub(crate) comments: String,
    /// 父部门 id（org_rr_subordinate 桥 ref_left；可空=根部门）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    pub(crate) parent_id: Option<i64>,
    /// 组织叶表选择：department（默认）/ non_banking_legal；legal/bank-commercial → 400
    #[serde(default)]
    pub(crate) leaf: Option<String>,
}

/// PUT /departments/{id} 请求体
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDepartmentRequest {
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) code: Option<String>,
    #[serde(default)]
    pub(crate) comments: Option<String>,
    /// 父部门调整（None=不变；显式 null 语义未启用）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    pub(crate) parent_id: Option<i64>,
}

// ═══════════════════════════════════════════════════════════
// DTO — 岗位
// ═══════════════════════════════════════════════════════════

/// 岗位任职员工项（任职桥 `zc_id_subj-post_rr_employee` 活行）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionEmployeeDto {
    #[serde(with = "common::serde_zuid")]
    id: i64,
    /// 任职主体名称（`zc_id_subjects` 根 → `zc_id_contacts` 兜底；两树皆无 → null）
    name: Option<String>,
}

/// 岗位列表/详情 DTO（camelCase，L2 语义）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionDto {
    #[serde(with = "common::serde_zuid")]
    id: i64,
    name: String,
    code: String,
    comments: String,
    /// fk_user → userId（经 JOIN zc_id_entity 解析任职人名称）
    #[serde(with = "common::serde_zuid::opt")]
    user_id: Option<i64>,
    user_name: Option<String>,
    /// 上级岗位 ID
    #[serde(with = "common::serde_zuid::opt")]
    parent_id: Option<i64>,
    /// 岗位分类
    category: String,
    /// 所属组织 ID 列表（org_rr_position：ref_left=组织 / ref_right=岗位）
    #[serde(with = "common::serde_zuid::seq")]
    org_ids: Vec<i64>,
    /// 下辖组织 ID 列表（post_rr_subordinate：ref_left=岗位 / ref_right=下辖组织）
    #[serde(with = "common::serde_zuid::seq")]
    sub_org_ids: Vec<i64>,
    /// 任职员工（post_rr_employee：ref_left=岗位 / ref_right=任职主体）
    ///
    /// 与 `user_id`（主表 `fk_user` 标量任职人，岗位表单从不写）**不同源**：
    /// 「添加任职员工」写端即本桥，岗位管理页展示 MUST 取本字段
    /// （曾因读 `fk_user` 导致绑定成功却恒不展示）。
    employees: Vec<PositionEmployeeDto>,
}

/// POST /positions 请求体
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePositionRequest {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) code: String,
    #[serde(default)]
    pub(crate) comments: String,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    pub(crate) user_id: Option<i64>,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    pub(crate) parent_id: Option<i64>,
    #[serde(default)]
    pub(crate) category: String,
    /// 所属组织 ID 列表（全量替换，org_rr_position）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::seq")]
    pub(crate) org_ids: Vec<i64>,
    /// 下辖组织 ID 列表（全量替换，post_rr_subordinate）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::seq")]
    pub(crate) sub_org_ids: Vec<i64>,
}

/// PUT /positions/{id} 请求体
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePositionRequest {
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) code: Option<String>,
    #[serde(default)]
    pub(crate) comments: Option<String>,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    pub(crate) user_id: Option<i64>,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    pub(crate) parent_id: Option<i64>,
    #[serde(default)]
    pub(crate) category: Option<String>,
    /// 所属组织 ID 列表（全量替换；缺省=不变更）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::seq")]
    pub(crate) org_ids: Vec<i64>,
    /// 下辖组织 ID 列表（全量替换；缺省=不变更）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::seq")]
    pub(crate) sub_org_ids: Vec<i64>,
}

/// 岗位编制范例（模板）DTO
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionTemplateDto {
    #[serde(with = "common::serde_zuid")]
    id: i64,
    /// 范例名（notice；实例行 notice 由此派生）
    name: String,
    code: String,
    /// 岗位类别（ck_category 基表行 code）
    category: String,
    /// 编制说明（comments 列自由文本原样；编制上限另载 projection，见 [`headcount_carriers`]）
    comments: String,
}

/// POST /positions/templates 请求体 — 建岗位编制范例（D-2a 设计态）
///
/// 编制元数据落既有结构（零 DDL；comments 回归自由文本）：
/// 说明 `note` → comments 列自由文本；上限 `max_heads` → `projection` 文本载荷列
/// （见 [`headcount_carriers`]）。缺省键不落列（NULL = 无说明/不设限）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePositionTemplateRequest {
    /// 范例名（可选；缺省回退类别 code 作为名——类别即编制语义锚点）
    #[serde(default)]
    name: String,
    /// 业务编号（可选；空 → NULL）
    #[serde(default)]
    code: String,
    /// 必填：岗位类别 code——须为 `zc_id_category` 基表行（B-1 派生同源约束；
    /// 子族字典如 zc_id_cate-position 不派生 UA，不用于范例）
    category: String,
    /// 编制人数上限（可空；落 projection 文本载荷，实例化时回解析执行）
    #[serde(default)]
    max_heads: Option<i64>,
    /// 编制说明/任职规则（可空；落 comments 自由文本）
    #[serde(default)]
    note: Option<String>,
}

// ═══════════════════════════════════════════════════════════
// DTO — 部门↔岗位分配关系
// ═══════════════════════════════════════════════════════════

/// 部门岗位分配关系 DTO（POST /departments/{id}/positions 响应）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeptPositionRelationDto {
    #[serde(with = "common::serde_zuid")]
    pub(crate) id: i64,
    #[serde(with = "common::serde_zuid")]
    pub(crate) department_id: i64,
    #[serde(with = "common::serde_zuid")]
    pub(crate) position_id: i64,
    pub(crate) position_name: String,
}

/// 部门岗位列表项（GET /departments/{id}/positions）：完整岗位字段 + 关系行 id
///
/// relId 为关联表 `zc_id_subj-org_rr_position` 行 id，前端解除分配时回传
/// `DELETE /departments/{id}/positions/{relId}`（不是 Position.id）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeptPositionItem {
    /// 关系行 id（解除分配用）
    #[serde(with = "common::serde_zuid")]
    rel_id: i64,
    /// 岗位 id
    #[serde(with = "common::serde_zuid")]
    id: i64,
    name: String,
    code: String,
    comments: String,
    /// fk_user → userId（经 JOIN zc_id_entity 解析任职人名称）
    #[serde(with = "common::serde_zuid::opt")]
    user_id: Option<i64>,
    user_name: Option<String>,
    /// 上级岗位 ID
    #[serde(with = "common::serde_zuid::opt")]
    parent_id: Option<i64>,
    /// 岗位分类
    category: String,
}

/// POST /departments/{id}/positions — 分配岗位到部门（幂等）
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignPositionRequest {
    #[serde(with = "common::serde_zuid")]
    position_id: i64,
}

/// 分页查询参数
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginationQuery {
    #[serde(default)]
    page: Option<i64>,
    #[serde(default)]
    page_size: Option<i64>,
}

// ═══════════════════════════════════════════════════════════
// 写路径参数校验（对齐 subjects.rs ensure_subject_exists 范式）
// ═══════════════════════════════════════════════════════════

/// name 非空校验（trim 后为空 → 400）
pub(crate) fn validate_name(name: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::BadRequest("name 不能为空".into()));
    }
    Ok(())
}

/// 部门存在性校验（未删除 → 404）
pub(crate) async fn ensure_department_exists(pool: &PgPool, dept_id: i64) -> Result<(), ApiError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM isahl.\"zc_id_orga-department\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(dept_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !exists {
        return Err(ApiError::NotFound(format!(
            "Department not found: {}",
            dept_id
        )));
    }
    Ok(())
}

/// 桥行写入（幂等 + 复活）：软删全量替换语义下，同键旧行被软删后仍占唯一约束
/// （uq_*_ref_left_ref_right_qk_period 为含 COALESCE 的表达式约束，ON CONFLICT
/// 无法按列推断）→ 两步：先复活同键软删行（deleted_at 置空），再 INSERT
/// ON CONFLICT DO NOTHING（复活成功即跳过；行不存在则插入）。
pub(crate) async fn revive_then_insert_bridge(
    conn: &mut sqlx::PgConnection,
    table: &str,
    left: i64,
    right: i64,
    user_id: i64,
) -> Result<(), ApiError> {
    let revive = format!(
        r#"UPDATE isahl."{}" SET deleted_at = NULL, deleted_by_id = NULL
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NOT NULL"#,
        table
    );
    sqlx::query(sqlx::AssertSqlSafe(revive.as_str()))
        .bind(left)
        .bind(right)
        .execute(&mut *conn)
        .await
        .map_err(ApiError::from_sqlx)?;
    let insert = format!(
        r#"INSERT INTO isahl."{}" (ref_left, ref_right, created_by_id)
           VALUES ($1, $2, $3) ON CONFLICT DO NOTHING"#,
        table
    );
    sqlx::query(sqlx::AssertSqlSafe(insert.as_str()))
        .bind(left)
        .bind(right)
        .bind(user_id)
        .execute(&mut *conn)
        .await
        .map_err(ApiError::from_sqlx)?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════
// 部门 Handler
// ═══════════════════════════════════════════════════════════

pub(crate) type DepartmentRow = (i64, String, String, String, Option<i64>);

pub(crate) fn dept_row_to_dto(row: DepartmentRow) -> DepartmentDto {
    DepartmentDto {
        id: row.0,
        name: row.1,
        code: row.2,
        comments: row.3,
        parent_id: row.4,
    }
}

/// 部门行 SELECT（父 id 经 org_rr_subordinate 桥派生，多父取最小桥 id 为主链）
pub(crate) const DEPARTMENT_SELECT: &str = r#"SELECT d.id, d.notice::text, COALESCE(d.code, ''), COALESCE(d.comments, ''),
       (SELECT r.ref_left FROM isahl."zc_id_subj-org_rr_subordinate" r
        WHERE r.ref_right = d.id AND r.deleted_at IS NULL ORDER BY r.id LIMIT 1) AS parent_id
FROM isahl."zc_id_orga-department" d"#;

/// GET /departments
pub async fn list_departments(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    query: web::Query<PaginationQuery>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "departments", 0, "list").await?;
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * page_size;

    let items: Vec<DepartmentDto> = sqlx::query_as(AssertSqlSafe(
        format!(
            "{} WHERE d.deleted_at IS NULL ORDER BY d.id LIMIT $1 OFFSET $2",
            DEPARTMENT_SELECT
        )
        .as_str(),
    ))
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?
    .into_iter()
    .map(dept_row_to_dto)
    .collect();

    let total: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM isahl.\"zc_id_orga-department\" WHERE deleted_at IS NULL",
    )
    .fetch_one(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "items": items,
            "total": total.0,
            "page": page,
            "page_size": page_size,
        }))),
    )
}

/// POST /departments — 认证/权限门后薄委托 service::org_write::create_department（ADR A-1b 写收束）
pub async fn create_department(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<CreateDepartmentRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "departments", 0, "create").await?;
    crate::service::org_write::create_department(pool.get_ref(), body.into_inner(), user_id).await
}

/// GET /departments/{id}
pub async fn get_department(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "departments", id, "read").await?;

    let row: Option<DepartmentRow> = sqlx::query_as(AssertSqlSafe(
        format!(
            "{} WHERE d.id = $1 AND d.deleted_at IS NULL",
            DEPARTMENT_SELECT
        )
        .as_str(),
    ))
    .bind(id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    match row {
        Some(r) => Ok(HttpResponse::Ok().json(ApiResponse::success(dept_row_to_dto(r)))),
        None => Err(ApiError::NotFound("Department not found".into())),
    }
}

/// PUT /departments/{id} — 局部更新（None 字段保持不变）；薄委托 service::org_write::update_department（ADR A-1b 写收束）
pub async fn update_department(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<UpdateDepartmentRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "departments", id, "update").await?;
    crate::service::org_write::update_department(pool.get_ref(), id, body.into_inner(), user_id)
        .await
}
/// DELETE /departments/{id} — 软删除（同事务级联软删部门桥行）；薄委托 service::org_write::delete_department（ADR A-1b 写收束）
pub async fn delete_department(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "departments", id, "delete").await?;
    crate::service::org_write::delete_department(pool.get_ref(), id, user_id).await
}

// ═══════════════════════════════════════════════════════════
// 岗位 Handler
// ═══════════════════════════════════════════════════════════

/// 岗位完整行：基础字段 + 三桥聚合数组（org_rr_position / post_rr_subordinate / post_rr_employee）
pub type PositionRow = (
    i64,
    String,
    String,
    String,
    Option<i64>,
    Option<i64>,
    String,
    Option<String>,
    serde_json::Value,
    serde_json::Value,
    serde_json::Value,
);

pub fn position_row_to_dto(row: PositionRow) -> PositionDto {
    PositionDto {
        id: row.0,
        name: row.1,
        code: row.2,
        comments: row.3,
        user_id: row.4,
        user_name: row.7,
        parent_id: row.5,
        category: row.6,
        org_ids: json_id_list(&row.8),
        sub_org_ids: json_id_list(&row.9),
        employees: json_employee_list(&row.10),
    }
}

/// json_agg 聚合结果 → i64 列表（SQL 侧 COALESCE '[]' 兜底）
fn json_id_list(v: &serde_json::Value) -> Vec<i64> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
        .unwrap_or_default()
}

/// json_agg 聚合结果 → 任职员工列表（SQL 侧 COALESCE '[]' 兜底）
fn json_employee_list(v: &serde_json::Value) -> Vec<PositionEmployeeDto> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    let id = x.get("id").and_then(|i| i.as_i64())?;
                    Some(PositionEmployeeDto {
                        id,
                        name: x
                            .get("name")
                            .and_then(|n| n.as_str())
                            .map(str::to_string)
                            .filter(|s| !s.is_empty()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub const POSITION_SELECT: &str = r#"SELECT p.id, p.notice::text, COALESCE(p.code, ''), COALESCE(p.comments, ''),
       p.fk_user, p.fk_parent AS parent_id,
       COALESCE((SELECT c.code FROM isahl."zc_id_cate-position" c WHERE c.id = p.ck_category AND c.deleted_at IS NULL), (SELECT c2.notice FROM isahl.zc_id_category c2 WHERE c2.id = p.ck_category AND c2.deleted_at IS NULL), '') AS ck_category,
       COALESCE(u.name::text, NULL) AS user_name,
       COALESCE((SELECT json_agg(rp.ref_left ORDER BY rp.ref_left) FROM isahl."zc_id_subj-org_rr_position" rp WHERE rp.ref_right = p.id AND rp.deleted_at IS NULL), '[]'::json) AS org_ids,
       COALESCE((SELECT json_agg(ps.ref_right ORDER BY ps.ref_right) FROM isahl."zc_id_subj-post_rr_subordinate" ps WHERE ps.ref_left = p.id AND ps.deleted_at IS NULL), '[]'::json) AS sub_org_ids,
       COALESCE((SELECT json_agg(json_build_object(
                            'id', b.ref_right,
                            'name', COALESCE(s.notice::text, c.notice::text))
                        ORDER BY b.ref_right)
                 FROM isahl."zc_id_subj-post_rr_employee" b
                 LEFT JOIN isahl.zc_id_subjects s ON s.id = b.ref_right AND s.deleted_at IS NULL
                 LEFT JOIN isahl.zc_id_contacts c ON c.id = b.ref_right AND c.deleted_at IS NULL
                 WHERE b.ref_left = p.id AND b.deleted_at IS NULL), '[]'::json) AS employees
FROM isahl."zc_id_subj-position" p
LEFT JOIN isahl_auth.auth_users u ON u.id = p.fk_user"#;

/// GET /positions
pub async fn list_positions(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    query: web::Query<PaginationQuery>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "positions", 0, "list").await?;

    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * page_size;

    // _f_ IS NULL：真实岗位视图排除编制范例行（_f_='设计' AND _t_='范例'，D-2a）
    let sql = format!(
        "{} WHERE p.deleted_at IS NULL AND p._f_ IS NULL ORDER BY p.id LIMIT $1 OFFSET $2",
        POSITION_SELECT
    );
    let items: Vec<PositionDto> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?
        .into_iter()
        .map(position_row_to_dto)
        .collect();

    let total: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM isahl.\"zc_id_subj-position\" WHERE deleted_at IS NULL AND _f_ IS NULL",
    )
    .fetch_one(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "items": items,
            "total": total.0,
            "page": page,
            "page_size": page_size,
        }))),
    )
}

/// POST /positions
/// POST /positions — 认证/权限门后薄委托 service::org_write::create_position（ADR A-1 写收束）
pub async fn create_position(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<CreatePositionRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "positions", 0, "create").await?;
    crate::service::org_write::create_position(pool.get_ref(), body.into_inner(), user_id).await
}

/// GET /positions/{id}
pub async fn get_position(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "positions", id, "read").await?;

    // _f_ IS NULL：岗位详情为真实岗位视图（编制范例行不可经 /positions/{id} 读）
    let sql = format!(
        "{} WHERE p.id = $1 AND p.deleted_at IS NULL AND p._f_ IS NULL",
        POSITION_SELECT
    );
    let row: Option<PositionRow> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
        .bind(id)
        .fetch_optional(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?;

    match row {
        Some(r) => Ok(HttpResponse::Ok().json(ApiResponse::success(position_row_to_dto(r)))),
        None => Err(ApiError::NotFound("Position not found".into())),
    }
}

/// PUT /positions/{id} — 局部更新（None 字段保持不变）
/// PUT /positions/{id} — 局部更新（None 字段保持不变）；薄委托 service::org_write::update_position
pub async fn update_position(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<UpdatePositionRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "positions", id, "update").await?;
    crate::service::org_write::update_position(pool.get_ref(), id, body.into_inner(), user_id).await
}

/// DELETE /positions/{id} — 软删除（同事务级联软删岗位桥行）
/// DELETE /positions/{id} — 软删除（同事务级联软删岗位桥行）；薄委托 service::org_write::delete_position
pub async fn delete_position(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "positions", id, "delete").await?;
    crate::service::org_write::delete_position(pool.get_ref(), id, user_id).await
}

// ═══════════════════════════════════════════════════════════
// 岗位编制范例（D-2a 岗位 tpl 双态：建范例 = 设计态，实例化 = 落岗）
// ═══════════════════════════════════════════════════════════

/// 编制元数据 → 既有载体映射（P4 载体迁移；`comments` 回归自由文本，零 DDL）：
/// - 编制说明 `note` → `comments` 列自由文本（本体备注语义，本表既有 DTO 语义）；
/// - 编制上限 `max_heads` → `projection` 文本载荷列——本表无 `qk_*` 标量引用槽位、
///   无数值列、无编制域桥；既有先例：封签运单编号 / 定价协定新单价均落 `projection`。
///   数值以十进制文本落载，读侧回解析（见 [`parse_headcount_cap`]）。
/// 缺省语义：缺省键 → 对应列 NULL（两键全缺 → comments/projection 均 NULL）。
pub(crate) fn headcount_carriers(
    max_heads: Option<i64>,
    note: Option<&str>,
) -> (Option<String>, Option<String>) {
    let projection = max_heads.map(|n| n.to_string());
    let comments = note
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    (projection, comments)
}

/// 编制上限读回（`projection` 文本载荷，见 [`headcount_carriers`]）：
/// 缺失/空/非十进制文本（历史派生编号值等）→ None（无上限语义，fail-open 保持存量行为）。
/// 实例化端点与方案派生器（R-D3）以本函数读回上限做编制校验。
fn parse_headcount_cap(projection: Option<&str>) -> Option<i64> {
    projection
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .parse::<i64>()
        .ok()
}

/// 岗位范例类别校验（D-2a，B-1 align-cognition-ua-category 同源约束）：
/// code 必须命中 `zc_id_category` **基表行**（tableoid 过滤）——认知派生
/// `position:{类别code}` UA 只认基表行；子族字典（zc_id_cate-position 等）与
/// 空 code 一律 400（legacy create_position 的 zc_id_cate-position 字典路径
/// 不用于设计态范例）。
async fn resolve_template_category_id(pool: &PgPool, category: &str) -> Result<i64, ApiError> {
    let category = category.trim();
    if category.is_empty() {
        return Err(ApiError::BadRequest(
            "category 不能为空：岗位范例必须绑定岗位类别（zc_id_category 基表行）".into(),
        ));
    }
    let id: Option<i64> = sqlx::query_scalar(
        r#"SELECT c.id FROM isahl.zc_id_category c
           WHERE c.code = $1 AND c.deleted_at IS NULL
             AND c.tableoid = 'isahl.zc_id_category'::regclass"#,
    )
    .bind(category)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    match id {
        Some(id) => Ok(id),
        None => Err(ApiError::BadRequest(format!(
            "未知岗位类别 code: '{}'（须为 zc_id_category 基表行，子族字典不派生）",
            category
        ))),
    }
}

/// POST /positions/templates — 建岗位编制范例（D-2a 设计态）
///
/// 落 `zc_id_subj-position` 范例行：tpl_id=NULL、`_f_='设计' AND _t_='范例'`
/// （tpl_id 同表关联铁律；类写入契约 §4.3.3 形态 2 显式字面量对——本表无
/// LifecycleBizTemplate 触发器，列值即落库值）。类别必填（基表行校验见
/// [`resolve_template_category_id`]）；编制元数据落既有结构（见 [`headcount_carriers`]：
/// 说明→comments 自由文本、上限→projection 文本载荷）。实例化经
/// `POST /positions/templates/{id}/instantiate`，不在此落关系。
pub async fn create_position_template(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<CreatePositionTemplateRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "positions", 0, "create").await?;

    let category = body.category.trim().to_string();
    let category_id = resolve_template_category_id(pool.get_ref(), &category).await?;
    // notice 可选：缺省以类别 code 为名（类别 = 编制语义锚点；范例名供实例 notice 派生）
    let name = if body.name.trim().is_empty() {
        category.clone()
    } else {
        body.name.clone()
    };
    validate_name(&name)?;
    let code = if body.code.trim().is_empty() {
        None
    } else {
        Some(body.code.trim().to_string())
    };
    let (projection, comments) = headcount_carriers(body.max_heads, body.note.as_deref());

    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(pool.get_ref(), ("TX", "FJA", "↓_GG"))
            .await
            .map_err(ApiError::from_sqlx)?;
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_subj-position"
             (notice, code, comments, projection, ck_category, tpl_id, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, NULL, '设计', '范例', $6, $7, $8)
           RETURNING id"#,
    )
    .bind(&name)
    .bind(code)
    .bind(&comments)
    .bind(&projection)
    .bind(category_id)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    Ok(
        HttpResponse::Created().json(ApiResponse::success(PositionTemplateDto {
            id,
            name,
            code: body.code.trim().to_string(),
            category,
            comments: comments.unwrap_or_default(),
        })),
    )
}

/// POST /positions/templates/{id}/instantiate — 岗位范例实例化落岗（D-2a 实现态）
///
/// 校验：范例行在册（未删除、`_f_='设计' AND _t_='范例'`、tpl_id NULL）→ 404；
/// R-D3 编制上限：范例行 `projection` 文本载荷 `max_heads>0`（见 [`headcount_carriers`]）
/// 且该范例链在册实例数已达上限（含本次将超编）→ 400（fail-closed 防超员，B-2 接入）。
/// 落实例行：`tpl_id`=范例 id（tpl_id 同表关联铁律）、`ck_category` 继承范例类别、
/// `notice`=范例名（同范例已有在册实例时追加 `-{序号}` 消歧）。实例行类列 NULL，
/// 即真实岗位（legacy 直建同判）——后续经部门分配/任职挂接端点接线
/// （B-2 heal 于分配/任职时触发，实例化本身无关系可 heal）。
pub async fn instantiate_position_template(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "positions", 0, "create").await?;
    let tpl_id = path.into_inner();

    // handler 无外层事务；序号计算+插入必须原子 → 自包事务，且先锁范例行
    // （FOR UPDATE）串行化同范例的并发实例化（ReviewerD2aS2 P3 notice 序号竞态）。
    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    // 编制上限读 `projection` 文本载荷（comments 保持自由文本，不再解析 JSON）
    let tpl: Option<(String, Option<String>, Option<i64>)> = sqlx::query_as(
        r#"SELECT notice::text, projection, ck_category FROM isahl."zc_id_subj-position"
           WHERE id = $1 AND deleted_at IS NULL AND _f_ = '设计' AND _t_ = '范例'
             AND tpl_id IS NULL
           FOR UPDATE"#,
    )
    .bind(tpl_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    let Some((tpl_name, tpl_projection, category_id)) = tpl else {
        return Err(ApiError::NotFound(format!(
            "Position template not found: {}",
            tpl_id
        )));
    };
    let Some(category_id) = category_id else {
        // 建范例强制类别非空；NULL 仅脏数据可达——fail-closed 拒绝落岗
        return Err(ApiError::BadRequest(format!(
            "Position template {} 缺类别（ck_category NULL），不可实例化",
            tpl_id
        )));
    };

    // notice = 范例名；同范例已有在册实例 → 追加序号消歧（首实例不带序号）
    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl.\"zc_id_subj-position\" WHERE tpl_id = $1 AND deleted_at IS NULL",
    )
    .bind(tpl_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    // R-D3 编制上限（B-2）：projection 载荷 max_heads>0 → 实例化前 COUNT 该范例链
    // 在册实例，已达上限（含本次新增将超编）→ 400（fail-closed；与 org_scheme
    // 激活派生模板路径同规）。范例行行锁已持——计数与插入间无并发窗口。
    if let Some(cap) = parse_headcount_cap(tpl_projection.as_deref()) {
        if cap > 0 && live >= cap {
            return Err(ApiError::BadRequest(format!(
                "岗位范例 {} 在册实例 {} 已达 maxHeads={} 编制上限，不可再实例化",
                tpl_id, live, cap
            )));
        }
    }
    let notice = if live == 0 {
        tpl_name
    } else {
        format!("{}-{}", tpl_name, live + 1)
    };

    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve_conn(&mut *tx, ("TX", "FJA", "↓_GG"))
            .await
            .map_err(ApiError::from_sqlx)?;
    let instance_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_subj-position" (notice, comments, ck_category, tpl_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, '', $2, $3, $4, $5, $6)
           RETURNING id"#,
    )
    .bind(&notice)
    .bind(category_id)
    .bind(tpl_id)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    tx.commit().await.map_err(ApiError::from_sqlx)?;

    // 读回完整实例 DTO（实例类列 NULL → _f_ IS NULL 视图可见）
    let sql = format!(
        "{} WHERE p.id = $1 AND p.deleted_at IS NULL",
        POSITION_SELECT
    );
    let full: PositionRow = sqlx::query_as(AssertSqlSafe(sql.as_str()))
        .bind(instance_id)
        .fetch_one(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?;

    Ok(HttpResponse::Created().json(ApiResponse::success(position_row_to_dto(full))))
}

/// DELETE /positions/templates/{id} — 软删岗位编制范例（D-2a 设计态回收；?10/?14 审计闭环项）
///
/// 门控：范例行在册（未删除、`_f_='设计' AND _t_='范例'`、tpl_id NULL）→ 404；
/// 已有在册实例（`tpl_id=$1 AND deleted_at IS NULL`）→ 400 且消息含实例数——
/// 实例即真实岗位，须先删尽实例方可回收范例。删除仅软删范例行自身
/// （deleted_at/deleted_by_id=操作人），不触碰实例行。
///
/// 事务内先锁范例行（FOR UPDATE），与 [`instantiate_position_template`] 同锁
/// 串行化并发：实例化先行则其行锁先取、本端点计数可见；本端点先行则范例软删后
/// 实例化行锁重读落空 404——计数与删除之间无插入窗口。
/// 审计留痕：与其余 tpl 端点一致经注释契约（D-2a 端点审计入 ?10/?14 backlog）。
pub async fn delete_position_template(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "positions", 0, "delete").await?;
    let tpl_id = path.into_inner();

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    let tpl: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_subj-position"
           WHERE id = $1 AND deleted_at IS NULL AND _f_ = '设计' AND _t_ = '范例'
             AND tpl_id IS NULL
           FOR UPDATE"#,
    )
    .bind(tpl_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    if tpl.is_none() {
        return Err(ApiError::NotFound(format!(
            "Position template not found: {}",
            tpl_id
        )));
    }

    // 在册实例计数（含软删行之外的实岗；实例本身不再有派生行）
    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM isahl.\"zc_id_subj-position\" WHERE tpl_id = $1 AND deleted_at IS NULL",
    )
    .bind(tpl_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    if live > 0 {
        return Err(ApiError::BadRequest(format!(
            "岗位范例 {} 仍有 {} 个在册实例，须先删除全部实例方可删除范例",
            tpl_id, live
        )));
    }

    let deleted = sqlx::query(
        r#"UPDATE isahl."zc_id_subj-position"
           SET deleted_at = NOW(), deleted_by_id = $2, updated_at = NOW()
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(tpl_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?
    .rows_affected();

    tx.commit().await.map_err(ApiError::from_sqlx)?;

    if deleted == 0 {
        return Err(ApiError::NotFound(format!(
            "Position template not found: {}",
            tpl_id
        )));
    }

    Ok(HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({ "deleted": true }))))
}

// ═══════════════════════════════════════════════════════════
// 部门↔岗位分配 Handler
// ═══════════════════════════════════════════════════════════

/// GET /departments/{id}/positions — 返回该部门的所有岗位（完整 PositionDto + relId）
pub async fn list_department_positions(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let dept_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "departments", dept_id, "read").await?;

    ensure_department_exists(pool.get_ref(), dept_id).await?;

    // 关联表 JOIN 岗位表（LEFT JOIN zc_id_entity 解析任职人名称），
    // 返回前端渲染所需的完整 PositionDto 字段 + 关系行 id（rel_id）。
    #[allow(clippy::type_complexity)] // sqlx 行类型
    let items: Vec<DeptPositionItem> = sqlx::query_as(
        r#"SELECT r.id AS rel_id, p.id, p.notice::text, p.code, p.comments,
                  p.fk_user, p.fk_parent AS parent_id,
                  COALESCE((SELECT c.code FROM isahl."zc_id_cate-position" c WHERE c.id = p.ck_category AND c.deleted_at IS NULL), (SELECT c2.notice FROM isahl.zc_id_category c2 WHERE c2.id = p.ck_category AND c2.deleted_at IS NULL), '') AS ck_category,
                  COALESCE(u.name::text, NULL) AS user_name
           FROM isahl."zc_id_subj-org_rr_position" r
           JOIN isahl."zc_id_subj-position" p ON p.id = r.ref_right
           LEFT JOIN isahl_auth.auth_users u ON u.id = p.fk_user
           WHERE r.ref_left = $1 AND r.deleted_at IS NULL AND p.deleted_at IS NULL
           ORDER BY r.id"#,
    )
    .bind(dept_id)
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?
    .into_iter()
    .map(|row: (i64, i64, String, String, String, Option<i64>, Option<i64>, String, Option<String>)| {
        DeptPositionItem {
            rel_id: row.0,
            id: row.1,
            name: row.2,
            code: row.3,
            comments: row.4,
            user_id: row.5,
            user_name: row.8,
            parent_id: row.6,
            category: row.7,
        }
    })
    .collect();

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "items": items,
            "total": items.len(),
        }))),
    )
}

/// POST /departments/{id}/positions — 分配岗位到部门（幂等：已存在则返回已有记录）
/// POST /departments/{id}/positions — 分配岗位到部门（幂等）；薄委托 service::org_write::assign_position_to_department
pub async fn assign_position_to_department(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<AssignPositionRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let dept_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "departments", dept_id, "update").await?;
    let position_id = body.position_id;
    crate::service::org_write::assign_position_to_department(pool.get_ref(), dept_id, position_id)
        .await
}

/// DELETE /departments/{id}/positions/{relId} — 移除部门岗位关联（软删除）
/// DELETE /departments/{id}/positions/{relId} — 移除部门岗位关联（软删）；薄委托 service::org_write::remove_position_from_department
pub async fn remove_position_from_department(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (dept_id, rel_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "departments", dept_id, "update").await?;
    crate::service::org_write::remove_position_from_department(pool.get_ref(), dept_id, rel_id)
        .await
}

// ═══════════════════════════════════════════════════════════
// 注册路由
// ═══════════════════════════════════════════════════════════
// ═══════════════════════════════════════════════════════════
// DTO — 组织树 / 任职 / 组成员
// ═══════════════════════════════════════════════════════════

/// POST /org-tree/{id}/children 请求体
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildRefRequest {
    #[serde(with = "common::serde_zuid")]
    child_id: i64,
}

/// POST .../employees、POST /groups/{id}/members 请求体
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectRefRequest {
    #[serde(with = "common::serde_zuid")]
    subject_id: i64,
}

/// 组织子树项（GET /org-tree/{id}/subtree）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgSubtreeItem {
    #[serde(with = "common::serde_zuid")]
    id: i64,
    notice: String,
    level: i32,
}

// ═══════════════════════════════════════════════════════════
// 写路径校验
// ═══════════════════════════════════════════════════════════

/// 组织存在性校验（orga-department ∪ orga-non-banking-legal，未删除 → 404）
pub(crate) async fn ensure_org_exists(pool: &PgPool, org_id: i64) -> Result<(), ApiError> {
    // 组织族判定（2026-09-11 放宽）：`zc_id_subj-org` 子树 = 组织类（zc_id_orga-department /
    // zc_id_orga-legal / zc_id_orga-non-banking-legal / zc_id_bank-commercial 及其自身），
    // PG 继承语义下按父表查询即覆盖全部叶表与**中间表**。原实现只认两张叶表
    // （department / non-banking-legal），使落在中间表 zc_id_orga-legal 的既有法人
    // （如平台运营主体 WZ-BIZ-YYKJ-01）无法挂雇员（返回 404 Organization not found），
    // 而"运营录司机并挂组织"正是该端点的主用途（运营代录模式，2026-09-11 裁决）。
    let exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
            SELECT 1 FROM isahl."zc_id_subj-org" WHERE id = $1 AND deleted_at IS NULL
        )"#,
    )
    .bind(org_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !exists {
        return Err(ApiError::NotFound(format!(
            "Organization not found: {}",
            org_id
        )));
    }
    Ok(())
}

/// 群组存在性校验（zc_id_subj-group，未删除 → 404）
async fn ensure_group_exists(pool: &PgPool, group_id: i64) -> Result<(), ApiError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM isahl.\"zc_id_subj-group\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(group_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !exists {
        return Err(ApiError::NotFound(format!("Group not found: {}", group_id)));
    }
    Ok(())
}

/// 主体存在性校验（subjects 继承链统一可见，未删除 → 404）
async fn ensure_subject_exists(pool: &PgPool, subject_id: i64) -> Result<(), ApiError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM isahl.\"zc_id_subjects\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !exists {
        return Err(ApiError::NotFound(format!(
            "Subject not found: {}",
            subject_id
        )));
    }
    Ok(())
}

/// 任职主体类型路由：subjectId 必须命中 zc_id_empl-natural / zc_id_empl-agent 叶表
/// （两者继承 zc_id_subjects），或本模块「员工=通讯录档案」语义下的 zc_id_contacts
/// 联系人行（employee-list 页任职绑定主体；fix-avic-employee-assignment-contacts）；
/// 皆无 → 400（IoT 设备请走 empl-agent 通道）
pub(crate) async fn route_employee_subject(
    pool: &PgPool,
    subject_id: i64,
) -> Result<&'static str, ApiError> {
    let in_natural: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM isahl.\"zc_id_empl-natural\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if in_natural {
        return Ok("natural");
    }
    let in_agent: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM isahl.\"zc_id_empl-agent\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if in_agent {
        return Ok("agent");
    }
    let in_contact: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM isahl.zc_id_contacts WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if in_contact {
        return Ok("contact");
    }
    Err(ApiError::BadRequest(format!(
        "任职主体不存在或类型不支持（subjectId={} 不在 zc_id_empl-natural / zc_id_empl-agent / zc_id_contacts 任意表中）",
        subject_id
    )))
}

/// GET /users — 系统用户列表（任职人候选；auth_users，限 200 行）
pub async fn list_users(
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "list").await?;

    #[derive(Debug, Serialize, sqlx::FromRow)]
    struct UserItem {
        #[serde(with = "common::serde_zuid")]
        id: i64,
        name: Option<String>,
    }
    let items: Vec<UserItem> = sqlx::query_as(
        r#"SELECT id, name FROM isahl_auth.auth_users
           WHERE status = 'active' AND is_active
           ORDER BY name LIMIT 200"#,
    )
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

// ═══════════════════════════════════════════════════════════
// 组织树 Handler（zc_id_subj-org_rr_subordinate：ref_left=上级 / ref_right=下属）
// ═══════════════════════════════════════════════════════════

/// POST /org-tree/{id}/children — 挂接下属组织（幂等）；薄委托 service::org_write::add_org_tree_child（ADR A-1b 写收束）
pub async fn add_org_tree_child(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<ChildRefRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let parent_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "org-tree", parent_id, "update").await?;
    crate::service::org_write::add_org_tree_child(
        pool.get_ref(),
        parent_id,
        body.into_inner().child_id,
        user_id,
    )
    .await
}

/// DELETE /org-tree/{id}/children/{childId} — 解除挂接（软删）；薄委托 service::org_write::remove_org_tree_child（ADR A-1b 写收束）
pub async fn remove_org_tree_child(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (parent_id, child_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "org-tree", parent_id, "update").await?;
    crate::service::org_write::remove_org_tree_child(pool.get_ref(), parent_id, child_id, user_id)
        .await
}

/// GET /org-tree/{id}/subtree — 递归子树（WITH RECURSIVE 沿 org_rr_subordinate 下钻，
/// 双叶表解析 notice；level 0 = 根自身；深度上限防脏数据环）
pub async fn get_org_subtree(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let root_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "org-tree", root_id, "read").await?;
    ensure_org_exists(pool.get_ref(), root_id).await?;

    #[allow(clippy::type_complexity)]
    let rows: Vec<(i64, String, i32)> = sqlx::query_as(
        r#"WITH RECURSIVE subtree AS (
            SELECT o.id, COALESCE(o.notice, '') AS notice, 0 AS level
            FROM (
                SELECT id, notice FROM isahl."zc_id_orga-department" WHERE deleted_at IS NULL
                UNION ALL
                SELECT id, notice FROM isahl."zc_id_orga-non-banking-legal" WHERE deleted_at IS NULL
            ) o WHERE o.id = $1
            UNION ALL
            SELECT n.id, COALESCE(n.notice, ''), s.level + 1
            FROM subtree s
            JOIN isahl."zc_id_subj-org_rr_subordinate" r ON r.ref_left = s.id AND r.deleted_at IS NULL
            JOIN (
                SELECT id, notice FROM isahl."zc_id_orga-department" WHERE deleted_at IS NULL
                UNION ALL
                SELECT id, notice FROM isahl."zc_id_orga-non-banking-legal" WHERE deleted_at IS NULL
            ) n ON n.id = r.ref_right
            WHERE s.level < 64
        )
        SELECT id, notice, level FROM subtree ORDER BY level, id"#,
    )
    .bind(root_id)
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    let items: Vec<OrgSubtreeItem> = rows
        .into_iter()
        .map(|(id, notice, level)| OrgSubtreeItem { id, notice, level })
        .collect();
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "items": items,
            "total": items.len(),
        }))),
    )
}

// ═══════════════════════════════════════════════════════════
// 任职 Handler（post_rr_employee / org_rr_employee：ref_left=岗位/组织，ref_right=任职者）
// ═══════════════════════════════════════════════════════════

/// POST /positions/{id}/employees — 岗位任职挂接（幂等）
/// POST /positions/{id}/employees — 岗位任职挂接（幂等）；薄委托 service::org_write::add_position_employee
pub async fn add_position_employee(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<SubjectRefRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let position_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "positions", position_id, "update").await?;
    let subject_id = body.subject_id;
    crate::service::org_write::add_position_employee(
        pool.get_ref(),
        position_id,
        subject_id,
        user_id,
    )
    .await
}

/// DELETE /positions/{id}/employees/{subjectId} — 解除任职（软删除）
/// DELETE /positions/{id}/employees/{subjectId} — 解除任职（软删）；薄委托 service::org_write::remove_position_employee
pub async fn remove_position_employee(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (position_id, subject_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "positions", position_id, "update").await?;
    crate::service::org_write::remove_position_employee(pool.get_ref(), position_id, subject_id)
        .await
}

/// POST /organizations/{id}/employees — 组织任职挂接（幂等）
pub async fn add_org_employee(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<SubjectRefRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let org_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "organizations", org_id, "update").await?;
    ensure_org_exists(pool.get_ref(), org_id).await?;
    let subject_id = body.subject_id;
    let kind = route_employee_subject(pool.get_ref(), subject_id).await?;

    let existing: Option<(i64,)> = sqlx::query_as(
        r#"SELECT id FROM isahl."zc_id_subj-org_rr_employee"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(org_id)
    .bind(subject_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    if let Some((rel_id,)) = existing {
        return Ok(
            HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                "id": rel_id.to_string(),
                "organizationId": org_id.to_string(),
                "subjectId": subject_id.to_string(),
                "subjectKind": kind,
                "created": false,
            }))),
        );
    }

    // 复活同键软删行（唯一约束含 qk_period 表达式无法 ON CONFLICT 推断）
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-org_rr_employee" SET deleted_at = NULL, deleted_by_id = NULL
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NOT NULL"#,
    )
    .bind(org_id)
    .bind(subject_id)
    .execute(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    let row: Option<(i64,)> = sqlx::query_as(
        r#"INSERT INTO isahl."zc_id_subj-org_rr_employee" (ref_left, ref_right, created_by_id)
           VALUES ($1, $2, $3)
           ON CONFLICT DO NOTHING
           RETURNING id"#,
    )
    .bind(org_id)
    .bind(subject_id)
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    let row = match row {
        Some(r) => r,
        None => sqlx::query_as(
            r#"SELECT id FROM isahl."zc_id_subj-org_rr_employee"
                   WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
        )
        .bind(org_id)
        .bind(subject_id)
        .fetch_one(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?,
    };

    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": row.0.to_string(),
            "organizationId": org_id.to_string(),
            "subjectId": subject_id.to_string(),
            "subjectKind": kind,
            "created": true,
        }))),
    )
}

/// DELETE /organizations/{id}/employees/{subjectId} — 解除任职（软删除）
pub async fn remove_org_employee(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (org_id, subject_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "organizations", org_id, "update").await?;

    let deleted = sqlx::query(
        r#"UPDATE isahl."zc_id_subj-org_rr_employee"
           SET deleted_at = NOW()
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(org_id)
    .bind(subject_id)
    .execute(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?
    .rows_affected();

    if deleted == 0 {
        return Err(ApiError::NotFound("Employment relation not found".into()));
    }
    Ok(HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({ "deleted": true }))))
}

// ═══════════════════════════════════════════════════════════
// 组成员 Handler（zc_id_subj-group_rr_member：ref_left=群组 / ref_right=成员主体）
// ═══════════════════════════════════════════════════════════

/// POST /groups/{id}/members — 组成员挂接（幂等；成员主体须 ∈ zc_id_subjects 继承链）
pub async fn add_group_member(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<SubjectRefRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let group_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "groups", group_id, "update").await?;
    ensure_group_exists(pool.get_ref(), group_id).await?;
    let subject_id = body.subject_id;
    ensure_subject_exists(pool.get_ref(), subject_id).await?;

    let existing: Option<(i64,)> = sqlx::query_as(
        r#"SELECT id FROM isahl."zc_id_subj-group_rr_member"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(group_id)
    .bind(subject_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    if let Some((rel_id,)) = existing {
        return Ok(
            HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                "id": rel_id.to_string(),
                "groupId": group_id.to_string(),
                "subjectId": subject_id.to_string(),
                "created": false,
            }))),
        );
    }

    // 复活同键软删行（唯一约束含 qk_period 表达式无法 ON CONFLICT 推断）
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-group_rr_member" SET deleted_at = NULL, deleted_by_id = NULL
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NOT NULL"#,
    )
    .bind(group_id)
    .bind(subject_id)
    .execute(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    let row: Option<(i64,)> = sqlx::query_as(
        r#"INSERT INTO isahl."zc_id_subj-group_rr_member" (ref_left, ref_right, created_by_id)
           VALUES ($1, $2, $3)
           ON CONFLICT DO NOTHING
           RETURNING id"#,
    )
    .bind(group_id)
    .bind(subject_id)
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    let row = match row {
        Some(r) => r,
        None => sqlx::query_as(
            r#"SELECT id FROM isahl."zc_id_subj-group_rr_member"
                   WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
        )
        .bind(group_id)
        .bind(subject_id)
        .fetch_one(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?,
    };

    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": row.0.to_string(),
            "groupId": group_id.to_string(),
            "subjectId": subject_id.to_string(),
            "created": true,
        }))),
    )
}

/// DELETE /groups/{id}/members/{subjectId} — 移除组成员（软删除）
pub async fn remove_group_member(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (group_id, subject_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "groups", group_id, "update").await?;

    let deleted = sqlx::query(
        r#"UPDATE isahl."zc_id_subj-group_rr_member"
           SET deleted_at = NOW()
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(group_id)
    .bind(subject_id)
    .execute(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?
    .rows_affected();

    if deleted == 0 {
        return Err(ApiError::NotFound("Group member relation not found".into()));
    }
    Ok(HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({ "deleted": true }))))
}

/// GET /employees/{subjectId}/positions — 某任职主体（员工/通讯录联系人）的岗位任职列表
///
/// 权威来源 = `zc_id_subj-post_rr_employee` 桥（ref_left=岗位, ref_right=主体）。
/// employee-list 抽屉「任职绑定」以此为准（此前前端误用 position.sub_org_ids 派生
/// 恒空，fix-avic-employee-assignment-display）。返回数组（与 /departments、/positions 一致）。
pub async fn list_subject_position_assignments(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "positions", 0, "list").await?;
    let subject_id = path.into_inner();

    let items: Vec<SubjectAssignmentItem> = sqlx::query_as(
        r#"SELECT r.id, p.id, p.notice::text,
                  (SELECT rp.ref_left FROM isahl."zc_id_subj-org_rr_position" rp
                   WHERE rp.ref_right = p.id AND rp.deleted_at IS NULL
                   ORDER BY rp.id LIMIT 1),
                  COALESCE((SELECT d.notice FROM isahl."zc_id_orga-department" d
                     WHERE d.id = (SELECT rp2.ref_left FROM isahl."zc_id_subj-org_rr_position" rp2
                                   WHERE rp2.ref_right = p.id AND rp2.deleted_at IS NULL
                                   ORDER BY rp2.id LIMIT 1)
                       AND d.deleted_at IS NULL), '')
           FROM isahl."zc_id_subj-post_rr_employee" r
           JOIN isahl."zc_id_subj-position" p ON p.id = r.ref_left AND p.deleted_at IS NULL
           WHERE r.ref_right = $1 AND r.deleted_at IS NULL
           ORDER BY r.id"#,
    )
    .bind(subject_id)
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?
    .into_iter()
    .map(
        |row: (i64, i64, String, Option<i64>, String)| SubjectAssignmentItem {
            rel_id: row.0,
            position_id: row.1,
            position_name: row.2,
            org_id: row.3,
            org_name: row.4,
        },
    )
    .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

/// GET /employees/{subjectId}/positions 列表项（id 一律字符串化，ID_JSON_PRECISION）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectAssignmentItem {
    #[serde(with = "common::serde_zuid")]
    rel_id: i64,
    #[serde(with = "common::serde_zuid")]
    position_id: i64,
    position_name: String,
    #[serde(with = "common::serde_zuid::opt")]
    org_id: Option<i64>,
    #[serde(default)]
    org_name: String,
}

pub fn register(cfg: &mut actix_web::web::ServiceConfig) {
    cfg.service(web::resource("/users").route(web::get().to(list_users)));
    cfg.service(
        web::resource("/employees/{subjectId}/positions")
            .route(web::get().to(list_subject_position_assignments)),
    );
    cfg.service(
        web::resource("/departments")
            .route(web::get().to(list_departments))
            .route(web::post().to(create_department)),
    )
    .service(
        web::resource("/departments/{id}")
            .route(web::get().to(get_department))
            .route(web::put().to(update_department))
            .route(web::delete().to(delete_department)),
    )
    .service(
        web::resource("/departments/{id}/positions")
            .route(web::get().to(list_department_positions))
            .route(web::post().to(assign_position_to_department)),
    )
    .service(
        web::resource("/departments/{id}/positions/{relId}")
            .route(web::delete().to(remove_position_from_department)),
    )
    .service(
        web::resource("/positions")
            .route(web::get().to(list_positions))
            .route(web::post().to(create_position)),
    )
    .service(web::resource("/positions/templates").route(web::post().to(create_position_template)))
    .service(
        web::resource("/positions/templates/{id}/instantiate")
            .route(web::post().to(instantiate_position_template)),
    )
    .service(
        web::resource("/positions/templates/{id}")
            .route(web::delete().to(delete_position_template)),
    )
    .service(
        web::resource("/positions/{id}")
            .route(web::get().to(get_position))
            .route(web::put().to(update_position))
            .route(web::delete().to(delete_position)),
    )
    .service(
        web::resource("/positions/{id}/employees").route(web::post().to(add_position_employee)),
    )
    .service(
        web::resource("/positions/{id}/employees/{subjectId}")
            .route(web::delete().to(remove_position_employee)),
    )
    .service(web::resource("/organizations/{id}/employees").route(web::post().to(add_org_employee)))
    .service(
        web::resource("/organizations/{id}/employees/{subjectId}")
            .route(web::delete().to(remove_org_employee)),
    )
    .service(web::resource("/org-tree/{id}/children").route(web::post().to(add_org_tree_child)))
    .service(
        web::resource("/org-tree/{id}/children/{childId}")
            .route(web::delete().to(remove_org_tree_child)),
    )
    .service(web::resource("/org-tree/{id}/subtree").route(web::get().to(get_org_subtree)))
    .service(web::resource("/groups/{id}/members").route(web::post().to(add_group_member)))
    .service(
        web::resource("/groups/{id}/members/{subjectId}")
            .route(web::delete().to(remove_group_member)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_department_dto_serialization() {
        let dto = DepartmentDto {
            id: 1,
            name: "技术部".into(),
            code: "TECH".into(),
            comments: String::new(),
            parent_id: None,
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains(r#""name":"技术部""#));
        assert!(json.contains(r#""code":"TECH""#));
    }

    #[test]
    fn test_position_dto_serialization() {
        let dto = PositionDto {
            id: 1,
            name: "经理".into(),
            code: "MGR".into(),
            comments: "部门负责人".into(),
            user_id: Some(100),
            user_name: Some("张三".into()),
            parent_id: None,
            category: "management".into(),
            org_ids: vec![1, 2],
            sub_org_ids: vec![3],
            employees: vec![PositionEmployeeDto {
                id: 42,
                name: Some("李四".into()),
            }],
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains(r#""userId":"100""#));
        assert!(json.contains(r#""userName":"张三""#));
        assert!(json.contains(r#""parentId":null"#));
        assert!(json.contains(r#""orgIds":["1","2"]"#));
        assert!(json.contains(r#""subOrgIds":["3"]"#));
        assert!(json.contains(r#""employees":[{"id":"42","name":"李四"}]"#));
    }

    #[test]
    fn test_assign_position_request_deserialization() {
        let json = r#"{"positionId": 5}"#;
        let req: AssignPositionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.position_id, 5);
    }

    #[test]
    fn test_dept_position_relation_dto() {
        let dto = DeptPositionRelationDto {
            id: 1,
            department_id: 10,
            position_id: 20,
            position_name: "部长".into(),
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains(r#""departmentId":"10""#));
        assert!(json.contains(r#""positionId":"20""#));
    }

    /// 编制载体映射（P4 迁移）：note → comments 自由文本、max_heads → projection 载荷；
    /// 缺省键不落列（NULL），note 空串/空白不落；读侧 fail-open（缺省/非数 → None）。
    #[test]
    fn test_headcount_carriers_roundtrip_and_defaults() {
        let (projection, comments) = headcount_carriers(Some(5), Some("  需持证上岗  "));
        assert_eq!(projection.as_deref(), Some("5"));
        assert_eq!(comments.as_deref(), Some("需持证上岗"));
        // comments 侧不得出现 JSON 信封（键名/花括号）
        assert!(!comments.unwrap().contains(['{', '}', ':']));
        // 读回一致 + 边界
        assert_eq!(parse_headcount_cap(projection.as_deref()), Some(5));
        assert_eq!(parse_headcount_cap(None), None);
        assert_eq!(parse_headcount_cap(Some("")), None);
        assert_eq!(parse_headcount_cap(Some("DIM-8f3a2b1c")), None);
        // 缺省键 → 列 NULL
        let (projection, comments) = headcount_carriers(None, Some("   "));
        assert!(projection.is_none());
        assert!(comments.is_none());
    }

    /// 无说明仅上限：comments 保持 NULL（自由文本语义未被结构化载荷污染）。
    #[test]
    fn test_headcount_carriers_comments_null_without_note() {
        let (projection, comments) = headcount_carriers(Some(1), None);
        assert_eq!(projection.as_deref(), Some("1"));
        assert!(comments.is_none());
    }
}
