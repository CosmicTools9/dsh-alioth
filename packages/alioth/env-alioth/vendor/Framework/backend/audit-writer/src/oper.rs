//! 审计操作行（oper 族）——账单确认（`zc_id_oper-confirm_bill`）、结算复盘
//! （`zc_id_oper-smtv_review`）与证后监督审计桥接（`zc_id_oper-audit_prj`）。
//! change wire-business-audit-domain 组 3/4（D5/D2）。
//!
//! - **id 口径**：三表 id 列默认 `gen_next_zuid()`——INSERT 省略 id（db-uid-defaults 首选）。
//! - **坐标**：JC/FTA/↓_EZ（管理·审批处理·操作族），`resolve_conn` 事务内解析
//!   （对齐 approval::initiate 的 oper-approve 先例）；解析失败即失败（fail-closed）。
//! - **挂钩语义（D5）**：核销方调用**在核销事务边界之外** best-effort 落
//!   `confirm_bill` 行（失败 warn 不阻断）；`smtv_review` 为独立复盘操作。

use common::AliothError as WriterError;
use sqlx::PgConnection;

/// 操作行写输入（confirm_bill / smtv_review 共用形态；无 JSON 边界）。
#[derive(Debug, Clone, Default)]
pub struct OperationRowInput {
    /// 操作编号（调用方约定：`CFB-PM-{match_id}` / `CFB-RM-{match_id}` / `SRV-{code}`）
    pub code: String,
    pub notice: String,
    /// 纯文本摘要（MUST NOT 承载结构化数据）
    pub comments: Option<String>,
    /// 关联主体（受审方/核销对方；审计视角聚合锚点）
    pub fk_subject: Option<i64>,
    /// 操作人（user id）
    pub fk_operator: Option<i64>,
}

async fn insert_oper_row_tx(
    conn: &mut PgConnection,
    table: &str,
    input: &OperationRowInput,
    user_id: i64,
) -> Result<i64, WriterError> {
    // 白名单三表（表名来自本 crate 常量面，非调用方输入——动态表名门禁豁免口径）
    let sql = match table {
        "zc_id_oper-confirm_bill" => {
            r#"INSERT INTO "isahl"."zc_id_oper-confirm_bill"
               (code, notice, comments, fk_subject, fk_operator, dk_scene, dk_factor, dk_function, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING id"#
        }
        "zc_id_oper-smtv_review" => {
            r#"INSERT INTO "isahl"."zc_id_oper-smtv_review"
               (code, notice, comments, fk_subject, fk_operator, dk_scene, dk_factor, dk_function, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING id"#
        }
        "zc_id_oper-audit_prj" => {
            r#"INSERT INTO "isahl"."zc_id_oper-audit_prj"
               (code, notice, comments, fk_subject, fk_operator, dk_scene, dk_factor, dk_function, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING id"#
        }
        _ => return Err(WriterError::BadRequest("未知操作行表".to_string())),
    };
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve_conn(&mut *conn, ("JC", "FTA", "↓_EZ"))
            .await
            .map_err(WriterError::from_sqlx)?;
    let id: i64 = sqlx::query_scalar(sql)
        .bind(&input.code)
        .bind(&input.notice)
        .bind(&input.comments)
        .bind(input.fk_subject)
        .bind(input.fk_operator)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(WriterError::from_sqlx)?;
    Ok(id)
}

/// 账单确认操作行（核销留痕；调用方在核销事务边界之外 best-effort 落行）。
pub async fn insert_confirm_bill_tx(
    conn: &mut PgConnection,
    input: &OperationRowInput,
    user_id: i64,
) -> Result<i64, WriterError> {
    insert_oper_row_tx(conn, "zc_id_oper-confirm_bill", input, user_id).await
}

/// 结算复盘操作行（独立复盘操作）。
pub async fn insert_smtv_review_tx(
    conn: &mut PgConnection,
    input: &OperationRowInput,
    user_id: i64,
) -> Result<i64, WriterError> {
    insert_oper_row_tx(conn, "zc_id_oper-smtv_review", input, user_id).await
}

/// 双轨桥接（change 组 4.1，D2 修正口径）：把既有证后监督审计行
/// （`zc_id_even-approve`，AVIC 实现保留不迁移）挂接为审计项目的关联面——
/// ① `zc_id_oper-audit_prj` 项目审计操作行（`ak_source` 来源指针承载 even-approve 行，
/// 框架列窄化承载，先例 = after-sales D3）；② 审计项目 `ak_source` 追加同指针
/// （审计读侧可达）。幂等：操作行 code 唯一约定；已桥接返回 0（no-op）。
pub async fn link_supervision_audit_tx(
    conn: &mut PgConnection,
    audit_id: i64,
    approve_row_id: i64,
    user_id: i64,
) -> Result<i64, WriterError> {
    let link_code = format!("EXT-{audit_id}-{approve_row_id}");
    let exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (SELECT 1 FROM "isahl"."zc_id_oper-audit_prj"
           WHERE code = $1 AND deleted_at IS NULL)"#,
    )
    .bind(&link_code)
    .fetch_optional(&mut *conn)
    .await
    .map_err(WriterError::from_sqlx)?
    .unwrap_or(false);
    if exists {
        return Ok(0);
    }
    let input = OperationRowInput {
        code: link_code,
        notice: "证后监督审计挂接".to_string(),
        comments: Some(format!(
            "关联审计项目 {audit_id} ↔ 证后监督审计行 {approve_row_id}"
        )),
        fk_subject: None,
        fk_operator: Some(user_id),
    };
    let oper_id = insert_oper_row_tx(conn, "zc_id_oper-audit_prj", &input, user_id).await?;
    // 操作行自身 ak_source 落来源指针（even-approve 行——框架来源列承载）
    sqlx::query(
        r#"UPDATE "isahl"."zc_id_oper-audit_prj" SET ak_source = ARRAY[$2::bigint] WHERE id = $1"#,
    )
    .bind(oper_id)
    .bind(approve_row_id)
    .execute(&mut *conn)
    .await
    .map_err(WriterError::from_sqlx)?;
    // 审计项目 ak_source 追加（去重）
    sqlx::query(
        r#"UPDATE "isahl"."zc_id_audit"
           SET ak_source = ARRAY(SELECT DISTINCT unnest(ak_source || ARRAY[$2::bigint]))
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(audit_id)
    .bind(approve_row_id)
    .execute(&mut *conn)
    .await
    .map_err(WriterError::from_sqlx)?;
    Ok(oper_id)
}
