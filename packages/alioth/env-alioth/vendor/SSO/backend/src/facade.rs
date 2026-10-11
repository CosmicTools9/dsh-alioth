//! `gateway-sso` 的**对外 interface**（窄 facade）——change `extract-sso-crate-facade`。
//!
//! 为什么存在：本 crate 有 13 个 `pub mod`，而三个外部消费者（`Gateway/host`、
//! `Gateway/backend`、`OpenActivity/backend`；ns 组合根经模板 `main.rs` 同形消费）此前各自
//! **直抵内部 module 路径**——于是内部重命名/搬迁即跨仓破坏，且没有任何地方记录「谁依赖什么」。
//! 实测代价：退役 `ngac::age_projection` 时宿主侧残留调用导致 `--features sso` 编不过
//! （change `extract-embedded-sso-assembly`）。
//!
//! 契约（判据正本 = 能力 `sso-crate-interface-facade`）：
//! - 消费者（`Gateway/**`、`OpenActivity/**`）MUST 只经本 facade 引用 `gateway_sso::…`；
//!   门禁 = `scripts/check/check-sso-crate-seam.ts`。
//! - 本 facade 的**名字与分组是承诺**：内部 module 可自由重组，只要 facade 名字不变；
//!   新增对外项 MUST 在此登记（分类 = 关注点：auth / config / routes / lifecycle / admin / ngac）。
//! - 本 crate 自身的实现与**集成测试**（`SSO/backend/tests/**`，48 个文件）不在本契约内——
//!   它们与本 crate 同版本演进，走内部路径是刻意选择（避免每次内部重构都改 48 个文件）。

/// 认证面：JWT 材料与校验域、认证中间件、请求级认证状态。
pub mod auth {
    pub use crate::auth::jwt::{
        configure_token_validation, decode_token_any, derive_public_key, encode_access_token, jwks,
        Claims,
    };
    pub use crate::auth::middleware::NgacPep;
    pub use crate::auth::AuthState;
}

/// 配置面：`Config::from_env` 是唯一构造入口。
pub mod config {
    pub use crate::config::Config;
}

/// 路由装配面：`/auth|/api/auth` 公开面与受保护面，以及独立形态的 server 构造。
pub mod routes {
    pub use crate::{
        build_server, configure_protected_routes, configure_public_routes,
        configure_public_routes_without_scope,
    };
}

/// 生命周期面：内嵌形态装配（含进程级令牌校验域注入）与后台任务。
pub mod lifecycle {
    pub use crate::{assemble_embedded, spawn_background, EmbeddedSso};
}

/// 管理面鉴权：SSO admin 端点与 chat 工具共用的门控。
pub mod admin {
    pub use crate::admin::handlers::require_admin;
}

/// NGAC 授权面（决策/分析/图快照/策略写）。
pub mod ngac {
    /// 只读分析：dry-run 仿真与 explain-paths 证据。
    pub mod analysis {
        pub use crate::ngac::analysis::{
            dry_run_impact, explain_paths_evidence, DryRunError, DryRunRequest, ExplainPathsError,
            ExplainPathsRequest,
        };
    }

    /// 策略图快照与图投影类型（前端/工具面只读消费）。
    pub mod graph {
        pub use crate::ngac::graph::{
            graph_snapshot, GraphAccessRight, GraphAssociation, GraphObjectAttribute,
            GraphPolicyClass, GraphProhibition, GraphSnapshot, GraphUserAttribute,
            ProjectionHealth,
        };
    }

    /// org-policy 服务层（管理面与 chat 工具共用的写径，内含 Rhai 强校验）。
    pub mod org_policy {
        pub use crate::ngac::org_policy::{
            create_org_policy_class, create_org_policy_label, create_org_policy_rule,
            delete_org_policy_label, delete_org_policy_rule, update_org_policy_class,
            update_org_policy_label, update_org_policy_rule, CreateOrgPolicyClass,
            CreateOrgPolicyLabel, CreateOrgPolicyRule, UpdateOrgPolicyClass, UpdateOrgPolicyLabel,
            UpdateOrgPolicyRule,
        };
    }

    /// NGAC 扩展表 ensure（启动期收敛；Gateway 侧集成测试夹具亦需）。
    pub mod ensure {
        pub use crate::ngac::ensure::ensure_ngac_extension_tables;
    }

    /// 策略写径原语（PIP 批量写，含 `AssociationPatch` / `ProhibitionPatch` / `Actor`）。
    pub use crate::ngac::policy_write;
}
