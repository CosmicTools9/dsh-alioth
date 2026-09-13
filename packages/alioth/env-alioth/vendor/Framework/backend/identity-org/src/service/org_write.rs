//! 岗位/部门/组织树写 Service（ADR A-1 岗位写收束 + A-1b 部门/树写收束 +
//! A-2 approver 岗位行收束）— org_tree handler 的岗位/部门/组织树写数据逻辑层。
//!
//! 归口：岗位 CRUD（create/update/delete_position）、Approver 岗位行写
//! （create/update/delete_approver_position，ADR A-2 authority 改调）、部门↔岗位分配
//! （assign / remove_position_from_department）、岗位任职（add/remove_position_employee）、
//! 部门 CRUD（create/update/delete_department）、组织树挂接（add/remove_org_tree_child）的
//! SQL 事务与校验全部在本模块；handlers/org_tree.rs 仅保留路由参数解析 + 认证/
//! 权限门，薄委托本模块。
//! NGAC B-2 heal（heal_position_scope / heal_department_scope）于各写成功后事务外
//! 触发，归口于本模块写函数（删除 handler 内 heal hook，行为等价：heal 时机/幂等语义不变）。

use actix_web::HttpResponse;
use common::data::ApiResponse;
use common::AliothError as ApiError;
use sqlx::{AssertSqlSafe, PgPool};

use crate::handlers::org_tree::{
    dept_row_to_dto, ensure_department_exists, ensure_org_exists, position_row_to_dto,
    revive_then_insert_bridge, route_employee_subject, validate_name, CreateDepartmentRequest,
    CreatePositionRequest, DepartmentDto, DepartmentRow, DeptPositionRelationDto, PositionRow,
    UpdateDepartmentRequest, UpdatePositionRequest, DEPARTMENT_SELECT, POSITION_SELECT,
};

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

/// POST /positions
pub async fn create_position(
    pool: &PgPool,
    body: CreatePositionRequest,
    user_id: i64,
) -> Result<HttpResponse, ApiError> {
    validate_name(&body.name)?;
    if let Some(pid) = body.parent_id {
        ensure_position_exists(pool, pid).await?;
    }
    if let Some(uid) = body.user_id {
        ensure_user_exists(pool, uid).await?;
    }
    let category_id = resolve_category_id(pool, &body.category).await?;
    // code 非业务唯一标识（用户裁决 2026-08-31）——空缺落 NULL，不再自动生成
    let code = if body.code.trim().is_empty() {
        None
    } else {
        Some(body.code.clone())
    };

    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .map_err(ApiError::from)?;

    // 单事务：主表写 + 双桥全量替换（软删旧关联 → 插新关联），保证原子性
    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    let base: PositionBaseRow = sqlx::query_as(
        r#"INSERT INTO isahl."zc_id_subj-position" (notice, code, comments, fk_user, fk_parent, ck_category, created_by_id, dk_scene, dk_factor, dk_function)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id, notice::text, COALESCE(code, ''), COALESCE(comments, ''), fk_user, fk_parent AS parent_id,
                      COALESCE((SELECT c.code FROM isahl."zc_id_cate-position" c WHERE c.id = isahl."zc_id_subj-position".ck_category AND c.deleted_at IS NULL), (SELECT c2.notice FROM isahl.zc_id_category c2 WHERE c2.id = isahl."zc_id_subj-position".ck_category AND c2.deleted_at IS NULL), ''),
                      NULL::text AS user_name"#,
    )
    .bind(&body.name)
    .bind(&code)
    .bind(&body.comments)
    .bind(body.user_id)
    .bind(body.parent_id)
    .bind(category_id)
    .bind(user_id)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
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
    crate::ngac_org_ensure::heal_position_scope(pool, base.0).await;

    Ok(HttpResponse::Created().json(ApiResponse::success(position_row_to_dto(full))))
}

/// PUT /positions/{id} — 局部更新（None 字段保持不变）
pub async fn update_position(
    pool: &PgPool,
    id: i64,
    body: UpdatePositionRequest,
    user_id: i64,
) -> Result<HttpResponse, ApiError> {
    if let Some(name) = &body.name {
        validate_name(name)?;
    }
    if let Some(pid) = body.parent_id {
        ensure_position_exists(pool, pid).await?;
    }
    if let Some(uid) = body.user_id {
        ensure_user_exists(pool, uid).await?;
    }
    let category_id = match &body.category {
        Some(cat) => resolve_category_id(pool, cat).await?,
        None => None,
    };

    // 单事务：环检测（如变更 parent_id）→ 主表写 → 双桥全量替换
    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    if let Some(new_parent) = body.parent_id {
        check_parent_cycle(pool, id, new_parent).await?;
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
                     COALESCE((SELECT c.code FROM isahl."zc_id_cate-position" c WHERE c.id = isahl."zc_id_subj-position".ck_category AND c.deleted_at IS NULL), (SELECT c2.notice FROM isahl.zc_id_category c2 WHERE c2.id = isahl."zc_id_subj-position".ck_category AND c2.deleted_at IS NULL), ''),
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
    pool: &PgPool,
    id: i64,
    user_id: i64,
) -> Result<HttpResponse, ApiError> {
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
// Approver 岗位行写收束（ADR A-2 approver-to-org-service）
// ═══════════════════════════════════════════════════════════

/// Approver 岗位行创建入口（最小面，供 authority ApproverRepository 改调）。
///
/// Approver = `zc_id_subj-position` 真实岗位行（新建行 `_f_` 恒空，非编制范例），
/// 与 [`create_position`] 存在语义差、不可复用主入口，故按 Approver 语义收束：
/// - 类别 = `role` id **直绑** `ck_category`（id 空间 = `zc_id_cate-approve_role`；
///   不经 `zc_id_cate-position` code 字典解析——approver 侧无字典校验语义）；
/// - 维度绑定经 `ontology_binding::resolve` 解析 TX/FJA/↓_GG（现 repo 直写同值）；
/// - 不写 code / 不写任何岗位桥、无 parent/任职人校验；
/// - 无 ON CONFLICT/软删复活路径（表无业务唯一键——与现 repo 纯 INSERT 等价）。
///
/// NGAC B-2 heal 内建（事务外幂等、失败仅 warn），归口同本模块 pool 级写函数；
/// authority repo 内 M222 heal 调用随之移除（写后 heal 时机不变）。
/// 返回新行 id；行读取（RETURNING 形态）由调用方以其既有 SELECT 完成。
pub async fn create_approver_position(
    pool: &PgPool,
    name: &str,
    role_id: Option<i64>,
    description: Option<&str>,
    user_id: i64,
) -> Result<i64, ApiError> {
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .map_err(ApiError::from_sqlx)?;
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_subj-position"
           (notice, ck_category, comments, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id"#,
    )
    .bind(name)
    .bind(role_id)
    .bind(description)
    .bind(user_id)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    crate::ngac_org_ensure::heal_position_scope(pool, id).await;
    Ok(id)
}

/// Approver 岗位行局部更新入口。仅写 notice/ck_category/comments/updated_by_id 四列；
/// None-门（未提供字段保持现值）由调用方先读 current 行合并终值后传入——本入口收
/// **已合并终值**，与现 repo UPDATE 等价。`_f_ IS NULL` 守卫同现 repo（范例行不可改）。
/// 未命中（不存在/已软删/范例行）→ Ok(None) 且不触发 heal；命中 → heal 后 Ok(Some(id))。
pub async fn update_approver_position(
    pool: &PgPool,
    id: i64,
    name: &str,
    role_id: Option<i64>,
    description: Option<&str>,
    user_id: i64,
) -> Result<Option<i64>, ApiError> {
    let updated: Option<i64> = sqlx::query_scalar(
        r#"UPDATE isahl."zc_id_subj-position"
           SET notice = $2, ck_category = $3, comments = $4, updated_by_id = $5
           WHERE id = $1 AND deleted_at IS NULL AND _f_ IS NULL
           RETURNING id"#,
    )
    .bind(id)
    .bind(name)
    .bind(role_id)
    .bind(description)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    let Some(updated_id) = updated else {
        return Ok(None);
    };
    crate::ngac_org_ensure::heal_position_scope(pool, updated_id).await;
    Ok(Some(updated_id))
}

/// Approver 岗位行软删入口。与现 repo 等价：仅软删岗位行本身（不级联岗位桥——
/// 与 [`delete_position`] 的四桥级联不同）；未命中**静默成功**（approvers 端点
/// 语义：无 NotFound 错误映射）；heal 无条件触发（现 repo M222 heal 同款——
/// 岗位已软删时 heal 内部无对象即跳过）。`_f_ IS NULL` 守卫同现 repo。
pub async fn delete_approver_position(
    pool: &PgPool,
    id: i64,
    user_id: i64,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-position"
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE id = $1 AND deleted_at IS NULL AND _f_ IS NULL"#,
    )
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    crate::ngac_org_ensure::heal_position_scope(pool, id).await;
    Ok(())
}

/// POST /departments/{id}/positions — 分配岗位到部门（幂等：已存在则返回已有记录）
pub async fn assign_position_to_department(
    pool: &PgPool,
    dept_id: i64,
    position_id: i64,
) -> Result<HttpResponse, ApiError> {
    // 前置存在性预检：部门与岗位必须存在（未删除），否则 404
    ensure_department_exists(pool, dept_id).await?;
    ensure_position_exists(pool, position_id).await?;

    // 幂等检查：是否已存在未删除的关联
    let existing: Option<(i64,)> = sqlx::query_as(
        r#"SELECT id FROM isahl."zc_id_subj-org_rr_position"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(dept_id)
    .bind(position_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?;

    if let Some((rel_id,)) = existing {
        let position_name: String = sqlx::query_scalar(
            "SELECT notice::text FROM isahl.\"zc_id_subj-position\" WHERE id = $1",
        )
        .bind(position_id)
        .fetch_one(pool)
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
    .execute(pool)
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
    .fetch_optional(pool)
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
            .fetch_one(pool)
            .await
            .map_err(ApiError::from_sqlx)?
        }
    };

    // NGAC B-2：岗位新增在任分配部门 → 刷新岗位 OA ancestor 域闭包（幂等 heal）
    crate::ngac_org_ensure::heal_position_scope(pool, position_id).await;

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
    pool: &PgPool,
    dept_id: i64,
    rel_id: i64,
) -> Result<HttpResponse, ApiError> {
    // 软删并取回被移除岗位（ref_right）——供事务外 NGAC heal 收敛 OA ancestor 域闭包
    let deleted: Option<(i64,)> = sqlx::query_as(
        r#"UPDATE isahl."zc_id_subj-org_rr_position"
           SET deleted_at = NOW()
           WHERE id = $1 AND ref_left = $2 AND deleted_at IS NULL
           RETURNING ref_right"#,
    )
    .bind(rel_id)
    .bind(dept_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?;

    let Some((position_id,)) = deleted else {
        return Err(ApiError::NotFound("Relation not found".into()));
    };

    // NGAC B-2：岗位移除在任分配部门 → 刷新岗位 OA ancestor 域闭包（事务外幂等 heal，失败仅 warn）
    crate::ngac_org_ensure::heal_position_scope(pool, position_id).await;

    Ok(HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({ "deleted": true }))))
}

/// POST /positions/{id}/employees — 岗位任职挂接（幂等）
pub async fn add_position_employee(
    pool: &PgPool,
    position_id: i64,
    subject_id: i64,
    user_id: i64,
) -> Result<HttpResponse, ApiError> {
    ensure_position_exists(pool, position_id).await?;
    let kind = route_employee_subject(pool, subject_id).await?;

    let existing: Option<(i64,)> = sqlx::query_as(
        r#"SELECT id FROM isahl."zc_id_subj-post_rr_employee"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(position_id)
    .bind(subject_id)
    .fetch_optional(pool)
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
    .execute(pool)
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
    .fetch_optional(pool)
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
        .fetch_one(pool)
        .await
        .map_err(ApiError::from_sqlx)?,
    };

    // NGAC B-2：任职写端后岗位行 OA/层级收敛（事务外幂等 heal，失败仅 warn）
    crate::ngac_org_ensure::heal_position_scope(pool, position_id).await;

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

/// 岗位任职桥最小写入口（事务内复用 — M226/ADR A-3 binding-to-org-service）。
///
/// 幂等守卫与 add_position_employee 同源：活行 EXISTS → 空操作返回；否则
/// 复活同键软删行 + INSERT ON CONFLICT DO NOTHING（revive_then_insert_bridge）。
/// 与 add_position_employee 的差异：接受调用方事务内连接（&mut PgConnection）
/// 而非 &PgPool——供 Gateway bind_personal 等四写原子事务内复用（service 独立
/// 入口自管理提交，无法嵌入外部事务；其 subject 路由校验也读不到调用方事务内
/// 未提交的新建主体行）。本入口不触发 heal：事务外 heal 由调用方 commit 后
/// 统一触发（heal 归口不变——本模块 pool 级写函数内建 heal 仍覆盖独立入口路径）。
pub async fn attach_employee_bridge(
    conn: &mut sqlx::PgConnection,
    position_id: i64,
    employee_id: i64,
    user_id: i64,
) -> Result<(), ApiError> {
    let existing: Option<(i64,)> = sqlx::query_as(
        r#"SELECT id FROM isahl."zc_id_subj-post_rr_employee"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(position_id)
    .bind(employee_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(ApiError::from_sqlx)?;
    if existing.is_some() {
        return Ok(());
    }
    revive_then_insert_bridge(
        conn,
        "zc_id_subj-post_rr_employee",
        position_id,
        employee_id,
        user_id,
    )
    .await
}

/// DELETE /positions/{id}/employees/{subjectId} — 解除任职（软删除）
pub async fn remove_position_employee(
    pool: &PgPool,
    position_id: i64,
    subject_id: i64,
) -> Result<HttpResponse, ApiError> {
    let deleted = sqlx::query(
        r#"UPDATE isahl."zc_id_subj-post_rr_employee"
           SET deleted_at = NOW()
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(position_id)
    .bind(subject_id)
    .execute(pool)
    .await
    .map_err(ApiError::from_sqlx)?
    .rows_affected();

    if deleted == 0 {
        return Err(ApiError::NotFound("Employment relation not found".into()));
    }
    Ok(HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({ "deleted": true }))))
}

// ═══════════════════════════════════════════════════════════
// 部门/组织树写（ADR A-1b 部门/树写收束 — 自 handlers/org_tree.rs 原样移体）
// ═══════════════════════════════════════════════════════════

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

/// POST /departments
///
/// （ADR A-1b：自 handlers/org_tree.rs 原样移体；SQL/校验/错误语义不变，仅 pool 适配）
pub async fn create_department(
    pool: &PgPool,
    body: CreateDepartmentRequest,
    user_id: i64,
) -> Result<HttpResponse, ApiError> {
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
        .fetch_one(pool)
        .await
        .map_err(ApiError::from_sqlx)?;

    // 父部门挂接（org_rr_subordinate 桥；可复活软删行）
    if let Some(pid) = body.parent_id {
        ensure_org_exists(pool, pid).await?;
        let mut conn = pool.acquire().await.map_err(ApiError::from_sqlx)?;
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
    crate::ngac_org_ensure::heal_department_scope(pool, row.0).await;

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
/// PUT /departments/{id} — 局部更新（None 字段保持不变）
///
/// （ADR A-1b：自 handlers/org_tree.rs 原样移体；SQL/校验/错误语义不变，仅 pool 适配）
pub async fn update_department(
    pool: &PgPool,
    id: i64,
    body: UpdateDepartmentRequest,
    user_id: i64,
) -> Result<HttpResponse, ApiError> {
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
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?;

    let Some(_) = updated else {
        return Err(ApiError::NotFound("Department not found".into()));
    };

    // 父部门调整（org_rr_subordinate 桥差量替换；环检测复用挂接同一守卫）
    if let Some(pid) = body.parent_id {
        check_org_tree_cycle(pool, id, pid).await?;
        ensure_org_exists(pool, pid).await?;
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_subj-org_rr_subordinate"
               SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
               WHERE ref_right = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(ApiError::from_sqlx)?;
        let mut conn = pool.acquire().await.map_err(ApiError::from_sqlx)?;
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
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?;

    match row {
        Some(r) => Ok(HttpResponse::Ok().json(ApiResponse::success(dept_row_to_dto(r)))),
        None => Err(ApiError::NotFound("Department not found".into())),
    }
}
/// DELETE /departments/{id} — 软删除（同事务级联软删部门桥行）
///
/// （ADR A-1b：自 handlers/org_tree.rs 原样移体；SQL/校验/错误语义不变，仅 pool 适配）
pub async fn delete_department(
    pool: &PgPool,
    id: i64,
    user_id: i64,
) -> Result<HttpResponse, ApiError> {
    // 子部门守卫：仍是父节点（桥 ref_left）时拒绝删除，先移除/迁移子部门
    let children: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM "isahl"."zc_id_subj-org_rr_subordinate"
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .fetch_one(pool)
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
/// POST /org-tree/{id}/children — 挂接下属组织（幂等：已存在未删关联 → 返回现有）
///
/// （ADR A-1b：自 handlers/org_tree.rs 原样移体；SQL/校验/错误语义不变，仅 pool 适配）
pub async fn add_org_tree_child(
    pool: &PgPool,
    parent_id: i64,
    child_id: i64,
    user_id: i64,
) -> Result<HttpResponse, ApiError> {
    ensure_org_exists(pool, parent_id).await?;
    ensure_org_exists(pool, child_id).await?;

    // 幂等：已存在未删除关联 → 返回现有记录
    let existing: Option<(i64,)> = sqlx::query_as(
        r#"SELECT id FROM isahl."zc_id_subj-org_rr_subordinate"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(parent_id)
    .bind(child_id)
    .fetch_optional(pool)
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

    check_org_tree_cycle(pool, child_id, parent_id).await?;

    // 复活同键软删行（唯一约束含 qk_period 表达式无法 ON CONFLICT 推断）
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-org_rr_subordinate" SET deleted_at = NULL, deleted_by_id = NULL
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NOT NULL"#,
    )
    .bind(parent_id)
    .bind(child_id)
    .execute(pool)
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
    .fetch_optional(pool)
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
        .fetch_one(pool)
        .await
        .map_err(ApiError::from_sqlx)?,
    };

    // NGAC B-2：新 org 节点 → 部门子集 OA 树链 ensure（事务外幂等 heal，失败仅 warn）
    crate::ngac_org_ensure::heal_department_scope(pool, child_id).await;

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
///
/// （ADR A-1b：自 handlers/org_tree.rs 原样移体；SQL/校验/错误语义不变，仅 pool 适配）
pub async fn remove_org_tree_child(
    pool: &PgPool,
    parent_id: i64,
    child_id: i64,
    _user_id: i64,
) -> Result<HttpResponse, ApiError> {
    let deleted = sqlx::query(
        r#"UPDATE isahl."zc_id_subj-org_rr_subordinate"
           SET deleted_at = NOW()
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(parent_id)
    .bind(child_id)
    .execute(pool)
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
