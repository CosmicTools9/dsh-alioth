//! NGAC 策略变更审计写入（change `add-ngac-audit-trail-view` D1）。
//!
//! 语义：
//! - **确认式落库**：审计行与业务写入在**同一事务**内 INSERT——失败即整体
//!   回滚（策略变更低频高影响，不接受 fire-and-forget 丢失窗口；见
//!   SECURITY_SPEC §11.2 补偿指引的正面实例）。
//! - **取值集锁定**（spec `policy-change-audit-wiring` Requirement）：
//!   `action` ∈ `insert`/`update`/`delete`；`entity_type` ∈
//!   `association`/`prohibition`/`user_attribute`/`object_attribute`/`user_assignment`。
//! - **无变更不留痕**：`ON CONFLICT DO NOTHING` 等 rows_affected=0 场景
//!   由调用方判断，不调用本模块。
//! - old/new 镜像：行级 JSONB（`to_jsonb(row)`），delete 填 old、insert 填
//!   new、update 双填。
//!
//! 已知盲区（design D1 声明）：LDAP/SCIM 系统同步写路径不在本模块覆盖内，
//! 另立 change 评估。

use sqlx::PgConnection;

/// 一条策略变更审计记录。
pub struct AuditRecord {
    /// `insert` / `update` / `delete`
    pub action: &'static str,
    /// `association` / `prohibition` / `user_attribute` / `object_attribute` / `user_assignment`
    pub entity_type: &'static str,
    pub entity_id: i64,
    pub old_values: Option<serde_json::Value>,
    pub new_values: Option<serde_json::Value>,
    /// 操作管理员 id（require_admin 解析结果）
    pub actor: i64,
    /// JWT `sid`（会话绑定；无会话令牌 → None）
    pub session_id: Option<String>,
    pub ip_address: Option<String>,
}

/// 在既有事务内写入审计行（同事务语义：调用方持有 tx，失败随事务回滚）。
pub async fn write_audit_tx(tx: &mut PgConnection, rec: &AuditRecord) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO isahl_auth.ngac_policy_audit_log
            (action, entity_type, entity_id, old_values, new_values, fk_user, session_id, ip_address)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8::inet)
        "#,
    )
    .bind(rec.action)
    .bind(rec.entity_type)
    .bind(rec.entity_id)
    .bind(&rec.old_values)
    .bind(&rec.new_values)
    .bind(rec.actor)
    .bind(&rec.session_id)
    .bind(&rec.ip_address)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// 取行的 JSONB 镜像（`to_jsonb(t)`，审计 old/new 值用）。
/// `table` 仅允许白名单字面量（调用方硬编码），防注入。
pub async fn row_mirror_tx(
    tx: &mut PgConnection,
    table: &str,
    id: i64,
) -> sqlx::Result<Option<serde_json::Value>> {
    debug_assert!(
        matches!(
            table,
            "ngac_user_attribute"
                | "ngac_object_attribute"
                | "ngac_association"
                | "ngac_prohibition"
                | "ngac_user_rr_attribute"
        ),
        "row_mirror_tx: table not in whitelist: {}",
        table
    );
    let sql = format!(
        "SELECT to_jsonb(t) FROM (SELECT * FROM isahl_auth.{} WHERE id = $1) t",
        table
    );
    // 表名硬编码白名单（上方 debug_assert 校验），无用户输入拼接
    sqlx::query_scalar::<_, serde_json::Value>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
}

/// user_rr 绑定行的复合键镜像（无单 id 主键语义时用 (fk_user, fk_user_attribute) 定位）。
pub async fn user_rr_mirror_tx(
    tx: &mut PgConnection,
    fk_user: i64,
    fk_user_attribute: i64,
) -> sqlx::Result<Option<serde_json::Value>> {
    sqlx::query_scalar(
        "SELECT to_jsonb(t) FROM (SELECT * FROM isahl_auth.ngac_user_rr_attribute \
         WHERE fk_user = $1 AND fk_user_attribute = $2) t",
    )
    .bind(fk_user)
    .bind(fk_user_attribute)
    .fetch_optional(&mut *tx)
    .await
}

/// 从请求提取客户端 IP（审计 ip_address 列；尽力而为，缺省 None）。
pub fn client_ip(req: &actix_web::HttpRequest) -> Option<String> {
    req.peer_addr().map(|a| a.ip().to_string())
}
