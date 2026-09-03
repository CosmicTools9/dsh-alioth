//! ngac_org — 组织任职体系 ↔ NGAC 认知派生的 Framework 同源副本
//!
//! 唯一实现 = `SSO/backend/src/ngac/pip.rs`（`COGNITION_CTE` /
//! `cognition_derived_user_holders`，add-ngac-cognition-derived-ua）。
//! 本模块为跨 crate 同构副本，MUST 保持与 pip.rs 推导链一致（NGAC_SPEC
//! §2.2.3 消费同源义务）：user → empl-agent/empl-natural(fk_user) →
//! `zc_id_subj-post_rr_employee`(ref_left=岗位, ref_right=雇员) → position；
//! 全部边 `deleted_at IS NULL`。改动 pip.rs 时 grep 本文件同步。
//!
//! 语义收敛（integrate-framework-cognition-ua）：审批/通知成员解析目标可为
//! ① 认知派生名（`position:{code}` / `view:{code}`，任职桥持有者）、
//! ② 岗位标识（id / code / notice，直管 fk_user ∪ 任职桥持有者）、
//! ③ 指派型 UA 名（`ngac_user_rr_attribute` 物化成员，兼容既有角色配置）。
//! 三者并集去重——岗位任职（读侧派生）与指派 UA 不再双轨分叉。

use sqlx::PgConnection;

/// 认知派生名（`position:` / `view:` 前缀）的活跃持有者反解——同构 pip.rs
/// `cognition_derived_user_holders`（限前缀名；非前缀返回空集）。
async fn cognition_holders(conn: &mut PgConnection, o_name: &str) -> Result<Vec<i64>, sqlx::Error> {
    if !o_name.starts_with("position:") && !o_name.starts_with("view:") {
        return Ok(Vec::new());
    }
    sqlx::query_scalar(
        r#"
        WITH pos AS (
            SELECT ea.fk_user, sp.id AS position_id, sp.code
            FROM isahl."zc_id_empl-agent" ea
            JOIN isahl."zc_id_subj-post_rr_employee" spre
                ON spre.ref_right = ea.id AND spre.deleted_at IS NULL
            JOIN isahl."zc_id_subj-position" sp
                ON sp.id = spre.ref_left AND sp.deleted_at IS NULL
            WHERE ea.deleted_at IS NULL
            UNION ALL
            SELECT en.fk_user, sp.id, sp.code
            FROM isahl."zc_id_empl-natural" en
            JOIN isahl."zc_id_subj-post_rr_employee" spre
                ON spre.ref_right = en.id AND spre.deleted_at IS NULL
            JOIN isahl."zc_id_subj-position" sp
                ON sp.id = spre.ref_left AND sp.deleted_at IS NULL
            WHERE en.deleted_at IS NULL
        ),
        holders AS (
            SELECT fk_user FROM pos
            WHERE code IS NOT NULL AND 'position:' || code = $1
            UNION
            SELECT p.fk_user FROM pos p
            JOIN isahl."zc_id_relation-post_view_r_tags" r
                ON r.ref_left = p.position_id AND r.deleted_at IS NULL
            JOIN isahl."zc_id_tags-post_view" vt
                ON vt.id = r.ref_right AND vt.deleted_at IS NULL
            WHERE vt.code IS NOT NULL AND 'view:' || vt.code = $1
        )
        SELECT DISTINCT u.id
        FROM holders h
        JOIN isahl_auth.auth_users u ON u.id = h.fk_user
        WHERE u.is_active = TRUE
        ORDER BY u.id
        "#,
    )
    .bind(o_name)
    .fetch_all(conn)
    .await
}

/// 岗位标识（id / code / notice）的成员：直管 fk_user ∪ 任职桥持有者。
async fn position_members(
    conn: &mut PgConnection,
    val: &str,
    limit: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        WITH target AS (
            SELECT id, fk_user FROM isahl."zc_id_subj-position"
            WHERE deleted_at IS NULL
              AND (id::text = $1 OR code = $1 OR notice = $1)
            LIMIT 1
        ),
        employed AS (
            SELECT ea.fk_user AS uid
            FROM target t
            JOIN isahl."zc_id_subj-post_rr_employee" spre
                ON spre.ref_left = t.id AND spre.deleted_at IS NULL
            JOIN isahl."zc_id_empl-agent" ea
                ON ea.id = spre.ref_right AND ea.deleted_at IS NULL
            UNION
            SELECT en.fk_user AS uid
            FROM target t
            JOIN isahl."zc_id_subj-post_rr_employee" spre
                ON spre.ref_left = t.id AND spre.deleted_at IS NULL
            JOIN isahl."zc_id_empl-natural" en
                ON en.id = spre.ref_right AND en.deleted_at IS NULL
        ),
        members AS (
            SELECT fk_user AS uid FROM target WHERE fk_user IS NOT NULL
            UNION
            SELECT uid FROM employed WHERE uid IS NOT NULL
        )
        SELECT DISTINCT u.id
        FROM members m
        JOIN isahl_auth.auth_users u ON u.id = m.uid
        WHERE u.is_active = TRUE
        ORDER BY u.id
        LIMIT $2
        "#,
    )
    .bind(val)
    .bind(limit)
    .fetch_all(conn)
    .await
}

/// 指派型 UA 名（`ngac_user_rr_attribute` 物化成员）——legacy 角色路径。
async fn assigned_ua_members(
    conn: &mut PgConnection,
    ua_name: &str,
    limit: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT u.id FROM isahl_auth.auth_users u
           JOIN isahl_auth.ngac_user_rr_attribute rel
             ON rel.fk_user = u.id AND rel.deleted_at IS NULL
             AND (rel.expires_at IS NULL OR rel.expires_at > NOW())
           JOIN isahl_auth.ngac_user_attribute ua
             ON ua.id = rel.fk_user_attribute AND ua.deleted_at IS NULL
           WHERE ua.o_name = $1 AND u.is_active = TRUE
           ORDER BY u.id
           LIMIT $2"#,
    )
    .bind(ua_name)
    .bind(limit)
    .fetch_all(conn)
    .await
}

/// 收敛成员解析（详见模块头三路并集语义）。
///
/// - `val` 以 `position:`/`view:` 前缀 → 仅认知持有者
/// - 裸值 → 认知派生名不适用：指派 UA 成员 ∪ 岗位标识成员（id/code/notice）
///
/// 任何单路失败降级其他路（warn 由调用方语义兜底：空集 → 不投递/不建单）。
pub async fn resolve_member_user_ids(
    conn: &mut PgConnection,
    val: &str,
    limit: i64,
) -> Vec<i64> {
    if val.is_empty() {
        return Vec::new();
    }
    if val.starts_with("position:") || val.starts_with("view:") {
        return cognition_holders(conn, val).await.unwrap_or_default();
    }
    let mut acc: Vec<i64> = Vec::new();
    if let Ok(ua) = assigned_ua_members(conn, val, limit).await {
        acc.extend(ua);
    }
    if let Ok(pos) = position_members(conn, val, limit).await {
        acc.extend(pos);
    }
    acc.sort_unstable();
    acc.dedup();
    acc.truncate(limit as usize);
    acc
}
