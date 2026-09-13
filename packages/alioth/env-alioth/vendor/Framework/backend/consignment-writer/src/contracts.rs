//! 下单合同对写件（用户裁决 2026-09-11）：委托创建同事务生成
//! **诉求合同**（`zc_id_cont-request`）+ **销售合同**（`zc_id_cont-sales`）及其一式两份镜像，
//! 并挂 `zc_id_order_rr_contract`（订单 ↔ 合同）桥。
//!
//! 语义（七链矩阵 #2「运营代客户下单」）：
//! - 诉求合同 = 客户诉求的合同化（`CT-{委托code}-REQ`，甲 = 客户、乙 = 平台运营组织）；
//!   镜像落**同表**（诉求无相反方向；合同方与主合同**逐字段相同，甲/乙不互换**）。
//! - 销售合同 = 平台对客户的销售（`CT-{委托code}`）；镜像落**相反**叶表（采购），合同方与主合同相同。
//! - `_f_`/`_t_` 由 `contract-writer` 经 `trigger_registry::lifecycle::derive_form_type`
//!   从职能码派生后参数绑定（单一派生源，禁字面量对）。
//! - 桥 `zc_id_order_rr_contract` 逐合同一行（`ref_left` = 订单、`ref_right` = 合同），
//!   `(ref_left, ref_right)` 幂等；`code = ORC-{委托code}-{n}`。
//!
//! 边界：只承载合同写件与桥；合同驱动运输产品、状态桥、行级 NGAC 注册留在调用方。

use sqlx::PgConnection;

use common::AliothError;
use contract_writer::{insert_contract_pair_tx, ContractLeaf, ContractParty, ContractRowInput};

/// 下单合同对入参（主体按委托口径给定：甲 = 客户、乙 = 平台运营组织）。
#[derive(Debug, Clone)]
pub struct OrderContractsInput<'a> {
    /// 委托编号（合同编号与桥编号的公共前缀）
    pub consign_code: &'a str,
    /// 甲：客户主体（委托 `fk_subject`）
    pub buyer: i64,
    /// 乙：平台运营组织（委托 `fk_object`）
    pub seller: i64,
    /// 形态派生源（下单链用 `↓.GG` = 实现·范例）
    pub fn_code: &'a str,
    pub user_id: i64,
}

/// 下单合同对（诉求合同 + 销售合同 及其各自一式两份镜像）并挂 `zc_id_order_rr_contract` 桥。
///
/// 返回 `(诉求主, 诉求镜像, 销售主, 销售镜像)`。
pub async fn insert_order_contracts_tx(
    conn: &mut PgConnection,
    order_id: i64,
    input: &OrderContractsInput<'_>,
) -> Result<(i64, i64, i64, i64), AliothError> {
    // 甲乙顺序即 P1/P2；镜像合同方与主合同逐字段相同（`insert_contract_pair_tx` 不互换）
    let parties = vec![
        ContractParty {
            subject_id: Some(input.buyer),
            name: "甲方".into(),
            period_id: None,
        },
        ContractParty {
            subject_id: Some(input.seller),
            name: "乙方".into(),
            period_id: None,
        },
    ];

    // 诉求合同（`zc_id_cont-request`）：主 + 镜像同表
    let request_code = format!("CT-{}-REQ", input.consign_code);
    let request_notice = format!("{} 客户诉求合同", input.consign_code);
    let (request_id, request_mirror_id) = insert_contract_pair_tx(
        conn,
        &ContractRowInput {
            leaf: ContractLeaf::Request,
            code: &request_code,
            notice: &request_notice,
            comments: "委托下单自动生成",
            parties: parties.clone(),
            fn_code: input.fn_code,
            scene_code: "TD",
            factor_code: "FJA",
            qk_date_id: None,
            qk_valid_segm_id: None,
            o_number: None,
            projection: None,
            tpl_id: None,
            lk_health: None,
            draft_status_id: None,
            user_id: input.user_id,
        },
    )
    .await?;

    // 销售合同（`zc_id_cont-sales`，镜像落相反叶表 `cont-purchase`）
    let sales_code = format!("CT-{}", input.consign_code);
    let sales_notice = format!("{} 销售合同", input.consign_code);
    let (sales_id, sales_mirror_id) = insert_contract_pair_tx(
        conn,
        &ContractRowInput {
            leaf: ContractLeaf::Sales,
            code: &sales_code,
            notice: &sales_notice,
            comments: "委托下单自动生成",
            parties,
            fn_code: input.fn_code,
            scene_code: "TD",
            factor_code: "FJA",
            qk_date_id: None,
            qk_valid_segm_id: None,
            o_number: None,
            projection: None,
            tpl_id: None,
            lk_health: None,
            draft_status_id: None,
            user_id: input.user_id,
        },
    )
    .await?;

    // 订单 ↔ 合同桥：四张合同各一行（`(ref_left, ref_right)` 幂等；重放不重复挂接）
    let bridge_notice = format!("{} 下单合同", input.consign_code);
    for (idx, contract_id) in [request_id, request_mirror_id, sales_id, sales_mirror_id]
        .into_iter()
        .enumerate()
    {
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_order_rr_contract"
               (id, code, notice, ref_left, ref_right, created_by_id)
               SELECT isahl.gen_next_zuid(), $1, $2, $3, $4, $5
               WHERE NOT EXISTS (
                 SELECT 1 FROM "isahl"."zc_id_order_rr_contract"
                 WHERE ref_left = $3 AND ref_right = $4 AND deleted_at IS NULL)"#,
        )
        .bind(format!("ORC-{}-{}", input.consign_code, idx + 1))
        .bind(&bridge_notice)
        .bind(order_id)
        .bind(contract_id)
        .bind(input.user_id)
        .execute(&mut *conn)
        .await
        .map_err(AliothError::from_sqlx)?;
    }

    Ok((request_id, request_mirror_id, sales_id, sales_mirror_id))
}
