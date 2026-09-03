//! 真实 CRUD 路由 — 替代 dev mock handlers
//!
//! 替换自 mock.rs 的端点映射:
//!   /consignments        → Consignment CRUD
//!   /waybills            → 自定义: 按 ck_category 过滤
//!   /vehicles            → Vehicle CRUD
//!   /environments        → Environment CRUD
//!   /licenses            → License CRUD
//!   /transport-tracking  → TransportTracking CRUD
//!   /trade-orders        → TradeOrder CRUD
//!   /bill-checks         → BillCheck CRUD
//!   /deta-bill-checks    → DetaBillCheck CRUD
//!   /invoices            → Invoice CRUD
//!   /invoice-details     → InvoiceDetail CRUD
//!   /payments            → Payment CRUD
//!   /settlement-banks    → SettlementBank CRUD
//!   /settlement-cashes   → SettlementCash CRUD
//!   /settlement-channels → SettlementChannel CRUD
use crate::models::{
    BillCheck, Consignment, Contract, CreateBillCheckRequest, CreateConsignmentRequest,
    CreateContractRequest, CreateDetaBillCheckRequest, CreateEnvironmentRequest,
    CreateFenceRequest, CreateFreightProductRequest, CreateInventorySalesRequest,
    CreateInvoiceDetailRequest, CreateInvoiceRequest, CreateLicenseRequest, CreatePaymentRequest,
    CreatePricingAgreementRequest, CreateSealRequest, CreateSettlementBankRequest,
    CreateSettlementCashRequest, CreateSettlementChannelRequest, CreateTradeOrderRequest,
    CreateTrafficLineRequest, CreateTransitRouteRequest, CreateTransportTrackingRequest,
    CreateVehicleRequest, CreateWaybillRequest, DetaBillCheck, Environment, Fence, FreightProduct,
    InventorySales, Invoice, InvoiceDetail, License, Payment, PricingAgreement, Seal,
    SettlementBank, SettlementCash, SettlementChannel, TradeOrder, TrafficLine, TransitRoute,
    TransportTracking, UpdateBillCheckRequest, UpdateConsignmentRequest, UpdateContractRequest,
    UpdateDetaBillCheckRequest, UpdateEnvironmentRequest, UpdateFenceRequest,
    UpdateFreightProductRequest, UpdateInventorySalesRequest, UpdateInvoiceDetailRequest,
    UpdateInvoiceRequest, UpdateLicenseRequest, UpdatePaymentRequest,
    UpdatePricingAgreementRequest, UpdateSealRequest, UpdateSettlementBankRequest,
    UpdateSettlementCashRequest, UpdateSettlementChannelRequest, UpdateTradeOrderRequest,
    UpdateTrafficLineRequest, UpdateTransitRouteRequest, UpdateTransportTrackingRequest,
    UpdateVehicleRequest, UpdateWaybillRequest, Vehicle, Waybill,
};
use crate::repository::{
    BillCheckRepository, ConsignmentRepository, ContractRepository, DetaBillCheckRepository,
    EnvironmentRepository, FenceRepository, FreightProductRepository, InventorySalesRepository,
    InvoiceDetailRepository, InvoiceRepository, LicenseRepository, NaturalPersonRepository,
    PaymentRepository, PricingAgreementRepository, SealRepository, SettlementBankRepository,
    SettlementCashRepository, SettlementChannelRepository, TradeOrderRepository,
    TrafficLineRepository, TransitRouteRepository, TransportTrackingRepository, VehicleRepository,
};
use actix_web::web;

use common::AliothError as ApiError;
use crud::crud_routes;

/// 注册全部真实数据路由
pub fn register_business_domain(cfg: &mut web::ServiceConfig) {
    // Waybill CRUD (共享 zc_id_orde-land, type alias models.rs:341)
    cfg.configure(crud_routes::<
        Waybill,
        CreateWaybillRequest,
        UpdateWaybillRequest,
        ConsignmentRepository,
        ApiError,
    >("/waybills"));
    // 标准 CRUD: 用 .configure() 而非直接 FnOnce 调用
    cfg.configure(crud_routes::<
        Consignment,
        CreateConsignmentRequest,
        UpdateConsignmentRequest,
        ConsignmentRepository,
        ApiError,
    >("/consignments"));
    cfg.configure(crud_routes::<
        Environment,
        CreateEnvironmentRequest,
        UpdateEnvironmentRequest,
        EnvironmentRepository,
        ApiError,
    >("/environments"));
    cfg.configure(crud_routes::<
        License,
        CreateLicenseRequest,
        UpdateLicenseRequest,
        LicenseRepository,
        ApiError,
    >("/licenses"));
    cfg.configure(crud_routes::<
        Vehicle,
        CreateVehicleRequest,
        UpdateVehicleRequest,
        VehicleRepository,
        ApiError,
    >("/vehicles"));

    cfg.configure(crud_routes::<
        TransportTracking,
        CreateTransportTrackingRequest,
        UpdateTransportTrackingRequest,
        TransportTrackingRepository,
        ApiError,
    >("/transport-tracking"));
    cfg.configure(crud_routes::<
        TradeOrder,
        CreateTradeOrderRequest,
        UpdateTradeOrderRequest,
        TradeOrderRepository,
        ApiError,
    >("/trade-orders"));
    cfg.configure(crud_routes::<
        BillCheck,
        CreateBillCheckRequest,
        UpdateBillCheckRequest,
        BillCheckRepository,
        ApiError,
    >("/bill-checks"));
    cfg.configure(crud_routes::<
        DetaBillCheck,
        CreateDetaBillCheckRequest,
        UpdateDetaBillCheckRequest,
        DetaBillCheckRepository,
        ApiError,
    >("/deta-bill-checks"));
    cfg.configure(crud_routes::<
        Invoice,
        CreateInvoiceRequest,
        UpdateInvoiceRequest,
        InvoiceRepository,
        ApiError,
    >("/invoices"));
    cfg.configure(crud_routes::<
        InvoiceDetail,
        CreateInvoiceDetailRequest,
        UpdateInvoiceDetailRequest,
        InvoiceDetailRepository,
        ApiError,
    >("/invoice-details"));
    cfg.configure(crud_routes::<
        Payment,
        CreatePaymentRequest,
        UpdatePaymentRequest,
        PaymentRepository,
        ApiError,
    >("/payments"));
    cfg.configure(crud_routes::<
        SettlementBank,
        CreateSettlementBankRequest,
        UpdateSettlementBankRequest,
        SettlementBankRepository,
        ApiError,
    >("/settlement-banks"));
    cfg.configure(crud_routes::<
        SettlementCash,
        CreateSettlementCashRequest,
        UpdateSettlementCashRequest,
        SettlementCashRepository,
        ApiError,
    >("/settlement-cashes"));
    cfg.configure(crud_routes::<
        SettlementChannel,
        CreateSettlementChannelRequest,
        UpdateSettlementChannelRequest,
        SettlementChannelRepository,
        ApiError,
    >("/settlement-channels"));
    cfg.configure(crud_routes::<
        InventorySales,
        CreateInventorySalesRequest,
        UpdateInventorySalesRequest,
        InventorySalesRepository,
        ApiError,
    >("/inve-sales"));
    cfg.configure(crud_routes::<
        Fence,
        CreateFenceRequest,
        UpdateFenceRequest,
        FenceRepository,
        ApiError,
    >("/fences"));
    // add-wz-seal-batch-creation：批量创建路由必须先于 crud_routes 注册
    // （actix 顺序匹配，防 /{id} 抢占 /batch）
    cfg.configure(crate::handlers::seal::register);
    cfg.configure(crud_routes::<
        Seal,
        CreateSealRequest,
        UpdateSealRequest,
        SealRepository,
        ApiError,
    >("/seals"));
    cfg.configure(crud_routes::<
        TransitRoute,
        CreateTransitRouteRequest,
        UpdateTransitRouteRequest,
        TransitRouteRepository,
        ApiError,
    >("/transit-routes"));
    cfg.configure(crud_routes::<
        TrafficLine,
        CreateTrafficLineRequest,
        UpdateTrafficLineRequest,
        TrafficLineRepository,
        ApiError,
    >("/traffic-lines"));
    cfg.configure(crud_routes::<
        FreightProduct,
        CreateFreightProductRequest,
        UpdateFreightProductRequest,
        FreightProductRepository,
        ApiError,
    >("/freight-products"));
    cfg.configure(crud_routes::<
        PricingAgreement,
        CreatePricingAgreementRequest,
        UpdatePricingAgreementRequest,
        PricingAgreementRepository,
        ApiError,
    >("/pricing-agreements"));
    cfg.configure(crud_routes::<
        Contract,
        CreateContractRequest,
        UpdateContractRequest,
        ContractRepository,
        ApiError,
    >("/contracts"));
}

use crate::models::{CreateNaturalPersonRequest, NaturalPerson, UpdateNaturalPersonRequest};
/// 主体/组织/身份通用域 CRUD 路由——全 ns 壳可挂载（design D1）。
pub fn register_subject_domain(cfg: &mut web::ServiceConfig) {
    cfg.configure(crud_routes::<
        NaturalPerson,
        CreateNaturalPersonRequest,
        UpdateNaturalPersonRequest,
        NaturalPersonRepository,
        ApiError,
    >("/natural-persons"));
    register_subject_leaves(cfg);
}
macro_rules! register_subject_leaf {
    ($cfg:ident, $entity:ident, $create:ident, $update:ident, $repo:ident, $path:literal) => {
        $cfg.configure(crud_routes::<$entity, $create, $update, $repo, ApiError>(
            $path,
        ));
    };
}

/// 主体域叶表路由（strengthen-identity-org）：组/员工/智能体/国家/银行/部委/主权/超国家。
/// 全部 ns 壳经 register_subject_domain 挂载。
pub fn register_subject_leaves(cfg: &mut web::ServiceConfig) {
    use crate::models::{
        CreateEmploymentAgentRequest, CreateSubjectBankRequest, CreateSubjectCountryRequest,
        CreateSubjectEmployeeRequest, CreateSubjectGroupRequest, CreateSubjectMinistryRequest,
        CreateSubjectSovereignRequest, CreateSubjectSupranationalRequest, EmploymentAgent,
        SubjectBank, SubjectCountry, SubjectEmployee, SubjectGroup, SubjectMinistry,
        SubjectSovereign, SubjectSupranational, UpdateEmploymentAgentRequest,
        UpdateSubjectBankRequest, UpdateSubjectCountryRequest, UpdateSubjectEmployeeRequest,
        UpdateSubjectGroupRequest, UpdateSubjectMinistryRequest, UpdateSubjectSovereignRequest,
        UpdateSubjectSupranationalRequest,
    };
    use crate::repository::{
        EmploymentAgentRepository, SubjectBankRepository, SubjectCountryRepository,
        SubjectEmployeeRepository, SubjectGroupRepository, SubjectMinistryRepository,
        SubjectSovereignRepository, SubjectSupranationalRepository,
    };

    register_subject_leaf!(
        cfg,
        SubjectGroup,
        CreateSubjectGroupRequest,
        UpdateSubjectGroupRequest,
        SubjectGroupRepository,
        "/groups"
    );
    register_subject_leaf!(
        cfg,
        SubjectEmployee,
        CreateSubjectEmployeeRequest,
        UpdateSubjectEmployeeRequest,
        SubjectEmployeeRepository,
        "/employees"
    );
    register_subject_leaf!(
        cfg,
        EmploymentAgent,
        CreateEmploymentAgentRequest,
        UpdateEmploymentAgentRequest,
        EmploymentAgentRepository,
        "/agents"
    );
    register_subject_leaf!(
        cfg,
        SubjectCountry,
        CreateSubjectCountryRequest,
        UpdateSubjectCountryRequest,
        SubjectCountryRepository,
        "/countries"
    );
    register_subject_leaf!(
        cfg,
        SubjectBank,
        CreateSubjectBankRequest,
        UpdateSubjectBankRequest,
        SubjectBankRepository,
        "/banks"
    );
    register_subject_leaf!(
        cfg,
        SubjectMinistry,
        CreateSubjectMinistryRequest,
        UpdateSubjectMinistryRequest,
        SubjectMinistryRepository,
        "/ministries"
    );
    register_subject_leaf!(
        cfg,
        SubjectSovereign,
        CreateSubjectSovereignRequest,
        UpdateSubjectSovereignRequest,
        SubjectSovereignRepository,
        "/sovereigns"
    );
    register_subject_leaf!(
        cfg,
        SubjectSupranational,
        CreateSubjectSupranationalRequest,
        UpdateSubjectSupranationalRequest,
        SubjectSupranationalRepository,
        "/supranationals"
    );
}
