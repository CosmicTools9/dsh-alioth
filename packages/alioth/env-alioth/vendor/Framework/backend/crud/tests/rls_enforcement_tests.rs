//! RLS 空集语义集成测试（真实测试库，isahl schema 已 seed）。
//!
//! 契约（NGAC_SPEC §visible_ids）：`Some(空集)` = 显式无授权 → 恒假谓词零行；
//! `None` = 无 RLS 约束（admin/非 RLS 调用方）。覆盖列表 fetch/fetch_count
//! 与详情 get 两条查询路径。

use common::testing::connect_test_db;
use crud::entity::Identifiable;
use crud::query_builder::QueryBuilder;
use crud::AliothDbEntity;
use sqlx::PgPool;

#[derive(sqlx::FromRow, serde::Serialize, Clone)]
struct StatusEntity {
    id: i64,
}
impl Identifiable for StatusEntity {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for StatusEntity {
    fn table_name() -> &'static str {
        r#""isahl"."zc_id_status""#
    }
    const SELECT_FIELDS: &'static str = "id";
    const ENTITY_NAME: &'static str = "test-rls-status";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

/// 插入一行测试数据并返回 id；测试结束由 cleanup_status 硬删除。
async fn insert_status(pool: &PgPool, tag: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_status" (notice) VALUES ($1) RETURNING id"#,
    )
    .bind(tag)
    .fetch_one(pool)
    .await
    .expect("insert test status row")
}

async fn cleanup_status(pool: &PgPool, id: i64) {
    sqlx::query(r#"DELETE FROM isahl."zc_id_status" WHERE id = $1"#)
        .bind(id)
        .execute(pool)
        .await
        .expect("cleanup test status row");
}

#[tokio::test]
async fn list_empty_visible_set_returns_zero_rows() {
    let pool = connect_test_db().await;
    let id = insert_status(&pool, "rls-empty-set").await;

    // Some(空集) → 恒假谓词：items 与 total 都必须为 0（覆盖 fetch_items + fetch_count）
    let resp = QueryBuilder::<StatusEntity>::new(&pool)
        .with_visible_ids(vec![])
        .fetch(1, 10)
        .await
        .expect("fetch with empty visible set");
    assert!(resp.items.is_empty(), "空 visible 集必须返回零行");
    assert_eq!(resp.total, 0, "空 visible 集 fetch_count 必须为 0");

    cleanup_status(&pool, id).await;
}

#[tokio::test]
async fn list_none_visible_set_unconstrained() {
    let pool = connect_test_db().await;
    let id = insert_status(&pool, "rls-none-set").await;

    // None → 无约束：应能查到刚插入的行
    let resp = QueryBuilder::<StatusEntity>::new(&pool)
        .fetch(1, 500)
        .await
        .expect("fetch without visible ids");
    assert!(
        resp.items.iter().any(|e| e.id == id),
        "None visible_ids 应保持无约束（插入行可见）"
    );

    cleanup_status(&pool, id).await;
}

#[tokio::test]
async fn list_non_empty_visible_set_filters() {
    let pool = connect_test_db().await;
    let id = insert_status(&pool, "rls-filter-set").await;

    // 包含目标 id → 返回该行
    let resp = QueryBuilder::<StatusEntity>::new(&pool)
        .with_visible_ids(vec![id])
        .fetch(1, 10)
        .await
        .expect("fetch with containing id");
    assert_eq!(resp.items.len(), 1);
    assert_eq!(resp.items[0].id, id);

    // 不含目标 id（负 id 不可能存在）→ 不含该行
    let resp = QueryBuilder::<StatusEntity>::new(&pool)
        .with_visible_ids(vec![-1])
        .fetch(1, 500)
        .await
        .expect("fetch with foreign id");
    assert!(
        !resp.items.iter().any(|e| e.id == id),
        "visible_ids 不含目标 id 时该行不可见"
    );

    cleanup_status(&pool, id).await;
}

#[tokio::test]
async fn get_empty_visible_set_returns_none() {
    let pool = connect_test_db().await;
    let id = insert_status(&pool, "rls-get-empty").await;

    // Some(空集) → 详情也必须不可见（恒假谓词）
    let row = QueryBuilder::<StatusEntity>::get(&pool, id, Some(&[]), None)
        .await
        .expect("get with empty visible set");
    assert!(row.is_none(), "空 visible 集详情查询必须返回 None");

    // None → 无约束可见
    let row = QueryBuilder::<StatusEntity>::get(&pool, id, None, None)
        .await
        .expect("get without visible ids");
    assert!(row.is_some(), "None visible_ids 详情查询应保持可见");

    cleanup_status(&pool, id).await;
}

// ── 写路径行级可见性（enforce-write-path-rls）──────────────────────────────

use actix_web::dev::Service;
use actix_web::{test, App};
use actix_web::{web, HttpMessage};
use common::context::RequestContext;
use crud::handler::{crud_delete, crud_update};
use serde::Deserialize;

#[derive(Deserialize)]
struct UpdateStatusRequest {
    #[allow(dead_code)]
    notice: Option<String>,
}

// 只读 repository（create/update → NotImplemented）：写路径预检在 update/delete_with_rls 之前，
// 不可见行断言 NotFound，不会到达 NotImplemented 分支
#[derive(Clone)]
struct StatusReadRepo {
    generic: crud::GenericRepository<StatusEntity>,
}
impl From<sqlx::PgPool> for StatusReadRepo {
    fn from(pool: sqlx::PgPool) -> Self {
        Self {
            generic: crud::GenericRepository::new(pool),
        }
    }
}
#[async_trait::async_trait]
impl
    crud::AliothRepository<
        StatusEntity,
        UpdateStatusRequest,
        UpdateStatusRequest,
        common::AliothError,
    > for StatusReadRepo
{
    async fn list(
        &self,
        q: &crud::ListQuery,
    ) -> Result<crud::PaginatedResponse<StatusEntity>, common::AliothError> {
        self.generic.list(q).await
    }
    async fn list_with_rls(
        &self,
        q: &crud::ListQuery,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<crud::PaginatedResponse<StatusEntity>, common::AliothError> {
        self.generic
            .list_with_rls(q, visible_ids, authorized_columns)
            .await
    }
    async fn get(&self, id: i64) -> Result<Option<StatusEntity>, common::AliothError> {
        self.generic.get(id).await
    }
    async fn get_with_rls(
        &self,
        id: i64,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<Option<StatusEntity>, common::AliothError> {
        self.generic
            .get_with_rls(id, visible_ids, authorized_columns)
            .await
    }
    async fn create(
        &self,
        _r: UpdateStatusRequest,
        _u: i64,
    ) -> Result<StatusEntity, common::AliothError> {
        Err(common::AliothError::NotImplemented("create".into()))
    }
    async fn update(
        &self,
        _id: i64,
        _r: UpdateStatusRequest,
        _u: i64,
    ) -> Result<Option<StatusEntity>, common::AliothError> {
        Err(common::AliothError::NotImplemented("update".into()))
    }
    async fn delete(&self, _id: i64, _u: i64) -> Result<(), common::AliothError> {
        Err(common::AliothError::NotImplemented("delete".into()))
    }
}

#[actix_web::test]
async fn update_invisible_row_returns_not_found() {
    let pool = connect_test_db().await;
    let id = insert_status(&pool, "rls-update-invisible").await;
    let ctx = RequestContext::with_username(1, "test@test", "tester");
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/u")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .route(
                    "/{id}",
                    web::put().to(crud_update::<
                        StatusEntity,
                        UpdateStatusRequest,
                        UpdateStatusRequest,
                        StatusReadRepo,
                        common::AliothError,
                    >),
                )
                .route(
                    "/{id}",
                    web::delete().to(crud_delete::<
                        StatusEntity,
                        UpdateStatusRequest,
                        UpdateStatusRequest,
                        StatusReadRepo,
                        common::AliothError,
                    >),
                ),
        ),
    )
    .await;

    let req = test::TestRequest::put()
        .uri(&format!("/u/{}", id))
        .insert_header(("X-Visible-Ids", "999999"))
        .set_json(&serde_json::json!({ "notice": "hacked" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 404, "不可见行更新必须 404");
    let row = QueryBuilder::<StatusEntity>::get(&pool, id, None, None)
        .await
        .expect("get after rejected update");
    assert!(row.is_some(), "行应仍然存在");
    cleanup_status(&pool, id).await;
}

#[actix_web::test]
async fn delete_invisible_row_returns_not_found() {
    let pool = connect_test_db().await;
    let id = insert_status(&pool, "rls-delete-invisible").await;
    let ctx = RequestContext::with_username(1, "test@test", "tester");
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/u")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .route(
                    "/{id}",
                    web::put().to(crud_update::<
                        StatusEntity,
                        UpdateStatusRequest,
                        UpdateStatusRequest,
                        StatusReadRepo,
                        common::AliothError,
                    >),
                )
                .route(
                    "/{id}",
                    web::delete().to(crud_delete::<
                        StatusEntity,
                        UpdateStatusRequest,
                        UpdateStatusRequest,
                        StatusReadRepo,
                        common::AliothError,
                    >),
                ),
        ),
    )
    .await;

    let req = test::TestRequest::delete()
        .uri(&format!("/u/{}", id))
        .insert_header(("X-Visible-Ids", "999999"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 404, "不可见行删除必须 404");
    let row = QueryBuilder::<StatusEntity>::get(&pool, id, None, None)
        .await
        .expect("get after rejected delete");
    assert!(row.is_some(), "行应仍然存在（软删除未执行）");
    cleanup_status(&pool, id).await;
}

#[actix_web::test]
async fn update_visible_row_passes_precheck() {
    let pool = connect_test_db().await;
    let id = insert_status(&pool, "rls-update-visible").await;
    let ctx = RequestContext::with_username(1, "test@test", "tester");
    let app = test::init_service(
        App::new().app_data(web::Data::new(pool.clone())).service(
            web::scope("/u")
                .wrap_fn(move |req, srv| {
                    req.extensions_mut().insert(ctx.clone());
                    srv.call(req)
                })
                .route(
                    "/{id}",
                    web::put().to(crud_update::<
                        StatusEntity,
                        UpdateStatusRequest,
                        UpdateStatusRequest,
                        StatusReadRepo,
                        common::AliothError,
                    >),
                )
                .route(
                    "/{id}",
                    web::delete().to(crud_delete::<
                        StatusEntity,
                        UpdateStatusRequest,
                        UpdateStatusRequest,
                        StatusReadRepo,
                        common::AliothError,
                    >),
                ),
        ),
    )
    .await;

    let req = test::TestRequest::put()
        .uri(&format!("/u/{}", id))
        .insert_header(("X-Visible-Ids", id.to_string()))
        .set_json(&serde_json::json!({ "notice": "ok" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    // 预检放行后走到 update_with_rls（只读 repo 默认 → NotImplemented 500）
    // 断言不是 404 即证明可见行预检放行
    assert_ne!(resp.status(), 404, "可见行更新预检应放行");
    cleanup_status(&pool, id).await;
}
