//! 业务审计域写链实现（单源；crate 级文档见 lib.rs）。

use common::AliothError as WriterError;
use sqlx::PgConnection;

/// 审计项目写输入（`zc_id_audit`）。
///
/// 内部写链输入（无 JSON 边界，对齐 consignment-writer 先例）：
/// 坐标三元组由调用方解析注入（禁硬编码 ZUID）。
#[derive(Debug, Clone, Default)]
pub struct AuditProjectInput {
    /// 审计项目编号（调用方保证唯一；建议 `AUD-{code}` 前缀）
    pub code: String,
    pub notice: String,
    /// 纯文本摘要（MUST NOT 承载结构化数据——remove-comments-json-embedding）
    pub comments: Option<String>,
    /// 发起人主体（`fk_launcher` → `zc_id_subjects`）
    pub fk_launcher: Option<i64>,
    /// 来源指针（审计依据/立项来源行 id 数组——框架来源列语义）
    pub ak_source: Option<Vec<i64>>,
    pub dk_scene: i64,
    pub dk_factor: i64,
    pub dk_function: i64,
}

/// 审计项目主档落库（`zc_id_audit`；id 省略由列默认 `gen_next_zuid()` 决定）。
pub async fn insert_audit_project_tx(
    conn: &mut PgConnection,
    input: &AuditProjectInput,
    user_id: i64,
) -> Result<i64, WriterError> {
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_audit"
           (code, notice, comments, fk_launcher, ak_source,
            dk_scene, dk_factor, dk_function, created_by_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           RETURNING id"#,
    )
    .bind(&input.code)
    .bind(&input.notice)
    .bind(&input.comments)
    .bind(input.fk_launcher)
    .bind(&input.ak_source)
    .bind(input.dk_scene)
    .bind(input.dk_factor)
    .bind(input.dk_function)
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(WriterError::from_sqlx)?;
    Ok(id)
}

/// 挂接受审主体（`zc_id_audit_rr_auditee`，静态单目标 = `zc_id_subjects`；
/// uid 段 219；NOT EXISTS 幂等——重复挂接零副作用）。
pub async fn attach_auditee_tx(
    conn: &mut PgConnection,
    audit_id: i64,
    subject_id: i64,
    user_id: i64,
) -> Result<(), WriterError> {
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_audit_rr_auditee"
           (id, code, notice, ref_left, ref_right, created_by_id)
           SELECT isahl.gen_next_uid(219), $1, $2, $3, $4, $5
           WHERE NOT EXISTS (
             SELECT 1 FROM "isahl"."zc_id_audit_rr_auditee"
             WHERE ref_left = $3 AND ref_right = $4 AND deleted_at IS NULL)"#,
    )
    .bind(format!("AAB-{audit_id}-{subject_id}"))
    .bind(format!("审计 {audit_id} 受审主体 {subject_id}"))
    .bind(audit_id)
    .bind(subject_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(WriterError::from_sqlx)?;
    Ok(())
}

/// 登记评估结论（`zc_id_audit_rr_conclusion`；uid 段 220；NOT EXISTS 幂等；
/// `conclusion_id` = `zc_id_prod-conclusion` 叶行 id——调用方先行解析）。
pub async fn set_conclusion_tx(
    conn: &mut PgConnection,
    audit_id: i64,
    conclusion_id: i64,
    user_id: i64,
) -> Result<(), WriterError> {
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_audit_rr_conclusion"
           (id, code, notice, ref_left, ref_right, created_by_id)
           SELECT isahl.gen_next_uid(220), $1, $2, $3, $4, $5
           WHERE NOT EXISTS (
             SELECT 1 FROM "isahl"."zc_id_audit_rr_conclusion"
             WHERE ref_left = $3 AND deleted_at IS NULL)"#,
    )
    .bind(format!("ACB-{audit_id}"))
    .bind(format!("审计 {audit_id} 评估结论"))
    .bind(audit_id)
    .bind(conclusion_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(WriterError::from_sqlx)?;
    Ok(())
}
