//! 种子用户自愈：system 哨兵用户（id=1）
//!
//! 跨 namespace 通用：AVIC 等 namespace 业务种子以 created_by_id=1 引用 system 用户，
//! fresh 库未跑 seed-isahl-user.sh 时 NGAC 行级对象属性 FK 落空。此处保证结构存在。
//!
//! 边界：不携带凭据（password_hash=NULL，SECURITY_SPEC §6）；不创建业务主体绑定
//! （entity_table/entity_id 绑定引导保留在 SystemBootstrapPage，主体名称属部署决策）。

use sqlx::PgPool;

use super::SeedStats;

/// system 哨兵用户（id=1）检测/创建；system↔主体绑定状态告警（不自动绑定）。
pub async fn ensure(pool: &PgPool) -> SeedStats {
    let mut stats = SeedStats::default();

    // 1. 哨兵用户：id=1 / username='system' 存在性（对齐 seed-isahl-user.sh 契约）
    match ensure_system_user(pool).await {
        Ok(created) => {
            if created {
                stats.created += 1;
            } else {
                stats.existing += 1;
            }
        }
        Err(e) => {
            common::telemetry::warn!("seed[auth]: system 哨兵用户自愈失败: {e}");
        }
    }

    // 2. 主体绑定状态复核（同 main.rs seed_self_check_system_subject 语义）：
    //    未绑定不自动绑定（主体名称属部署决策），warn 引导「系统设置」页。
    match check_system_subject_binding(pool).await {
        Ok(()) => {}
        Err(msg) => common::telemetry::warn!("seed[auth]: {msg}"),
    }

    // 3. 零 UA 指派的 standard 用户补挂 user UA（fix-approval-endpoint-gates：
    //    注册默认身份自愈——覆盖历史用户；已有任何 UA 指派的用户不动，不干扰显式管理）
    match backfill_user_ua(pool).await {
        Ok(n) if n > 0 => {
            stats.healed += n as usize;
            common::telemetry::info!("seed[auth]: 补挂 {} 个用户的 user UA（注册默认身份）", n);
        }
        Ok(_) => {}
        Err(e) => common::telemetry::warn!("seed[auth]: user UA 补挂失败: {e}"),
    }

    stats
}

/// 零 UA 指派的 standard 用户批量补挂 user UA（幂等，返回补挂数）。
async fn backfill_user_ua(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let inserted = sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, o_name)
        SELECT u.id, ua.id, 'user'
        FROM isahl_auth.auth_users u
        CROSS JOIN isahl_auth.ngac_user_attribute ua
        WHERE ua.o_name = 'user' AND ua.deleted_at IS NULL
          AND u.user_type = 'standard' AND u.is_active = true
          AND NOT EXISTS (
              SELECT 1 FROM isahl_auth.ngac_user_rr_attribute r
              WHERE r.fk_user = u.id AND r.deleted_at IS NULL
          )
    "#,
    )
    .execute(pool)
    .await?
    .rows_affected();
    Ok(inserted as i64)
}

/// 幂等创建 system 哨兵用户（id=1）。返回是否本次新建。
async fn ensure_system_user(pool: &PgPool) -> Result<bool, sqlx::Error> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM isahl_auth.auth_users WHERE id = 1)")
            .fetch_one(pool)
            .await?;

    if exists {
        return Ok(false);
    }

    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, password_hash, status, user_type, is_active)
           VALUES (1, 'system', 'system', 'system@aliothstudio.local',
                   NULL, 'active', 'system', true)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .execute(pool)
    .await?;

    Ok(true)
}

/// system 用户 ↔ subjects 主体绑定复核（entity_table/entity_id）。
/// m2o 语义（fix-ngac-entity-binding-m2o）：system 绑定一个组织主体，
/// 不排斥其他 user 绑定同一主体。未绑定/悬空 → Err(引导信息)；正常 → Ok。
async fn check_system_subject_binding(pool: &PgPool) -> Result<(), String> {
    let row: Option<(Option<String>, Option<i64>)> =
        sqlx::query_as("SELECT entity_table, entity_id FROM isahl_auth.auth_users WHERE id = 1")
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("system 用户绑定查询失败: {e}"))?;

    match row {
        Some((Some(tbl), Some(eid))) if tbl == "zc_id_subjects" && eid > 0 => {
            let subj_ok: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM isahl.zc_id_subjects WHERE id = $1 AND deleted_at IS NULL)",
            )
            .bind(eid)
            .fetch_one(pool)
            .await
            .map_err(|e| format!("主体存在性复核失败: {e}"))?;
            if subj_ok {
                common::telemetry::info!("seed[auth]: system 用户已绑定主体 {eid}");
                Ok(())
            } else {
                Err(format!(
                    "system 用户 entity_id={eid} 指向不存在的主体——请到「系统设置」页重新绑定"
                ))
            }
        }
        _ => Err("system 用户未绑定主体（entity_table/entity_id 缺失）——请到「系统设置」页添加并绑定（可与其他 user 共享同一主体）".to_string()),
    }
}
