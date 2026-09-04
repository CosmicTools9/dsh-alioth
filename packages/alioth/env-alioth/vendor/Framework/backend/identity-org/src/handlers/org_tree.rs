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
    id: i64,
    /// notice → name
    name: String,
    code: String,
    comments: String,
    /// 父部门 id（org_rr_subordinate 桥派生；根部门为 null）
    #[serde(with = "common::serde_zuid::opt")]
    parent_id: Option<i64>,
}

/// POST /departments 请求体
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDepartmentRequest {
    name: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    comments: String,
    /// 父部门 id（org_rr_subordinate 桥 ref_left；可空=根部门）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    parent_id: Option<i64>,
    /// 组织叶表选择：department（默认）/ non_banking_legal；legal/bank-commercial → 400
    #[serde(default)]
    leaf: Option<String>,
}

/// PUT /departments/{id} 请求体
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDepartmentRequest {
    name: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    comments: Option<String>,
    /// 父部门调整（None=不变；显式 null 语义未启用）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    parent_id: Option<i64>,
}

// ═══════════════════════════════════════════════════════════
// DTO — 岗位
// ═══════════════════════════════════════════════════════════

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
}

/// POST /positions 请求体
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePositionRequest {
    name: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    comments: String,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    user_id: Option<i64>,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    parent_id: Option<i64>,
    #[serde(default)]
    category: String,
    /// 所属组织 ID 列表（全量替换，org_rr_position）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::seq")]
    org_ids: Vec<i64>,
    /// 下辖组织 ID 列表（全量替换，post_rr_subordinate）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::seq")]
    sub_org_ids: Vec<i64>,
}

/// PUT /positions/{id} 请求体
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePositionRequest {
    name: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    comments: Option<String>,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    user_id: Option<i64>,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    parent_id: Option<i64>,
    #[serde(default)]
    category: Option<String>,
    /// 所属组织 ID 列表（全量替换；缺省=不变更）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::seq")]
    org_ids: Vec<i64>,
    /// 下辖组织 ID 列表（全量替换；缺省=不变更）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::seq")]
    sub_org_ids: Vec<i64>,
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
    /// 编制元数据 JSON 文档（comments 列原样：`{"max_heads":…,"note":…}`）
    comments: String,
}

/// POST /positions/templates 请求体 — 建岗位编制范例（D-2a 设计态）
///
/// 编制元数据（人数上限/任职说明）以 JSON 文档承载于 comments 列——
/// 文档化契约 `{"max_heads": <i64|null>, "note": "<str|null>"}`（缺省字段省略）。
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
    /// 编制人数上限（可空；并入 comments JSON 文档）
    #[serde(default)]
    max_heads: Option<i64>,
    /// 编制说明/任职规则（可空；并入 comments JSON 文档）
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
    id: i64,
    #[serde(with = "common::serde_zuid")]
    department_id: i64,
    #[serde(with = "common::serde_zuid")]
    position_id: i64,
    position_name: String,
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
fn validate_name(name: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::BadRequest("name 不能为空".into()));
    }
    Ok(())
}

/// 部门存在性校验（未删除 → 404）
async fn ensure_department_exists(pool: &PgPool, dept_id: i64) -> Result<(), ApiError> {
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

/// 真实岗位存在性校验（未删除 → 404）。
/// D-2a 双态判别：`_f_ IS NULL` = 真实岗位（legacy 直建行 + 实例行）；
/// 编制范例行（`_f_='设计' AND _t_='范例'`）不视为可引用岗位——
/// 不得作上级岗位/部门分配/任职挂接目标（防设计态行污染实现态关系）。
async fn ensure_position_exists(pool: &PgPool, position_id: i64) -> Result<(), ApiError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM isahl.\"zc_id_subj-position\"
         WHERE id = $1 AND deleted_at IS NULL AND _f_ IS NULL",
    )
    .bind(position_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !exists {
        return Err(ApiError::NotFound(format!(
            "Position not found: {}",
            position_id
        )));
    }
    Ok(())
}

/// 岗位任职人（系统用户）存在性校验——fk_user 的 id 空间 = isahl_auth.auth_users，
/// 与存储列、展示 JOIN 同源（change: align-org-position-employment-chains）。
async fn ensure_user_exists(pool: &PgPool, user_id: i64) -> Result<(), ApiError> {
    let exists: bool =
        sqlx::query_scalar("SELECT COUNT(*) > 0 FROM isahl_auth.auth_users WHERE id = $1")
            .bind(user_id)
            .fetch_one(pool)
            .await
            .map_err(ApiError::from_sqlx)?;
    if !exists {
        return Err(ApiError::NotFound(format!("任职人用户不存在: {}", user_id)));
    }
    Ok(())
}

/// 空串/空白 → None（不写 ck_category）；未知 code → 400。
/// 岗位分类字典 = zc_id_cate-position（类目-岗位；change: align-org-position-employment-chains）。
async fn resolve_category_id(pool: &PgPool, category: &str) -> Result<Option<i64>, ApiError> {
    let category = category.trim();
    if category.is_empty() {
        return Ok(None);
    }
    let id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl.\"zc_id_cate-position\" WHERE code = $1 AND deleted_at IS NULL",
    )
    .bind(category)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    match id {
        Some(id) => Ok(Some(id)),
        None => Err(ApiError::BadRequest(format!(
            "未知岗位分类 code: '{}'（字典 zc_id_cate-position）",
            category
        ))),
    }
}
/// 岗位基础行（INSERT/UPDATE RETURNING 形态，不含桥数组子查询）
type PositionBaseRow = (
    i64,
    String,
    String,
    String,
    Option<i64>,
    Option<i64>,
    String,
    Option<String>,
);

/// parent_id 环检测：新上级 `new_parent_id` 的祖先链（parent_id 递归 CTE）含当前岗位 → 400。
/// 参考 structure position.rs _SQL_CYCLE_CHECK 模式（UNION 去重防脏数据环死循环）。
async fn check_parent_cycle(
    pool: &PgPool,
    position_id: i64,
    new_parent_id: i64,
) -> Result<(), ApiError> {
    let cycle: Option<i32> = sqlx::query_scalar(
        r#"WITH RECURSIVE anc AS (
            SELECT id, fk_parent FROM isahl."zc_id_subj-position" WHERE id = $1
            UNION
            SELECT p.id, p.fk_parent FROM isahl."zc_id_subj-position" p JOIN anc a ON a.fk_parent = p.id
        ) SELECT 1 FROM anc WHERE id = $2"#,
    )
    .bind(new_parent_id)
    .bind(position_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if cycle.is_some() {
        return Err(ApiError::BadRequest(format!(
            "岗位层级成环：岗位 {} 的祖先链含自身（新上级 {}）",
            position_id, new_parent_id
        )));
    }
    Ok(())
}

/// 岗位双桥全量替换（事务内）：软删旧关联 + 逐条插新关联。
/// - org_rr_position：ref_left=组织 / ref_right=岗位（组织设岗）
/// - post_rr_subordinate：ref_left=岗位 / ref_right=下辖组织（岗位管理范围）
async fn write_position_bridges(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    position_id: i64,
    org_ids: &[i64],
    sub_org_ids: &[i64],
    user_id: i64,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-org_rr_position"
           SET deleted_at = now(), deleted_by_id = $2
           WHERE ref_right = $1 AND deleted_at IS NULL"#,
    )
    .bind(position_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    for &org_id in org_ids {
        revive_then_insert_bridge(
            &mut *tx,
            "zc_id_subj-org_rr_position",
            org_id,
            position_id,
            user_id,
        )
        .await?;
    }
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-post_rr_subordinate"
           SET deleted_at = now(), deleted_by_id = $2
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(position_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    for &org_id in sub_org_ids {
        revive_then_insert_bridge(
            &mut *tx,
            "zc_id_subj-post_rr_subordinate",
            position_id,
            org_id,
            user_id,
        )
        .await?;
    }
    Ok(())
}

/// 桥行写入（幂等 + 复活）：软删全量替换语义下，同键旧行被软删后仍占唯一约束
/// （uq_*_ref_left_ref_right_qk_period 为含 COALESCE 的表达式约束，ON CONFLICT
/// 无法按列推断）→ 两步：先复活同键软删行（deleted_at 置空），再 INSERT
/// ON CONFLICT DO NOTHING（复活成功即跳过；行不存在则插入）。
async fn revive_then_insert_bridge(
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

type DepartmentRow = (i64, String, String, String, Option<i64>);

fn dept_row_to_dto(row: DepartmentRow) -> DepartmentDto {
    DepartmentDto {
        id: row.0,
        name: row.1,
        code: row.2,
        comments: row.3,
        parent_id: row.4,
    }
}

/// 部门行 SELECT（父 id 经 org_rr_subordinate 桥派生，多父取最小桥 id 为主链）
const DEPARTMENT_SELECT: &str = r#"SELECT d.id, d.notice::text, COALESCE(d.code, ''), COALESCE(d.comments, ''),
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

/// POST /departments
pub async fn create_department(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<CreateDepartmentRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "departments", 0, "create").await?;
    validate_name(&body.name)?;
    // code 非业务唯一标识（用户裁决 2026-08-31：主体族实体输入/显示不体现编码）——
    // 空缺落 NULL，不再自动生成
    let code = if body.code.trim().is_empty() {
        None
    } else {
        Some(body.code.clone())
    };

    // 组织叶表选择（白名单字面量，无注入）：department（默认）/ non_banking_legal；
    // legal → 400（法人中间层禁写，须指定具体叶表）；bank-commercial → 400（走银行专属通道）；未知值 → 400
    let target = match body.leaf.as_deref().unwrap_or("department") {
        "department" => "isahl.\"zc_id_orga-department\"",
        "non_banking_legal" => "isahl.\"zc_id_orga-non-banking-legal\"",
        // 叶表写入规则（2026-08-29 裁决）：法人中间层 zc_id_orga-legal 禁写——
        // "legal" 曾映射中间层，违规源头已掐灭；法人须明确叶表。
        "legal" => {
            return Err(ApiError::BadRequest(
                "法人必须指定具体叶表：non_banking_legal（非银行法人）或走银行专属通道（bank-commercial）"
                    .into(),
            ));
        }
        "bank-commercial" => {
            return Err(ApiError::BadRequest(
                "银行商业机构（bank-commercial）不支持经组织管理通道创建，请走银行专属通道".into(),
            ));
        }
        other => {
            return Err(ApiError::BadRequest(format!(
                "未知组织叶表 leaf: '{}'",
                other
            )));
        }
    };
    // 最小列集 INSERT（+ created_by_id 落 owner 槽）：orga-non-banking-legal 有 fk_representative 可空列（缺省不写）
    let sql = format!(
        r#"INSERT INTO {} (notice, code, comments, created_by_id)
            VALUES ($1, $2, $3, $4)
            RETURNING id, notice::text, COALESCE(code, ''), comments"#,
        target
    );
    let row: (i64, String, String, String) = sqlx::query_as(AssertSqlSafe(sql.as_str()))
        .bind(&body.name)
        .bind(&code)
        .bind(&body.comments)
        .bind(user_id)
        .fetch_one(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?;

    // 父部门挂接（org_rr_subordinate 桥；可复活软删行）
    if let Some(pid) = body.parent_id {
        ensure_org_exists(pool.get_ref(), pid).await?;
        let mut conn = pool
            .get_ref()
            .acquire()
            .await
            .map_err(ApiError::from_sqlx)?;
        revive_then_insert_bridge(
            &mut conn,
            "zc_id_subj-org_rr_subordinate",
            pid,
            row.0,
            user_id,
        )
        .await?;
    }

    // NGAC B-2：部门行 OA + 子集 OA 树链 ensure（事务外幂等 heal，失败仅 warn）
    crate::ngac_org_ensure::heal_department_scope(pool.get_ref(), row.0).await;

    Ok(
        HttpResponse::Created().json(ApiResponse::success(DepartmentDto {
            id: row.0,
            name: row.1,
            code: row.2,
            comments: row.3,
            parent_id: body.parent_id,
        })),
    )
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

/// PUT /departments/{id} — 局部更新（None 字段保持不变）
pub async fn update_department(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<UpdateDepartmentRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "departments", id, "update").await?;
    if let Some(name) = &body.name {
        validate_name(name)?;
    }

    let updated: Option<i64> = sqlx::query_scalar(
        r#"UPDATE isahl."zc_id_orga-department"
           SET notice = COALESCE($2, notice),
               code = COALESCE($3, code),
               comments = COALESCE($4, comments)
           WHERE id = $1 AND deleted_at IS NULL
           RETURNING id"#,
    )
    .bind(id)
    .bind(&body.name)
    .bind(&body.code)
    .bind(&body.comments)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    let Some(_) = updated else {
        return Err(ApiError::NotFound("Department not found".into()));
    };

    // 父部门调整（org_rr_subordinate 桥差量替换；环检测复用挂接同一守卫）
    if let Some(pid) = body.parent_id {
        check_org_tree_cycle(pool.get_ref(), id, pid).await?;
        ensure_org_exists(pool.get_ref(), pid).await?;
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_subj-org_rr_subordinate"
               SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
               WHERE ref_right = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?;
        let mut conn = pool
            .get_ref()
            .acquire()
            .await
            .map_err(ApiError::from_sqlx)?;
        revive_then_insert_bridge(&mut conn, "zc_id_subj-org_rr_subordinate", pid, id, user_id)
            .await?;
    }

    // 重查（父 id 经桥派生，保证响应与桥一致）
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
/// DELETE /departments/{id} — 软删除（同事务级联软删部门桥行）
pub async fn delete_department(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "departments", id, "delete").await?;

    // 子部门守卫：仍是父节点（桥 ref_left）时拒绝删除，先移除/迁移子部门
    let children: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM "isahl"."zc_id_subj-org_rr_subordinate"
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .fetch_one(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    if children > 0 {
        return Err(ApiError::BadRequest(
            "存在子部门，请先移除或迁移子部门".into(),
        ));
    }

    // 单事务：主表软删 → 父桥 + 下挂桥行（org_rr_position / org_rr_employee）级联软删
    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    let deleted = sqlx::query(
        r#"UPDATE isahl."zc_id_orga-department"
           SET deleted_at = NOW()
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?
    .rows_affected();

    // 软删自身的父桥行（ref_right = id）
    if deleted > 0 {
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_subj-org_rr_subordinate"
               SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
               WHERE ref_right = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        // 下挂桥行级联：部门↔岗位分配（ref_left = 部门）
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_subj-org_rr_position"
               SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
               WHERE ref_left = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        // 下挂桥行级联：组织任职（ref_left = 部门）
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_subj-org_rr_employee"
               SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
               WHERE ref_left = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
    }

    if deleted == 0 {
        return Err(ApiError::NotFound("Department not found".into()));
    }

    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({ "deleted": true }))))
}

// ═══════════════════════════════════════════════════════════
// 岗位 Handler
// ═══════════════════════════════════════════════════════════

/// 岗位完整行：基础字段 + 双桥聚合数组（org_rr_position / post_rr_subordinate）
type PositionRow = (
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
);

fn position_row_to_dto(row: PositionRow) -> PositionDto {
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
    }
}

/// json_agg 聚合结果 → i64 列表（SQL 侧 COALESCE '[]' 兜底）
fn json_id_list(v: &serde_json::Value) -> Vec<i64> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
        .unwrap_or_default()
}

const POSITION_SELECT: &str = r#"SELECT p.id, p.notice::text, COALESCE(p.code, ''), COALESCE(p.comments, ''),
       p.fk_user, p.fk_parent AS parent_id,
       COALESCE((SELECT c.code FROM isahl."zc_id_cate-position" c WHERE c.id = p.ck_category AND c.deleted_at IS NULL), p.ck_category::text, '') AS ck_category,
       COALESCE(u.name::text, NULL) AS user_name,
       COALESCE((SELECT json_agg(rp.ref_left ORDER BY rp.ref_left) FROM isahl."zc_id_subj-org_rr_position" rp WHERE rp.ref_right = p.id AND rp.deleted_at IS NULL), '[]'::json) AS org_ids,
       COALESCE((SELECT json_agg(ps.ref_right ORDER BY ps.ref_right) FROM isahl."zc_id_subj-post_rr_subordinate" ps WHERE ps.ref_left = p.id AND ps.deleted_at IS NULL), '[]'::json) AS sub_org_ids
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
pub async fn create_position(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<CreatePositionRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "positions", 0, "create").await?;
    validate_name(&body.name)?;
    if let Some(pid) = body.parent_id {
        ensure_position_exists(pool.get_ref(), pid).await?;
    }
    if let Some(uid) = body.user_id {
        ensure_user_exists(pool.get_ref(), uid).await?;
    }
    let category_id = resolve_category_id(pool.get_ref(), &body.category).await?;
    // code 非业务唯一标识（用户裁决 2026-08-31）——空缺落 NULL，不再自动生成
    let code = if body.code.trim().is_empty() {
        None
    } else {
        Some(body.code.clone())
    };

    // 单事务：主表写 + 双桥全量替换（软删旧关联 → 插新关联），保证原子性
    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    let base: PositionBaseRow = sqlx::query_as(
        r#"INSERT INTO isahl."zc_id_subj-position" (notice, code, comments, fk_user, fk_parent, ck_category, created_by_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, notice::text, COALESCE(code, ''), COALESCE(comments, ''), fk_user, fk_parent AS parent_id,
                      COALESCE((SELECT c.code FROM isahl."zc_id_cate-position" c WHERE c.id = isahl."zc_id_subj-position".ck_category AND c.deleted_at IS NULL), ck_category::text, ''),
                      NULL::text AS user_name"#,
    )
    .bind(&body.name)
    .bind(&code)
    .bind(&body.comments)
    .bind(body.user_id)
    .bind(body.parent_id)
    .bind(category_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    write_position_bridges(&mut tx, base.0, &body.org_ids, &body.sub_org_ids, user_id).await?;

    let full: PositionRow = sqlx::query_as::<_, PositionRow>(AssertSqlSafe(
        format!(
            "{} WHERE p.id = $1 AND p.deleted_at IS NULL",
            POSITION_SELECT
        )
        .as_str(),
    ))
    .bind(base.0)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    tx.commit().await.map_err(ApiError::from_sqlx)?;

    // NGAC B-2：岗位行 OA ensure（事务外幂等 heal，失败仅 warn 不阻断主写）
    crate::ngac_org_ensure::heal_position_scope(pool.get_ref(), base.0).await;

    Ok(HttpResponse::Created().json(ApiResponse::success(position_row_to_dto(full))))
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
pub async fn update_position(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<UpdatePositionRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "positions", id, "update").await?;
    if let Some(name) = &body.name {
        validate_name(name)?;
    }
    if let Some(pid) = body.parent_id {
        ensure_position_exists(pool.get_ref(), pid).await?;
    }
    if let Some(uid) = body.user_id {
        ensure_user_exists(pool.get_ref(), uid).await?;
    }
    let category_id = match &body.category {
        Some(cat) => resolve_category_id(pool.get_ref(), cat).await?,
        None => None,
    };

    // 单事务：环检测（如变更 parent_id）→ 主表写 → 双桥全量替换
    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    if let Some(new_parent) = body.parent_id {
        check_parent_cycle(pool.get_ref(), id, new_parent).await?;
    }

    let base: Option<PositionBaseRow> = sqlx::query_as(AssertSqlSafe(
        r#"UPDATE isahl."zc_id_subj-position"
           SET notice = COALESCE($2, notice),
               code = COALESCE($3, code),
               comments = COALESCE($4, comments),
               fk_user = COALESCE($5, fk_user),
               fk_parent = COALESCE($6, fk_parent),
               ck_category = COALESCE($7, ck_category)
           WHERE id = $1 AND deleted_at IS NULL AND _f_ IS NULL
           RETURNING id, notice::text, code, comments, fk_user, fk_parent AS parent_id,
                     COALESCE((SELECT c.code FROM isahl."zc_id_cate-position" c WHERE c.id = isahl."zc_id_subj-position".ck_category AND c.deleted_at IS NULL), ck_category::text, ''),
                     NULL::text AS user_name"#,
    ))
    .bind(id)
    .bind(&body.name)
    .bind(&body.code)
    .bind(&body.comments)
    .bind(body.user_id)
    .bind(body.parent_id)
    .bind(category_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    let Some(base) = base else {
        return Err(ApiError::NotFound("Position not found".into()));
    };

    write_position_bridges(&mut tx, base.0, &body.org_ids, &body.sub_org_ids, user_id).await?;

    let full: PositionRow = sqlx::query_as::<_, PositionRow>(AssertSqlSafe(
        format!(
            "{} WHERE p.id = $1 AND p.deleted_at IS NULL",
            POSITION_SELECT
        )
        .as_str(),
    ))
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(HttpResponse::Ok().json(ApiResponse::success(position_row_to_dto(full))))
}

/// DELETE /positions/{id} — 软删除（同事务级联软删岗位桥行）
pub async fn delete_position(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "positions", id, "delete").await?;

    // 单事务：主表软删 → 四类岗位桥行级联软删（org_rr_position / post_rr_subordinate /
    // post_rr_view / post_rr_employee，alive 行；ref 方向见各 UPDATE WHERE）
    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    // _f_ IS NULL：真实岗位视图；编制范例行删除（含实例守卫）暂无删除端点
    let deleted = sqlx::query(
        r#"UPDATE isahl."zc_id_subj-position"
           SET deleted_at = NOW()
           WHERE id = $1 AND deleted_at IS NULL AND _f_ IS NULL"#,
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?
    .rows_affected();

    // 部门↔岗位分配桥（ref_right = 岗位）
    if deleted > 0 {
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_subj-org_rr_position"
               SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
               WHERE ref_right = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        // 岗位管理范围桥（ref_left = 岗位）
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_subj-post_rr_subordinate"
               SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
               WHERE ref_left = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        // 主体视角桥（ref_left = 岗位）
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_subj-post_rr_view"
               SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
               WHERE ref_left = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        // 岗位任职桥（ref_left = 岗位）
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_subj-post_rr_employee"
               SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
               WHERE ref_left = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
    }

    if deleted == 0 {
        return Err(ApiError::NotFound("Position not found".into()));
    }

    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({ "deleted": true }))))
}

// ═══════════════════════════════════════════════════════════
// 岗位编制范例（D-2a 岗位 tpl 双态：建范例 = 设计态，实例化 = 落岗）
// ═══════════════════════════════════════════════════════════

/// 编制元数据 JSON 文档构建（comments 列承载；缺省字段省略，全缺 → "{}"）。
/// 文档化契约（D-2a）：`{"max_heads": <i64|null>, "note": "<str|null>"}`——
/// 读侧（方案发布/派生器）按此解析；本文件不读回，comments 对岗位读径保持不透明文本。
fn headcount_comments_doc(max_heads: Option<i64>, note: Option<&str>) -> String {
    let mut doc = serde_json::Map::new();
    if let Some(n) = max_heads {
        doc.insert("max_heads".to_string(), serde_json::json!(n));
    }
    if let Some(note) = note.map(str::trim).filter(|s| !s.is_empty()) {
        doc.insert("note".to_string(), serde_json::json!(note));
    }
    if doc.is_empty() {
        return "{}".to_string();
    }
    serde_json::to_string(&serde_json::Value::Object(doc))
        .unwrap_or_else(|_| "{}".to_string())
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
/// [`resolve_template_category_id`]）；编制元数据存 comments JSON 文档
/// （见 [`headcount_comments_doc`]）。实例化经
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
    let comments = headcount_comments_doc(body.max_heads, body.note.as_deref());

    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_subj-position"
             (notice, code, comments, ck_category, tpl_id, _f_, _t_)
           VALUES ($1, $2, $3, $4, NULL, '设计', '范例')
           RETURNING id"#,
    )
    .bind(&name)
    .bind(code)
    .bind(&comments)
    .bind(category_id)
    .fetch_one(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    Ok(HttpResponse::Created().json(ApiResponse::success(PositionTemplateDto {
        id,
        name,
        code: body.code.trim().to_string(),
        category,
        comments,
    })))
}

/// POST /positions/templates/{id}/instantiate — 岗位范例实例化落岗（D-2a 实现态）
///
/// 校验：范例行在册（未删除、`_f_='设计' AND _t_='范例'`、tpl_id NULL）→ 404。
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

    let tpl: Option<(String, Option<i64>)> = sqlx::query_as(
        r#"SELECT notice::text, ck_category FROM isahl."zc_id_subj-position"
           WHERE id = $1 AND deleted_at IS NULL AND _f_ = '设计' AND _t_ = '范例'
             AND tpl_id IS NULL
           FOR UPDATE"#,
    )
    .bind(tpl_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    let Some((tpl_name, category_id)) = tpl else {
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
    let notice = if live == 0 {
        tpl_name
    } else {
        format!("{}-{}", tpl_name, live + 1)
    };

    let instance_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_subj-position" (notice, comments, ck_category, tpl_id)
           VALUES ($1, '', $2, $3)
           RETURNING id"#,
    )
    .bind(&notice)
    .bind(category_id)
    .bind(tpl_id)
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
                  COALESCE((SELECT c.code FROM isahl."zc_id_cate-position" c WHERE c.id = p.ck_category AND c.deleted_at IS NULL), p.ck_category::text, '') AS ck_category,
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

    // 前置存在性预检：部门与岗位必须存在（未删除），否则 404
    ensure_department_exists(pool.get_ref(), dept_id).await?;
    ensure_position_exists(pool.get_ref(), position_id).await?;

    // 幂等检查：是否已存在未删除的关联
    let existing: Option<(i64,)> = sqlx::query_as(
        r#"SELECT id FROM isahl."zc_id_subj-org_rr_position"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(dept_id)
    .bind(position_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    if let Some((rel_id,)) = existing {
        let position_name: String = sqlx::query_scalar(
            "SELECT notice::text FROM isahl.\"zc_id_subj-position\" WHERE id = $1",
        )
        .bind(position_id)
        .fetch_one(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?;

        return Ok(
            HttpResponse::Ok().json(ApiResponse::success(DeptPositionRelationDto {
                id: rel_id,
                department_id: dept_id,
                position_id,
                position_name,
            })),
        );
    }

    // 创建新关联：先复活同键软删行（唯一约束含 qk_period 表达式无法 ON CONFLICT 推断）
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-org_rr_position" SET deleted_at = NULL, deleted_by_id = NULL
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NOT NULL"#,
    )
    .bind(dept_id)
    .bind(position_id)
    .execute(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    let row: Option<(i64, String)> = sqlx::query_as(
        r#"INSERT INTO isahl."zc_id_subj-org_rr_position" (ref_left, ref_right)
            VALUES ($1, $2)
            ON CONFLICT DO NOTHING
            RETURNING id, (SELECT notice::text FROM isahl."zc_id_subj-position" WHERE id = $2)"#,
    )
    .bind(dept_id)
    .bind(position_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    let row = match row {
        Some(r) => r,
        None => {
            // 冲突（复活行）：取既有 id
            sqlx::query_as(
                r#"SELECT id, (SELECT notice::text FROM isahl."zc_id_subj-position" WHERE id = $2)
                   FROM isahl."zc_id_subj-org_rr_position" WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
            )
            .bind(dept_id)
            .bind(position_id)
            .fetch_one(pool.get_ref())
            .await
            .map_err(ApiError::from_sqlx)?
        }
    };

    // NGAC B-2：岗位新增在任分配部门 → 刷新岗位 OA ancestor 域闭包（幂等 heal）
    crate::ngac_org_ensure::heal_position_scope(pool.get_ref(), position_id).await;

    Ok(
        HttpResponse::Created().json(ApiResponse::success(DeptPositionRelationDto {
            id: row.0,
            department_id: dept_id,
            position_id,
            position_name: row.1,
        })),
    )
}

/// DELETE /departments/{id}/positions/{relId} — 移除部门岗位关联（软删除）
pub async fn remove_position_from_department(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (dept_id, rel_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "departments", dept_id, "update").await?;

    // 软删并取回被移除岗位（ref_right）——供事务外 NGAC heal 收敛 OA ancestor 域闭包
    let deleted: Option<(i64,)> = sqlx::query_as(
        r#"UPDATE isahl."zc_id_subj-org_rr_position"
           SET deleted_at = NOW()
           WHERE id = $1 AND ref_left = $2 AND deleted_at IS NULL
           RETURNING ref_right"#,
    )
    .bind(rel_id)
    .bind(dept_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    let Some((position_id,)) = deleted else {
        return Err(ApiError::NotFound("Relation not found".into()));
    };

    // NGAC B-2：岗位移除在任分配部门 → 刷新岗位 OA ancestor 域闭包（事务外幂等 heal，失败仅 warn）
    crate::ngac_org_ensure::heal_position_scope(pool.get_ref(), position_id).await;

    Ok(HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({ "deleted": true }))))
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
async fn ensure_org_exists(pool: &PgPool, org_id: i64) -> Result<(), ApiError> {
    let exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
            SELECT 1 FROM isahl."zc_id_orga-department" WHERE id = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT 1 FROM isahl."zc_id_orga-non-banking-legal" WHERE id = $1 AND deleted_at IS NULL
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
/// （两者继承 zc_id_subjects）；皆无 → 400（仅支持自然人/智能体；IoT 设备走 empl-agent 通道）
async fn route_employee_subject(pool: &PgPool, subject_id: i64) -> Result<&'static str, ApiError> {
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
    Err(ApiError::BadRequest(format!(
        "任职主体仅支持自然人/智能体（subjectId={} 不在 zc_id_empl-natural / zc_id_empl-agent 叶表）；IoT 设备请走 empl-agent 通道",
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

/// 组织树环检测：child 的祖先链/子树（org_rr_subordinate 双向递归，UNION 去重防脏数据环）
/// 不得含 parent，否则挂接将成环/成菱形 → 400
async fn check_org_tree_cycle(
    pool: &PgPool,
    child_id: i64,
    parent_id: i64,
) -> Result<(), ApiError> {
    if child_id == parent_id {
        return Err(ApiError::BadRequest("组织不能挂接为自身的子节点".into()));
    }
    let cycle: Option<i32> = sqlx::query_scalar(
        // 单向上溯（PG 递归 CTE 限制：递归引用须在 UNION 链尾，单向更稳）：
        // parent ∈ anc(child) ⇔ child 已是 parent 的祖先 → 挂接成环。
        r#"WITH RECURSIVE anc AS (
            SELECT ref_left AS node FROM isahl."zc_id_subj-org_rr_subordinate" WHERE ref_right = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT r.ref_left FROM isahl."zc_id_subj-org_rr_subordinate" r
            JOIN anc a ON a.node = r.ref_right WHERE r.deleted_at IS NULL
        )
        SELECT 1 FROM anc WHERE node = $2 LIMIT 1"#,
    )
    .bind(child_id)
    .bind(parent_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if cycle.is_some() {
        return Err(ApiError::BadRequest(format!(
            "组织树成环：组织 {} 的祖先链/子树含组织 {}，挂接被拒绝",
            child_id, parent_id
        )));
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════
// 组织树 Handler（zc_id_subj-org_rr_subordinate：ref_left=上级 / ref_right=下属）
// ═══════════════════════════════════════════════════════════

/// POST /org-tree/{id}/children — 挂接下属组织（幂等：已存在未删关联 → 返回现有）
pub async fn add_org_tree_child(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<ChildRefRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let parent_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "org-tree", parent_id, "update").await?;
    let child_id = body.child_id;

    ensure_org_exists(pool.get_ref(), parent_id).await?;
    ensure_org_exists(pool.get_ref(), child_id).await?;

    // 幂等：已存在未删除关联 → 返回现有记录
    let existing: Option<(i64,)> = sqlx::query_as(
        r#"SELECT id FROM isahl."zc_id_subj-org_rr_subordinate"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(parent_id)
    .bind(child_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    if let Some((rel_id,)) = existing {
        return Ok(
            HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                "id": rel_id.to_string(),
                "parentId": parent_id.to_string(),
                "childId": child_id.to_string(),
                "created": false,
            }))),
        );
    }

    check_org_tree_cycle(pool.get_ref(), child_id, parent_id).await?;

    // 复活同键软删行（唯一约束含 qk_period 表达式无法 ON CONFLICT 推断）
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-org_rr_subordinate" SET deleted_at = NULL, deleted_by_id = NULL
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NOT NULL"#,
    )
    .bind(parent_id)
    .bind(child_id)
    .execute(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    let row: Option<(i64,)> = sqlx::query_as(
        r#"INSERT INTO isahl."zc_id_subj-org_rr_subordinate" (ref_left, ref_right, created_by_id)
           VALUES ($1, $2, $3)
           ON CONFLICT DO NOTHING
           RETURNING id"#,
    )
    .bind(parent_id)
    .bind(child_id)
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    let row = match row {
        Some(r) => r,
        None => sqlx::query_as(
            r#"SELECT id FROM isahl."zc_id_subj-org_rr_subordinate"
                   WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
        )
        .bind(parent_id)
        .bind(child_id)
        .fetch_one(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?,
    };

    // NGAC B-2：新 org 节点 → 部门子集 OA 树链 ensure（事务外幂等 heal，失败仅 warn）
    crate::ngac_org_ensure::heal_department_scope(pool.get_ref(), child_id).await;

    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": row.0.to_string(),
            "parentId": parent_id.to_string(),
            "childId": child_id.to_string(),
            "created": true,
        }))),
    )
}

/// DELETE /org-tree/{id}/children/{childId} — 解除挂接（软删除）
pub async fn remove_org_tree_child(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (parent_id, child_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "org-tree", parent_id, "update").await?;

    let deleted = sqlx::query(
        r#"UPDATE isahl."zc_id_subj-org_rr_subordinate"
           SET deleted_at = NOW()
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(parent_id)
    .bind(child_id)
    .execute(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?
    .rows_affected();

    if deleted == 0 {
        return Err(ApiError::NotFound(
            "Organization tree relation not found".into(),
        ));
    }
    Ok(HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({ "deleted": true }))))
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
pub async fn add_position_employee(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<SubjectRefRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let position_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "positions", position_id, "update").await?;
    ensure_position_exists(pool.get_ref(), position_id).await?;
    let subject_id = body.subject_id;
    let kind = route_employee_subject(pool.get_ref(), subject_id).await?;

    let existing: Option<(i64,)> = sqlx::query_as(
        r#"SELECT id FROM isahl."zc_id_subj-post_rr_employee"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(position_id)
    .bind(subject_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    if let Some((rel_id,)) = existing {
        return Ok(
            HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                "id": rel_id.to_string(),
                "positionId": position_id.to_string(),
                "subjectId": subject_id.to_string(),
                "subjectKind": kind,
                "created": false,
            }))),
        );
    }

    // 复活同键软删行（唯一约束含 qk_period 表达式无法 ON CONFLICT 推断）
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-post_rr_employee" SET deleted_at = NULL, deleted_by_id = NULL
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NOT NULL"#,
    )
    .bind(position_id)
    .bind(subject_id)
    .execute(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    let row: Option<(i64,)> = sqlx::query_as(
        r#"INSERT INTO isahl."zc_id_subj-post_rr_employee" (ref_left, ref_right, created_by_id)
           VALUES ($1, $2, $3)
           ON CONFLICT DO NOTHING
           RETURNING id"#,
    )
    .bind(position_id)
    .bind(subject_id)
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    let row = match row {
        Some(r) => r,
        None => sqlx::query_as(
            r#"SELECT id FROM isahl."zc_id_subj-post_rr_employee"
                   WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
        )
        .bind(position_id)
        .bind(subject_id)
        .fetch_one(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?,
    };

    // NGAC B-2：任职写端后岗位行 OA/层级收敛（事务外幂等 heal，失败仅 warn）
    crate::ngac_org_ensure::heal_position_scope(pool.get_ref(), position_id).await;

    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": row.0.to_string(),
            "positionId": position_id.to_string(),
            "subjectId": subject_id.to_string(),
            "subjectKind": kind,
            "created": true,
        }))),
    )
}

/// DELETE /positions/{id}/employees/{subjectId} — 解除任职（软删除）
pub async fn remove_position_employee(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (position_id, subject_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "positions", position_id, "update").await?;

    let deleted = sqlx::query(
        r#"UPDATE isahl."zc_id_subj-post_rr_employee"
           SET deleted_at = NOW()
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(position_id)
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

pub fn register(cfg: &mut actix_web::web::ServiceConfig) {
    cfg.service(web::resource("/users").route(web::get().to(list_users)));
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
    .service(
        web::resource("/positions/templates")
            .route(web::post().to(create_position_template)),
    )
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
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains(r#""userId":"100""#));
        assert!(json.contains(r#""userName":"张三""#));
        assert!(json.contains(r#""parentId":null"#));
        assert!(json.contains(r#""orgIds":["1","2"]"#));
        assert!(json.contains(r#""subOrgIds":["3"]"#));
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
}
