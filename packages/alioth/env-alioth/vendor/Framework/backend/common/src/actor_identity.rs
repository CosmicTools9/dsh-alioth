//! actor_identity — 单据当事人「我」的**单一解析入口**。
//!
//! 用户裁决（2026-09-14）：销售合同乙方 = 「我」、采购合同甲方 = 「我」；「我」按**登录账号**判定，
//! 链路为「账号 → 绑定主体 → 岗位 → 岗位视角」；平台侧锚 = **system 哨兵（id=1）绑定主体**
//! （用户裁决 2026-09-14「system 绑定的主体是易运」；判据 = 名称含「物产中大」，WZ = `90R0` 易运；
//! MUST NOT 用系统占位 `SUBJ-SYSTEM` 作运营组织槽——见 `WZ_EXTERNAL_CHAIN_SPEC` §7 2026-09-14）；
//! 视角真源 = 本体字典 `zc_id_tags-post_view`（门户 `external_authorizations.role_kind` 只做门禁）。
//!
//! 三段链（第 0 步为占位主体规范化，见 [`normalize_business_subject`]）：
//! 0. 账号绑定主体若为**占位/系统主体**（`SUBJ-ISAH-ADMIN` / `SUBJ-SYSTEM` / `POS-SYSTEM-ADMIN`
//!    或 `notice = 'isahl 管理员'`）→ 归到 WZ 运营主体（名称含「物产中大」，即 `90R0` 易运），
//!    使「我」恒为**业务主体**而非种子占位账号；判据同 `WZ_EXTERNAL_CHAIN_SPEC` §7；
//! 1. `isahl_auth.auth_users.entity_id` —— 账号绑定主体（缺 → `OPERATOR_ORG_UNBOUND`，**无特权兜底**）；
//! 2. 岗位 —— `zc_id_subj-org_rr_position.ref_left = 主体` ∪ `zc_id_subj-post_rr_view.ref_right = 主体`
//!    ∪ `zc_id_subj-post_rr_employee.ref_right = 主体`（**任职桥**：雇员账号经此拿到其任职岗位）
//!    三支桥取并集（历史建档多桥并存，取并集兼容）；
//! 3. 视角 —— **该主体名下**的视角关联行（`zc_id_subj-post_rr_view`：`ref_left`=岗位、`ref_right`=被看待主体；
//!    取 `ref_right` = 本主体——视角集描述**被看待的主体**，岗位名下其他主体的关联行标签 MUST NOT 并入）
//!    经 `zc_id_relation-post_view_r_tags`（`ref_left` = 关联行 id）→ `zc_id_tags-post_view.code` 去重。
//!
//! 视角为空**不是**错误（主体可能只做非平台侧身份）；是否需要某个视角由调用方按链声明
//! （如平台侧单据要求 [`VIEW_BUSINESS`]）。消费方 MUST 调用本函数，**禁止复制 SQL**。
use sqlx::PgConnection;

use crate::AliothError;

/// 平台/业务主体视角（`zc_id_tags-post_view.code`，模型级种子单一供给）。
pub const VIEW_BUSINESS: &str = "VIEW-BIZ";

/// 「我」的三段解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorIdentity {
    /// 「我」= 账号绑定主体 id
    pub subject_id: i64,
    /// 该主体名下岗位 id（两桥并集，升序去重）
    pub position_ids: Vec<i64>,
    /// 岗位视角标签 code（我的岗位名下**视角关联行**的标签并集，升序去重）
    pub view_tags: Vec<String>,
}

impl ActorIdentity {
    /// 是否持有某视角标签（平台侧单据用 [`VIEW_BUSINESS`]）。
    pub fn has_view(&self, code: &str) -> bool {
        self.view_tags.iter().any(|t| t == code)
    }
}

/// 三段链解析「我」。账号未绑定主体 → `OPERATOR_ORG_UNBOUND`（保留既有错误契约；
/// **特权角色不再兜底**，见 `wz-trade-chain-ownership::dual-layer-order-parties`）。
pub async fn resolve_actor_identity(
    conn: &mut PgConnection,
    user_id: i64,
) -> Result<ActorIdentity, AliothError> {
    let subject_id: Option<i64> = sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT entity_id
           FROM isahl_auth.auth_users
           WHERE id = $1 AND entity_id IS NOT NULL"#,
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?
    .flatten();
    let Some(subject_id) = subject_id else {
        return Err(AliothError::Validation {
            field: "actor".into(),
            message: "OPERATOR_ORG_UNBOUND: 当前账号未绑定主体，无法确定单据中的「我」".into(),
        });
    };

    // 第 0 步：占位/系统主体 → 业务主体。MUST 在岗位/视角两跳**之前**施加——否则岗位与视角
    // 会锚在占位主体上（种子回退主体无名下岗位），最终返回的 `subject_id` 也会是占位主体。
    // 规范化幂等（非占位主体原样返回），故返回值即业务主体，无需两跳后再来一次。
    let subject_id = normalize_business_subject(&mut *conn, subject_id).await?;

    actor_identity_of_subject(&mut *conn, subject_id).await
}

/// 占位/系统主体 → WZ 运营主体规范化（**单一实现**）。
///
/// 种子脚本对「无法人主体」的账号（如 dev 默认登录 `isahl`）回退绑定到系统占位主体
/// `SUBJ-ISAH-ADMIN`（名称/`notice` = 「isahl 管理员」）。占位主体 MUST NOT 流入业务写链
/// （合同方、委托 B、产品属权、当事人「我」槽位）——平台侧业务主体恒为 WZ 运营主体
/// （名称含「物产中大」，`90R0` 物产中大易运科技（浙江）有限公司）；`90R0` 不可得才回落
/// `SUBJ-SYSTEM`（`WZ_EXTERNAL_CHAIN_SPEC` §7，2026-09-14 裁决）。
///
/// 主体按 **code 查 id**（禁硬编码 ZUID）；非占位主体原样返回（幂等）。
pub async fn normalize_business_subject(
    conn: &mut PgConnection,
    subject_id: i64,
) -> Result<i64, AliothError> {
    // 判据本身 = `common::not_placeholder_subject!` 的补集（**单一实现**，勿在本函数重抄 code 集）
    let is_placeholder: bool = sqlx::query_scalar(concat!(
        "SELECT EXISTS (SELECT 1 FROM \"isahl\".\"zc_id_subjects\" s ",
        "WHERE s.id = $1 AND s.deleted_at IS NULL AND NOT (",
        crate::not_placeholder_subject!(s),
        "))"
    ))
    .bind(subject_id)
    .fetch_one(&mut *conn)
    .await?;
    if !is_placeholder {
        return Ok(subject_id);
    }
    let ops: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM "isahl"."zc_id_subjects"
           WHERE code = '90R0' AND deleted_at IS NULL
           LIMIT 1"#,
    )
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(ops) = ops {
        return Ok(ops);
    }
    let fallback: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM "isahl"."zc_id_subjects"
           WHERE code = 'SUBJ-SYSTEM' AND deleted_at IS NULL
           LIMIT 1"#,
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(fallback.unwrap_or(subject_id))
}

/// 显式主体的岗位视角解析（同一三段链的后两跳）。
///
/// 供「我」主体已由当事人行确定、无需回推账号的调用方复用（询价/报价/选商按合同方主体派生），
/// 保证与 [`resolve_actor_identity`] 同一真源（禁第二份 SQL）。
pub async fn actor_identity_of_subject(
    conn: &mut PgConnection,
    subject_id: i64,
) -> Result<ActorIdentity, AliothError> {
    let position_ids: Vec<i64> = sqlx::query_scalar(
        r#"SELECT DISTINCT p.id FROM (
               SELECT ref_right AS id FROM "isahl"."zc_id_subj-org_rr_position"
                WHERE ref_left = $1 AND deleted_at IS NULL
               UNION
               SELECT ref_left AS id FROM "isahl"."zc_id_subj-post_rr_view"
                WHERE ref_right = $1 AND deleted_at IS NULL
               UNION
               SELECT ref_left AS id FROM "isahl"."zc_id_subj-post_rr_employee"
                WHERE ref_right = $1 AND deleted_at IS NULL
           ) p ORDER BY p.id"#,
    )
    .bind(subject_id)
    .fetch_all(&mut *conn)
    .await?;

    let view_tags: Vec<String> = if position_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar(
            r#"SELECT DISTINCT t.code
                 FROM "isahl"."zc_id_subj-post_rr_view" v
                 JOIN "isahl"."zc_id_relation-post_view_r_tags" rt
                   ON rt.ref_left = v.id AND rt.deleted_at IS NULL
                 JOIN "isahl"."zc_id_tags-post_view" t
                   ON t.id = rt.ref_right AND t.deleted_at IS NULL
                WHERE v.deleted_at IS NULL AND v.ref_left = ANY($1) AND v.ref_right = $2
                  AND t.code IS NOT NULL
                ORDER BY t.code"#,
        )
        .bind(&position_ids)
        .bind(subject_id)
        .fetch_all(&mut *conn)
        .await?
    };

    Ok(ActorIdentity {
        subject_id,
        position_ids,
        view_tags,
    })
}
