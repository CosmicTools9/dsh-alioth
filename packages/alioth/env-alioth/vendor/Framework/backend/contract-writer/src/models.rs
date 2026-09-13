//! 写件入参模型（不含 SQL）。

/// 买卖主体对（甲 = 买方 / 乙 = 卖方）。
///
/// 产品行按方向取用：`fk_subj-demand` ← 买方、`fk_subj-provider` ← 卖方；
/// 合约行按 `ContractParty` 顺序定 P1/P2。
///
/// 一式两份镜像**不互换**该主体对（用户裁决 2026-09-13）：镜像 = 同一单据的对方账副本，
/// 双方账本内容一致，双边性由镜像行属权表达。
#[derive(Debug, Clone, Copy)]
pub struct DocParties {
    pub buyer: i64,
    pub seller: i64,
}

/// 合同方行（顺序即 `code` 序号：`P1` 甲 / `P2` 乙 / `P3` 结算方…）。
#[derive(Debug, Clone)]
pub struct ContractParty {
    /// 主体 id（`ref_right`；缺省 = 未绑主体，仅登记名称）
    pub subject_id: Option<i64>,
    pub name: String,
    /// 生效周期（`qk_period` → `zc_id_segm-date`）
    pub period_id: Option<i64>,
}

/// 合约叶表（`zc_id_contract` 继承族的三个可写叶）。
///
/// 一式两份镜像的叶表规则（用户裁决 2026-09-11）：销售 ↔ 采购落**相反**叶表；
/// 诉求（Request）无相反方向，镜像落**同表**。镜像是同一单据的对方账副本——
/// 合同方/角色/结算方与主行一致（**甲/乙不互换**，2026-09-13 裁决），双边性由行属权表达。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractLeaf {
    /// `zc_id_cont-sales`
    Sales,
    /// `zc_id_cont-purchase`
    Purchase,
    /// `zc_id_cont-request`
    Request,
}

impl ContractLeaf {
    /// 销售向（`zc_id_cont-sales`）——其余两叶均为非销售向。
    pub fn is_sales(self) -> bool {
        matches!(self, ContractLeaf::Sales)
    }

    /// 一式两份镜像叶：Sales ↔ Purchase（相反叶表）；Request → Request（同表）。
    pub fn mirrored(self) -> Self {
        match self {
            ContractLeaf::Sales => ContractLeaf::Purchase,
            ContractLeaf::Purchase => ContractLeaf::Sales,
            ContractLeaf::Request => ContractLeaf::Request,
        }
    }

    /// 叶表名（INSERT 目标；读径按此识别）。
    pub fn table(self) -> &'static str {
        match self {
            ContractLeaf::Sales => "zc_id_cont-sales",
            ContractLeaf::Purchase => "zc_id_cont-purchase",
            ContractLeaf::Request => "zc_id_cont-request",
        }
    }
}

/// 单张合约行写件入参（主行与镜像行同构；叶表由 `leaf` 定）。
///
/// `o_number` / `projection` / `tpl_id` / `lk_health` / `qk_*` 为可选列：
/// 询价/报价/分单链传 `None`，合同链按既有契约供给。
#[derive(Debug, Clone)]
pub struct ContractRowInput<'a> {
    /// 目标叶表（销售 / 采购 / 诉求；镜像叶由 `ContractLeaf::mirrored` 定）
    pub leaf: ContractLeaf,
    pub code: &'a str,
    pub notice: &'a str,
    pub comments: &'a str,
    /// 合同方行（顺序定 P 序号）
    pub parties: Vec<ContractParty>,
    /// 职能码（`↑.GG` / `↑_GG` / `↓.GG` / `↓_GG`）——`_f_`/`_t_` 派生源
    pub fn_code: &'a str,
    /// 场景/因子码（`zc_id_scene.code` / `zc_id_factor.code`）
    pub scene_code: &'a str,
    pub factor_code: &'a str,
    pub qk_date_id: Option<i64>,
    pub qk_valid_segm_id: Option<i64>,
    pub o_number: Option<&'a str>,
    pub projection: Option<&'a str>,
    pub tpl_id: Option<i64>,
    pub lk_health: Option<i64>,
    /// 草稿主状态桥（`zc_id_stus-contract.draft`）；`None` = 不落状态桥
    pub draft_status_id: Option<i64>,
    pub user_id: i64,
}

impl ContractRowInput<'_> {
    /// 镜像编号约定：`{主code}-R`（编号唯一性约定，非识别谓词）。
    pub fn mirrored_code(&self) -> String {
        format!("{}-R", self.code)
    }
}

/// 运输产品行写件入参（主/镜像同构：族由 `is_sales` 定，主体由调用方给定）。
#[derive(Debug, Clone)]
pub struct ProductRowInput<'a> {
    /// 销售向 = `zc_id_prod-freight_road-sales`；采购向 = `-purchase`
    pub is_sales: bool,
    pub code: &'a str,
    pub notice: &'a str,
    pub comments: &'a str,
    /// `fk_subj-demand`（买方主体）
    pub demand_subject: i64,
    /// `fk_subj-provider`（卖方主体）
    pub provider_subject: i64,
    pub line_id: Option<i64>,
    pub vehicle_form_id: Option<i64>,
    pub price_id: Option<i64>,
    pub weight_id: Option<i64>,
    pub period_id: Option<i64>,
    /// 版本链（`fk_previous`：镜像产品挂镜像单据 / 续约产品挂新合同；`None` 不写）
    pub previous_id: Option<i64>,
    /// 职能码——`_f_`/`_t_` 派生源
    pub fn_code: &'a str,
    pub scene_code: &'a str,
    pub factor_code: &'a str,
    pub user_id: i64,
}

/// 运输产品**成对**写件入参（一式两份：主族 + 相反族 + `{code}-R`；**同一买卖主体对，不互换**）。
#[derive(Debug, Clone)]
pub struct ProductPairInput<'a> {
    /// 主产品方向（销售向 = `-sales`；镜像自动落相反族）
    pub is_sales: bool,
    /// 主产品编号（镜像 = `{code}-R`）
    pub code: &'a str,
    pub notice: &'a str,
    pub comments: &'a str,
    /// 镜像文案（`None` = 沿用主文案）
    pub mirror_notice: Option<&'a str>,
    pub mirror_comments: Option<&'a str>,
    pub demand_subject: i64,
    pub provider_subject: i64,
    pub line_id: Option<i64>,
    pub vehicle_form_id: Option<i64>,
    pub price_id: Option<i64>,
    pub weight_id: Option<i64>,
    pub period_id: Option<i64>,
    /// 主产品 `fk_previous`（挂主单据）
    pub previous_main: Option<i64>,
    /// 镜像产品 `fk_previous`（挂镜像单据）
    pub previous_mirror: Option<i64>,
    pub fn_code: &'a str,
    pub scene_code: &'a str,
    pub factor_code: &'a str,
    pub user_id: i64,
}

/// 合同驱动运输产品组装入参（单侧；主/镜像各一次调用——业务方向由 `is_sales`/`is_single` 给定）。
#[derive(Debug, Clone)]
pub struct ContractProductInput<'a> {
    pub contract_id: i64,
    /// 合同向（销售 / 采购）——决定产品族与合同桥型
    pub is_sales: bool,
    /// 合同形态（single → `rr_deal`；master → `rr_goods`；采购恒 `rr_demand`）
    pub is_single: bool,
    /// 合同编号（产品 `code = PRD-{contract_code}`；起讫桥 `code = STOP-PRD-{...}`）
    pub contract_code: &'a str,
    pub notice: &'a str,
    pub comments: &'a str,
    pub demand_subject: i64,
    pub provider_subject: i64,
    pub line_id: i64,
    pub vehicle_form_id: i64,
    pub origin_place_id: i64,
    pub dest_place_id: i64,
    pub price_id: Option<i64>,
    pub weight_id: Option<i64>,
    pub period_id: Option<i64>,
    /// 合同有效期（`qk_valid-segm` → 合同桥 `qk_period`）
    pub valid_segm_id: Option<i64>,
    pub fn_code: &'a str,
    pub scene_code: &'a str,
    pub factor_code: &'a str,
    pub user_id: i64,
}
