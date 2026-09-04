//! Bootstrap 自检与 system 用户主体绑定
//!
//! `GET /api/admin/bootstrap/system-subject` — 查询 system 用户主体绑定状态
//! `POST /api/admin/bootstrap/system-subject` — 创建主体并绑定（系统设置页引导）

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use common::ApiResponse;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSubjectStatus {
    /// system 用户是否存在
    pub system_user_exists: bool,
    /// 是否已绑定主体（entity_table='zc_id_subjects' 且 entity_id 有效）
    pub bound: bool,
    /// 绑定主体 id（未绑定时 null）
    #[serde(with = "common::serde_zuid::opt")]
    pub subject_id: Option<i64>,
    /// 绑定主体 code（未绑定时 null）
    pub subject_code: Option<String>,
    /// 认证状态（auth_users.status）
    pub auth_status: Option<String>,
    /// 实名认证状态（settings.real_name_status，可选、不阻塞链路；无则 unverified）
    pub real_name_status: String,
}

#[derive(Debug, Deserialize)]
pub struct BindSystemSubjectRequest {
    /// 主体名称（notice）——创建模式使用（entity_id 模式可空）
    pub notice: String,
    /// 主体编码（可选，默认 SUBJ-SYSTEM）
    pub code: Option<String>,
    /// 绑定已有主体（add-subject-rebind-management）：跳过创建，校验
    /// zc_id_subjects 行存在后直接绑定（tableoid 解析叶表名）。
    #[serde(with = "common::serde_zuid::opt")]
    pub entity_id: Option<i64>,
    /// 创建模式的主体类型（fix-system-subject-seat-by-type）：选定现有叶表类型
    /// 推定落点（org/position/employee/bank/group/central-bank/department/country/
    /// supranational/agent，默认 org；目标均为 subjects 树真叶）。
    /// 不新增专用承载子表——禁止直插基座父表。
    pub subject_type: Option<String>,
    /// 显式改绑：system 已绑有效主体时须 true 才调整；缺省维持幂等返回（防误触）。
    pub rebind: Option<bool>,
}

/// GET /api/admin/bootstrap/system-subject — 查询 system 绑定状态
pub async fn get_system_subject_status(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    state: web::Data<crate::auth::AuthState>,
) -> Result<HttpResponse, actix_web::Error> {
    let _admin_id = match super::require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };
    #[allow(clippy::type_complexity)] // sqlx 行类型
    let row: Option<(
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<serde_json::Value>,
    )> = sqlx::query_as(
        "SELECT entity_table, entity_id, status, settings \
             FROM isahl_auth.auth_users WHERE username = 'system'",
    )
    .fetch_optional(pool.get_ref())
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?;

    let Some((_entity_table, entity_id, auth_status, settings)) = row else {
        return Ok(
            HttpResponse::Ok().json(ApiResponse::success(SystemSubjectStatus {
                system_user_exists: false,
                bound: false,
                subject_id: None,
                subject_code: None,
                auth_status: None,
                real_name_status: "unverified".to_string(),
            })),
        );
    };

    // bound = 有效绑定（任意叶表/基表行存在）——add-subject-rebind-management：
    // 同步/绑定/调整流的正常产物是叶表绑定（如 zc_id_orga-legal），旧判定
    // 只认基表会把它们误报为未绑定。
    let bound = match entity_id {
        Some(eid) => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM isahl.zc_id_subjects WHERE id = $1 AND deleted_at IS NULL)",
        )
        .bind(eid)
        .fetch_one(pool.get_ref())
        .await
        .unwrap_or(false),
        None => false,
    };
    let subject_code: Option<String> = if bound {
        sqlx::query_scalar(
            "SELECT code FROM isahl.zc_id_subjects WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(entity_id)
        .fetch_optional(pool.get_ref())
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?
        .flatten()
    } else {
        None
    };
    let real_name_status = settings
        .as_ref()
        .and_then(|s| s.get("real_name_status").and_then(|v| v.as_str()))
        .unwrap_or("unverified")
        .to_string();
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(SystemSubjectStatus {
            system_user_exists: true,
            bound,
            subject_id: entity_id,
            subject_code,
            auth_status,
            real_name_status,
        })),
    )
}

/// POST /api/admin/bootstrap/system-subject — 创建/选择主体并绑定 system 用户。
/// m2o 语义（fix-ngac-entity-binding-m2o）：system 绑定一个组织主体，
/// 不排斥其他 user 绑定同一主体（一个组织实体 ↔ 多个 user）。
/// 调整语义（add-subject-rebind-management）：已绑有效主体时须显式 rebind=true
/// 才改绑；entity_id 提供时绑定已有主体（不新建）。
pub async fn bind_system_subject(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    state: web::Data<crate::auth::AuthState>,
    body: web::Json<BindSystemSubjectRequest>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = match super::require_admin(&req, pool.get_ref(), state.get_ref()).await {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    let mut tx = pool
        .begin()
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?;

    // 1. 幂等门（add-subject-rebind-management）：已绑有效主体（行存在）且未显式
    //    rebind → 直接返回现有状态；改绑须显式 rebind=true。
    let rebind = body.rebind.unwrap_or(false);
    let existing: Option<(Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT entity_table, entity_id FROM isahl_auth.auth_users WHERE username = 'system'",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?;
    if !rebind {
        if let Some((_, Some(eid))) = existing.filter(|(_, id)| id.is_some()) {
            let bound_valid: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM isahl.zc_id_subjects WHERE id = $1 AND deleted_at IS NULL)",
            )
            .bind(eid)
            .fetch_one(&mut *tx)
            .await
            .map_err(actix_web::error::ErrorInternalServerError)?;
            if bound_valid {
                let _ = tx.commit().await;
                return Ok(
                    HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                        "bound": true,
                        "subject_id": eid.to_string(),
                        "message": "已绑定，无需重复创建",
                    }))),
                );
            }
        }
    }

    // 2. 目标主体：entity_id（绑定已有主体，tableoid 解析叶表名）或创建（code 幂等）
    let (subject_id, entity_table, code): (i64, String, String) = match body.entity_id {
        Some(eid) => {
            let row: Option<(String, Option<String>)> = sqlx::query_as(
                r#"SELECT c.relname::text, s.code
                   FROM isahl.zc_id_subjects s
                   JOIN pg_class c ON c.oid = s.tableoid
                   WHERE s.id = $1 AND s.deleted_at IS NULL"#,
            )
            .bind(eid)
            .fetch_optional(&mut *tx)
            .await
            .map_err(actix_web::error::ErrorInternalServerError)?;
            match row {
                Some((table_name, code)) => (eid, table_name, code.unwrap_or_default()),
                None => {
                    return Ok(HttpResponse::BadRequest().json(
                        serde_json::json!({"error": "VALIDATION", "message": "主体不存在"}),
                    ));
                }
            }
        }
        None => {
            let code = body
                .code
                .clone()
                .unwrap_or_else(|| "SUBJ-SYSTEM".to_string());
            // 落点（fix-system-subject-seat-by-type）：管理员选定主体类型 → 推定
            // subjects 树现有叶表（标准过程）。禁止直插基座父表（父表 FROM ONLY
            // 恒 0 契约 add-inheritance-ancestor-audit；v10.0.10 事故即源于此），
            // 不新增专用承载子表——zc_id_subj-system 不应出现（用户裁决）。
            let subject_type = body.subject_type.as_deref().unwrap_or("org");
            // 类型 → 叶表（静态白名单；SqlSafeStr：全部固定 SQL，值走 bind 参数）。
            // 判装目标叶表 to_regclass（模型中心登记面）；未知类型 VALIDATION。
            let seat = match subject_type {
                // 目标必须为 subjects 树真叶（无子表继承；org 等族根/中间层
                // 如 subj-org/subj-hierarchy 非叶，不得作落点——用户裁决）
                "org" => "zc_id_orga-non-banking-legal",
                "position" => "zc_id_subj-position",
                "employee" => "zc_id_empl-natural",
                "bank" => "zc_id_bank-commercial",
                "group" => "zc_id_subj-group",
                "central-bank" => "zc_id_bank-central",
                "department" => "zc_id_orga-department",
                "country" => "zc_id_subj-country",
                "supranational" => "zc_id_subj-supranational",
                "agent" => "zc_id_empl-agent",
                other => {
                    return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                        "error": "VALIDATION",
                        "message": format!(
                            "未知主体类型: {other}（可选 org/position/employee/bank/group/central-bank/department/country/supranational/agent）"
                        ),
                    })));
                }
            };
            let seat_exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
                // seat 含连字符（zc_id_orga-non-banking-legal 等）——schema.table 解析须引号包裹表名，
                // 否则 to_regclass 恒 NULL（SUBJECT_SEAT_MISSING 误报；entity_binding 同款正确形态）
                .bind(format!("isahl.\"{seat}\""))
                .fetch_one(&mut *tx)
                .await
                .map_err(actix_web::error::ErrorInternalServerError)?;
            if !seat_exists {
                return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                    "error": "SUBJECT_SEAT_MISSING",
                    "message": format!(
                        "主体类型 {subject_type} 对应叶表 isahl.{seat} 不存在/未登记。                         系统主体必须在 subjects 树现有叶表中选定落点（模型中心登记后重试）——                         禁止直插基座父表。"
                    ),
                })));
            }
            // INSERT/查重均落在推定叶表（类型白名单 → 静态 SQL）
            let (insert_sql, dup_sql): (&str, &str) = match subject_type {
                "org" => (
                    "INSERT INTO isahl.\"zc_id_orga-non-banking-legal\" (id, code, notice, created_by_id) \
                     SELECT isahl.gen_next_zuid(), $1, $2, $3 \
                     WHERE NOT EXISTS (SELECT 1 FROM isahl.\"zc_id_orga-non-banking-legal\" \
                                       WHERE code = $1 AND deleted_at IS NULL) \
                     RETURNING id",
                    "SELECT id FROM isahl.\"zc_id_orga-non-banking-legal\" \
                     WHERE code = $1 AND deleted_at IS NULL",
                ),
                "position" => (
                    "INSERT INTO isahl.\"zc_id_subj-position\" (id, code, notice, created_by_id) \
                     SELECT isahl.gen_next_zuid(), $1, $2, $3 \
                     WHERE NOT EXISTS (SELECT 1 FROM isahl.\"zc_id_subj-position\" \
                                       WHERE code = $1 AND deleted_at IS NULL) \
                     RETURNING id",
                    "SELECT id FROM isahl.\"zc_id_subj-position\" \
                     WHERE code = $1 AND deleted_at IS NULL",
                ),
                "employee" => (
                    "INSERT INTO isahl.\"zc_id_empl-natural\" (id, code, notice, created_by_id) \
                     SELECT isahl.gen_next_zuid(), $1, $2, $3 \
                     WHERE NOT EXISTS (SELECT 1 FROM isahl.\"zc_id_empl-natural\" \
                                       WHERE code = $1 AND deleted_at IS NULL) \
                     RETURNING id",
                    "SELECT id FROM isahl.\"zc_id_empl-natural\" \
                     WHERE code = $1 AND deleted_at IS NULL",
                ),
                "bank" => (
                    "INSERT INTO isahl.\"zc_id_bank-commercial\" (id, code, notice, created_by_id) \
                     SELECT isahl.gen_next_zuid(), $1, $2, $3 \
                     WHERE NOT EXISTS (SELECT 1 FROM isahl.\"zc_id_bank-commercial\" \
                                       WHERE code = $1 AND deleted_at IS NULL) \
                     RETURNING id",
                    "SELECT id FROM isahl.\"zc_id_bank-commercial\" \
                     WHERE code = $1 AND deleted_at IS NULL",
                ),
                "group" => (
                    "INSERT INTO isahl.\"zc_id_subj-group\" (id, code, notice, created_by_id) \
                     SELECT isahl.gen_next_zuid(), $1, $2, $3 \
                     WHERE NOT EXISTS (SELECT 1 FROM isahl.\"zc_id_subj-group\" \
                                       WHERE code = $1 AND deleted_at IS NULL) \
                     RETURNING id",
                    "SELECT id FROM isahl.\"zc_id_subj-group\" \
                     WHERE code = $1 AND deleted_at IS NULL",
                ),
                "central-bank" => (
                    "INSERT INTO isahl.\"zc_id_bank-central\" (id, code, notice, created_by_id) \
                     SELECT isahl.gen_next_zuid(), $1, $2, $3 \
                     WHERE NOT EXISTS (SELECT 1 FROM isahl.\"zc_id_bank-central\" \
                                       WHERE code = $1 AND deleted_at IS NULL) \
                     RETURNING id",
                    "SELECT id FROM isahl.\"zc_id_bank-central\" \
                     WHERE code = $1 AND deleted_at IS NULL",
                ),
                "department" => (
                    "INSERT INTO isahl.\"zc_id_orga-department\" (id, code, notice, created_by_id) \
                     SELECT isahl.gen_next_zuid(), $1, $2, $3 \
                     WHERE NOT EXISTS (SELECT 1 FROM isahl.\"zc_id_orga-department\" \
                                       WHERE code = $1 AND deleted_at IS NULL) \
                     RETURNING id",
                    "SELECT id FROM isahl.\"zc_id_orga-department\" \
                     WHERE code = $1 AND deleted_at IS NULL",
                ),
                "country" => (
                    "INSERT INTO isahl.\"zc_id_subj-country\" (id, code, notice, created_by_id) \
                     SELECT isahl.gen_next_zuid(), $1, $2, $3 \
                     WHERE NOT EXISTS (SELECT 1 FROM isahl.\"zc_id_subj-country\" \
                                       WHERE code = $1 AND deleted_at IS NULL) \
                     RETURNING id",
                    "SELECT id FROM isahl.\"zc_id_subj-country\" \
                     WHERE code = $1 AND deleted_at IS NULL",
                ),
                "supranational" => (
                    "INSERT INTO isahl.\"zc_id_subj-supranational\" (id, code, notice, created_by_id) \
                     SELECT isahl.gen_next_zuid(), $1, $2, $3 \
                     WHERE NOT EXISTS (SELECT 1 FROM isahl.\"zc_id_subj-supranational\" \
                                       WHERE code = $1 AND deleted_at IS NULL) \
                     RETURNING id",
                    "SELECT id FROM isahl.\"zc_id_subj-supranational\" \
                     WHERE code = $1 AND deleted_at IS NULL",
                ),
                "agent" => (
                    "INSERT INTO isahl.\"zc_id_empl-agent\" (id, code, notice, created_by_id) \
                     SELECT isahl.gen_next_zuid(), $1, $2, $3 \
                     WHERE NOT EXISTS (SELECT 1 FROM isahl.\"zc_id_empl-agent\" \
                                       WHERE code = $1 AND deleted_at IS NULL) \
                     RETURNING id",
                    "SELECT id FROM isahl.\"zc_id_empl-agent\" \
                     WHERE code = $1 AND deleted_at IS NULL",
                ),
                _ => unreachable!("subject_type 已在 seat match 校验"),
            };
            let subject_id: Option<i64> = sqlx::query_scalar(insert_sql)
                .bind(&code)
                .bind(&body.notice)
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(actix_web::error::ErrorInternalServerError)?;
            let subject_id = match subject_id {
                Some(id) => id,
                None => sqlx::query_scalar(dup_sql)
                    .bind(&code)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(actix_web::error::ErrorInternalServerError)?,
            };
            (subject_id, seat.to_string(), code)
        }
    };

    // 3. 绑定 system 用户（entity_table/entity_id；m2o：不排斥其他 user 绑同一主体）
    //    + 实名状态（settings JSONB，零 DDL）。占位标记仅基表行写入——entity_id
    //    模式绑定叶表业务主体不标占位并清除旧标记（add-subject-rebind-management）。
    // 静态字面量二选一（SqlSafeStr 守卫；两分支均为固定 SQL，值走 bind 参数）：
    // 基表行 → 写占位标记；叶表业务主体 → 清占位标记。
    // 占位标记仅历史基表直插行（entity_table='zc_id_subjects'）；治理后叶表落点
    // （org/position/... 推定）与 entity_id 绑定同语义 → 清标记分支
    if entity_table == "zc_id_subjects" {
        sqlx::query(
            "UPDATE isahl_auth.auth_users \
             SET entity_table = $1, entity_id = $2, updated_at = NOW(), updated_by_id = $3, \
                 settings = COALESCE(settings, '{}'::jsonb) \
                     || '{\"real_name_status\":\"verified\",\"subject_binding\":\"system\"}'::jsonb \
             WHERE username = 'system'",
        )
    } else {
        sqlx::query(
            "UPDATE isahl_auth.auth_users \
             SET entity_table = $1, entity_id = $2, updated_at = NOW(), updated_by_id = $3, \
                 settings = COALESCE(settings, '{}'::jsonb) - 'subject_binding' \
                     || '{\"real_name_status\":\"verified\"}'::jsonb \
             WHERE username = 'system'",
        )
    }
    .bind(&entity_table)
    .bind(subject_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?;

    // 4. 自动建立系统管理员岗位链（幂等；与 scripts/db/seed-isahl-user.sh 同构）：
    //    system 绑定主体后 → 建系统管理员岗位 → 关联 主体↔岗位、岗位↔雇员(mgm-agent)，
    //    使 isahl 登录时经 fk_user→empl-agent→post_rr_employee 可反查自身岗位。
    // 4a. 系统管理员岗位（zc_id_subj-position，code 幂等）
    let pos_id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_subj-position\" (code, notice, created_by_id) \
         SELECT 'POS-SYSTEM-ADMIN', '系统管理员', $1 \
         WHERE NOT EXISTS (SELECT 1 FROM isahl.\"zc_id_subj-position\" \
                           WHERE code = 'POS-SYSTEM-ADMIN' AND deleted_at IS NULL) \
         RETURNING id",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?;
    let pos_id = match pos_id {
        Some(id) => id,
        None => sqlx::query_scalar(
            "SELECT id FROM isahl.\"zc_id_subj-position\" \
                 WHERE code = 'POS-SYSTEM-ADMIN' AND deleted_at IS NULL",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?,
    };

    // 4b. mgm-agent 智体（若 isahl 用户存在；fk_user=isahl，code 幂等）
    let agent_id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_empl-agent\" (code, notice, fk_user, created_by_id) \
         SELECT 'mgm-agent', 'mgm-agent 智能体', u.id, $1 \
         FROM isahl_auth.auth_users u \
         WHERE u.username = 'isahl' \
           AND NOT EXISTS (SELECT 1 FROM isahl.\"zc_id_empl-agent\" a \
                           WHERE a.code = 'mgm-agent' AND a.deleted_at IS NULL) \
         RETURNING id",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?;
    let agent_id = match agent_id {
        Some(id) => Some(id),
        None => sqlx::query_scalar(
            "SELECT id FROM isahl.\"zc_id_empl-agent\" \
                 WHERE code = 'mgm-agent' AND deleted_at IS NULL",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?
        .flatten(),
    };
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-org_rr_position" (code, notice, ref_left, ref_right, created_by_id)
           SELECT $3, '系统平台主体↔系统管理员岗位', subj.id, $1, $2
           FROM isahl.zc_id_subjects subj
           WHERE subj.id = $4
             AND NOT EXISTS (SELECT 1 FROM isahl."zc_id_subj-org_rr_position" o
                             WHERE o.ref_left = subj.id AND o.ref_right = $1 AND o.deleted_at IS NULL)"#,
    )
    .bind(pos_id)
    .bind(user_id)
    .bind(format!("SYSPOS-{code}"))
    .bind(subject_id)
    .execute(&mut *tx)
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?;

    // 4d. 岗位↔雇员（POS-SYSTEM-ADMIN ↔ mgm-agent，post_rr_employee）
    if let Some(agent_id) = agent_id {
        sqlx::query(
            "INSERT INTO isahl.\"zc_id_subj-post_rr_employee\" (code, notice, ref_left, ref_right, created_by_id) \
             SELECT 'EMP-POS-MGM-AGENT', '系统管理员岗位↔mgm-agent', $1, $2, $3 \
             WHERE NOT EXISTS (SELECT 1 FROM isahl.\"zc_id_subj-post_rr_employee\" e \
                               WHERE e.ref_left = $1 AND e.ref_right = $2 AND e.deleted_at IS NULL)",
        )
        .bind(pos_id)
        .bind(agent_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?;
    }

    tx.commit()
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "bound": true,
            "subject_id": subject_id.to_string(),
            "subject_code": code,
            "message": "system 主体已创建并绑定",
        }))),
    )
}
