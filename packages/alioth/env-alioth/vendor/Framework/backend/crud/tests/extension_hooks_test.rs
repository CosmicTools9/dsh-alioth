//! 扩展钩子经**标准 CRUD 路由**执行的回归测试。
//!
//! 背景（2026-09-23 集成缺口复核）：`extensions/*.yaml` 四类声明在 Gateway 启动期加载并注册，
//! 但生产路由全部走 `crud_routes` / `crud_routes_with_refs`，而钩子只挂在零调用的
//! `crud_routes_with_extensions` 上 ⇒ **声明永不执行**。本测试固化修复后的契约：
//! 标准工厂 + 装配 `AppExtensionRegistry` ⇒ `before_*` 钩子执行、约束阻断 ⇒ 400。
//!
//! 前两条用例分别以**业务名**（`ENTITY_NAME`）与**物理表名**声明约束——两种既有声明口径
//! MUST 同时可解析（历史 YAML 用业务名，2026-09-22 起的 YAML 用表名）。
//!
//! 无 DB 依赖：`PgPool::connect_lazy`（不建连接），阻断路径在触库前返回；反例（无声明）落到
//! 仓储 ⇒ DB 不可达（5xx），据 status 区分「钩子未执行」与「钩子执行后放行」。

use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::{middleware::from_fn, test, web, App, Error, HttpMessage};
use common::error::AliothError;
use crud::{crud_routes, AliothDbEntity, GenericRepository, Identifiable};
use runtime_engine::{
    AppExtensionRegistry, AppLogicExtension, ConstraintExtension, ConstraintSeverity, RuleExtension,
};
use sqlx::PgPool;

#[derive(sqlx::FromRow, serde::Serialize, Clone)]
struct ExtProbe {
    id: i64,
    /// 行上下文探针：删除期钩子的约束求值 MUST 能看到**存量行**字段（不只是 `id`）
    #[sqlx(default)]
    amount: Option<i64>,
}

impl Identifiable for ExtProbe {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for ExtProbe {
    fn table_name() -> &'static str {
        r#"isahl."ext-probe""#
    }
    const SELECT_FIELDS: &'static str = "id";
    const ENTITY_NAME: &'static str = "ext_probe";
    const SOFT_DELETE: bool = true;
}

#[derive(serde::Deserialize, Debug)]
struct ExtProbeDto {
    amount: i64,
}

/// 严格 DTO（`deny_unknown_fields`）：扩展 mutation 写入未声明字段 ⇒ 反序列化期 fail-closed（400）
#[derive(serde::Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct StrictProbeDto {
    amount: i64,
}

/// 严格路由用仓储：请求应在**触库前**被反序列化拒绝 ⇒ 各方法不可达（触及即 panic = 断言失败信号）
#[derive(Clone)]
struct StrictProbeRepo;

impl From<PgPool> for StrictProbeRepo {
    fn from(_pool: PgPool) -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl crud::AliothRepository<ExtProbe, StrictProbeDto, StrictProbeDto, AliothError>
    for StrictProbeRepo
{
    async fn list(
        &self,
        _query: &crud::ListQuery,
    ) -> Result<crud::PaginatedResponse<ExtProbe>, AliothError> {
        unreachable!("严格 DTO：请求 MUST 在反序列化期被拒")
    }

    async fn get(&self, _id: i64) -> Result<Option<ExtProbe>, AliothError> {
        unreachable!("严格 DTO：请求 MUST 在反序列化期被拒")
    }

    async fn create(&self, req: StrictProbeDto, _user_id: i64) -> Result<ExtProbe, AliothError> {
        unreachable!(
            "严格 DTO：请求 MUST 在反序列化期被拒（amount={}）",
            req.amount
        )
    }

    async fn update(
        &self,
        _id: i64,
        _req: StrictProbeDto,
        _user_id: i64,
    ) -> Result<Option<ExtProbe>, AliothError> {
        unreachable!("严格 DTO：请求 MUST 在反序列化期被拒")
    }

    async fn delete(&self, _id: i64, _user_id: i64) -> Result<(), AliothError> {
        unreachable!("严格 DTO：请求 MUST 在反序列化期被拒")
    }
}

#[derive(Clone)]
struct ExtProbeRepo {
    inner: GenericRepository<ExtProbe>,
}

impl From<PgPool> for ExtProbeRepo {
    fn from(pool: PgPool) -> Self {
        Self {
            inner: GenericRepository::new(pool),
        }
    }
}

#[async_trait::async_trait]
impl crud::AliothRepository<ExtProbe, ExtProbeDto, ExtProbeDto, AliothError> for ExtProbeRepo {
    async fn list(
        &self,
        query: &crud::ListQuery,
    ) -> Result<crud::PaginatedResponse<ExtProbe>, AliothError> {
        self.inner.list(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<ExtProbe>, AliothError> {
        self.inner.get(id).await
    }

    async fn create(&self, req: ExtProbeDto, _user_id: i64) -> Result<ExtProbe, AliothError> {
        // 哨兵：放行证据（请求抵达仓储层）+ 回显收到的字段值（证明规则 mutation 是否落到写载荷）
        Err(AliothError::Internal(format!(
            "REPO_REACHED amount={}",
            req.amount
        )))
    }

    async fn update(
        &self,
        _id: i64,
        _req: ExtProbeDto,
        _user_id: i64,
    ) -> Result<Option<ExtProbe>, AliothError> {
        panic!("本测试不触发更新径")
    }

    async fn delete(&self, _id: i64, _user_id: i64) -> Result<(), AliothError> {
        panic!("本测试不触发删除径")
    }
}

/// 删除径用仓储：`get` 返回固定存量行（amount=500），删除写径以哨兵错误暴露「是否抵达仓储」
#[derive(Clone)]
struct DeleteProbeRepo;

impl From<PgPool> for DeleteProbeRepo {
    fn from(_pool: PgPool) -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl crud::AliothRepository<ExtProbe, ExtProbeDto, ExtProbeDto, AliothError> for DeleteProbeRepo {
    async fn list(
        &self,
        _query: &crud::ListQuery,
    ) -> Result<crud::PaginatedResponse<ExtProbe>, AliothError> {
        panic!("本测试不触列表径")
    }

    /// 存量行：删除期约束求值的判据字段（只给 `{id}` 的上下文无法求值 `amount`）
    async fn get(&self, id: i64) -> Result<Option<ExtProbe>, AliothError> {
        Ok(Some(ExtProbe {
            id,
            amount: Some(500),
        }))
    }

    async fn create(&self, _req: ExtProbeDto, _user_id: i64) -> Result<ExtProbe, AliothError> {
        panic!("本测试不触创建径")
    }

    async fn update(
        &self,
        _id: i64,
        _req: ExtProbeDto,
        _user_id: i64,
    ) -> Result<Option<ExtProbe>, AliothError> {
        panic!("本测试不触更新径")
    }

    async fn delete(&self, _id: i64, _user_id: i64) -> Result<(), AliothError> {
        Err(AliothError::Internal(
            "REPO_DELETE_REACHED single".to_string(),
        ))
    }

    async fn batch_delete(&self, ids: Vec<i64>, _user_id: i64) -> Result<(), AliothError> {
        Err(AliothError::Internal(format!(
            "REPO_DELETE_REACHED batch n={}",
            ids.len()
        )))
    }
}

/// 行不存在场景用仓储：`get` 返回 None（无目标行），删除仍以哨兵暴露是否抵达
#[derive(Clone)]
struct GoneProbeRepo;

impl From<PgPool> for GoneProbeRepo {
    fn from(_pool: PgPool) -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl crud::AliothRepository<ExtProbe, ExtProbeDto, ExtProbeDto, AliothError> for GoneProbeRepo {
    async fn list(
        &self,
        _query: &crud::ListQuery,
    ) -> Result<crud::PaginatedResponse<ExtProbe>, AliothError> {
        panic!("本测试不触列表径")
    }

    async fn get(&self, _id: i64) -> Result<Option<ExtProbe>, AliothError> {
        Ok(None)
    }

    async fn create(&self, _req: ExtProbeDto, _user_id: i64) -> Result<ExtProbe, AliothError> {
        panic!("本测试不触创建径")
    }

    async fn update(
        &self,
        _id: i64,
        _req: ExtProbeDto,
        _user_id: i64,
    ) -> Result<Option<ExtProbe>, AliothError> {
        panic!("本测试不触更新径")
    }

    async fn delete(&self, _id: i64, _user_id: i64) -> Result<(), AliothError> {
        panic!("本测试不触单删径")
    }

    async fn batch_delete(&self, ids: Vec<i64>, _user_id: i64) -> Result<(), AliothError> {
        Err(AliothError::Internal(format!(
            "REPO_DELETE_REACHED batch n={}",
            ids.len()
        )))
    }
}

/// 惰性池（不建连接）：`acquire_timeout` 压到 250ms——单删径在钩子之后会解析 dk 上下文（触库），
/// 无 DB 环境下须快速失败，避免单例等待 60s（默认连接超时）
fn lazy_pool() -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(250))
        .max_connections(1)
        .connect_lazy("postgres://ext_probe:ext_probe@127.0.0.1:1/ext_probe")
        .expect("lazy pool")
}

/// 测试用身份注入（生产由 auth 中间件写 `RequestContext`；此处写 handler 读取的 `i64` 回退槽）
async fn inject_user(
    req: ServiceRequest,
    next: actix_web::middleware::Next<impl MessageBody>,
) -> Result<ServiceResponse<impl MessageBody>, Error> {
    req.extensions_mut().insert(1_i64);
    next.call(req).await
}

/// 单条约束的扩展注册表（`entity` = 声明名，两种口径分别由用例传入）
fn registry_with_constraint(entity: &str, expression: &str) -> AppExtensionRegistry {
    let registry = AppExtensionRegistry::new();
    let mut ext = AppLogicExtension::new("ext-probe-app");
    ext.constraints = vec![ConstraintExtension {
        entity: entity.to_string(),
        field: None,
        expression: expression.to_string(),
        level: ConstraintSeverity::Error,
        message: "金额必须大于 100".to_string(),
    }];
    registry.register(ext);
    registry
}

/// 单条业务规则（`onCreate`，非阻塞）的扩展注册表：`action` 形如 `字段 = 表达式`
fn registry_with_rule(entity: &str, action: &str) -> AppExtensionRegistry {
    let registry = AppExtensionRegistry::new();
    let mut ext = AppLogicExtension::new("ext-probe-app");
    ext.business_rules = vec![RuleExtension {
        entity: entity.to_string(),
        name: "probe_rule".to_string(),
        trigger: "onCreate".to_string(),
        condition: "true".to_string(),
        action: action.to_string(),
        priority: 0,
        error_message: String::new(),
        blocking: false,
    }];
    registry.register(ext);
    registry
}

async fn post_amount(registry: AppExtensionRegistry, amount: i64) -> (u16, String) {
    let app = test::init_service(
        App::new()
            .wrap(from_fn(inject_user))
            .app_data(web::Data::new(lazy_pool()))
            .app_data(web::Data::new(registry))
            .configure(crud_routes::<
                ExtProbe,
                ExtProbeDto,
                ExtProbeDto,
                ExtProbeRepo,
                AliothError,
            >("/ext-probe")),
    )
    .await;
    let resp = test::TestRequest::post()
        .uri("/ext-probe")
        .insert_header(("content-type", "application/json"))
        .set_payload(format!("{{\"amount\":{amount}}}"))
        .send_request(&app)
        .await;
    let status = resp.status().as_u16();
    let body = test::read_body(resp).await;
    (status, String::from_utf8_lossy(&body).to_string())
}

/// 业务名声明（`EXT_PROBE` 的 `ENTITY_NAME`）⇒ 标准路由执行钩子并阻断
#[actix_web::test]
async fn constraint_blocks_create_by_business_name() {
    let (status, body) =
        post_amount(registry_with_constraint("ext_probe", "amount > 100"), 5).await;
    assert_eq!(status, 400, "约束阻断 MUST 返回 400；body={body}");
    assert!(
        body.contains("金额必须大于 100"),
        "body MUST 含约束文案：{body}"
    );
}

/// 物理表名声明（`isahl."ext-probe"` 去 schema/引号）⇒ 同样可解析并阻断
#[actix_web::test]
async fn constraint_blocks_create_by_table_name() {
    let (status, body) =
        post_amount(registry_with_constraint("ext-probe", "amount > 100"), 5).await;
    assert_eq!(status, 400, "表名声明 MUST 同样命中；body={body}");
    assert!(
        body.contains("金额必须大于 100"),
        "body MUST 含约束文案：{body}"
    );
}

/// 约束通过（`amount > 100` 成立）⇒ 钩子放行，抵达仓储（哨兵错误可见）
#[actix_web::test]
async fn constraint_pass_lets_request_through() {
    let (status, body) =
        post_amount(registry_with_constraint("ext_probe", "amount > 100"), 500).await;
    assert!(
        status >= 500 && body.contains("REPO_REACHED"),
        "约束通过 ⇒ MUST 抵达仓储；实际 {status} body={body}"
    );
}

/// 无该实体声明 ⇒ 钩子直通（不误阻断），同样抵达仓储
#[actix_web::test]
async fn unrelated_declaration_does_not_block() {
    let (status, body) = post_amount(
        registry_with_constraint("some_other_entity", "amount > 100"),
        5,
    )
    .await;
    assert!(
        status >= 500 && body.contains("REPO_REACHED"),
        "无声明 ⇒ MUST 直通至仓储；实际 {status} body={body}"
    );
}

// ── 删除径钩子（单行 + 批量）：上下文 = 存量行，阻断 = 一行不删 ──

/// 删除径服务：`registry=None` ⇒ 不装配扩展（直通）
fn delete_app(
    registry: Option<AppExtensionRegistry>,
) -> App<
    impl actix_web::dev::ServiceFactory<
        ServiceRequest,
        Config = (),
        Response = ServiceResponse<impl MessageBody>,
        Error = Error,
        InitError = (),
    >,
> {
    let mut app = App::new()
        .wrap(from_fn(inject_user))
        .app_data(web::Data::new(lazy_pool()));
    if let Some(registry) = registry {
        app = app.app_data(web::Data::new(registry));
    }
    app.configure(crud_routes::<
        ExtProbe,
        ExtProbeDto,
        ExtProbeDto,
        DeleteProbeRepo,
        AliothError,
    >("/ext-probe-del"))
}

/// 单删 + 批删请求（`X-` 头未设 ⇒ 无 RLS 过滤）
async fn delete_single(registry: Option<AppExtensionRegistry>, id: i64) -> (u16, String) {
    let app = test::init_service(delete_app(registry)).await;
    let resp = test::TestRequest::delete()
        .uri(&format!("/ext-probe-del/{id}"))
        .send_request(&app)
        .await;
    let status = resp.status().as_u16();
    let body = test::read_body(resp).await;
    (status, String::from_utf8_lossy(&body).to_string())
}

async fn delete_batch(registry: Option<AppExtensionRegistry>, ids: &[i64]) -> (u16, String) {
    let app = test::init_service(delete_app(registry)).await;
    let payload: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
    let resp = test::TestRequest::delete()
        .uri("/ext-probe-del/batch")
        .insert_header(("content-type", "application/json"))
        .set_payload(format!("[{}]", payload.join(",")))
        .send_request(&app)
        .await;
    let status = resp.status().as_u16();
    let body = test::read_body(resp).await;
    (status, String::from_utf8_lossy(&body).to_string())
}

/// 失败判据：约束表达式引用**存量行**字段 `amount` ⇒ 只有携带行上下文才可求值。
/// 该约束（`amount > 100`，Error）在存量行（amount=500）上通过 ⇒ 证明钩子拿到的是行而不只是 id
/// （若只给 `{id}`，求值失败按 Error 级处理 ⇒ 400，本用例会红）。
#[actix_web::test]
async fn single_delete_hook_sees_existing_row() {
    let (status, body) = delete_single(
        Some(registry_with_constraint("ext_probe", "amount > 100")),
        7,
    )
    .await;
    // 钩子之后单删径会解析 dk 上下文（触库；本测试无 DB）⇒ 只断言「未被钩子阻断」：
    // 约束引用存量行字段 `amount`，只给 `{id}` 时求值失败即 400 —— 非 400 即证明钩子拿到行上下文
    assert!(
        status >= 500,
        "行上下文可求值 ⇒ MUST NOT 被钩子阻断（后续触库失败属预期）；实际 {status} body={body}"
    );
    assert!(
        !body.contains("金额必须大于 100"),
        "MUST NOT 报出约束文案（否则说明按 id-only 上下文求值失败）：{body}"
    );
}

#[actix_web::test]
async fn batch_delete_hook_sees_existing_row() {
    let (status, body) = delete_batch(
        Some(registry_with_constraint("ext_probe", "amount > 100")),
        &[7, 8],
    )
    .await;
    assert!(
        status >= 500 && body.contains("REPO_DELETE_REACHED batch n=2"),
        "批删 MUST 逐行执行 before_delete 并抵达仓储；实际 {status} body={body}"
    );
}

#[actix_web::test]
async fn batch_delete_blocked_by_constraint_deletes_nothing() {
    let (status, body) = delete_batch(
        Some(registry_with_constraint("ext_probe", "amount < 100")),
        &[7, 8],
    )
    .await;
    assert_eq!(status, 400, "存量行违反 Error 级约束 ⇒ 400；body={body}");
    assert!(
        body.contains("金额必须大于 100"),
        "body MUST 含约束文案：{body}"
    );
    assert!(
        !body.contains("REPO_DELETE_REACHED"),
        "any 行阻断 ⇒ MUST NOT 删除任何行（未触仓储）：{body}"
    );
}

#[actix_web::test]
async fn batch_delete_without_registry_passes_through() {
    let (status, body) = delete_batch(None, &[7, 8]).await;
    assert!(
        status >= 500 && body.contains("REPO_DELETE_REACHED batch n=2"),
        "未装配注册表 ⇒ MUST 直通（不读行、不阻断）；实际 {status} body={body}"
    );
}

/// 行不存在 ⇒ 无目标行可校验（DB 侧 no-op）⇒ MUST 跳过钩子（否则幂等批删被误 400）
#[actix_web::test]
async fn batch_delete_skips_hooks_for_absent_rows() {
    let app = test::init_service(
        App::new()
            .wrap(from_fn(inject_user))
            .app_data(web::Data::new(lazy_pool()))
            .app_data(web::Data::new(registry_with_constraint(
                "ext_probe",
                "amount > 100",
            )))
            .configure(crud_routes::<
                ExtProbe,
                ExtProbeDto,
                ExtProbeDto,
                GoneProbeRepo,
                AliothError,
            >("/ext-probe-gone")),
    )
    .await;
    let resp = test::TestRequest::delete()
        .uri("/ext-probe-gone/batch")
        .insert_header(("content-type", "application/json"))
        .set_payload("[7,8]")
        .send_request(&app)
        .await;
    let status = resp.status().as_u16();
    let body = test::read_body(resp).await;
    let body = String::from_utf8_lossy(&body).to_string();
    assert!(
        status >= 500 && body.contains("REPO_DELETE_REACHED batch n=2"),
        "行不存在 ⇒ MUST 跳过钩子并直通仓储（不得因约束求值失败误 400）；实际 {status} body={body}"
    );
}

// ── 公开装配面（手写写径入口）：`crud::ext_hooks` ──

/// 手写写径经公开 API：线载荷上跑 `before_create` ⇒ 阻断 400 / mutations 合并 / 无装配直通
#[tokio::test]
async fn ext_hooks_public_api_runs_on_wire_payload() {
    let registry = registry_with_constraint("ext_probe", "amount > 100");

    // ① 违反约束 ⇒ 阻断（400 语义）
    let mut payload = serde_json::json!({ "amount": 5 });
    let blocked = crud::ext_hooks::before_create(
        Some(&registry),
        &["ext_probe"],
        "ext-probe-app",
        &mut payload,
    );
    assert!(
        matches!(blocked, Err(AliothError::BadRequest(_))),
        "违反 Error 级约束 MUST 以 BadRequest 阻断：{blocked:?}"
    );

    // ② 规则 mutation 合并入**线载荷**（手写写径随后反序列化即得改写值）
    let rule_registry = registry_with_rule("ext_probe", "amount = 777");
    let mut payload = serde_json::json!({ "amount": 5 });
    crud::ext_hooks::before_create(
        Some(&rule_registry),
        &["ext_probe"],
        "ext-probe-app",
        &mut payload,
    )
    .expect("规则 mutation 不阻断");
    assert_eq!(
        payload.get("amount"),
        Some(&serde_json::json!(777)),
        "mutations MUST 合并入线载荷：{payload}"
    );

    // ③ 未装配注册表 ⇒ 直通（载荷不变）
    let mut payload = serde_json::json!({ "amount": 5 });
    crud::ext_hooks::before_create(None, &["ext_probe"], "ext-probe-app", &mut payload)
        .expect("无装配 MUST 直通");
    assert_eq!(payload.get("amount"), Some(&serde_json::json!(5)));
}

/// 删除上下文唯一构造点：存量行字段 + `id` 同时在；无行 ⇒ 仅 `id`
#[tokio::test]
async fn ext_hooks_delete_variables_carries_row_and_id() {
    let row = ExtProbe {
        id: 7,
        amount: Some(500),
    };
    let variables = crud::ext_hooks::delete_variables(Some(&row), 7);
    assert_eq!(variables.get("amount"), Some(&serde_json::json!(500)));
    assert_eq!(variables.get("id"), Some(&serde_json::json!(7)));

    let id_only = crud::ext_hooks::delete_variables::<ExtProbe>(None, 9);
    assert_eq!(id_only.len(), 1, "无存量行 ⇒ 仅 id：{id_only:?}");
    assert_eq!(id_only.get("id"), Some(&serde_json::json!(9)));
}

/// 严格 DTO 路由（`/ext-probe-strict`）：同钩子装配，DTO 带 `deny_unknown_fields`
async fn post_amount_strict(registry: AppExtensionRegistry, amount: i64) -> (u16, String) {
    let app = test::init_service(
        App::new()
            .wrap(from_fn(inject_user))
            .app_data(web::Data::new(lazy_pool()))
            .app_data(web::Data::new(registry))
            .configure(crud_routes::<
                ExtProbe,
                StrictProbeDto,
                StrictProbeDto,
                StrictProbeRepo,
                AliothError,
            >("/ext-probe-strict")),
    )
    .await;
    let resp = test::TestRequest::post()
        .uri("/ext-probe-strict")
        .insert_header(("content-type", "application/json"))
        .set_payload(format!("{{\"amount\":{amount}}}"))
        .send_request(&app)
        .await;
    let status = resp.status().as_u16();
    let body = test::read_body(resp).await;
    (status, String::from_utf8_lossy(&body).to_string())
}

// ── 规则 mutation 与 DTO 面的交互（语义锚）──

/// 规则的 mutation MUST 落到写载荷（`before_create` 在线载荷上合并 mutations 后才反序列化）
#[actix_web::test]
async fn rule_mutation_reaches_write_payload() {
    let (status, body) = post_amount(registry_with_rule("ext_probe", "amount = 777"), 5).await;
    assert!(
        status >= 500 && body.contains("REPO_REACHED amount=777"),
        "规则 mutation MUST 生效于写载荷；实际 {status} body={body}"
    );
}

/// mutation 写入 DTO 未声明字段：DTO 带 `deny_unknown_fields` ⇒ 400（fail-closed）
#[actix_web::test]
async fn undeclared_mutation_field_fails_closed_with_strict_dto() {
    let (status, body) = post_amount_strict(registry_with_rule("ext_probe", "ghost = 1"), 5).await;
    assert_eq!(status, 400, "严格 DTO 下未声明字段 MUST 400；body={body}");
    assert!(
        body.contains("请求体与 DTO 不匹配"),
        "body MUST 报出反序列化失败：{body}"
    );
}

/// 同一 mutation 在**宽松 DTO**（无 `deny_unknown_fields`）下被 serde 丢弃 ⇒ 请求照常放行、其余字段不变
/// （语义：规则目标字段不在服务 DTO 面 ⇒ 该写为无效写；声明面的机械拦截归组合期 `verify-extensions`）
#[actix_web::test]
async fn undeclared_mutation_field_dropped_with_permissive_dto() {
    let (status, body) = post_amount(registry_with_rule("ext_probe", "ghost = 1"), 5).await;
    assert!(
        status >= 500 && body.contains("REPO_REACHED amount=5"),
        "宽松 DTO 下未声明字段被丢弃且其余字段不变；实际 {status} body={body}"
    );
}
