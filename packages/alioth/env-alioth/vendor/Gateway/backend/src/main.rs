use actix_web::{http::header, middleware::Logger, web, App, HttpResponse, HttpServer};
use alioth_gateway::i18n::middleware::LocaleMiddleware;
use alioth_gateway::monitoring::middleware::MetricsMiddleware;
use alioth_gateway::pep::NgacEnforcer;
use alioth_gateway::{Config, Database, Metrics};
use encoding::zuid_init::init_zuid_function;
use prometheus::Encoder;
use std::sync::Arc;
// Mutex 仅 preproc 发现服务使用（消费方门控见 mod preproc）
#[cfg(any(
    feature = "preproc-proxy",
    not(any(feature = "sso", feature = "sso-remote"))
))]
use std::sync::Mutex;
use tokio::signal;

// sso 与 sso-remote 互斥：内嵌 SSO 与远程委托 SSO 不能同时开启
#[cfg(all(feature = "sso", feature = "sso-remote"))]
compile_error!(
    "`sso` 与 `sso-remote` 特性互斥：内嵌 SSO 与远程委托 SSO 不能同时启用，请只选其一。"
);

// preproc 模块仅在消费方编译时启用：
//   - preproc-proxy feature（/preproc 反代路由）
//   - standalone 认证（/auth/me 消费 PreProcDiscovery）
// sso/sso-remote + 无 proxy 的生产构建无消费方 → 整个模块不编译（消除 dead_code 告警）
#[cfg(any(
    feature = "preproc-proxy",
    not(any(feature = "sso", feature = "sso-remote"))
))]
mod preproc;
#[cfg(not(any(feature = "sso", feature = "sso-remote")))]
mod standalone_auth;
use alioth_gateway::api::approval_formula;
use alioth_gateway::api::approvals;
use alioth_gateway::api::chat_sessions;
use alioth_gateway::api::entity_binding;
// OpenAPI 数据服务产品 backend（namespace 级通用，{ns}/openapi/）。
// alioth 与 wz feature 组均启用（Gateway/backend/Cargo.toml），
// 其余 namespace 编译时不链接 openapi crate（namespace 隔离）。
#[cfg(feature = "sso")]
use alioth_gateway::api::admin_ngac_assist;
use alioth_gateway::api::contacts;
use alioth_gateway::api::dashboard;
use alioth_gateway::api::global_overview;
use alioth_gateway::api::inbox;
use alioth_gateway::api::legal_search;
use alioth_gateway::api::profile;
use alioth_gateway::api::standard_search;
use alioth_gateway::api::system_push;
use alioth_gateway::notification::NotificationService;
#[cfg(feature = "alioth-service-openapi")]
use alioth_service_openapi as openapi;
#[cfg(all(
    not(feature = "alioth-service-openapi"),
    feature = "wz-service-openapi"
))]
use wz_service_openapi as openapi;

#[cfg(feature = "preproc-proxy")]
use preproc::configure_preproc_routes;
#[cfg(any(
    feature = "preproc-proxy",
    not(any(feature = "sso", feature = "sso-remote"))
))]
use preproc::PreprocDiscovery;
use runtime_engine::{AppContext, AppExtensionRegistry, ExtensionLoader};
mod service_registry;
use alioth_gateway::schedule::configure_routes as configure_schedule_routes;
use system_config::crypto::init_encryption as init_system_config_encryption;
use system_config::handlers::configure_routes as configure_system_config_routes;

// SSO embedded module
#[cfg(feature = "sso")]
use gateway_sso::auth::jwt::derive_public_key;
#[cfg(not(any(feature = "sso", feature = "sso-remote")))]
use standalone_auth::{
    configure_routes as configure_standalone_auth,
    configure_routes_without_scope as configure_standalone_auth_without_scope,
    init_auth_config as init_standalone_auth,
};

// sso-remote 认证反代模块（Gateway 不内嵌 SSO，透明转发认证/NGAC 请求到远程 SSO）
#[cfg(feature = "sso-remote")]
mod sso_remote_proxy;

/// 初始化 Config、Database、SSO JWT public key、ZUID 函数和系统配置加密
async fn init_state() -> std::io::Result<(Config, sqlx::PgPool, Vec<u8>)> {
    let config = Config::from_env().map_err(std::io::Error::other)?;
    let database = Database::new(&config)
        .await
        .map_err(std::io::Error::other)?;

    #[cfg(feature = "sso")]
    let sso_jwt_public_key_bytes = {
        // 内嵌模式：同进程持私钥，PEP 回退公钥由私钥派生（恒配对，
        // 消除静态公钥文件漂移导致的运行期 InvalidSignature）
        let sso_cfg = gateway_sso::Config::from_env()
            .map_err(|e| std::io::Error::other(format!("SSO config: {e}")))?;
        gateway_sso::auth::jwt::derive_public_key(sso_cfg.sso_jwt_private_key.as_bytes())
            .map_err(|e| std::io::Error::other(format!("SSO JWT key: {e}")))?
    };
    #[cfg(all(feature = "sso-remote", not(feature = "sso")))]
    let sso_jwt_public_key_bytes = config.sso_jwt_public_key.as_bytes().to_vec();
    #[cfg(not(any(feature = "sso", feature = "sso-remote")))]
    let sso_jwt_public_key_bytes = {
        standalone_auth::init_auth_config();
        standalone_auth::auth_config().public_key_pem.clone()
    };
    let pool = database.pool().clone();

    // 初始化 ZUID 函数
    if let Err(e) = init_zuid_function(&pool).await {
        common::telemetry::error!("Failed to initialize ZUID function: {}", e);
        return Err(std::io::Error::other(e));
    }

    // 初始化系统配置加密（凭证字段 AES-256-GCM 加密）
    if let Ok(enc_key) = std::env::var("SYSTEM_CONFIG_ENC_KEY") {
        if let Err(e) = init_system_config_encryption(&enc_key) {
            common::telemetry::warn!("Failed to initialize system-config encryption: {}", e);
        } else {
            common::telemetry::info!("System-config encryption initialized");
        }
    } else {
        common::telemetry::warn!(
            "SYSTEM_CONFIG_ENC_KEY not set, system-config credentials will not be encrypted"
        );
    }

    Ok((config, pool, sso_jwt_public_key_bytes))
}

/// 初始化 Trigger Registry、log_event 分区与 isahl.mv_inventory 自愈
async fn init_framework(state: &(Config, sqlx::PgPool, Vec<u8>)) -> std::io::Result<()> {
    // 初始化 Trigger Registry（Gateway 模式：禁止访问 isahl_meta，使用硬编码层次结构）
    if let Err(e) = trigger_registry::init::init_smart_registry_global(
        &state.1,
        trigger_registry::AppContainer::Gateway,
    )
    .await
    {
        common::telemetry::warn!("Failed to initialize smart trigger registry: {}", e);
    }

    // 侦测 log_event 分区状态，未分表则自动补充
    if let Err(e) = ensure_log_event_partitions(&state.1).await {
        common::telemetry::warn!("Failed to ensure log_event partitions: {}", e);
    }

    // isahl.mv_inventory 库存物化视图自检自愈（用户 2026-08-07 定稿：视图落地 = Rust 自愈）。
    // 幂等：已存在直接跳过；基表 zc_id_production_rr_storage 缺失（pre/prod 旧模型）内部降级
    // warn + Ok；其余失败 fail-fast 阻止启动（视图落地失败 = 配置错误）。
    if let Err(e) = trigger_registry::stock_materialization::ensure_mv_inventory(&state.1).await {
        return Err(std::io::Error::other(format!(
            "isahl.mv_inventory self-heal failed: {e}"
        )));
    }

    // 凭证幂等唯一索引自检自愈（fix-wz-capacity-inventory-guard）：tsp/com 部分唯一索引
    // (code) WHERE deleted_at IS NULL——守卫原语 code 幂等退化的并发硬约束。
    // 内部失败 warn 降级（索引缺失仅弱化并发兜底，不阻断启动）。
    if let Err(e) =
        trigger_registry::stock_materialization::ensure_voucher_idempotency(&state.1).await
    {
        common::telemetry::warn!("voucher idempotency index self-heal degraded: {e}");
    }

    // ── 业务定时器（framework-scheduler）统一装配于 main() event_bus 就绪后 ────
    // （见 main() 中 SchedulerService 注册块；init_framework 不承载调度装配，
    //  避免 scheduler 实例生命周期跨函数传播。全部 5 个计划 handler 在
    //  main() 同一实例注册 + start。）
    Ok(())
}

// （原 seed_self_check_system_subject 已并入 seed::auth_seed——见 seed 模块）

/// 智能查找应用数据基础目录（支持多种 CWD）
///
/// 优先级（高 → 低）:
///   1. `DEPLOY_PATH` 环境变量 — release 模式，指向 `Deploy/{namespace}/`
///   2. `PREPROC_APPS_PATH` 环境变量 — dev 模式覆盖
///   3. 自动探测 (Pre-Proc/ → ../Pre-Proc → ../../Pre-Proc)
fn resolve_preproc_path() -> String {
    // 1. DEPLOY_PATH 优先（release 模式）
    if let Ok(p) = std::env::var("DEPLOY_PATH") {
        if std::path::Path::new(&p).exists() {
            common::telemetry::info!("Using DEPLOY_PATH: {}", p);
            return p;
        }
        common::telemetry::warn!("DEPLOY_PATH set but not found: {}, falling back", p);
    }

    // 2. PREPROC_APPS_PATH 覆盖（dev 模式）
    if let Ok(p) = std::env::var("PREPROC_APPS_PATH") {
        if std::path::Path::new(&p).exists() {
            return p;
        }
    }

    // 3. 自动探测
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => return "../../Pre-Proc".to_string(),
    };
    let candidates = [
        cwd.join("Pre-Proc"),       // from project root
        cwd.join("../Pre-Proc"),    // from Gateway/
        cwd.join("../../Pre-Proc"), // from Gateway/backend/
    ];
    candidates
        .into_iter()
        .find(|p| p.exists())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "../../Pre-Proc".to_string())
}

/// 加载应用扩展（来自 DEPLOY_PATH / PREPROC_APPS_PATH 指向的目录）
///
/// 扫描 {base}/Apps/{app}/extensions/*.yaml，加载到 AppExtensionRegistry。
/// - DEPLOY_PATH 设置 → release 模式，base = Deploy/{namespace}/（已含 namespace 层级）
/// - PREPROC_APPS_PATH 或自动探测 → dev 模式，base = Pre-Proc/{namespace}/
async fn load_extensions(_pool: &sqlx::PgPool) -> AppExtensionRegistry {
    let extension_registry = AppExtensionRegistry::new();

    let preproc_path = resolve_preproc_path();
    let namespace = std::env::var("NAMESPACE").unwrap_or_default();

    // DEPLOY_PATH 已包含 namespace 层级（Deploy/{namespace}/），dev 模式需补 namespace。
    // 判断条件与 resolve_preproc_path 内部一致：DEPLOY_PATH 设置且存在。
    let deploy_mode = std::env::var("DEPLOY_PATH")
        .map(|p| std::path::Path::new(&p).exists())
        .unwrap_or(false);
    let base_dir = if deploy_mode {
        std::path::PathBuf::from(&preproc_path)
    } else {
        std::path::Path::new(&preproc_path).join(&namespace)
    };
    let apps_dir = base_dir.join("Apps");

    if !apps_dir.is_dir() {
        common::telemetry::warn!(
            "Apps directory not found: {}. Gateway will start without app extensions.",
            apps_dir.display()
        );
        return extension_registry;
    }

    let mut scanned = 0usize;
    let mut loaded = 0usize;

    let entries = match std::fs::read_dir(&apps_dir) {
        Ok(e) => e,
        Err(e) => {
            common::telemetry::warn!(
                "Failed to read Apps directory '{}': {}. Gateway will start without app extensions.",
                apps_dir.display(),
                e
            );
            return extension_registry;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // 跳过隐藏目录（如 .I_need_a.bak），与 preproc/discovery.rs 一致；
        // app_code 取目录名，与 ExtensionLoader::load_from_dir 参考实现一致
        let app_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if !n.starts_with('.') => n.to_string(),
            _ => continue,
        };
        scanned += 1;

        let ext_dir = path.join("extensions");
        let ext_dir_str = ext_dir.to_string_lossy().to_string();
        match ExtensionLoader::load_from_dir(&app_name, &ext_dir_str) {
            Ok(ext) => {
                if !ext.is_empty() {
                    loaded += 1;
                    common::telemetry::info!("Loaded logic extensions for app '{}': {} constraints, {} rules, {} state machines, {} workflows",
                    app_name,
                    ext.constraints.len(),
                    ext.business_rules.len(),
                    ext.state_machines.len(),
                    ext.workflows.len());
                    extension_registry.register(ext);
                }
            }
            Err(e) => {
                common::telemetry::error!("Failed to load extensions for app '{}': {}. Gateway cannot start with broken extensions.",
                app_name, e);
                std::process::exit(1);
            }
        }
    }

    common::telemetry::info!(
        "Extension loading complete for namespace '{}': {} apps scanned, {} with extensions",
        namespace,
        scanned,
        loaded
    );

    extension_registry
}

/// 注册 OpenAPI 数据服务产品路由（feature-gated）。
/// alioth 与 wz feature 组启用；其余 namespace 不链接 openapi crate。
#[cfg(any(feature = "alioth-service-openapi", feature = "wz-service-openapi"))]
fn register_openapi_routes(cfg: &mut web::ServiceConfig) {
    openapi::register_service_routes(cfg);
}
#[cfg(not(any(feature = "alioth-service-openapi", feature = "wz-service-openapi")))]
fn register_openapi_routes(_cfg: &mut web::ServiceConfig) {}

/// 提取 system_config 的 configure closure 为命名函数
fn register_system_config_routes(cfg: &mut web::ServiceConfig, pool: sqlx::PgPool) {
    let repo = alioth_gateway::system_config_repo::SystemConfigRepo::new(pool.clone());
    // 先注册精确路由再注册 /system-config scope——actix scope 前缀匹配后内部
    // 无命中即 404 不回溯外层（实测），精确路由必须在 scope 之前才有机会命中。
    cfg.route(
        "/system-config/llm/test",
        web::post().to(alioth_gateway::api::system_config_llm_test::llm_test),
    )
    .app_data(web::Data::new(pool));
    configure_system_config_routes(repo, cfg);
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // 记录进程启动时间（system_check /system 端点用）
    STARTED_AT.store(
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        std::sync::atomic::Ordering::Relaxed,
    );
    // 加载 .env 文件
    dotenvy::dotenv().ok();
    // 未设 RUST_LOG 时默认 info——本地启动即可看到 FSSC 对接报文等业务日志；
    // 设了 RUST_LOG 仍以其为准（如 RUST_LOG=debug 或定向模块级别）
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    common::telemetry::info!("Starting AliothStudio Gateway Server...");

    let state = init_state().await?;
    let server_addr = state.0.server_addr.clone();

    init_framework(&state).await?;

    // Namespace schema sync + migration
    // When NAMESPACE env var is set, syncs isahl/isahl_auth/isahl_audit schemas
    // from reference DB and runs pending migrations.
    alioth_gateway::namespace_schema::sync_namespace_schema(&state.1).await;

    // 跨 namespace 通用种子自检自愈（add-gateway-seed-self-heal）：
    // system 哨兵用户 + 审批基态/流程模板/一致性 + NGAC 通用策略，统一入口幂等自愈。
    alioth_gateway::seed::ensure_gateway_seed_self_check(&state.1).await;

    // 进程内启动种子自愈（add-gateway-startup-seed-autoload）：
    // 模型级（Framework/seed 或 Deploy 软链）+ namespace 业务种子（seed-manifest 驱动）
    // 幂等重放——任何启动方式（wrapper 脚本 / mise run dev / 直接二进制）都自动载入，
    // 失败 WARN 不阻断启动（与 Deploy start.sh 4b/4c 语义对齐）。
    alioth_gateway::seed::ensure_startup_seed_self_check(&state.1).await;

    // WZ namespace 专属：yecai 业务 schema 自检自愈（invoice-sync 开票申请单 4 表 + receipt-sync 收款单 2 表）
    // 幂等检查 yecai.* 业务表是否存在，缺失则自动执行内嵌 DDL 建表；
    // 失败则 fail-fast 阻止启动（与 sync_namespace_schema 的框架 schema 同步点并列）。
    #[cfg(feature = "wz")]
    {
        wz_service_invoice_sync::db_init::ensure_yecai_schema(&state.1)
            .await
            .map_err(|e| std::io::Error::other(format!("WZ yecai schema self-heal failed: {e}")))?;
        wz_service_receipt_sync::db_init::ensure_yecai_receipt_schema(&state.1)
            .await
            .map_err(|e| {
                std::io::Error::other(format!("WZ yecai receipt schema self-heal failed: {e}"))
            })?;
        wz_service_accounts_receivable::db_init::ensure_yecai_claim_schema(&state.1)
            .await
            .map_err(|e| {
                std::io::Error::other(format!("WZ yecai claim schema self-heal failed: {e}"))
            })?;
        common::telemetry::info!(
            "WZ yecai schema self-heal passed (invoice-sync + receipt-sync tables ready)"
        );
    }

    let metrics = Arc::new(Metrics::new());
    let extension_registry = load_extensions(&state.1).await;

    // 文件存储公共服务状态：从活库 `isahl.zc_id_prot-oss_config` 构造后端
    // （settings 内嵌 enabled/is_default；local/s3/oss scheme 路由；
    //  namespace 隔离 + 行级授权见 api/files）。
    let files_state =
        Arc::new(alioth_gateway::api::files::FilesState::from_live_db(state.1.clone()).await);

    // 应用上下文标识：扩展感知 CRUD handler 通过 `web::Data<AppContext>` 提取
    // 当前应用代码，用于在 AppExtensionRegistry 中按 app_code 匹配扩展。
    // Gateway 实例绑定单一 namespace/app 作用域，app_code 优先级：
    //   GATEWAY_APP_CODE > NAMESPACE > "default"
    let gateway_app_code = std::env::var("GATEWAY_APP_CODE")
        .ok()
        .or_else(|| std::env::var("NAMESPACE").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_string());
    let app_context = AppContext {
        app_code: gateway_app_code.clone(),
    };
    common::telemetry::info!("AppContext injected for app_code='{}'", gateway_app_code);

    // 初始化 Pre-Proc 应用发现（懒加载：启动时不扫描，首次访问时自动扫描）
    // NAMESPACE 控制当前 Gateway 实例绑定的 namespace（每个实例只加载一个 namespace 的 App）
    // 消费方（preproc-proxy 反代 / standalone /auth/me）编译时才构造；
    // sso + 无 proxy 的生产构建无消费方 → 不构造，避免 dead_code 告警。
    // gateway_namespace 仅此处使用（avic-caasec 订阅改直接读 env，见下），故一并移入。
    #[cfg(any(
        feature = "preproc-proxy",
        not(any(feature = "sso", feature = "sso-remote"))
    ))]
    let preproc_data = {
        let gateway_namespace = std::env::var("NAMESPACE").ok();
        let preproc_path = resolve_preproc_path();
        let preproc_discovery = PreprocDiscovery::new(&preproc_path, gateway_namespace.clone());
        common::telemetry::info!(
            "Pre-Proc discovery initialized (lazy): base_path={}, namespace={:?}",
            preproc_path,
            gateway_namespace
        );
        web::Data::new(Mutex::new(preproc_discovery))
    };
    let i18n_manager = alioth_gateway::i18n::init_i18n_manager();

    // 初始化事件总线（factor handler 通过 publish/subscribe 通信）
    let event_bus: Arc<dyn common::event_bus::DomainEventBus> =
        Arc::new(common::event_bus::InMemoryEventBus::new());
    let event_bus_data: web::Data<Arc<dyn common::event_bus::DomainEventBus>> =
        web::Data::new(event_bus.clone());

    // ── 业务定时器（framework-scheduler 统一调度，消费 zc_id_plan.cron）────────
    // 原自建轮询（sla_timeout / spawn_*_poller）全部收编为调度计划 handler：
    // approval-sla-timeout / fssc-payment-status-sync / fssc-audit-compensate /
    // wz-demurrage-check / shared-flow-sync（计划种子见 seed-wz-scheduler-plans.sql）。
    // 需在事件总线就绪后装配（SLA handler 依赖 bus 发布 ApprovalCompleted）。
    let scheduler =
        std::sync::Arc::new(framework_scheduler::SchedulerService::new(state.1.clone()));
    // 审批自动触发（fix-flow-designer-runtime-chain 遗留项①）：业务实体
    // （三域叶表行）创建 → EntityCreated 事件 → 绑定范畴流程自动发起
    approval::handlers::auto_initiate::subscribe_auto_initiate(event_bus.clone(), state.1.clone());
    // SLA 超时自动驳回（D6：驳回复用 reject 链路并发布 ApprovalCompleted）
    // D7 升级通知（fix-approval-engine-semantics）：注入消息服务——超时驳回后
    // 向 admin UA 成员投递升级通知；失败仅 warn 不阻断驳回主流程。
    let sla_messaging: std::sync::Arc<dyn common::messaging::MessagingService> =
        std::sync::Arc::new(
            alioth_gateway::notification::db_messaging::DbMessagingService::new(state.1.clone()),
        );
    scheduler
        .register(std::sync::Arc::new(
            approval::sla_timeout::ApprovalSlaHandler::new(state.1.clone(), event_bus.clone())
                .with_messaging(sla_messaging.clone()),
        ))
        .await;
    // 审批抄送通知消费（fix-approval-engine-gap-closure D9）：消费引擎推进 cc 节点
    // 发布的 ApprovalCc——resolvedUsers 逐人站内信（messaging 复用 SLA 注入实例）；
    // resolvedUsers 空（legacy 文本收件人）warn 跳过，投递失败不阻断发布方。
    approval::handlers::cc_notify::subscribe_cc_notify(
        event_bus.clone(),
        state.1.clone(),
        Some(sla_messaging),
    );
    // 日程提醒（S1：plan comments reminder_offset_min 到点站内信，schedule-reminder 计划）
    scheduler
        .register(std::sync::Arc::new(
            framework_schedule::ScheduleReminderHandler::new(state.1.clone()),
        ))
        .await;
    // 任务到期引擎（任务驱动侧：task qk_period 到期 → 操作痕迹 + 站内信提醒）
    scheduler
        .register(std::sync::Arc::new(
            framework_scheduler::TaskDeadlineHandler::new(state.1.clone()),
        ))
        .await;
    // 库存物化视图周期自动刷新（校准兜底：业务写路径显式刷新 + 周期兜底，消除判定/展示分叉）
    scheduler
        .register(std::sync::Arc::new(
            framework_scheduler::MvInventoryRefreshHandler::new(state.1.clone()),
        ))
        .await;
    #[cfg(feature = "wz")]
    {
        // FSSC 付款状态同步 + 应付审核补偿（accounts-payable）
        scheduler
            .register(std::sync::Arc::new(
                wz_service_accounts_payable::FsscPaymentStatusHandler::new(state.1.clone()),
            ))
            .await;
        scheduler
            .register(std::sync::Arc::new(
                wz_service_accounts_payable::FsscAuditCompHandler::new(state.1.clone()),
            ))
            .await;
        // 共享流水同步（accounts-receivable）
        scheduler
            .register(std::sync::Arc::new(
                wz_service_accounts_receivable::SharedFlowHandler::new(state.1.clone()),
            ))
            .await;
        // WZ 压车费自动计核（transport-operations）
        scheduler
            .register(std::sync::Arc::new(
                wz_service_transport_operations::handlers::demurrage::DemurrageHandler::new(
                    state.1.clone(),
                ),
            ))
            .await;
    }
    scheduler.start(60);
    common::telemetry::info!(
        "framework-scheduler started（5 个业务定时计划：SLA/FSSC×2/压车费/共享流水）"
    );
    common::telemetry::info!(
        "DomainEventBus: InMemoryEventBus (module.json event declarations no longer loaded)"
    );
    let (_messaging_service, _cluster_service, _player_service) = ((), (), ());

    // SLA 超时监控已收编至 framework-scheduler（init_framework 的 scheduler 注册块，
    // plan_code=approval-sla-timeout）；事件总线就绪后由调度循环驱动自动驳回，
    // D6 链路不变（驳回复用 reject 链路并发布 ApprovalCompleted）。

    // ── 领域事件订阅装配（PAGE-适航-07 闭环）────────────────────────
    // auto_service_registry 只注册 HTTP 路由；事件订阅需手动装配。
    // avic-caasec namespace 下，airworthiness 订阅 ApprovalCompleted →
    // 审批结果驱动适航证件状态流转。pool 在 state.1 中，此处延迟到
    // server 闭包内（state.1 已就绪）装配，避免与路由注册顺序耦合。
    #[cfg(feature = "avic-caasec-service-airworthiness")]
    if std::env::var("NAMESPACE").as_deref() == Ok("AVIC-CAASEC") {
        let bus_for_events = event_bus.clone();
        let pool_for_events = state.1.clone();
        actix_web::rt::spawn(async move {
            // 等 state 完全就绪（短延迟避免启动竞态；事件为幂等重放，漏单可重试）
            actix_web::rt::time::sleep(std::time::Duration::from_millis(500)).await;
            avic_caasec_service_airworthiness::events::subscribe_airworthiness_events(
                bus_for_events,
                pool_for_events,
            );
        });
        common::telemetry::info!(
            "[events] airworthiness ApprovalCompleted 订阅已装配（AVIC-CAASEC）"
        );
    }

    // avic-caasec namespace 下，monitor 订阅 ApprovalCompleted →
    // 审批结果驱动项目门禁状态与阶段推进（D1 闭环）。与 airworthiness
    // 装配点并列，同 feature（avic-caasec-service-monitor）条件。
    #[cfg(feature = "avic-caasec-service-monitor")]
    if std::env::var("NAMESPACE").as_deref() == Ok("AVIC-CAASEC") {
        let bus_for_events = event_bus.clone();
        let pool_for_events = state.1.clone();
        actix_web::rt::spawn(async move {
            // 等 state 完全就绪（短延迟避免启动竞态；事件为幂等重放，漏单可重试）
            actix_web::rt::time::sleep(std::time::Duration::from_millis(500)).await;
            avic_caasec_service_monitor::events::subscribe_monitor_events(
                bus_for_events,
                pool_for_events,
            );
        });
        common::telemetry::info!("[events] monitor ApprovalCompleted 订阅已装配（AVIC-CAASEC）");
    }

    // wz namespace 下，contract 订阅 ApprovalCompleted →
    // 审批结果驱动合同状态桥（wire-contract-approval-engine）：
    // submit 创建审批实例（FLOW-CONTRACT 法务→总经理），终态事件回写
    // 合同 active/draft。与 AVIC 装配点并列，feature（wz-service-contract）条件。
    #[cfg(feature = "wz-service-contract")]
    if std::env::var("NAMESPACE").as_deref() == Ok("WZ") {
        let bus_for_events = event_bus.clone();
        let pool_for_events = state.1.clone();
        actix_web::rt::spawn(async move {
            // 等 state 完全就绪（短延迟避免启动竞态；事件为幂等重放，漏单可重试）
            actix_web::rt::time::sleep(std::time::Duration::from_millis(500)).await;
            wz_service_contract::events::subscribe_contract_events(bus_for_events, pool_for_events);
        });
        common::telemetry::info!("[events] contract ApprovalCompleted 订阅已装配（WZ）");
    }

    // wz namespace 下，employee-onboarding 审批闭环订阅 ApprovalCompleted →
    // 审批结果驱动雇员入职副作用（G4 收口）：通过 → 员工创建/UA 指派/profile/
    // 用户激活；驳回 → 用户禁用。原 approve()/reject() 内联同步副作用已迁入
    // approval::handlers::employee_onboarding，审批 handler 回归纯发布端。
    // 与 contract 装配点并列（广播语义：多个订阅者各自 receiver，无冲突），
    // feature（wz-service-contract）条件。
    #[cfg(feature = "wz-service-contract")]
    if std::env::var("NAMESPACE").as_deref() == Ok("WZ") {
        let bus_for_events = event_bus.clone();
        let pool_for_events = state.1.clone();
        actix_web::rt::spawn(async move {
            // 等 state 完全就绪（短延迟避免启动竞态；事件为幂等重放，漏单可重试）
            actix_web::rt::time::sleep(std::time::Duration::from_millis(500)).await;
            approval::handlers::employee_onboarding::subscribe_employee_onboarding_events(
                bus_for_events,
                pool_for_events,
            );
        });
        common::telemetry::info!("[events] employee-onboarding ApprovalCompleted 订阅已装配（WZ）");
    }

    // Cosmic-Tools namespace 下，ct-git 订阅 ApprovalCompleted →
    // 审批结果驱动版本固化落库（verctrl freeze 事务：通过回调写版本、驳回丢弃，
    // add-ct-git-vc-interop）。与 WZ 装配点并列，feature
    // （cosmic-tools-service-ct-git）条件。
    #[cfg(feature = "cosmic-tools-service-ct-git")]
    if std::env::var("NAMESPACE").as_deref() == Ok("Cosmic-Tools") {
        let bus_for_events = event_bus.clone();
        let pool_for_events = state.1.clone();
        actix_web::rt::spawn(async move {
            // 等 state 完全就绪（短延迟避免启动竞态；事件为幂等重放，漏单可重试）
            actix_web::rt::time::sleep(std::time::Duration::from_millis(500)).await;
            cosmic_tools_service_ct_git::events::subscribe_verctrl_events(
                bus_for_events,
                pool_for_events,
            );
        });
        common::telemetry::info!("[events] verctrl ApprovalCompleted 订阅已装配（Cosmic-Tools）");
    }

    // ── 本体坐标缓存初始化（FUNC-001/NFR-008 运行时关联）────────────
    // 维度表（zc_id_scene/factor/function）code→id 全量加载到内存，
    // 服务 CRUD 按 code 解析 dk 坐标（而非硬编码 ID），新增维度自动感知。
    {
        let pool = state.1.clone();
        actix_web::rt::spawn(async move {
            actix_web::rt::time::sleep(std::time::Duration::from_millis(300)).await;
            match common::ontology::OntologyCache::init(&pool).await {
                Ok(()) => common::telemetry::info!(
                    "[ontology] OntologyCache 初始化完成（scene/factor/function code→id）"
                ),
                Err(e) => common::telemetry::error!(
                    "[ontology] OntologyCache 初始化失败: {e}（dk 按 code 解析将不可用）"
                ),
            }
        });
    }

    // 初始化 DB 消息服务（数据变更自动写入站内信）
    let db_messaging: Arc<dyn common::messaging::MessagingService> = Arc::new(
        alioth_gateway::notification::db_messaging::DbMessagingService::new(state.1.clone()),
    );
    let notification_service =
        NotificationService::new(state.1.clone(), Some(db_messaging.clone()));
    alioth_gateway::trigger_crud::set_notification_service(notification_service.clone());
    common::telemetry::info!("NotificationService initialized: data change subscriptions ready");
    // === Auth 初始化 ===
    #[cfg(feature = "sso")]
    let sso_config = gateway_sso::Config::from_env()
        .map_err(|e| std::io::Error::other(format!("SSO config: {e}")))?;
    // 内嵌 sso 模式 issuer 护栏（与下方 sso-remote 硬校验同类）：
    // SSO 签发端用 OIDC_ISSUER（config.oidc_issuer）盖章 iss/aud（SSO lib.rs
    // configure_token_validation），Gateway PEP 用 SSO_JWT_ISSUER 校验 iss/aud 绑定。
    // 两者不一致 → 所有 JWT 静默 401（ES256 迁移后无对称密钥兜底）。启动即失败，防止带病上线。
    #[cfg(feature = "sso")]
    if state.0.sso_jwt_issuer != sso_config.oidc_issuer {
        return Err(std::io::Error::other(format!(
            "内嵌 sso 模式：SSO_JWT_ISSUER（{}）与 SSO OIDC_ISSUER（{}）不一致，\
             JWT iss/aud 绑定静默不匹配（全部请求 401）；请同步配置，\
             或两者均不显式设置恢复默认 http://localhost:9002",
            state.0.sso_jwt_issuer, sso_config.oidc_issuer
        )));
    }
    #[cfg(feature = "sso")]
    let sso_jwt_private_key = sso_config.sso_jwt_private_key.as_bytes().to_vec();
    #[cfg(feature = "sso")]
    let sso_jwt_public_key = derive_public_key(&sso_jwt_private_key)
        .map_err(|e| std::io::Error::other(format!("SSO JWT key: {e}")))?;
    #[cfg(feature = "sso")]
    let sso_jwt_public_keys_prev = match &sso_config.sso_jwt_public_key_prev {
        Some(pem) => {
            let pem_bytes = pem.as_bytes().to_vec();
            let kid = gateway_sso::auth::jwt::public_key_kid(&pem_bytes)
                .map_err(|e| std::io::Error::other(format!("SSO JWT prev key kid: {e}")))?;
            vec![(kid, pem_bytes)]
        }
        None => vec![],
    };
    #[cfg(feature = "sso")]
    let sso_auth_state = gateway_sso::AuthState {
        jwt_private_key: sso_jwt_private_key,
        jwt_public_key: sso_jwt_public_key,
        jwt_public_keys_prev: sso_jwt_public_keys_prev,
        encryption_key: sso_config.encryption_key.as_bytes().to_vec(),
        ngac_preview_dir: sso_config.ngac_preview_dir.clone(),
        jwt_access_expiry_secs: sso_config.jwt_access_expiry,
        jwt_refresh_expiry_secs: sso_config.jwt_refresh_expiry,
        identity_verify_mode: sso_config.identity_verify_mode.clone(),
        identity_external_verify_url: sso_config.identity_external_verify_url.clone(),
    };
    #[cfg(feature = "sso")]
    let sso_ws_app_state = web::Data::new(gateway_sso::websocket::AppState::new());
    #[cfg(feature = "sso")]
    let sso_pool = state.1.clone();
    #[cfg(feature = "sso")]
    let sso_config_data = web::Data::new(sso_config);
    #[cfg(feature = "sso")]
    let sso_auth_state_data = web::Data::new(sso_auth_state);
    #[cfg(feature = "sso")]
    let sso_email_data =
        web::Data::new(Box::new(common::SmtpEmailService::new(sso_pool.clone()))
            as Box<dyn common::EmailService>);
    #[cfg(feature = "sso")]
    let sso_sms_data = web::Data::new(
        Box::new(common::CloudSmsService::new(sso_pool)) as Box<dyn common::SmsService>
    );
    #[cfg(feature = "sso")]
    let sso_oauth_data = web::Data::new(gateway_sso::auth::oauth_callback::OAuthAuthState {
        jwt_private_key: sso_auth_state_data.jwt_private_key.clone(),
        jwt_public_key: sso_auth_state_data.jwt_public_key.clone(),
        jwt_access_expiry_secs: sso_auth_state_data.jwt_access_expiry_secs,
        jwt_refresh_expiry_secs: sso_auth_state_data.jwt_refresh_expiry_secs,
    });
    #[cfg(feature = "sso")]
    {
        common::telemetry::info!("SSO module initialized (embedded in Gateway)");
    }
    #[cfg(not(any(feature = "sso", feature = "sso-remote")))]
    {
        init_standalone_auth();
        common::telemetry::info!("Standalone auth initialized (no SSO dependency)");
    }
    #[cfg(all(feature = "sso-remote", not(feature = "sso")))]
    {
        if state.0.sso_service_url.is_empty() {
            return Err(std::io::Error::other(
                "sso-remote 模式需要设置 SSO_SERVICE_URL 指向独立部署的 SSO 服务",
            ));
        }
        // 远程 SSO 的 oidc_issuer 通常不等于默认值；若沿用 localhost:9002 会导致
        // Gateway PEP 验签时 iss/aud 不匹配，所有 JWT 直接 401。强制显式配置。
        if state.0.sso_jwt_issuer.is_empty() || state.0.sso_jwt_issuer == "http://localhost:9002" {
            return Err(std::io::Error::other(
                "sso-remote 模式必须设置 SSO_JWT_ISSUER 为远程 SSO 的 oidc_issuer，\
                 不能是默认值 http://localhost:9002（否则 JWT iss/aud 校验失败）",
            ));
        }
        common::telemetry::info!(
            "SSO remote delegation mode: PDP/JWT 校验委托给 {}",
            state.0.sso_service_url
        );
    }

    common::telemetry::info!("Gateway Server listening on {}", server_addr);

    // ADR D-010：审计 outbox worker 内嵌（业务运行时侧；独立事务批量转写
    // data_change_logs，崩溃/失败由 outbox 持久行重放兜底）
    let _audit_worker_shutdown = crud::audit_outbox::spawn_worker(state.1.clone());

    // OpenAPI 计量/配额/限流中间件：在 HttpServer::new 前构造一次（跨 worker 共享
    // rate_buckets 与订阅缓存；计量 worker 仅 spawn 一次），闭包内 clone。
    let openapi_metering =
        alioth_gateway::openapi::metering::ApiUsageMiddleware::new(state.1.clone());
    // OpenAPI 幂等键中间件：第三方写请求（POST/PUT/PATCH /api/service/* + Idempotency-Key）
    // 服务端幂等快照重放。注册在 metering 之后（内层）——重放请求先经 metering 计量，
    // 再命中幂等快照直接返回（重放仍计入用量与配额，对齐 idempotency.rs 文档）。
    let openapi_idempotency =
        alioth_gateway::openapi::idempotency::IdempotencyMiddleware::new(state.1.clone());

    // 构建 HTTP 服务器
    let server = HttpServer::new(move || {
        let cors = common::build_cors()
            .expect("Failed to build CORS configuration");

        let app = App::new()
            // Layer 9: 安全响应头
            .wrap(
                actix_web::middleware::DefaultHeaders::new()
                    .add((header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
                    .add((header::X_FRAME_OPTIONS, "DENY"))
                    .add((header::CACHE_CONTROL, "no-store, no-cache, must-revalidate"))
                    .add(("Content-Security-Policy", "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'"))
                    .add(("Referrer-Policy", "strict-origin-when-cross-origin"))
                    .add(("Permissions-Policy", "geolocation=(), microphone=(), camera=()"))
            )
            .wrap(cors)
            .wrap(Logger::default())
            .wrap(LocaleMiddleware::new())
            .app_data(web::Data::new(state.1.clone()))
            .app_data(web::Data::new(state.0.clone()))
            .app_data(web::Data::new(state.2.clone()))
            .app_data(web::Data::new(metrics.clone()))
            .app_data(web::Data::new(i18n_manager.clone()))
            .app_data(web::Data::new(extension_registry.clone()))
            .app_data(web::Data::new(app_context.clone()))
            .app_data(web::Data::new(notification_service.clone()))
            .app_data(web::Data::new(db_messaging.clone()))
            // 批注（用户验证附件上传发现）：files_state 是 Arc<FilesState>，
            // handler 提取 web::Data<FilesState>——需解 Arc 注入（原 Data<Arc<FilesState>> 不匹配）
            .app_data(web::Data::new((*files_state).clone()))            .app_data(event_bus_data.clone())
            // 文件上传 body 上限：handler 内 20MB 检查优先返回友好 400——
            // actix 默认 2MB payload limit 会把 >2MB 文件（PDF/word）在 handler 前断连
            .app_data(web::PayloadConfig::new(22 * 1024 * 1024));

        // SSO embedded app_data (SSO mode only)
        #[cfg(feature = "sso")]
        let app = app
            .app_data(sso_config_data.clone())
            .app_data(sso_auth_state_data.clone())
            .app_data(sso_ws_app_state.clone())
            .app_data(sso_email_data.clone())
            .app_data(sso_sms_data.clone())
            .app_data(sso_oauth_data.clone());

        // 每个 worker 创建独立的 awc::Client（awc::Client 非 Send，无法跨线程共享）。
        // 仅供 preproc-proxy 反代使用；feature 关闭时无需创建。
        #[cfg(feature = "preproc-proxy")]
        let http_client = web::Data::new(awc::Client::new());
        // ── 公共中间件 — 限流、健康检查 ──
        let app = app
            .wrap(common::RateLimitMiddleware::per_ip_any(&[
                "/auth/register", "/api/auth/register"
            ], 5.0, 5.0 / 60.0))
            .wrap(common::RateLimitMiddleware::per_ip_any(&[
                "/auth/login", "/api/auth/login"
            ], 10.0, 10.0 / 60.0))
            .wrap(common::RateLimitMiddleware::per_ip_any(&[
                "/auth/identity/submit", "/api/auth/identity/submit"
            ], 3.0, 3.0 / 3600.0))
            .wrap(common::RateLimitMiddleware::per_ip_any(&[
                "/auth/identity/verify", "/api/auth/identity/verify"
            ], 10.0, 10.0 / 3600.0))
            .wrap(common::RateLimitMiddleware::per_ip_any(&[
                "/auth/email/send-code", "/api/auth/email/send-code"
            ], 3.0, 3.0 / 3600.0))
            .wrap(common::RateLimitMiddleware::per_ip_any(&[
                "/auth/phone/send-code", "/api/auth/phone/send-code"
            ], 3.0, 3.0 / 3600.0))
            .wrap(common::RateLimitMiddleware::per_ip_any(&[
                "/auth/oauth/login", "/api/auth/oauth/login"
            ], 20.0, 20.0 / 3600.0))
            // 文件上传防滥用（SECURITY_SPEC §4：20 req/min/user；JWT sub 为 key，
            // 无 token 回退 IP——per_user 变体见 common::middleware）
            .wrap(common::RateLimitMiddleware::per_user_any(&[
                "/api/files"
            ], 20.0, 20.0 / 60.0))
            // 健康检查 (公开)
            .route("/health", web::get().to(health_check))
            // Prometheus metrics endpoint (公开)
            .route("/metrics", web::get().to(metrics_handler))
            // 系统信息 (公开运维端点，remove-public-whitelist 显式豁免)
            .route("/system", web::get().to(system_check));

        // ── 认证路由 ──
        #[cfg(feature = "sso")]
        let app = app
            // SSO 认证服务 — 外部公开路由（JWT 可选或公开）
            .configure(gateway_sso::configure_public_routes)
            // JWKS 端点：Gateway 内嵌 SSO 时，PEP 的 JWKS 客户端从本端点获取公钥
            .route("/.well-known/jwks.json", web::get().to(gateway_sso::auth::jwt::jwks));
        #[cfg(not(any(feature = "sso", feature = "sso-remote")))]
        let app = app
            // Standalone 认证服务 — ES256 JWT + isahl_auth.standalone_users
            .configure(configure_standalone_auth);

        // ── 公共路由 — 应用发现 ──
        let app = app
            // /api/apps 必须在所有 web::scope("/api") 之前注册，
            // 否则 scope 的 prefix 匹配会先捕获并返回 404
            .route("/api/apps", web::get().to(alioth_gateway::apps::list_apps))
            // /api/apps/routes 必须在所有 web::scope("/api") 之前注册，
            // 否则 scope 的 prefix 匹配会先捕获并返回 404
            .route("/api/apps/routes", web::get().to(alioth_gateway::apps::get_active_routes))
            // /api/apps/overrides 公开访问，返回 app 能力覆盖配置
            .route("/api/apps/overrides", web::get().to(alioth_gateway::apps::get_app_overrides));

        // Pre-Proc 发现数据（standalone /auth/me 依赖；必须放在 web::scope("/api")
        // 之前，否则被 SSO /api scope prefix 捕获）。消费方编译时才注册。
        #[cfg(any(feature = "preproc-proxy", not(any(feature = "sso", feature = "sso-remote"))))]
        let app = app.app_data(preproc_data.clone());

        // Pre-Proc 未认证透明反代（/preproc/* 与 /api/pre_proc/*）——dev 形态专用，
        // 由 feature preproc-proxy 门控：生产构建（build-ns.sh --no-default-features，
        // 不含本 feature）完全不注册这些路由 → 404（而非 401），杜绝未认证代理泄漏到生产。
        #[cfg(feature = "preproc-proxy")]
        let app = app
            .app_data(http_client.clone())
            .configure(configure_preproc_routes);

        // ── /api/auth 前缀别名 ──
        #[cfg(feature = "sso")]
        let app = app
            // SSO 认证服务 — /api/auth 前缀别名
            .service(
                web::scope("/api/auth")
                    .configure(gateway_sso::configure_public_routes_without_scope)
            );
        #[cfg(not(any(feature = "sso", feature = "sso-remote")))]
        let app = app
            // Standalone 认证服务 — /api/auth 前缀别名
            .service(
                web::scope("/api/auth")
                    .configure(configure_standalone_auth_without_scope)
            );

        // ── sso-remote 认证反代 ──
        // Gateway 不内嵌 SSO，将 /api/auth、/api/ngac 透明转发到 SSO_SERVICE_URL。
        // 必须在下方 /api PEP scope 之前注册，使这些前缀优先命中反代而非进入 PEP。
        #[cfg(feature = "sso-remote")]
        let app = {
            let proxy_client = web::Data::new(reqwest::Client::new());
            let proxy_base = web::Data::new(state.0.sso_service_url.clone());
            app
                .app_data(proxy_client.clone())
                .app_data(proxy_base.clone())
                .service(web::scope("/api/auth").configure(sso_remote_proxy::configure))
                .service(web::scope("/api/ngac").configure(sso_remote_proxy::configure))
        };
        // ── NL 辅助提案端点（refactor-ngac-admin-nl-graph；proposal-only）──
        // MUST 注册在 configure_protected_routes 之前：actix scope 前缀先注册先得，
        // 否则被 SSO 的 /api/admin scope 吞掉（scope 内无匹配即 404，不回落）。
        #[cfg(feature = "sso")]
        let app = app.configure(admin_ngac_assist::configure);

        // ── NGAC PDP/PIP 路由（SSO only） ──
        #[cfg(feature = "sso")]
        let app = app
            .configure(gateway_sso::configure_protected_routes);

        // JWT iss/aud 绑定（PEP 校验令牌签发方）：
        // - SSO/sso-remote：读 SSO_JWT_ISSUER（默认 http://localhost:9002，与 SSO oidc_issuer 一致）
        // - standalone：固定 STANDALONE_ISSUER（"gateway-standalone"）——与 standalone_auth
        //   签发值对齐（单一事实源）；否则 PEP 强制默认 issuer 导致 standalone 令牌全 401
        //   （历史死锁，见 openspec change fix-gateway-proxy-standalone-auth standalone-auth spec）。
        #[cfg(any(feature = "sso", feature = "sso-remote"))]
        let jwt_issuer = state.0.sso_jwt_issuer.clone();
        #[cfg(not(any(feature = "sso", feature = "sso-remote")))]
        let jwt_issuer = standalone_auth::STANDALONE_ISSUER.to_string();

        // ── 受保护路由 — 应用 PEP 中间件 ──
        // 资源注册表按 namespace 扩展：WZ 的 service 路由为 2 段风格
        // （/service/{entity}/...），注册 WZ 实体后 resolve 才正确解析
        // /service/contracts/create → contracts:0（而非 create:0）。
        let ns_registry = {
            let ns = std::env::var("NAMESPACE").unwrap_or_default();
            let reg = ngac_contract::ResourceRegistry::new().with_alioth_defaults();
            let reg = if ns.eq_ignore_ascii_case("WZ") {
                reg.with_wz_defaults()
            } else {
                reg
            };
            // Cosmic-Tools：ct-git 版本控制资源（verctrl/ver_branch 恒定资源，
            // string_id 集合判定，OA 由 ngac_seed 预置）
            if ns.eq_ignore_ascii_case("Cosmic-Tools") {
                reg.with_cosmic_tools_defaults()
            } else {
                reg
            }
        };

        app
            .service(
                web::scope("/api")
                    .wrap(
                        NgacEnforcer::new(
                            state.1.clone(),
                            state.2.clone(),
                            state.0.sso_jwt_public_key_prev.clone(),
                            if cfg!(any(feature = "sso", feature = "sso-remote")) { state.0.sso_service_url.clone() } else { String::new() },
                        )
                        .with_resource_registry(ns_registry)
                        .with_token_binding(jwt_issuer.clone(), jwt_issuer.clone())
                        .with_public_noauth_paths([
                            "/api/auth".to_string(),
                            "/api/ngac".to_string(),
                            // FSSC 外部回调无 JWT，服务侧以 X-FSSC-Callback-Key 共享密钥补偿校验（design D11）
                            // 仅放行两个回调子路径：GET /fssc-callbacks（回调历史查询）需 JWT+NGAC
                            "/api/service/accounts-payable/fssc-callbacks/audit".to_string(),
                            "/api/service/accounts-payable/fssc-callbacks/ocr".to_string(),
                        ].into())
                    )
                    .wrap(MetricsMiddleware)
                    // OpenAPI 计量/配额/per-plan 限流（P2）：服务令牌请求记录
                    // api_usage + 订阅状态强制（Gap A）+ per-plan 限流（Gap B）
                    // + 日/月配额检查（超限 429）。PEP 内层：仅处理已认证请求。
                    // 实例在 HttpServer::new 前构造（跨 worker 共享桶/缓存）。
                    .wrap(openapi_metering.clone())
                    // OpenAPI 幂等键（P2）：第三方写请求 Idempotency-Key 服务端幂等，
                    // 同 key 重放返回首次响应快照。metering 内层——重放请求仍计量
                    // （replay 是被服务的 API 调用，计入用量与配额）。
                    .wrap(openapi_idempotency.clone())
                    // Framework AI 聊天服务
                    .configure(chat_sessions::configure_routes)
                    // 通用法律本体检索（EmpAgent 上下文增强，所有 namespace）
                    .configure(legal_search::configure_routes)
                    .configure(standard_search::configure_routes)
                    // 系统推送服务（站内信 + 设备推送）
                    .configure(system_push::configure_routes)
                    // 全局工作区概览（审批 + 消息聚合）
                    .configure(global_overview::configure_routes)
                    // 审批操作（批准/拒绝）
                    .configure(approvals::configure_routes)
                    // 公式 AI 生成与模拟执行（formula-assist / expr-simulate）
                    .configure(approval_formula::configure_routes)
                    // 注册后主体绑定（个人自然人/企业法人 + 引导）
                    .configure(entity_binding::configure_routes)
                    // 用户订阅与通知服务
                    .configure(alioth_gateway::notification::configure_routes)
                    // 日程管理服务（schedule handlers 已内置 /schedule scope）
                    .configure(configure_schedule_routes)
                    // 系统配置服务（system-config handlers 已内置 /system-config scope）
                    // 站内信操作（标记已读/删除）
                    .configure(inbox::configure_routes)
                    // 文件存储公共服务（namespace 级：上传/下载/列表/删除）
                    .configure(alioth_gateway::api::files::configure_routes)
                    .configure(dashboard::configure_routes)
                    // 个人中心（NGAC 权限信息）
                    .configure(profile::configure_routes)
                    .configure(contacts::configure_routes)
                    .configure(|cfg| register_system_config_routes(cfg, state.1.clone()))
                    // Factor 路由（按 NAMESPACE 自动注册）
                    .configure(service_registry::register_service_routes)
                    // OpenAPI 数据服务产品（namespace 级通用 backend，位于
                    // {ns}/openapi/，不属 Sources/Services——不经 service_registry
                    // 扫描，此处显式注册；feature-gated，非 alioth 组 no-op）
                    .configure(register_openapi_routes)
                    // OpenAPI 文档 + Swagger UI playground + 用量报告（须认证，经 PEP）
                    .configure(|cfg| alioth_gateway::openapi::configure_routes(cfg, state.1.clone()))
            )
    })
    .bind(&server_addr)
    .map_err(|e| common::server::bind_error(&server_addr, e))?;

    // 启动服务器（非阻塞，获取 server handle）
    let server = server.run();
    let server_handle = server.handle();

    // 后台任务：监听退出信号并执行优雅关闭
    tokio::spawn(async move {
        // 等待 SIGINT (Ctrl+C) 或 SIGTERM
        signal::ctrl_c().await.ok();
        common::telemetry::info!("Received shutdown signal, starting graceful shutdown...");

        // 1) 停止接受新请求，等待 in-flight 请求完成（最长 30 秒）
        server_handle.stop(true).await;
        common::telemetry::info!("HTTP server stopped accepting new requests.");

        // 3) ——

        common::telemetry::info!("Gateway shutdown complete.");
        std::process::exit(0);
    });

    // 阻塞等待服务器运行结束
    server.await
}

async fn health_check() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({"status": "ok"}))
}

async fn metrics_handler(metrics: web::Data<Arc<Metrics>>) -> HttpResponse {
    let encoder = prometheus::TextEncoder::new();
    let metric_families = metrics.registry.gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    HttpResponse::Ok()
        .content_type("text/plain; version=0.0.4")
        .body(buffer)
}

/// 系统信息端点（/system，运维豁免——PEP 外，无 JWT 可访问）。
/// 仅返回非敏感信息：namespace / 版本 / uptime / 启动时间（不暴露配置与凭据）。
async fn system_check() -> HttpResponse {
    let namespace = std::env::var("NAMESPACE").unwrap_or_default();
    let started_epoch = STARTED_AT.load(std::sync::atomic::Ordering::Relaxed);
    let now_epoch = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let uptime_secs = now_epoch.saturating_sub(started_epoch);
    let started_iso = chrono::DateTime::<chrono::Utc>::from_timestamp(started_epoch as i64, 0)
        .map(|t| t.to_rfc3339())
        .unwrap_or_default();
    HttpResponse::Ok().json(serde_json::json!({
        "status": "ok",
        "namespace": namespace,
        "version": env!("CARGO_PKG_VERSION"),
        "uptimeSeconds": uptime_secs,
        "startedAt": started_iso,
    }))
}

/// 进程启动时间（system_check 用；main 开头设置）
static STARTED_AT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 侦测 log_event 分区状态，若未分表则自动调用 create_monthly_partition() 补充
async fn ensure_log_event_partitions(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    let has_partition: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM pg_inherits 
            WHERE inhparent = 'isahl_audit.log_event'::regclass
        )
        "#,
    )
    .fetch_one(pool)
    .await?;

    if !has_partition {
        common::telemetry::info!(
            "No partitions found for isahl_audit.log_event, creating current month partition..."
        );
        sqlx::query("SELECT isahl_audit.create_monthly_partition()")
            .execute(pool)
            .await?;
        common::telemetry::info!("log_event partition created successfully");
    } else {
        common::telemetry::info!("isahl_audit.log_event partitions already exist");
    }
    Ok(())
}
