//! Gateway 内置跨 namespace 通用种子自愈组件（add-gateway-seed-self-heal）
//!
//! 定位：Gateway 启动时对**跨 namespace 通用**的种子数据执行「检测 → 加载 → 自愈」。
//! 覆盖：种子用户、审批一致性自检（零模板）、NGAC 通用策略。
//!
//! 边界（NGAC_SPEC §7.3 四层种子边界，fix-register-binding-flow-gaps +
//! migrate-verctrl-seed-to-cosmic 后）：
//! - 本组件只持有**通用结构**（system 哨兵用户、NGAC 通用策略）——零流程模板、
//!   禁止出现任何 namespace 业务资源名；认证/授权域流程归模型级种子
//!   （`Framework/seed/seed-auth-approval-flows.sql`），ns 业务域流程（FLOW-VERCTRL）
//!   归 `Pre-Proc/{ns}/seed/`，均经 [`ns_seed::replay_model_seeds`] /
//!   ns manifest 启动供给。
//! - 凭据类种子（用户密码）不内嵌（SECURITY_SPEC §6）——归部署脚本。
//! - namespace 业务种子（资源 OA/行属性/ak_* 归位）由
//!   [`ns_seed::ensure_startup_seed_self_check`] 在进程启动时按
//!   `Pre-Proc/{ns}/seed/`（release 为 `Deploy/{ns}/seed/`）的 seed-manifest.json
//!   契约幂等重放（add-gateway-startup-seed-autoload）。
//!
//! 语义：幂等（先查后插 / NOT EXISTS / ON CONFLICT），单域失败 warn 不阻断启动，
//! 完成后汇总日志（各域 存在/新增/修复 计数）。

pub mod app_visibility_seed;
pub mod approval_seed;
pub mod auth_seed;
pub mod ngac_seed;
pub mod ns_seed;

use sqlx::PgPool;

pub use ns_seed::ensure_startup_seed_self_check;

/// 单域自检统计：已存在数 / 本次新增数 / 本次修复（回填）数
#[derive(Debug, Default, Clone, Copy)]
pub struct SeedStats {
    pub existing: usize,
    pub created: usize,
    pub healed: usize,
}

impl SeedStats {
    fn log(self, domain: &str) {
        common::telemetry::info!(
            "seed[{domain}]: 已存在 {}，新增 {}，修复 {}",
            self.existing,
            self.created,
            self.healed
        );
    }
}

/// 跨 namespace 通用种子自检统一入口。
///
/// 替代既有两处分散调用：main.rs 内联 `seed_self_check_system_subject` 与
/// `approval_self_check::ensure_approval_flow_self_check`。
pub async fn ensure_gateway_seed_self_check(pool: &PgPool) {
    let auth = auth_seed::ensure(pool).await;
    let approval = approval_seed::ensure(pool).await;
    let ngac = ngac_seed::ensure(pool).await;
    // App 可见性（依赖 ngac policy_class/UA/access_right 就绪）：发现面驱动，
    // 零 ns 硬编码——覆盖任意 ns（含新增 ns/新增 App），fail-closed 承载物
    let app_vis = app_visibility_seed::ensure(pool).await;

    auth.log("auth");
    approval.log("approval");
    ngac.log("ngac");
    app_vis.log("app-visibility");

    common::telemetry::info!(
        "Gateway 通用种子自检完成：auth 新增 {}/修复 {}，approval 新增 {}/修复 {}，ngac 新增 {}/修复 {}，app-visibility 新增 {}/修复 {}",
        auth.created,
        auth.healed,
        approval.created,
        approval.healed,
        ngac.created,
        ngac.healed,
        app_vis.created,
        app_vis.healed,
    );
}
