//! consignment-writer — 模型（input/context/output）

/// 写上下文：主体语义参数化（组织解析外置——调用方注入）。
///
/// - Gateway WZ（transport-dispatch）：`customer_subject_id` = 委托客户主体（req.customer_id）、
///   `operator_org_id` = 当前用户绑定运营组织（原 resolve_operator_org 结果）、`actor_user_id` = 1（原写死）。
/// - OpenActivity 门户：`customer_subject_id` = 绑定企业组织（fk_subject）、
///   `operator_org_id` = SUBJ-SYSTEM 组织锚点、`actor_user_id` = 门户用户。
#[derive(Debug, Clone, Copy)]
pub struct WriteContext {
    /// 委托方主体（fk_subject / fk_subj-demand / deta fk_counterparty）
    pub customer_subject_id: i64,
    /// 运营组织（fk_object；承运商未显式选择时的 fk_subj-provider / deta fk_biller 兜底）
    pub operator_org_id: i64,
    /// 写入人（created_by_id；dispatch 传 1 保持原行为，portal 传门户用户）
    pub actor_user_id: i64,
}

/// 创建委托输入（业务字段；不承载主体/组织——见 WriteContext）
#[derive(Debug, Clone)]
pub struct CreateConsignmentInput {
    pub traffic_line_id: i64,
    /// 起运地：zc_id_place id（按 id 直取，无效则 rr_stop 桥空，fail-visible）
    pub origin: String,
    /// 目的地：zc_id_place id
    pub dest: String,
    pub cargo_desc: String,
    pub weight_ton: f64,
    pub volume_cbm: f64,
    pub goods_tag_ids: Vec<i64>,
    pub price_amount: f64,
    /// 询盘预期单价（可选；落 REQ 诉求实例 qk_price）
    pub expected_price: Option<f64>,
    /// 预计提货（本地 +08 或 UTC，接受 "YYYY-MM-DDTHH:MM" / "YYYY-MM-DD"）
    pub pickup_time: Option<String>,
    pub eta: Option<String>,
    /// 关联合同 id（order_rr_contract 幂等桥）
    pub contract_id: Option<i64>,
    /// 车辆类型 id（ck_vehicle-form）
    pub vehicle_type_id: Option<i64>,
    /// 物流服务商（承运商主体 id；不选时默认运营组织）
    pub carrier_id: Option<i64>,
    /// 单号覆盖（拆分小委托用 `{大委托code}-S{n}`；`None` = 既有 `CNS-{ts}` 硬生成）
    pub code_override: Option<String>,
    /// 父委托 id：非空则同事务写父子桥 `zc_id_order_rr_demand`（`ref_left`=本单 / `ref_right`=父单）——
    /// 幂等（同 `(ref_left, ref_right)` 已存在即跳过）
    pub parent_consignment_id: Option<i64>,
    /// 小委托裁剪（add-inquiry-allocation-flow D6）：为真时**跳过** ① 自动 `{code}-REQ`
    /// （否则以 `fk_previous`=小委托武装 `AWARD_NOT_CONFIRMED` 发单门禁）② 下单合同对及其一式两份镜像
    /// （合同足迹已由选商矩阵 #5 落库，不重复建）③ 运力占用 `ORD-INV` 与 com-voucher 扣可售
    /// （同一批货量大委托下单时已扣，不重复扣）。**保留** orde-land 主档、订单产品对
    /// （`fk_previous`=本单）、起讫 `rr_stop×2`、重量/金额标量与 `deta-trade_order` 明细。
    pub sub_consignment: bool,
}

/// 创建委托输出（id 裸 i64；调用方自行 serde_zuid 字符串化）
#[derive(Debug, Clone)]
pub struct CreateConsignmentOutput {
    pub consignment_id: i64,
    pub product_id: i64,
    pub loading_product_id: i64,
    /// 运力占用行 id（`production_rr_storage` ORD-INV-*）；**0 = 小委托裁剪未写占用**
    /// （`sub_consignment=true`，同一批货量大委托下单时已扣，不重复扣）
    pub inventory_id: i64,
    pub code: String,
}
