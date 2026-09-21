//! 售后段写链实现（单源；crate 级文档见 lib.rs）。

use common::AliothError as WriterError;
use sqlx::PgConnection;

/// 售后订单写输入（`zc_id_order-after_sales` 子叶）。
///
/// 内部写链输入（无 JSON 边界，对齐 `consignment-writer::CreateConsignmentInput` 先例）：
/// 标量列（`qk_date`）与类目（`ck_category` → `zc_id_cate-ope-title`）由调用方先行解析为
/// 行 id 后注入；坐标三元组同理由调用方按 ns service.json 声明解析（禁硬编码 ZUID）。
#[derive(Debug, Clone, Default)]
pub struct AfterSalesOrderInput {
    /// 售后单编号（调用方保证唯一；建议 `AS-{code}` 前缀约定，design D4）
    pub code: String,
    pub notice: String,
    /// 纯文本摘要（MUST NOT 承载结构化数据——remove-comments-json-embedding）
    pub comments: Option<String>,
    /// 售后发起方主体（绑合同时须为合同方，G3 口径）
    pub fk_subject: Option<i64>,
    /// 被服务/被诉方主体
    pub fk_object: Option<i64>,
    /// 售后日期（`zc_id_scal-date` 行 id，调用方先行解析）
    pub qk_date: Option<i64>,
    /// 售后类型类目（`zc_id_cate-ope-title` 行 id）
    pub ck_category: Option<i64>,
    /// 前因合约（可选；提供时走 G3 校验 + 幂等桥双写）
    pub fk_contract: Option<i64>,
    /// 来源指针（申诉/事件行 id 数组——框架来源列语义，D3）
    pub ak_source: Option<Vec<i64>>,
    pub dk_scene: i64,
    pub dk_factor: i64,
    pub dk_function: i64,
}

/// 售后申诉写输入（`zc_id_stat-appeal`）。
#[derive(Debug, Clone, Default)]
pub struct AppealInput {
    pub code: String,
    pub notice: String,
    /// 纯文本摘要（MUST NOT 承载结构化数据）
    pub comments: Option<String>,
    /// 申诉人主体
    pub fk_subject: Option<i64>,
    /// 被诉方主体
    pub fk_object: Option<i64>,
    /// 申诉日期（`zc_id_scal-date` 行 id）
    pub qk_date: Option<i64>,
    /// 来源事件指针（`even-accident`/`even-alert`/`even-report` 行 id 数组，D3）
    pub ak_source: Option<Vec<i64>>,
    pub dk_scene: i64,
    pub dk_factor: i64,
    pub dk_function: i64,
}

/// 售后订单落子叶 + 可选合同幂等桥（G5 双写口径）。
///
/// Step 0：`fk_contract` 提供时执行 G3 前置校验（复用
/// [`consignment_writer::ensure_order_contract_valid_tx`] 单源：存在 /
/// 状态 ∈ {active, executing}（无桥 legacy 放行）/ `fk_subject` ∈ 合同方）；
/// 绑合同但缺 `fk_subject` 直接 400（无主体无从校验合同方）。
/// Step 1：子叶主行（`gen_next_zuid`）。
/// Step 2：`zc_id_order_rr_contract` 幂等桥（`gen_next_uid(470)`，NOT EXISTS）。
pub async fn insert_after_sales_order_tx(
    conn: &mut PgConnection,
    input: &AfterSalesOrderInput,
    user_id: i64,
) -> Result<i64, WriterError> {
    if let Some(contract_id) = input.fk_contract {
        let subject_id = input.fk_subject.ok_or_else(|| {
            WriterError::BadRequest(
                "绑合同的售后单必须提供 fk_subject（合同方校验依据）".to_string(),
            )
        })?;
        consignment_writer::ensure_order_contract_valid_tx(conn, contract_id, subject_id).await?;
    }

    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_order-after_sales"
           (id, code, notice, comments, fk_subject, fk_object, qk_date, ck_category,
            fk_contract, ak_source, dk_scene, dk_factor, dk_function, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9,
                   $10, $11, $12, $13)
           RETURNING id"#,
    )
    .bind(&input.code)
    .bind(&input.notice)
    .bind(&input.comments)
    .bind(input.fk_subject)
    .bind(input.fk_object)
    .bind(input.qk_date)
    .bind(input.ck_category)
    .bind(input.fk_contract)
    .bind(&input.ak_source)
    .bind(input.dk_scene)
    .bind(input.dk_factor)
    .bind(input.dk_function)
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(WriterError::from_sqlx)?;

    if let Some(contract_id) = input.fk_contract {
        bind_after_sales_contract_tx(conn, id, &input.code, contract_id, user_id).await?;
    }
    Ok(id)
}

/// 售后订单 ↔ 前因合约幂等桥（独立入口：补挂/重挂安全，对齐
/// `consignment-writer` ORC- 桥口径）。
pub async fn bind_after_sales_contract_tx(
    conn: &mut PgConnection,
    order_id: i64,
    order_code: &str,
    contract_id: i64,
    user_id: i64,
) -> Result<(), WriterError> {
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_order_rr_contract"
           (id, code, notice, ref_left, ref_right, created_by_id)
           SELECT isahl.gen_next_uid(470), $1, $2, $3, $4, $5
           WHERE NOT EXISTS (
             SELECT 1 FROM "isahl"."zc_id_order_rr_contract"
             WHERE ref_left = $3 AND ref_right = $4 AND deleted_at IS NULL)"#,
    )
    .bind(format!("ORC-{order_code}"))
    .bind(format!("{order_code} 售后合同挂接"))
    .bind(order_id)
    .bind(contract_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(WriterError::from_sqlx)?;
    Ok(())
}

/// 售后申诉落 `zc_id_stat-appeal`（来源事件指针走 `ak_source`，D3）。
pub async fn insert_appeal_tx(
    conn: &mut PgConnection,
    input: &AppealInput,
    user_id: i64,
) -> Result<i64, WriterError> {
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_stat-appeal"
           (id, code, notice, comments, fk_subject, fk_object, qk_date, ak_source,
            dk_scene, dk_factor, dk_function, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7,
                   $8, $9, $10, $11)
           RETURNING id"#,
    )
    .bind(&input.code)
    .bind(&input.notice)
    .bind(&input.comments)
    .bind(input.fk_subject)
    .bind(input.fk_object)
    .bind(input.qk_date)
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
