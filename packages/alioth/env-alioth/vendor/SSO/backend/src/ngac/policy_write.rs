//! NGAC 策略**写服务层**（change `consolidate-ngac-write-path`）。
//!
//! **单一写实现**：关联 / 禁止 / UA 指派的行级写，连同**同事务**的行镜像与策略变更审计，
//! 集中在本 module。两类调用方 MUST 经此：
//! - `admin::handlers::ngac` 的 HTTP 写端点（`/api/admin/ngac/*`）；
//! - Gateway chat 动作 `ngac_strategy_apply`（`chat_sessions::adapters::strategy_action`）。
//!
//! 调用方 MUST NOT 各自裸写 `isahl_auth.ngac_*`——活规约
//! `ngac-admin::nl-assist-assistant-wiring` 要求动作「经 SSO 服务层逐项执行」，
//! 而裸 SQL 会绕开本 module 的事务/审计口径。
//!
//! **语义（与既有 handler 逐字一致，MUST NOT 改动）**：
//! - 每次写入在同一事务内取变更前后行镜像（`to_jsonb`）并按 `action` ∈
//!   `insert`/`update`/`delete` 写 `isahl_auth.ngac_policy_audit_log`；失败随事务回滚。
//! - **无变更不留痕**：`rows_affected = 0`、`ON CONFLICT DO NOTHING` 未插入、
//!   幂等 `grant_rights` 零新增 ⇒ MUST NOT 写审计行。
//! - delete 一律软删（`deleted_at = NOW()`）；条件语义唯一承载列 = `conditions`。
//! - 授权的 `right` 以**名字**（`ngac_access_right.o_name`）入参，在事务内解析为 id；
//!   未知名字与既有 `grant_rights` 同口径**静默丢弃**（MUST NOT 报错——保持一致）。

use serde_json::Value;
use sqlx::{PgConnection, PgPool};

use super::audit_writer::{row_mirror_tx, user_rr_mirror_tx, write_audit_tx, AuditRecord};

/// 审计主体。HTTP 端点用 `require_admin_claims` 的结果（带 `sid` 与客户端 IP）；
/// chat 动作在服务端会话内执行 ⇒ `session_id`/`ip_address` 为 `None`。
#[derive(Debug, Clone)]
pub struct Actor {
    pub user_id: i64,
    pub session_id: Option<String>,
    pub ip_address: Option<String>,
}

impl Actor {
    /// 无 sid / 无 IP 的进程内主体（chat 动作路径）。
    pub fn internal(user_id: i64) -> Self {
        Self {
            user_id,
            session_id: None,
            ip_address: None,
        }
    }
}

/// 授权右：名字 → id（事务内解析；未知名字静默丢弃，与 `grant_rights` 同口径）。
async fn resolve_right_ids(
    tx: &mut PgConnection,
    names: &[String],
) -> Result<Vec<i64>, sqlx::Error> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = ANY($1) ORDER BY id",
    )
    .bind(names)
    .fetch_all(&mut *tx)
    .await
}

/// 写一行审计（调用方 MUST 仅在确有变更时调用）。
async fn audit(
    tx: &mut PgConnection,
    actor: &Actor,
    action: &'static str,
    entity_type: &'static str,
    entity_id: i64,
    table: &str,
    old_values: Option<Value>,
) -> Result<(), sqlx::Error> {
    let new_values = if action == "delete" {
        None
    } else {
        row_mirror_tx(tx, table, entity_id).await?
    };
    write_audit_tx(
        tx,
        &AuditRecord {
            action,
            entity_type,
            entity_id,
            old_values,
            new_values,
            actor: actor.user_id,
            session_id: actor.session_id.clone(),
            ip_address: actor.ip_address.clone(),
        },
    )
    .await
}

// ============================================================
// 关联（association）
// ============================================================

/// 给资源上的 UA 授一组 right —— **图级授权**（幂等并集，语义由
/// `common::ngac_grants::grant_rights` 单源承担），本 fn 只补事务边界与审计。
///
/// 返回实际受影响行数（0 = 幂等命中，无变更 ⇒ 不写审计）。
pub async fn grant_resource_rights(
    pool: &PgPool,
    actor: &Actor,
    resource_type: &str,
    resource_id: i64,
    ua_name: &str,
    rights: &[String],
) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let refs: Vec<&str> = rights.iter().map(String::as_str).collect();
    let affected =
        common::ngac_grants::grant_rights(&mut *tx, resource_type, resource_id, ua_name, &refs)
            .await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
    if affected > 0 {
        // grant 只回行数 ⇒ 同事务内回查受影响行以取镜像
        if let Some(id) = sqlx::query_scalar::<_, i64>(
            "SELECT a.id FROM isahl_auth.ngac_association a \
             JOIN isahl_auth.ngac_user_attribute ua ON ua.id = a.fk_user_attribute \
             WHERE ua.o_name = $1 AND a.fk_object_attribute = $2 AND a.deleted_at IS NULL \
             ORDER BY a.id LIMIT 1",
        )
        .bind(ua_name)
        .bind(resource_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            audit(
                &mut tx,
                actor,
                "insert",
                "association",
                id,
                "ngac_association",
                None,
            )
            .await?;
        }
    }
    tx.commit().await?;
    Ok(affected)
}

/// UA/OA 直连关联的输入（`None` = 保持原值；COALESCE 语义在 SQL 内）。
#[derive(Debug, Default, Clone)]
pub struct AssociationPatch {
    pub o_name: Option<String>,
    /// 授权 right **名字**（`None` = 不改；`Some(vec![])` = 清空）。
    pub rights: Option<Vec<String>>,
    pub conditions: Option<Value>,
}

/// 更新关联（行级；`false` = 目标不存在）。
pub async fn update_association(
    pool: &PgPool,
    actor: &Actor,
    id: i64,
    patch: &AssociationPatch,
) -> Result<bool, sqlx::Error> {
    // conditions 写径白名单（tasks 1.5）：未知键在核心 `evaluate_conditions` 中 fail-closed
    // （该策略永不满足）⇒ 写径 MUST 拒绝，避免「写入成功但永不生效」的静默面。
    // 键集唯一真源 = `ngac_decision::CONDITION_KEYS`。本 module 是 admin HTTP 与 chat 动作的
    // 共用写服务，故校验落此（调用方无须重复）。
    if let Some(v) = &patch.conditions {
        ngac_host::validate_condition_keys(v)
            .map_err(|e| sqlx::Error::Protocol(format!("conditions 非法：{e}")))?;
    }
    let mut tx = pool.begin().await?;
    let old_values = row_mirror_tx(&mut tx, "ngac_association", id).await?;
    let right_ids = match &patch.rights {
        Some(names) => Some(resolve_right_ids(&mut tx, names).await?),
        None => None,
    };
    let r = sqlx::query(
        "UPDATE isahl_auth.ngac_association SET \
           o_name = COALESCE($2, o_name), \
           ak_access_rights = COALESCE($3, ak_access_rights), \
           conditions = COALESCE($4, conditions), \
           updated_at = NOW(), updated_by_id = $5 \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(&patch.o_name)
    .bind(&right_ids)
    .bind(&patch.conditions)
    .bind(actor.user_id)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 0 {
        return Ok(false);
    }
    audit(
        &mut tx,
        actor,
        "update",
        "association",
        id,
        "ngac_association",
        old_values,
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// 软删关联（`false` = 目标不存在）。
pub async fn delete_association(
    pool: &PgPool,
    actor: &Actor,
    id: i64,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let old_values = row_mirror_tx(&mut tx, "ngac_association", id).await?;
    let r = sqlx::query(
        "UPDATE isahl_auth.ngac_association \
         SET deleted_at = NOW(), updated_at = NOW(), updated_by_id = $1 \
         WHERE id = $2 AND deleted_at IS NULL",
    )
    .bind(actor.user_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 0 {
        return Ok(false);
    }
    audit(
        &mut tx,
        actor,
        "delete",
        "association",
        id,
        "ngac_association",
        old_values,
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

// ============================================================
// 禁止（prohibition）
// ============================================================

/// 新建禁止（返回新行 id）。
pub async fn create_prohibition(
    pool: &PgPool,
    actor: &Actor,
    ua_id: i64,
    oa_id: i64,
    rights: &[String],
) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let right_ids = resolve_right_ids(&mut tx, rights).await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO isahl_auth.ngac_prohibition \
         (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_by_id) \
         SELECT $1, $2, $3, \
                COALESCE((SELECT fk_policy_class FROM isahl_auth.ngac_object_attribute WHERE id = $2), 1), \
                $4 \
         RETURNING id",
    )
    .bind(ua_id)
    .bind(oa_id)
    .bind(&right_ids)
    .bind(actor.user_id)
    .fetch_one(&mut *tx)
    .await?;
    audit(
        &mut tx,
        actor,
        "insert",
        "prohibition",
        id,
        "ngac_prohibition",
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// 禁止的更新输入（`None` = 保持原值）。
#[derive(Debug, Default, Clone)]
pub struct ProhibitionPatch {
    pub rights: Option<Vec<String>>,
    pub is_active: Option<bool>,
    pub conditions: Option<Value>,
}

/// 更新禁止（`false` = 目标不存在）。
pub async fn update_prohibition(
    pool: &PgPool,
    actor: &Actor,
    id: i64,
    patch: &ProhibitionPatch,
) -> Result<bool, sqlx::Error> {
    // conditions 写径白名单（同 `update_association`：核心 fail-closed + 写径早拒）。
    if let Some(v) = &patch.conditions {
        ngac_host::validate_condition_keys(v)
            .map_err(|e| sqlx::Error::Protocol(format!("conditions 非法：{e}")))?;
    }
    let mut tx = pool.begin().await?;
    let old_values = row_mirror_tx(&mut tx, "ngac_prohibition", id).await?;
    let right_ids = match &patch.rights {
        Some(names) => Some(resolve_right_ids(&mut tx, names).await?),
        None => None,
    };
    let r = sqlx::query(
        "UPDATE isahl_auth.ngac_prohibition SET \
           ak_access_rights = COALESCE($2, ak_access_rights), \
           is_active = COALESCE($3, is_active), \
           conditions = COALESCE($4, conditions), \
           updated_at = NOW(), updated_by_id = $5 \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(&right_ids)
    .bind(patch.is_active)
    .bind(&patch.conditions)
    .bind(actor.user_id)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 0 {
        return Ok(false);
    }
    audit(
        &mut tx,
        actor,
        "update",
        "prohibition",
        id,
        "ngac_prohibition",
        old_values,
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// 软删禁止（`false` = 目标不存在）。
pub async fn delete_prohibition(
    pool: &PgPool,
    actor: &Actor,
    id: i64,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let old_values = row_mirror_tx(&mut tx, "ngac_prohibition", id).await?;
    let r = sqlx::query(
        "UPDATE isahl_auth.ngac_prohibition \
         SET deleted_at = NOW(), updated_at = NOW(), updated_by_id = $1 \
         WHERE id = $2 AND deleted_at IS NULL",
    )
    .bind(actor.user_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 0 {
        return Ok(false);
    }
    audit(
        &mut tx,
        actor,
        "delete",
        "prohibition",
        id,
        "ngac_prohibition",
        old_values,
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

// ============================================================
// UA 指派（user_assignment）
// ============================================================

/// 指派 UA（`false` = 已存在，幂等无变更 ⇒ 不写审计）。
pub async fn assign_user_attribute(
    pool: &PgPool,
    actor: &Actor,
    user_id: i64,
    ua_id: i64,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let inserted: Option<(i64,)> = sqlx::query_as(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_by_id) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (fk_user, fk_user_attribute) DO NOTHING \
         RETURNING id",
    )
    .bind(user_id)
    .bind(ua_id)
    .bind(actor.user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((binding_id,)) = inserted else {
        return Ok(false);
    };
    let new_values = user_rr_mirror_tx(&mut tx, user_id, ua_id).await?;
    write_audit_tx(
        &mut tx,
        &AuditRecord {
            action: "insert",
            entity_type: "user_assignment",
            entity_id: binding_id,
            old_values: None,
            new_values,
            actor: actor.user_id,
            session_id: actor.session_id.clone(),
            ip_address: actor.ip_address.clone(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// 解除 UA 指派（软删；`false` = 未绑定）。
pub async fn remove_user_attribute(
    pool: &PgPool,
    actor: &Actor,
    user_id: i64,
    ua_id: i64,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let binding: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user = $1 AND fk_user_attribute = $2 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(ua_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((binding_id,)) = binding else {
        return Ok(false);
    };
    let old_values = user_rr_mirror_tx(&mut tx, user_id, ua_id).await?;
    let r = sqlx::query(
        "UPDATE isahl_auth.ngac_user_rr_attribute \
         SET deleted_at = NOW(), updated_at = NOW() \
         WHERE fk_user = $1 AND fk_user_attribute = $2 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(ua_id)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 0 {
        return Ok(false);
    }
    write_audit_tx(
        &mut tx,
        &AuditRecord {
            action: "delete",
            entity_type: "user_assignment",
            entity_id: binding_id,
            old_values,
            new_values: None,
            actor: actor.user_id,
            session_id: actor.session_id.clone(),
            ip_address: actor.ip_address.clone(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}
