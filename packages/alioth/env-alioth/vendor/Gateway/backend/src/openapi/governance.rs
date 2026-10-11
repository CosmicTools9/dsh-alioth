//! OpenAPI 第三方请求治理——执行序与装配单源（change `fix-openapi-review-findings` §1）。
//!
//! # 目标执行序（请求方向）
//!
//! `Metrics（观测，最外）→ PEP（鉴权）→ metering（订阅/限流/配额/计量）→ idempotency（幂等）→ handler`
//!
//! 不变量（由本模块装配序保证，`tests/openapi_governance_order_test.rs` 钉住）：
//! 1. metering/idempotency 消费的 claims 一律「PEP 已验」——未通过鉴权的请求
//!    MUST NOT 计量（不写 `api_usage`）、MUST NOT 消耗限流桶/配额、MUST NOT 写幂等键；
//! 2. 幂等重放经 metering 计量（计入用量与配额）；
//! 3. Metrics 观测全流量（含被拒请求；口径变化记录于 change design D1）。
//!
//! # actix-web 注册序语义（本模块的判据基石）
//!
//! `Scope::wrap` 逐次把中间件包在既有 endpoint **外侧**——actix-web 4.15
//! `src/middleware/mod.rs`「Ordering」明文：the request _first_ gets processed by the
//! middleware specified _last_（**后注册者先执行**）。因此本模块按「执行序的逆序」
//! 登记 `.wrap()`；另有标记中间件回归测试（无 DB）锚定该语义，防依赖升级漂移。
//!
//! 历史缺陷（2026-10-08 架构评审 C2 发现，本 change 修复）：组合根曾按**执行序**登记
//! wrap（PEP 最先注册），实际执行全部倒置为 `idempotency → metering → Metrics → PEP`
//! ——① 重放请求绕过 metering（不计用量/配额）；② 未通过鉴权的请求（伪造签名即可，
//! `service_client_id` 不验签）仍消耗目标 client 的限流桶、写计量行计入配额、写/占用
//! 幂等键行。修复 = 装配收进本模块（唯一装配点）+ 按逆序登记。

use actix_web::{
    body::{BoxBody, EitherBody},
    dev::{ServiceFactory, ServiceRequest},
    web, Error, Scope,
};
use sqlx::PgPool;

use super::{idempotency::IdempotencyMiddleware, metering::ApiUsageMiddleware};
use crate::monitoring::middleware::MetricsMiddleware;
use crate::pep::NgacEnforcer;

/// 治理管线装配后的 scope body（四层 `EitherBody`：Metrics → PEP → metering → idempotency
/// 各自的直出/透传分支）。
type GovernanceBody = EitherBody<EitherBody<EitherBody<EitherBody<BoxBody>>>>;

/// OpenAPI 治理中间件实例（跨 worker 共享内部状态：限流桶 / 订阅缓存 / 计量队列）。
#[derive(Clone)]
pub struct OpenApiGovernance {
    metering: ApiUsageMiddleware,
    idempotency: IdempotencyMiddleware,
}

impl OpenApiGovernance {
    /// 在 `HttpServer::new` 前构造一次、闭包内按 worker 复用（内部状态跨 worker 共享）。
    pub fn new(pool: PgPool) -> Self {
        Self {
            metering: ApiUsageMiddleware::new(pool.clone()),
            idempotency: IdempotencyMiddleware::new(pool),
        }
    }

    /// 装配 `/api` 治理管线（**唯一装配点**：组合根与集成测试 MUST 同走本函数）。
    ///
    /// `pep` 为组合根配置完成的 PEP 实例；返回的 scope 由调用方继续 `.configure(...)`
    /// 注册业务路由。登记序 = 执行序逆序（见模块头注）。
    pub fn install(
        &self,
        pep: NgacEnforcer,
    ) -> Scope<
        impl ServiceFactory<
            ServiceRequest,
            Config = (),
            Response = actix_web::dev::ServiceResponse<GovernanceBody>,
            Error = Error,
            InitError = (),
        >,
    > {
        // actix-web：后注册者先执行（外层）。目标执行序 = Metrics → PEP → metering →
        // idempotency ⇒ 按逆序登记。
        web::scope("/api")
            .wrap(self.idempotency.clone())
            .wrap(self.metering.clone())
            .wrap(pep)
            .wrap(MetricsMiddleware)
    }
}
