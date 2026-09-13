//! operator_org — 运营组织主体解析**唯一实现**。
//!
//! 写操作门禁：当前用户 → 运营组织主体（`zc_id_subjects`）解析，供各 Service
//! 写链注入「我方主体」（产品属权 `fk_subj-provider`/`fk_subj-demand`、订单双层双方等）。
//! 消费方 MUST 调用本函数，**禁止复制 SQL**。
//!
//! 优先级：
//! 1. `auth_users.entity_id`（登录后绑定运营组织主体）→ 直接返回；
//! 2. 特权 UA（admin/auditor/operator/enterprise）未绑定时回退系统主体
//!    `zc_id_subjects.code='SUBJ-SYSTEM'`（存量特权用户豁免，不误阻断）；
//! 3. 非特权未绑定 → 业务错误 `OPERATOR_ORG_UNBOUND`（写操作门禁，422 语义，
//!    由 handler 层映射；AliothError 无 Unprocessable 变体，走 Validation）。
use crate::AliothError;

pub async fn resolve_operator_org(
    conn: &mut sqlx::PgConnection,
    user_id: i64,
) -> Result<i64, AliothError> {
    let bound: Option<i64> = sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT entity_id
           FROM isahl_auth.auth_users
           WHERE id = $1 AND entity_id IS NOT NULL"#,
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?
    .flatten();
    if let Some(b) = bound {
        return Ok(b);
    }
    let privileged: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
               SELECT 1 FROM isahl_auth.ngac_user_rr_attribute rr
               JOIN isahl_auth.ngac_user_attribute ua ON ua.id = rr.fk_user_attribute
               WHERE rr.fk_user = $1 AND rr.deleted_at IS NULL AND ua.deleted_at IS NULL
                 AND ua.o_name IN ('admin','auditor','operator','enterprise')
           )"#,
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if privileged {
        let fallback: Option<i64> = sqlx::query_scalar::<_, Option<i64>>(
            r#"SELECT id FROM "isahl"."zc_id_subjects"
               WHERE code = 'SUBJ-SYSTEM' AND deleted_at IS NULL
               LIMIT 1"#,
        )
        .fetch_optional(&mut *conn)
        .await?
        .flatten();
        if let Some(b) = fallback {
            return Ok(b);
        }
        return Err(AliothError::Validation {
            field: "operator_org".into(),
            message: "OPERATOR_ORG_UNBOUND: 特权用户未绑定运营组织且系统主体 SUBJ-SYSTEM 缺失"
                .into(),
        });
    }
    Err(AliothError::Validation {
        field: "operator_org".into(),
        message: "OPERATOR_ORG_UNBOUND: 当前用户未绑定运营组织，无法执行订单创建类操作".into(),
    })
}
