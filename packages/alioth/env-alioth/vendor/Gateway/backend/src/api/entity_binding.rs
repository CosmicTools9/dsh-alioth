//! Entity Binding HTTP Handler — 注册后主体绑定
//!
//! 业务决策（2026-08-17）：注册只建 auth_users；审批通过后登录引导绑定主体：
//! - 个人 → 自然人叶表 zc_id_empl-natural + 任职关系（org_rr_employee/post_rr_employee）
//! - 企业 → 非银行法人叶表 zc_id_orga-non-banking-legal + 可选上级归属 + enterprise UA
//!
//! 仅此两类；其余类型后续扩展。
//!
//! 全部接口 require_auth（用户绑自己，不做 resource_access——绑定前基础 UA 之外无资源）。

use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use common::context::RequestContext;
use serde::Deserialize;
use sqlx::PgPool;

fn current_user(req: &HttpRequest) -> Result<i64, HttpResponse> {
    req.extensions()
        .get::<RequestContext>()
        .map(|ctx| ctx.user_id)
        .ok_or_else(|| {
            HttpResponse::Unauthorized().json(serde_json::json!({"error": "UNAUTHORIZED"}))
        })
}

fn bad_request(code: &str, message: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(serde_json::json!({"error": code, "message": message}))
}

/// GET /api/auth/entity-binding/status
pub async fn status(req: HttpRequest, pool: web::Data<PgPool>) -> HttpResponse {
    let user_id = match current_user(&req) {
        Ok(u) => u,
        Err(resp) => return resp,
    };
    #[allow(clippy::type_complexity)] // sqlx 行类型
    let row: Option<(Option<String>, Option<i64>, bool, Option<serde_json::Value>)> =
        sqlx::query_as(
            r#"SELECT entity_table, entity_id,
                  EXISTS (
                      SELECT 1 FROM isahl_auth.ngac_user_rr_attribute ur
                      JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute
                      WHERE ur.fk_user = $1 AND ur.deleted_at IS NULL
                        AND ua.o_name IN ('admin', 'auditor', 'operator', 'enterprise')
                  ) AS privileged,
                      settings
           FROM isahl_auth.auth_users WHERE id = $1"#,
        )
        .bind(user_id)
        .fetch_optional(pool.get_ref())
        .await
        .ok()
        .flatten();

    let Some((table, id, privileged, settings)) = row else {
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": "USER_NOT_FOUND"}));
    };
    let bind_type = match table.as_deref() {
        Some("zc_id_empl-natural") => Some("personal"),
        Some("zc_id_orga-non-banking-legal") => Some("enterprise"),
        _ => None,
    };
    // 运营组织 B（refactor-wz-trade-chain-ownership D2）：entity 为组织类叶表时回传
    // entity_id——非银行法人 / 组织子表 / subjects 父表直落组织行（WZ 等库组织、SUBJ-SYSTEM）；
    // 自然人（zc_id_empl-natural）等其余绑定 → null。
    let operator_org_id = match table.as_deref() {
        Some("zc_id_orga-non-banking-legal") | Some("zc_id_subj-org") | Some("zc_id_subjects") => {
            id
        }
        _ => None,
    };
    // m2o（fix-ngac-entity-binding-m2o）：当前绑定实体下的 user 总数（含自身）。
    // 一个组织实体 ↔ 多个 user，各 user 经 UA 指派持有不同分级权限。
    let entity_member_count: i64 = match (table.as_deref(), id) {
        (Some(t), Some(eid)) => sqlx::query_scalar(
            "SELECT COUNT(*) FROM isahl_auth.auth_users WHERE entity_table = $1 AND entity_id = $2",
        )
        .bind(t)
        .bind(eid)
        .fetch_one(pool.get_ref())
        .await
        .unwrap_or(1),
        _ => 0,
    };
    // bound 判定（三重）：
    // 1. entity 已绑（两类叶表或其他历史表）→ bound
    // 2. 存量特权用户（admin/auditor/operator/enterprise UA，功能上线前无 entity）→ 豁免，
    //    防止正常使用中的账号被引导页劫持（实测回归：admin entity 空被误跳转）
    // 3. 仅 employee UA 的新注册用户 → 不豁免（employee 是 approve 时基础指派，主体待绑）
    // 4. 行存在性校验（fix-subject-cognition-residual-gaps D5）：entity_id 指向的主体
    //    行被硬删/软删时视为未绑定——否则门控放行但 /auth/me 的 subject=null，双轨漂移。
    //    校验查询失败（DB 抖动）保持原判定，只读端点不因故障锁死用户。
    let entity_row_exists = match id {
        Some(eid) => sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM isahl.zc_id_subjects WHERE id = $1 AND deleted_at IS NULL)",
        )
        .bind(eid)
        .fetch_one(pool.get_ref())
        .await
        .unwrap_or(true),
        None => false,
    };
    let bound = (entity_row_exists
        || (id.is_none() && table.as_deref().is_some_and(|t| !t.is_empty())))
        || privileged;
    // 占位判定（fix-seed-user-subject-binding）：基表行 + seed/bootstrap 写入的
    // subject_binding 标记 = 占位绑定（「我」非业务主体，首次登录引导升级）。
    // 优先于 privileged 豁免——admin 绑占位同样标记，前端引导。
    let placeholder = table.as_deref() == Some("zc_id_subjects")
        && settings
            .as_ref()
            .and_then(|s| s.get("subject_binding"))
            .is_some();
    HttpResponse::Ok().json(serde_json::json!({
        "bound": bound,
        "placeholder": placeholder,
        "type": bind_type,
        "entity_table": table,
        "entity_id": id.map(|v| v.to_string()),
        "operator_org_id": operator_org_id.map(|v| v.to_string()),
        "entity_member_count": entity_member_count,
    }))
}

/// GET /api/auth/entity-binding/options
///
/// 雇佣主体/上级法人：subjects 中的组织类主体（tableoid 排除自然人叶表行；
/// WZ 等库组织直接落 subjects 父表、主库走 subj-org 子表——父表查询两种落法通吃）
/// 岗位：subj-position 非删除行
pub async fn options(req: HttpRequest, pool: web::Data<PgPool>) -> HttpResponse {
    if current_user(&req).is_err() {
        return HttpResponse::Unauthorized().json(serde_json::json!({"error": "UNAUTHORIZED"}));
    }
    let orgs: Vec<(i64, String)> = sqlx::query_as(
        r#"SELECT id, COALESCE(notice, code, id::text) FROM isahl.zc_id_subjects
           WHERE deleted_at IS NULL
             AND tableoid NOT IN ('isahl."zc_id_empl-natural"'::regclass, 'isahl."zc_id_empl-agent"'::regclass)
           ORDER BY notice NULLS LAST, id"#,
    )
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_default();
    let positions: Vec<(i64, String)> = sqlx::query_as(
        r#"SELECT id, COALESCE(notice, code, id::text) FROM isahl."zc_id_subj-position"
           WHERE deleted_at IS NULL ORDER BY notice NULLS LAST, id"#,
    )
    .fetch_all(pool.get_ref())
    .await
    .unwrap_or_default();

    let to_list = |v: &[(i64, String)]| {
        v.iter()
            .map(|(id, name)| serde_json::json!({"id": id.to_string(), "name": name}))
            .collect::<Vec<_>>()
    };
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "employers": to_list(&orgs),
        "positions": to_list(&positions),
        "parent_orgs": to_list(&orgs),
    }))
}

#[derive(Debug, Deserialize)]
pub struct PersonalBindingBody {
    pub real_name: String,
    pub employer_org_id: i64,
    pub position_id: i64,
}

/// 组织类实体白名单（m2o 绑定已有组织实体校验；fix-ngac-entity-binding-m2o）
fn is_org_table(t: &str) -> bool {
    matches!(
        t,
        "zc_id_orga-non-banking-legal" | "zc_id_subj-org" | "zc_id_subjects" | "zc_id_entity"
    )
}

/// 当前 user 已绑实体类型 + 是否占位绑定（Err=用户不存在）。
/// 占位 = 基表行（zc_id_subjects）+ settings.subject_binding 标记
/// （seed 脚本 / SSO bootstrap 写入）——「我」非业务主体，允许替换。
/// Ok((None, _)) = 未绑；Ok((Some(t), placeholder)) = 已绑。
async fn bound_entity_type(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
) -> Result<(Option<String>, bool), BoundLookupError> {
    let row: Option<(Option<String>, Option<serde_json::Value>)> = sqlx::query_as(
        "SELECT entity_table, settings FROM isahl_auth.auth_users WHERE id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(BoundLookupError::Db)?;
    match row {
        None => Err(BoundLookupError::UserNotFound),
        Some((t, settings)) => {
            let placeholder = t.as_deref() == Some("zc_id_subjects")
                && settings
                    .as_ref()
                    .and_then(|s| s.get("subject_binding"))
                    .is_some();
            Ok((t, placeholder))
        }
    }
}

enum BoundLookupError {
    UserNotFound,
    Db(sqlx::Error),
}

/// m2o 幂等门：已绑同类型实体才拒（个人与组织互不阻塞）。
/// 个人类已绑不拦组织绑定，组织类已绑不拦个人绑定——entity_id 主锚点切换，
/// 旧身份仍保留于业务表（empl-natural/任职关系或组织行），非删除。
fn already_bound_for(is_personal_bind: bool, bound_table: &str) -> bool {
    if is_personal_bind {
        bound_table == "zc_id_empl-natural"
    } else {
        is_org_table(bound_table)
    }
}
/// system 哨兵用户主体同步（add-system-subject-sync-on-admin-binding）：
/// 特权用户绑定成功后，把同一主体同步给 system（id=1）。first-wins——system
/// 已绑有效业务主体不覆盖；仅未绑定/占位（settings.subject_binding）/悬空三态
/// 可同步。commit 后增强投递：失败仅 warn 不阻断绑定主流程（对齐
/// ensure_enterprise_ua 模式）。返回本次是否发生同步。
async fn sync_system_subject_binding(
    pool: &PgPool,
    operator_id: i64,
    entity_table: &str,
    entity_id: i64,
) -> bool {
    // 门控：特权 UA（admin/auditor/operator/enterprise，与 status 特权豁免同集）
    let privileged: bool = match sqlx::query_scalar(
        r#"SELECT EXISTS (
            SELECT 1 FROM isahl_auth.ngac_user_rr_attribute ur
            JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute
            WHERE ur.fk_user = $1 AND ur.deleted_at IS NULL
              AND ua.o_name IN ('admin', 'auditor', 'operator', 'enterprise'))"#,
    )
    .bind(operator_id)
    .fetch_one(pool)
    .await
    {
        Ok(p) => p,
        Err(e) => {
            common::telemetry::warn!("sync_system_subject: 特权 UA 查询失败: {e}");
            return false;
        }
    };
    if !privileged {
        return false;
    }

    // 单条条件 UPDATE：三态守卫（未绑/占位/悬空）与写入原子完成，无先查后写竞态；
    // 清占位标记防复发 + subject_sync 溯源；自绑守卫（操作者即 system 时跳过）。
    match sqlx::query(
        r#"UPDATE isahl_auth.auth_users u
           SET entity_table = $1, entity_id = $2, updated_at = NOW(), updated_by_id = $3,
               settings = COALESCE(u.settings, '{}'::jsonb) - 'subject_binding'
                          || jsonb_build_object('subject_sync', $3::text)
           WHERE u.id = 1 AND u.id <> $3
             AND (u.entity_id IS NULL
                  OR COALESCE(u.settings, '{}'::jsonb) ->> 'subject_binding' IS NOT NULL
                  OR NOT EXISTS (SELECT 1 FROM isahl.zc_id_subjects s
                                 WHERE s.id = u.entity_id AND s.deleted_at IS NULL))"#,
    )
    .bind(entity_table)
    .bind(entity_id)
    .bind(operator_id)
    .execute(pool)
    .await
    {
        Ok(res) if res.rows_affected() > 0 => {
            common::telemetry::info!(
                "sync_system_subject: system 已同步绑定 {entity_table}/{entity_id}（操作者 {operator_id}）"
            );
            true
        }
        Ok(_) => false,
        Err(e) => {
            common::telemetry::warn!("sync_system_subject: system 同步失败: {e}");
            false
        }
    }
}

/// POST /api/auth/entity-binding/personal
///
/// 事务四写：empl-natural + org_rr_employee + post_rr_employee + entity_id 更新。
/// FOR UPDATE 拿用户行锁后判空——并发双击第二事务等待后见已绑定 → 400。
pub async fn bind_personal(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<PersonalBindingBody>,
) -> HttpResponse {
    let user_id = match current_user(&req) {
        Ok(u) => u,
        Err(resp) => return resp,
    };
    let b = body.into_inner();
    if b.real_name.trim().is_empty() {
        return bad_request("VALIDATION", "姓名必填");
    }

    let mut tx = match pool.get_ref().begin().await {
        Ok(t) => t,
        Err(e) => {
            common::telemetry::warn!("bind_personal 开启事务失败: {e}");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "INTERNAL"}));
        }
    };

    // 幂等门（行锁）：m2o 语义——个人绑定只拒同类型（已绑个人实体）重复绑定；
    // 已绑组织实体不阻塞个人绑定（fix-ngac-entity-binding-m2o，类型分组判定）；
    // 占位绑定（fix-seed-user-subject-binding）视为未绑定，允许替换。
    let (bound_type, placeholder) = match bound_entity_type(&mut tx, user_id).await {
        Ok(t) => t,
        Err(BoundLookupError::UserNotFound) => {
            return bad_request("USER_NOT_FOUND", "用户不存在");
        }
        Err(BoundLookupError::Db(e)) => {
            common::telemetry::warn!("bind_personal 查询绑定状态失败: {e}");
            return bad_request("INTERNAL", "查询绑定状态失败");
        }
    };
    if !placeholder
        && bound_type
            .as_deref()
            .is_some_and(|t| already_bound_for(true, t))
    {
        return bad_request("ALREADY_BOUND", "个人主体已绑定，不可重复绑定");
    }

    // 雇佣主体/岗位存在性
    let org_exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(SELECT 1 FROM isahl.zc_id_subjects WHERE id = $1 AND deleted_at IS NULL)"#,
    )
    .bind(b.employer_org_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap_or(false);
    let pos_exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(SELECT 1 FROM isahl."zc_id_subj-position" WHERE id = $1 AND deleted_at IS NULL)"#,
    )
    .bind(b.position_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap_or(false);
    if !org_exists {
        return bad_request("VALIDATION", "雇佣主体不存在");
    }
    if !pos_exists {
        return bad_request("VALIDATION", "岗位不存在");
    }

    // 1. 自然人
    let empl_id: i64 = match sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_empl-natural" (id, notice, code, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3) RETURNING id"#,
    )
    .bind(b.real_name.trim())
    .bind(format!("emp-{}", user_id))
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            common::telemetry::warn!("bind_personal 建自然人失败: {e}");
            return bad_request("INTERNAL", "创建自然人失败");
        }
    };

    // 2/3. 任职关系（雇佣主体 + 岗位）
    for (sql, left) in [
        (
            r#"INSERT INTO isahl."zc_id_subj-org_rr_employee" (id, ref_left, ref_right, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3)"#,
            b.employer_org_id,
        ),
        (
            r#"INSERT INTO isahl."zc_id_subj-post_rr_employee" (id, ref_left, ref_right, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3)"#,
            b.position_id,
        ),
    ] {
        if let Err(e) = sqlx::query(sql)
            .bind(left)
            .bind(empl_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
        {
            common::telemetry::warn!("bind_personal 建任职关系失败: {e}");
            return bad_request("INTERNAL", "创建任职关系失败");
        }
    }

    // 4. entity 绑定（清占位标记——fix-seed-user-subject-binding：替换占位防复发）
    if let Err(e) = sqlx::query(
        "UPDATE isahl_auth.auth_users SET entity_id = $1, entity_table = 'zc_id_empl-natural', \
         updated_at = NOW(), settings = COALESCE(settings, '{}'::jsonb) - 'subject_binding' \
         WHERE id = $2",
    )
    .bind(empl_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    {
        common::telemetry::warn!("bind_personal 绑定 entity 失败: {e}");
        return bad_request("INTERNAL", "绑定失败");
    }

    if let Err(e) = tx.commit().await {
        common::telemetry::warn!("bind_personal 提交失败: {e}");
        return bad_request("INTERNAL", "提交失败");
    }
    // system 同步（增强投递，失败不阻断）
    let system_synced =
        sync_system_subject_binding(pool.get_ref(), user_id, "zc_id_empl-natural", empl_id).await;
    HttpResponse::Ok().json(
        serde_json::json!({"success": true, "entity_id": empl_id.to_string(), "system_subject_synced": system_synced}),
    )
}

#[derive(Debug, Deserialize)]
pub struct EnterpriseBindingBody {
    pub company_name: String,
    pub representative_name: Option<String>,
    pub parent_org_id: Option<i64>,
    /// m2o（fix-ngac-entity-binding-m2o）：绑定已有组织实体（可选）。
    /// 提供 entity_id + entity_table 时直接绑定该实体，不新建企业。
    pub entity_id: Option<i64>,
    /// 已有组织实体的 entity_table（组织类白名单校验，见 is_org_table）。
    pub entity_table: Option<String>,
}

/// 企业 UA：查/建 + 幂等指派 + 增量同步 employee 授权
/// （对齐 Framework ngac_ensure::ensure_employee_ua 模式；授权复制收敛至
/// approval::ngac_ensure::sync_enterprise_from_employee 公共 helper）。
/// 失败仅 warn 不回滚——数据绑定成功即成功（容错模式：增强投递不阻断主流程）。
async fn ensure_enterprise_ua(pool: &PgPool, user_id: i64) {
    let policy_class: Option<i64> =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class LIMIT 1")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let Some(policy_class) = policy_class else {
        common::telemetry::warn!("ensure_enterprise_ua: 无 policy_class，跳过");
        return;
    };
    let ua_id: i64 = match sqlx::query_scalar(
        r#"SELECT id FROM isahl_auth.ngac_user_attribute
           WHERE o_name = 'enterprise' AND deleted_at IS NULL AND fk_policy_class = $1 LIMIT 1"#,
    )
    .bind(policy_class)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(id)) => id,
        _ => match sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO isahl_auth.ngac_user_attribute
               (id, o_name, fk_policy_class, ancestor_ids, children_ids)
               VALUES (isahl.gen_next_zuid(), 'enterprise', $1, '{}'::bigint[], '{}'::bigint[])
               RETURNING id"#,
        )
        .bind(policy_class)
        .fetch_one(pool)
        .await
        {
            Ok(id) => id,
            Err(e) => {
                common::telemetry::warn!("ensure_enterprise_ua: 创建 enterprise UA 失败: {e}");
                return;
            }
        },
    };

    // 幂等指派 user → enterprise UA（helper 经 rr 活行定位目标 UA，须先指派后调用）
    if let Err(e) = sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, o_name)
           VALUES ($1, $2, 'enterprise')
           ON CONFLICT (fk_user, fk_user_attribute)
           DO UPDATE SET deleted_at = NULL, updated_at = NOW()"#,
    )
    .bind(user_id)
    .bind(ua_id)
    .execute(pool)
    .await
    {
        common::telemetry::warn!("ensure_enterprise_ua: user {user_id} 指派失败: {e}");
        return;
    }

    // 增量同步 employee 授权（Phase C G6 收口）：绑定时刻的静态快照复制已废除——
    // 经公共 helper 把本用户 employee UA 的 live association 幂等复制到 enterprise UA。
    // 此后 employee 新增授权无需回拷：ensure_employee_ua 内置同一 helper，覆盖全部
    // 授予路径（绑定此处仅补齐存量，helper 幂等无双调风险）；内部容错（失败 warn
    // 返回 0），不阻断返回。
    let synced = approval::ngac_ensure::sync_enterprise_from_employee(pool, user_id).await;
    if synced > 0 {
        common::telemetry::info!(
            "ensure_enterprise_ua: user {user_id} enterprise UA 增量同步 {synced} 条授权"
        );
    }
}

/// POST /api/auth/entity-binding/enterprise
pub async fn bind_enterprise(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<EnterpriseBindingBody>,
) -> HttpResponse {
    let user_id = match current_user(&req) {
        Ok(u) => u,
        Err(resp) => return resp,
    };
    let b = body.into_inner();
    if b.company_name.trim().is_empty() && b.entity_id.is_none() {
        return bad_request("VALIDATION", "企业名称必填");
    }

    let mut tx = match pool.get_ref().begin().await {
        Ok(t) => t,
        Err(e) => {
            common::telemetry::warn!("bind_enterprise 开启事务失败: {e}");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "INTERNAL"}));
        }
    };

    // 幂等门（行锁）：m2o 语义——组织绑定只拒同类型（已绑组织实体）重复绑定；
    // 已绑个人实体不阻塞组织绑定（fix-ngac-entity-binding-m2o，类型分组判定）；
    // 占位绑定（fix-seed-user-subject-binding）视为未绑定，允许替换。
    let (bound_type, placeholder) = match bound_entity_type(&mut tx, user_id).await {
        Ok(t) => t,
        Err(BoundLookupError::UserNotFound) => {
            return bad_request("USER_NOT_FOUND", "用户不存在");
        }
        Err(BoundLookupError::Db(e)) => {
            common::telemetry::warn!("bind_enterprise 查询绑定状态失败: {e}");
            return bad_request("INTERNAL", "查询绑定状态失败");
        }
    };
    if !placeholder
        && bound_type
            .as_deref()
            .is_some_and(|t| already_bound_for(false, t))
    {
        return bad_request("ALREADY_BOUND", "组织主体已绑定，不可重复绑定");
    }

    // m2o：绑定已有组织实体路径（不新建企业）。entity_id/entity_table 成对出现。
    let existing_entity: Option<(i64, String)> = match (b.entity_id, b.entity_table.as_deref()) {
        (Some(eid), Some(et)) => {
            if !is_org_table(et) {
                return bad_request("VALIDATION", "不支持的实体类型，仅限组织类实体");
            }
            let exists: bool = sqlx::query_scalar(
                r#"SELECT EXISTS(SELECT 1 FROM isahl."zc_id_entity" WHERE id = $1 AND deleted_at IS NULL)"#,
            )
            .bind(eid)
            .fetch_one(&mut *tx)
            .await
            .unwrap_or(false);
            if !exists {
                return bad_request("VALIDATION", "组织实体不存在");
            }
            Some((eid, et.to_string()))
        }
        (Some(_), None) | (None, Some(_)) => {
            return bad_request("VALIDATION", "entity_id 与 entity_table 必须同时提供");
        }
        (None, None) => None,
    };

    // 上级法人存在性（可选，仅新建路径）
    if existing_entity.is_none() {
        if let Some(pid) = b.parent_org_id {
            let exists: bool = sqlx::query_scalar(
                r#"SELECT EXISTS(SELECT 1 FROM isahl.zc_id_subjects WHERE id = $1 AND deleted_at IS NULL)"#,
            )
            .bind(pid)
            .fetch_one(&mut *tx)
            .await
            .unwrap_or(false);
            if !exists {
                return bad_request("VALIDATION", "上级法人不存在");
            }
        }
    }

    // m2o：绑定已有组织实体——跳过新建企业/法人/归属，直接绑定（entity_table 取请求值）
    let (org_id, entity_table): (i64, &str) = match &existing_entity {
        Some((eid, et)) => (*eid, et.as_str()),
        None => {
            // 1. 法人代表（可选）：查同名自然人行，无则新建——fk_representative 实列化
            //    （断线盘点 #3：此前仅存 comments 文本，法人信息未结构化）
            let rep_name = b
                .representative_name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let rep_id: Option<i64> = match rep_name {
                None => None,
                Some(name) => {
                    let existing: Option<i64> = sqlx::query_scalar(
                        r#"SELECT id FROM isahl.zc_id_subjects
                           WHERE notice = $1 AND deleted_at IS NULL
                             AND tableoid = 'isahl."zc_id_empl-natural"'::regclass
                           ORDER BY id LIMIT 1"#,
                    )
                    .bind(name)
                    .fetch_optional(&mut *tx)
                    .await
                    .unwrap_or(None);
                    match existing {
                        Some(id) => Some(id),
                        None => sqlx::query_scalar(
                            r#"INSERT INTO isahl."zc_id_empl-natural" (id, notice, code, created_by_id)
                               VALUES (isahl.gen_next_zuid(), $1, $2, $3) RETURNING id"#,
                        )
                        .bind(name)
                        .bind(format!("rep-{}", user_id))
                        .bind(user_id)
                        .fetch_optional(&mut *tx)
                        .await
                        .ok()
                        .flatten(),
                    }
                }
            };
            let comments =
                rep_name.map(|n| serde_json::json!({"representative_name": n}).to_string());
            // 2. 非银行法人（法人代表 fk_representative 实列化 + comments 文本快照）
            let org_id: i64 = match sqlx::query_scalar(
                r#"INSERT INTO isahl."zc_id_orga-non-banking-legal"
                   (id, notice, code, comments, fk_representative, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5) RETURNING id"#,
            )
            .bind(b.company_name.trim())
            .bind(format!("org-{}", user_id))
            .bind(comments)
            .bind(rep_id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            {
                Ok(id) => id,
                Err(e) => {
                    common::telemetry::warn!("bind_enterprise 建法人失败: {e}");
                    return bad_request("INTERNAL", "创建法人主体失败");
                }
            };

            // 2. 上级归属（可选）
            if let Some(pid) = b.parent_org_id {
                if let Err(e) = sqlx::query(
                    r#"INSERT INTO isahl."zc_id_subj-org_rr_subordinate" (id, ref_left, ref_right, created_by_id)
                       VALUES (isahl.gen_next_zuid(), $1, $2, $3)"#,
                )
                .bind(pid)
                .bind(org_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                {
                    common::telemetry::warn!("bind_enterprise 建归属关系失败: {e}");
                    return bad_request("INTERNAL", "创建上级归属失败");
                }
            }

            (org_id, "zc_id_orga-non-banking-legal")
        }
    };

    // 3. entity 绑定（清占位标记——fix-seed-user-subject-binding：替换占位防复发）
    if let Err(e) = sqlx::query(
        "UPDATE isahl_auth.auth_users SET entity_id = $1, entity_table = $2, \
         updated_at = NOW(), settings = COALESCE(settings, '{}'::jsonb) - 'subject_binding' \
         WHERE id = $3",
    )
    .bind(org_id)
    .bind(entity_table)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    {
        common::telemetry::warn!("bind_enterprise 绑定 entity 失败: {e}");
        return bad_request("INTERNAL", "绑定失败");
    }

    if let Err(e) = tx.commit().await {
        common::telemetry::warn!("bind_enterprise 提交失败: {e}");
        return bad_request("INTERNAL", "提交失败");
    }

    // enterprise UA（事务外，失败仅 warn）
    ensure_enterprise_ua(pool.get_ref(), user_id).await;

    // system 同步（置于 ensure_enterprise_ua 后：企业自注册创始人 UA 后指派也能命中门控）
    let system_synced =
        sync_system_subject_binding(pool.get_ref(), user_id, entity_table, org_id).await;
    HttpResponse::Ok().json(
        serde_json::json!({"success": true, "entity_id": org_id.to_string(), "system_subject_synced": system_synced}),
    )
}

/// 可绑定主体类型白名单（fix-seed-user-subject-binding）：
/// (叶表名, 展示名, 是否法人, 编码是否必填——法人类 code 即统一社会信用代码)
///
/// 叶表写入规则（不可破）：仅继承链叶子可写。法人必须落具体叶表——
/// zc_id_bank-commercial（商业银行）或 zc_id_orga-non-banking-legal（非银行法人），
/// 中间层 zc_id_orga-legal 禁写；组织同理，zc_id_subj-org 禁写
/// （其子类型 zc_id_orga-department / 法人两叶表承接）。
const SUBJECT_TYPES: &[(&str, &str, bool, bool)] = &[
    ("zc_id_orga-non-banking-legal", "非银行法人", true, true),
    ("zc_id_bank-commercial", "商业银行", true, true),
    ("zc_id_empl-natural", "个人（自然人）", false, false),
    ("zc_id_empl-agent", "智能体", false, false),
    ("zc_id_orga-department", "部门（组织）", false, false),
    ("zc_id_subj-group", "组", false, false),
];

/// GET /api/auth/entity-binding/subject-types
///
/// 可绑定主体类型清单（subjects 继承链叶表白名单；to_regclass 校验表存在）。
pub async fn subject_types(req: HttpRequest, pool: web::Data<PgPool>) -> HttpResponse {
    if current_user(&req).is_err() {
        return HttpResponse::Unauthorized().json(serde_json::json!({"error": "UNAUTHORIZED"}));
    }
    let mut items = Vec::new();
    for (table, label, is_legal, code_required) in SUBJECT_TYPES {
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(format!("isahl.\"{table}\""))
            .fetch_one(pool.get_ref())
            .await
            .unwrap_or(false);
        if exists {
            items.push(serde_json::json!({
                "subject_type": table,
                "label": label,
                "is_legal": is_legal,
                "code_required": code_required,
            }));
        }
    }
    HttpResponse::Ok().json(serde_json::json!({"success": true, "subject_types": items}))
}

#[derive(Debug, Deserialize)]
pub struct SubjectBindingBody {
    /// 白名单叶表名（subject-types 返回）
    pub subject_type: String,
    /// 主体名称（notice）
    pub notice: String,
    /// 主体编码（法人类必填 = 统一社会信用代码）
    pub code: Option<String>,
    /// 选择已有主体（提供时跳过创建，须与 subject_type 类型一致）
    pub entity_id: Option<i64>,
    /// 显式改绑（add-subject-rebind-management）：true 时已绑真实主体也放行替换
    /// （m2o 非破坏——旧实体行/关系保留，仅 entity_table/entity_id 锚点切换）；
    /// 缺省维持幂等门（首登引导防误触）。
    pub rebind: Option<bool>,
}

/// 白名单叶表内创建主体行（固定 SQL 模板枚举分发，表名不拼接——防注入）。
/// 调用方 MUST 先经白名单校验。
async fn insert_subject_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    table: &str,
    notice: &str,
    code: &str,
    user_id: i64,
) -> Result<i64, sqlx::Error> {
    let sql = match table {
        "zc_id_orga-non-banking-legal" => {
            r#"INSERT INTO isahl."zc_id_orga-non-banking-legal" (id, notice, code, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3) RETURNING id"#
        }
        "zc_id_bank-commercial" => {
            r#"INSERT INTO isahl."zc_id_bank-commercial" (id, notice, code, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3) RETURNING id"#
        }
        "zc_id_empl-natural" => {
            r#"INSERT INTO isahl."zc_id_empl-natural" (id, notice, code, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3) RETURNING id"#
        }
        "zc_id_empl-agent" => {
            r#"INSERT INTO isahl."zc_id_empl-agent" (id, notice, code, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3) RETURNING id"#
        }
        "zc_id_orga-department" => {
            r#"INSERT INTO isahl."zc_id_orga-department" (id, notice, code, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3) RETURNING id"#
        }
        "zc_id_subj-group" => {
            r#"INSERT INTO isahl."zc_id_subj-group" (id, notice, code, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3) RETURNING id"#
        }
        _ => return Err(sqlx::Error::Protocol("invalid subject_type".into())),
    };
    sqlx::query_scalar(sql)
        .bind(notice)
        .bind(code)
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await
}

/// POST /api/auth/entity-binding/subject
///
/// 任意类型主体绑定（fix-seed-user-subject-binding）：创建（白名单叶表）或选择
/// 已有主体，替换占位绑定。m2o：不排斥其他 user 绑定同一主体。
/// 替换占位时清除 settings.subject_binding 标记（防再次判定占位）。
pub async fn bind_subject(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<SubjectBindingBody>,
) -> HttpResponse {
    let user_id = match current_user(&req) {
        Ok(u) => u,
        Err(resp) => return resp,
    };
    let b = body.into_inner();

    // 白名单校验（防表名注入——subject_type 仅作枚举分发）
    let Some((_, _, _, code_required)) = SUBJECT_TYPES
        .iter()
        .find(|(table, ..)| *table == b.subject_type)
    else {
        return bad_request("VALIDATION", "不支持的实体类型");
    };
    if b.entity_id.is_none() && b.notice.trim().is_empty() {
        return bad_request("VALIDATION", "主体名称必填");
    }
    let code = b.code.as_deref().map(str::trim).unwrap_or("");
    // code 必填仅创建模式生效（选择已有主体时主体已存在，编码不校验）
    if b.entity_id.is_none() && *code_required && code.is_empty() {
        return bad_request("VALIDATION", "法人主体编码（统一社会信用代码）必填");
    }

    let mut tx = match pool.get_ref().begin().await {
        Ok(t) => t,
        Err(e) => {
            common::telemetry::warn!("bind_subject 开启事务失败: {e}");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": "INTERNAL"}));
        }
    };

    // 幂等门（行锁）：已绑真实业务主体（非占位）拒绝重复绑定；占位/未绑允许替换
    let (bound_type, placeholder) = match bound_entity_type(&mut tx, user_id).await {
        Ok(t) => t,
        Err(BoundLookupError::UserNotFound) => return bad_request("USER_NOT_FOUND", "用户不存在"),
        Err(BoundLookupError::Db(e)) => {
            common::telemetry::warn!("bind_subject 查询绑定状态失败: {e}");
            return bad_request("INTERNAL", "查询绑定状态失败");
        }
    };
    // 改绑 opt-in（add-subject-rebind-management）：rebind=true 放行真实绑定替换；
    // 缺省维持幂等门。
    if bound_type.is_some() && !placeholder && !b.rebind.unwrap_or(false) {
        return bad_request("ALREADY_BOUND", "业务主体已绑定，不可重复绑定");
    }

    let (subject_id, subject_table): (i64, String) = if let Some(eid) = b.entity_id {
        // entity_id 模式：tableoid 服务端解析实际叶表（add-subject-rebind-management）
        // ——主体落表以 DB 为准，不依赖请求声明的类型（选择器无从得知行落点）。
        let resolved: Result<Option<(String,)>, sqlx::Error> = sqlx::query_as(
            "SELECT c.relname::text FROM isahl.zc_id_subjects s \
             JOIN pg_class c ON c.oid = s.tableoid \
             WHERE s.id = $1 AND s.deleted_at IS NULL",
        )
        .bind(eid)
        .fetch_optional(&mut *tx)
        .await;
        match resolved {
            Ok(Some((table_name,))) => (eid, table_name),
            Ok(None) => {
                return bad_request("VALIDATION", "主体不存在");
            }
            Err(e) => {
                common::telemetry::warn!("bind_subject tableoid 解析失败: {e}");
                return bad_request("INTERNAL", "主体解析失败");
            }
        }
    } else {
        match insert_subject_row(&mut tx, &b.subject_type, b.notice.trim(), code, user_id).await {
            Ok(id) => (id, b.subject_type.clone()),
            Err(e) => {
                common::telemetry::warn!("bind_subject 创建主体失败: {e}");
                return bad_request("INTERNAL", "创建主体失败");
            }
        }
    };

    // 改绑（替换占位）：清 subject_binding 标记防再次判定占位
    if let Err(e) = sqlx::query(
        "UPDATE isahl_auth.auth_users \
         SET entity_id = $1, entity_table = $2, updated_at = NOW(), updated_by_id = $3, \
             settings = COALESCE(settings, '{}'::jsonb) - 'subject_binding' \
         WHERE id = $4",
    )
    .bind(subject_id)
    .bind(&subject_table)
    .bind(user_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    {
        common::telemetry::warn!("bind_subject 绑定 entity 失败: {e}");
        return bad_request("INTERNAL", "绑定失败");
    }

    if let Err(e) = tx.commit().await {
        common::telemetry::warn!("bind_subject 提交失败: {e}");
        return bad_request("INTERNAL", "提交失败");
    }
    // system 同步（增强投递，失败不阻断）
    let system_synced =
        sync_system_subject_binding(pool.get_ref(), user_id, &subject_table, subject_id).await;
    HttpResponse::Ok().json(serde_json::json!(
        {"success": true, "entity_id": subject_id.to_string(), "system_subject_synced": system_synced}
    ))
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/entity-binding")
            .route("/status", web::get().to(status))
            .route("/options", web::get().to(options))
            .route("/personal", web::post().to(bind_personal))
            .route("/enterprise", web::post().to(bind_enterprise))
            .route("/subject-types", web::get().to(subject_types))
            .route("/subject", web::post().to(bind_subject)),
    );
}
