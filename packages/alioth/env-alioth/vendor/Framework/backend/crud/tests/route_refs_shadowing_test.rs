//! 路由层回归测试 —— refs 读径（list_refs/get_refs）被 CRUD scope 前缀吞并
//!
//! 实测缺陷（bv-local 交付验收暴露）：`GET /service/{entity}/refs` 全部 404/400。
//! 两层根因：
//! 1. `crud_routes` 的 `/{id}` pattern 无约束，字母段 "refs" 被吞进 get handler
//!    （400 `can not parse "refs" to a i64`）；
//! 2. actix scope 前缀吞并：`web::scope("/probe")` 一旦前缀命中就不再回溯到后注册的
//!    `web::scope("/probe/refs")`——即使 `{id}` 加了 `\d+` 约束，`/refs` 仍在第一个
//!    scope 内 404。**因此 refs 读径必须先注册**（本测试即固化该注册约定）。
//!
//! 本测试用 `PgPool::connect_lazy`（不建连接）+ actix test：
//! - refs 命中后的 DB 查询失败（5xx），但绝不是 id 解析的 400——用 status/body 区分
//!   「命中 refs handler」与「被 {id} 吞掉」；
//! - 非数字段（`abc`）应 404（不再被 `/{id}` 误吞）；
//! - 数字段（`123`）仍命中 get handler（→ DB 失败 5xx，证明路由在）。
//!
//! 验证的是路由注册的可观察行为，不依赖任何表数据。

use actix_web::{test, web, App};
use common::error::AliothError;
use crud::{
    crud_ref_routes, crud_routes, crud_routes_with_refs, AliothDbEntity, GenericRepository,
    HasReferenceJoins, Identifiable,
};
use sqlx::PgPool;

#[derive(sqlx::FromRow, serde::Serialize, Clone)]
struct ProbeEntity {
    id: i64,
    notice: Option<String>,
}

impl Identifiable for ProbeEntity {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for ProbeEntity {
    fn table_name() -> &'static str {
        "isahl.probe_refs_route" // 不存在的表——handler 里 DB 查询必失败，我们只关心路由命中
    }
    const SELECT_FIELDS: &'static str = "id, notice";
    const ENTITY_NAME: &'static str = "probe-refs-route";
    const SOFT_DELETE: bool = true;
}

impl HasReferenceJoins for ProbeEntity {
    fn reference_joins() -> Vec<crud::ReferenceJoin> {
        vec![]
    }
}

type Dto = serde_json::Value;

/// 测试用仓储包装：`GenericRepository` 不直接实现 `AliothRepository`/`From<PgPool>`
///（全仓服务均为 wrapper 委托模式），此处同样包装。
#[derive(Clone)]
struct ProbeRepo {
    inner: GenericRepository<ProbeEntity>,
}

impl From<PgPool> for ProbeRepo {
    fn from(pool: PgPool) -> Self {
        Self {
            inner: GenericRepository::new(pool),
        }
    }
}

#[async_trait::async_trait]
impl crud::AliothRepository<ProbeEntity, Dto, Dto, AliothError> for ProbeRepo {
    async fn list(
        &self,
        query: &crud::ListQuery,
    ) -> Result<crud::PaginatedResponse<ProbeEntity>, AliothError> {
        self.inner.list(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<ProbeEntity>, AliothError> {
        self.inner.get(id).await
    }

    async fn create(&self, _req: Dto, _user_id: i64) -> Result<ProbeEntity, AliothError> {
        unimplemented!("路由层测试只触发读径")
    }

    async fn update(
        &self,
        _id: i64,
        _req: Dto,
        _user_id: i64,
    ) -> Result<Option<ProbeEntity>, AliothError> {
        unimplemented!("路由层测试只触发读径")
    }

    async fn delete(&self, _id: i64, _user_id: i64) -> Result<(), AliothError> {
        unimplemented!("路由层测试只触发读径")
    }
}

type Repo = ProbeRepo;

fn lazy_pool() -> PgPool {
    // 惰性连接：不触网；handler 内查询时才失败（5xx），用于区分路由命中层
    PgPool::connect_lazy("postgres://probe:probe@127.0.0.1:1/probe").expect("lazy pool")
}

async fn body_text(resp: actix_web::dev::ServiceResponse) -> String {
    let body = test::read_body(resp).await;
    String::from_utf8(body.to_vec()).unwrap_or_default()
}

/// 注册约定（唯一正确形态）：refs 读径 scope 必须先于 CRUD scope 注册。
macro_rules! probe_app {
    ($pool:expr, $path:literal) => {
        test::init_service(
            App::new()
                .app_data(web::Data::new($pool))
                // ⚠ 顺序是语义：refs 先于 CRUD（actix scope 前缀吞并，见文件头注释）
                .configure(crud_ref_routes::<ProbeEntity, AliothError>($path))
                .configure(crud_routes::<ProbeEntity, Dto, Dto, Repo, AliothError>($path)),
        )
        .await
    };
}

/// 核心回归断言：`/refs` 命中 refs handler（DB 失败 5xx），绝不能被 `/{id}` 吞（400 can-not-parse）。
#[actix_web::test]
async fn refs_route_not_shadowed_by_id_route() {
    let app = probe_app!(lazy_pool(), "/probe");

    let resp = test::TestRequest::get()
        .uri("/probe/refs")
        .send_request(&app)
        .await;
    let status = resp.status().as_u16();
    let text = body_text(resp).await;
    assert_ne!(
        status, 400,
        "/refs 被 /{{id}} 吞掉（400 can-not-parse = 路由阴影回归）：body={text}"
    );
    assert!(
        !text.contains("can not parse"),
        "refs 读径被 id 路由吞掉：body={text}"
    );
    assert_ne!(
        status, 404,
        "refs 路由不应 404（scope 吞并回归）：body={text}"
    );
}

/// refs/{id} 读径同样可达。
#[actix_web::test]
async fn refs_get_by_id_route_reachable() {
    let app = probe_app!(lazy_pool(), "/probe");

    let resp = test::TestRequest::get()
        .uri("/probe/refs/123")
        .send_request(&app)
        .await;
    let status = resp.status().as_u16();
    let text = body_text(resp).await;
    assert_ne!(status, 404, "refs/{{id}} 路由不应 404：body={text}");
    assert!(
        !text.contains("can not parse"),
        "refs/{{id}} 被误吞：body={text}"
    );
}

/// 字母段不再被 `/{id}` 误吞：`/probe/abc` → 404（路由不存在），而不是 400 can-not-parse。
#[actix_web::test]
async fn non_numeric_segment_no_longer_swallowed() {
    let app = probe_app!(lazy_pool(), "/probe");

    let resp = test::TestRequest::get()
        .uri("/probe/abc")
        .send_request(&app)
        .await;
    let status = resp.status().as_u16();
    let text = body_text(resp).await;
    assert_eq!(
        status, 404,
        "非数字段应 404（路由不存在），实际 body={text}"
    );
}

/// 数字 id 段仍命中 get handler（路由未失效）：lazy pool 下 DB 查询失败 → 5xx，绝不是 404/400。
#[actix_web::test]
async fn numeric_id_route_still_works() {
    let app = probe_app!(lazy_pool(), "/probe");

    let resp = test::TestRequest::get()
        .uri("/probe/123")
        .send_request(&app)
        .await;
    let status = resp.status().as_u16();
    let text = body_text(resp).await;
    assert!(
        status >= 500,
        "数字 id 应命中 get handler（DB 失败 5xx），实际 {status} body={text}"
    );
}

/// 另两个工厂（with_refs / 标准 crud_routes）的 `/{id}` 同样约束、同样注册顺序约定。
#[actix_web::test]
async fn other_factories_also_constrained() {
    let app_refs = test::init_service(
        App::new()
            .app_data(web::Data::new(lazy_pool()))
            .configure(crud_ref_routes::<ProbeEntity, AliothError>("/probe2"))
            .configure(crud_routes_with_refs::<
                ProbeEntity,
                Dto,
                Dto,
                Repo,
                AliothError,
            >("/probe2")),
    )
    .await;
    let resp = test::TestRequest::get()
        .uri("/probe2/refs")
        .send_request(&app_refs)
        .await;
    let status = resp.status().as_u16();
    let text = body_text(resp).await;
    assert_ne!(status, 400, "with_refs: /refs 被吞（回归）：body={text}");
    assert_ne!(status, 404, "with_refs: refs 路由不应 404：body={text}");

    let app_ext = test::init_service(
        App::new()
            .app_data(web::Data::new(lazy_pool()))
            .configure(crud_ref_routes::<ProbeEntity, AliothError>("/probe3"))
            .configure(crud_routes::<ProbeEntity, Dto, Dto, Repo, AliothError>(
                "/probe3",
            )),
    )
    .await;
    let resp = test::TestRequest::get()
        .uri("/probe3/refs")
        .send_request(&app_ext)
        .await;
    let status = resp.status().as_u16();
    let text = body_text(resp).await;
    assert_ne!(status, 400, "crud_routes: /refs 被吞（回归）：body={text}");
    assert_ne!(status, 404, "crud_routes: refs 路由不应 404：body={text}");
}

/// 反向回归：refs 后注册（旧调用形态）时 refs 不可达——固化「错误形态会失败」的判定面，
/// 证明本测试确实能区分两种注册顺序（防测试空转）。
#[actix_web::test]
async fn wrong_order_is_detectable() {
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(lazy_pool()))
            .configure(crud_routes::<ProbeEntity, Dto, Dto, Repo, AliothError>(
                "/probe",
            ))
            .configure(crud_ref_routes::<ProbeEntity, AliothError>("/probe")),
    )
    .await;
    let resp = test::TestRequest::get()
        .uri("/probe/refs")
        .send_request(&app)
        .await;
    let status = resp.status().as_u16();
    assert!(
        status == 400 || status == 404,
        "错误注册顺序下 refs 必须不可达（否则本测试的判定面失效），实际 {status}"
    );
}
