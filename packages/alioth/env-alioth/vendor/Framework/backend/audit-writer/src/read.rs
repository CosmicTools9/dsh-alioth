//! 业务审计域读面（审计域自有查询；D4——业务数据源复用 ns 服务面，本模块只读审计族表）。

use common::AliothError;
use sqlx::PgPool;

/// 审计项目列表行。
#[derive(Debug, serde::Serialize)]
pub struct AuditProjectSummary {
    pub id: String,
    pub code: Option<String>,
    pub notice: Option<String>,
    /// 主状态名（lifecycle 桥 → `zc_id_status`；无桥 = 未启动）
    pub status: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// 受审主体行（id + 名称）。
#[derive(Debug, serde::Serialize)]
pub struct AuditeeRow {
    pub id: String,
    pub name: Option<String>,
}

/// 审计项目列表（新→旧）。
pub async fn list_audit_projects(pool: &PgPool) -> Result<Vec<AuditProjectSummary>, AliothError> {
    let rows = sqlx::query_as::<
        _,
        (
            i64,
            Option<String>,
            Option<String>,
            Option<String>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"SELECT a.id, a.code, a.notice, st.notice, a.created_at
           FROM "isahl"."zc_id_audit" a
           LEFT JOIN "isahl"."zc_id_lifecycle_r_primary-status" lrs
                  ON lrs.ref_left = a.id AND lrs.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_status" st ON st.id = lrs.ref_right
           WHERE a.deleted_at IS NULL
           ORDER BY a.created_at DESC"#,
    )
    .fetch_all(pool)
    .await
    .map_err(AliothError::from_sqlx)?;
    Ok(rows
        .into_iter()
        .map(
            |(id, code, notice, status, created_at)| AuditProjectSummary {
                id: id.to_string(),
                code,
                notice,
                status,
                created_at,
            },
        )
        .collect())
}

/// 审计项目受审主体（静态单目标 = `zc_id_subjects`）。
pub async fn audit_project_auditees(
    pool: &PgPool,
    audit_id: i64,
) -> Result<Vec<AuditeeRow>, AliothError> {
    let rows = sqlx::query_as::<_, (i64, Option<String>)>(
        r#"SELECT s.id, s.notice
           FROM "isahl"."zc_id_audit_rr_auditee" b
           JOIN "isahl".zc_id_subjects s ON s.id = b.ref_right AND s.deleted_at IS NULL
           WHERE b.ref_left = $1 AND b.deleted_at IS NULL
           ORDER BY b.id"#,
    )
    .bind(audit_id)
    .fetch_all(pool)
    .await
    .map_err(AliothError::from_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|(id, name)| AuditeeRow {
            id: id.to_string(),
            name,
        })
        .collect())
}

/// 归档导出（审计工作底稿集）：审计项目附件（`ak_attachment` → `file-document`）+
/// 来源指针（`ak_source` 指向的 `docu-accounting` 会计凭证行）。
/// 输出 CSV 文本（调用方作为 attachment 响应体）。
pub async fn archive_csv(pool: &PgPool, audit_id: i64) -> Result<String, AliothError> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        r#"SELECT kind, identifier, name, created_at::text
           FROM (
             SELECT 'attachment' AS kind, f.code AS identifier, f.notice AS name, f.created_at
             FROM "isahl"."zc_id_audit" a
             JOIN "isahl"."zc_id_file-document" f ON f.id = ANY(a.ak_attachment)
             WHERE a.id = $1 AND a.deleted_at IS NULL AND f.deleted_at IS NULL
             UNION ALL
             SELECT 'accounting-voucher' AS kind, d.code AS identifier, d.notice AS name, d.created_at
             FROM "isahl"."zc_id_audit" a
             JOIN "isahl"."zc_id_docu-accounting" d ON d.id = ANY(a.ak_source)
             WHERE a.id = $1 AND a.deleted_at IS NULL AND d.deleted_at IS NULL
           ) t
           ORDER BY created_at"#,
    )
    .bind(audit_id)
    .fetch_all(pool)
    .await
    .map_err(AliothError::from_sqlx)?;
    let mut csv = String::from("kind,identifier,name,created_at\n");
    for (kind, identifier, name, created_at) in rows {
        csv.push_str(&format!(
            "{},{},{},{}\n",
            csv_field(&kind),
            csv_field(&identifier),
            csv_field(&name),
            csv_field(&created_at.unwrap_or_default()),
        ));
    }
    Ok(csv)
}

/// 结算复盘操作行列表（`zc_id_oper-smtv_review`，新→旧；审计域自有读面）。
pub async fn list_smtv_reviews(pool: &PgPool) -> Result<Vec<serde_json::Value>, AliothError> {
    let rows = sqlx::query_as::<
        _,
        (
            i64,
            Option<String>,
            Option<String>,
            Option<String>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"SELECT o.id, o.code, o.notice, o.comments, o.created_at
           FROM "isahl"."zc_id_oper-smtv_review" o
           WHERE o.deleted_at IS NULL
           ORDER BY o.created_at DESC"#,
    )
    .fetch_all(pool)
    .await
    .map_err(AliothError::from_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|(id, code, notice, comments, created_at)| {
            serde_json::json!({
                "id": id.to_string(),
                "code": code,
                "notice": notice,
                "comments": comments,
                "createdAt": created_at.to_rfc3339(),
            })
        })
        .collect())
}

/// CSV 字段转义（含逗号/引号/换行 → 双引号包裹 + 引号翻倍）。
pub fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
