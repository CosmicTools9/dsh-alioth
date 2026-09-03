//! 泛型 CRUD Handler 与路由工厂
//!
//! 提供 `crud_routes` 糖函数以及 `crud_list` / `crud_get` / `crud_create` /
//! `crud_update` / `crud_delete` 五个独立泛型 handler，供模块按需组合。

use actix_web::{web, HttpMessage, HttpRequest, HttpResponse, ResponseError};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::entity::AliothDbEntity;
use crate::pagination::ListQuery;
use crate::repository::AliothRepository;
use common::{AliothError, ApiResponse};
use runtime_engine::{AppContext, AppExtensionRegistry, ExtensionRuntimeError};

/// Derive NGAC resource type from entity name (replace - with _, append s)
fn ngac_resource_name<E: AliothDbEntity>() -> String {
    E::ENTITY_NAME.replace('-', "_") + "s"
}

/// NGAC：为 API 创建的资源注册行级 OA + 创建者 UA 全 CRUD 关联
/// （与通用 crud_create 一致；自定义 create handler 必须调用，否则新行对创建者
///  不可见/不可编辑/不可删除 —— NGAC_SPEC 创建者访问模式）。
pub async fn register_created_resource_ngac<E: AliothDbEntity>(
    pool: &sqlx::PgPool,
    item_id: i64,
    user_id: i64,
) {
    // 查询业务可读标识（notice → code，best-effort；列缺失/行不存在则回退编号）
    // NGAC_SPEC §2.2 resource_identifier 语义，见 add-ngac-oa-readable-identifier
    // 表名来自 E::table_name() 编译期常量（非用户输入），故用 AssertSqlSafe 包裹动态 SQL
    let sql = format!(
        "SELECT COALESCE(NULLIF(notice, ''), NULLIF(code, '')) FROM {} WHERE id = $1",
        E::table_name()
    );
    let readable: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(item_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let identifier =
        readable.unwrap_or_else(|| format!("{}-{}", ngac_resource_name::<E>(), item_id));

    // NGAC: create Object Attribute for this resource (best-effort)
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_object_attribute (o_name, fk_policy_class, resource_type, fk_resource, resource_identifier, created_by_id) \
         VALUES ($1, (SELECT id FROM isahl_auth.ngac_policy_class LIMIT 1), $2, $3, $4, $5) \
         ON CONFLICT(resource_type, fk_resource) DO NOTHING",
    ).bind(format!("{}-{}", ngac_resource_name::<E>(), item_id))
     .bind(ngac_resource_name::<E>()).bind(item_id).bind(identifier).bind(user_id)
     .execute(pool).await;
    // NGAC: associate creator's user attributes with the new resource OA (full CRUD)
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_association (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at) \
         SELECT rr.fk_user_attribute, oa.id, \
                ARRAY(SELECT id FROM isahl_auth.ngac_access_right WHERE o_name IN ('read','write','delete','update','create')), \
                oa.fk_policy_class, NOW() \
         FROM isahl_auth.ngac_user_rr_attribute rr \
         JOIN isahl_auth.ngac_object_attribute oa \
           ON oa.resource_type = $2 AND oa.fk_resource = $3 AND oa.deleted_at IS NULL \
         WHERE rr.fk_user = $1 AND rr.deleted_at IS NULL \
           AND NOT EXISTS ( \
               SELECT 1 FROM isahl_auth.ngac_association a2 \
               WHERE a2.fk_user_attribute = rr.fk_user_attribute AND a2.fk_object_attribute = oa.id AND a2.deleted_at IS NULL) \
         ON CONFLICT DO NOTHING",
    ).bind(user_id)
     .bind(ngac_resource_name::<E>()).bind(item_id)
     .execute(pool).await;
}

/// 从请求中提取用户 ID
///
/// 优先从 `RequestContext` 读取，其次尝试直接取 `i64` extension。
pub fn extract_user_id(req: &HttpRequest) -> Option<i64> {
    common::context::extract_user_id(req).or_else(|| req.extensions().get::<i64>().copied())
}

/// 解析本体坐标上下文（REQ-DATA-002 回退链）。
///
/// ① header 优先：`X-Alioth-Coord` 存在且合法 → 直接采用（历史路径，零回归）；
/// ② 实体声明回退：header 缺失/非法时按 `E::DK_SCENE/DK_FACTOR/DK_FUNCTION`
///    声明的坐标 **code**（BACKEND_FRAMEWORK §7.3.3）经 ontology-binding 解析 ZUID，
///    使前端 DTO 不再暴露 dk_* 的实体仍能注入三维坐标；
/// ③ 无声明 → `None`（`dk_*` 保持 NULL，不 fail-closed，兼容无坐标实体）+ warn 日志。
pub async fn resolve_dk_ctx<E: AliothDbEntity>(
    pool: &sqlx::PgPool,
    req: &HttpRequest,
) -> Option<common::dk_context::DkContext> {
    if let Ok(ctx) = common::dk_context::DkContext::from_request(req) {
        return Some(ctx);
    }
    let (Some(scene), Some(factor), Some(function)) = (E::DK_SCENE, E::DK_FACTOR, E::DK_FUNCTION)
    else {
        log::warn!(
            "X-Alioth-Coord header missing or invalid and entity {} declares no dk_* coordinates; dk_* columns will stay NULL",
            E::ENTITY_NAME
        );
        return None;
    };
    let (dk_scene, dk_factor, dk_function) =
        match ontology_binding::resolve(pool, (scene, factor, function)).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "dk code resolution failed for entity {}: {}",
                    E::ENTITY_NAME,
                    e
                );
                return None;
            }
        };
    common::dk_context::DkContext::from_declared(dk_scene, dk_factor, dk_function)
}

// ===================================================================
// 独立泛型 handlers
// ===================================================================

/// 从 X-Visible-Ids header 解析可见 ID 列表（Gateway PEP RLS 注入）
/// 字面量 `none` = 显式空授权（fail-closed → Some([])，与列控 `none` 约定对称）；
/// 缺失/空串 = None（无约束兼容语义，安全性由 Gateway PEP 全量注入与入站剥离保证）
pub fn parse_visible_ids(req: &HttpRequest) -> Option<Vec<i64>> {
    let header = req.headers().get("X-Visible-Ids")?.to_str().ok()?;
    if header.is_empty() {
        return None;
    }
    if header == "none" {
        return Some(vec![]);
    }
    let ids: Vec<i64> = header
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if ids.is_empty() {
        None
    } else {
        Some(ids)
    }
}

/// 从 x-authorized-columns header 解析列级授权（Gateway PEP 注入）
/// 空/缺失 = None（未启用列控，实体 SENSITIVE_COLUMNS 非空时按历史全量行为）
/// `none` = 显式无授权（fail-closed → Some([])，敏感列全裁）
pub fn parse_authorized_columns(req: &HttpRequest) -> Option<Vec<String>> {
    let header = req.headers().get("x-authorized-columns")?.to_str().ok()?;
    if header.is_empty() {
        return None;
    }
    if header == "none" {
        return Some(vec![]);
    }
    let cols: Vec<String> = header
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if cols.is_empty() {
        None
    } else {
        Some(cols)
    }
}

/// 标准列表 handler
pub async fn crud_list<E, C, U, R, Err>(
    pool: web::Data<sqlx::PgPool>,
    req: HttpRequest,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, Err>
where
    E: AliothDbEntity + Serialize,
    C: Send + Sync + 'static,
    U: Send + Sync + 'static,
    R: AliothRepository<E, C, U, Err> + From<sqlx::PgPool>,
    Err: ResponseError
        + std::error::Error
        + From<sqlx::Error>
        + From<AliothError>
        + Send
        + Sync
        + 'static,
{
    let repo = R::from(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    let response = repo
        .list_with_rls(
            &query,
            visible_ids.as_deref(),
            authorized_columns.as_deref(),
        )
        .await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(response)))
}

/// 标准单条获取 handler
pub async fn crud_get<E, C, U, R, Err>(
    pool: web::Data<sqlx::PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, Err>
where
    E: AliothDbEntity + Serialize,
    C: Send + Sync + 'static,
    U: Send + Sync + 'static,
    R: AliothRepository<E, C, U, Err> + From<sqlx::PgPool>,
    Err: ResponseError
        + std::error::Error
        + From<sqlx::Error>
        + From<AliothError>
        + Send
        + Sync
        + 'static,
{
    let repo = R::from(pool.get_ref().clone());
    let id = path.into_inner();
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    match repo
        .get_with_rls(id, visible_ids.as_deref(), authorized_columns.as_deref())
        .await?
    {
        Some(item) => Ok(HttpResponse::Ok().json(ApiResponse::success(item))),
        None => Err(AliothError::NotFound(format!("Entity {} not found", id)).into()),
    }
}

/// 标准创建 handler
///
/// 实体创建成功后发布 `EntityCreated` 领域事件（bus 未装配时静默跳过）——
/// fix-flow-designer-runtime-chain：审批自动触发（绑定范畴流程）的输入通道。
pub async fn crud_create<E, C, U, R, Err>(
    pool: web::Data<sqlx::PgPool>,
    req: HttpRequest,
    body: web::Json<C>,
    bus: Option<web::Data<Arc<dyn common::event_bus::DomainEventBus>>>,
) -> Result<HttpResponse, Err>
where
    E: AliothDbEntity + Serialize,
    C: DeserializeOwned + Send + Sync + 'static,
    U: Send + Sync + 'static,
    R: AliothRepository<E, C, U, Err> + From<sqlx::PgPool>,
    Err: ResponseError
        + std::error::Error
        + From<sqlx::Error>
        + From<AliothError>
        + Send
        + Sync
        + 'static,
{
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".to_string()))?;
    let repo = R::from(pool.get_ref().clone());
    let dk_ctx = resolve_dk_ctx::<E>(pool.get_ref(), &req).await;
    let item = repo
        .create_with_rls(body.into_inner(), user_id, dk_ctx.as_ref())
        .await?;
    let item_id = item.id();
    // NGAC: create Object Attribute for this resource (best-effort)
    register_created_resource_ngac::<E>(pool.get_ref(), item_id, user_id).await;
    // 领域事件：实体创建（审批自动触发订阅者的输入通道；best-effort）
    publish_entity_created(
        bus.as_ref().map(|b| b.get_ref()),
        E::table_name(),
        item_id,
        user_id,
    )
    .await;
    Ok(HttpResponse::Created().json(item))
}

/// 发布 EntityCreated 事件（bus 未装配 → 静默跳过；发布失败不影响主路径）
pub(crate) async fn publish_entity_created(
    bus: Option<&Arc<dyn common::event_bus::DomainEventBus>>,
    entity_table: &str,
    entity_id: i64,
    created_by: i64,
) {
    let Some(bus) = bus else { return };
    let bus: &dyn common::event_bus::DomainEventBus = bus.as_ref();
    let payload = serde_json::json!({
        "entity_table": entity_table,
        // id-json-ok 语义：entity_id 为业务行 zuid，字符串化传输防 2^53 截断
        "entity_id": entity_id.to_string(),
        "created_by": created_by,
    });
    if let Ok(evt) =
        common::event_bus::DomainEvent::new("EntityCreated", "crud", entity_id, payload)
    {
        let _ = bus.publish("EntityCreated", &evt).await;
    }
}

/// 标准更新 handler
pub async fn crud_update<E, C, U, R, Err>(
    pool: web::Data<sqlx::PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<U>,
) -> Result<HttpResponse, Err>
where
    E: AliothDbEntity + Serialize,
    C: Send + Sync + 'static,
    U: DeserializeOwned + Send + Sync + 'static,
    R: AliothRepository<E, C, U, Err> + From<sqlx::PgPool>,
    Err: ResponseError
        + std::error::Error
        + From<sqlx::Error>
        + From<AliothError>
        + Send
        + Sync
        + 'static,
{
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".to_string()))?;
    let repo = R::from(pool.get_ref().clone());
    let id = path.into_inner();
    // NGAC defense-in-depth
    common::permissions::require_resource_access(
        pool.get_ref(),
        user_id,
        &ngac_resource_name::<E>(),
        id,
        "update",
    )
    .await?;
    // 行级可见性预检（NGAC_SPEC visible_ids 语义）：目标行不可见 → NotFound，禁止越权写
    if let Some(visible_ids) = parse_visible_ids(&req) {
        let existing = repo.get_with_rls(id, Some(&visible_ids), None).await?;
        if existing.is_none() {
            return Err(AliothError::NotFound(format!("Entity {} not found", id)).into());
        }
    }
    let dk_ctx = resolve_dk_ctx::<E>(pool.get_ref(), &req).await;
    match repo
        .update_with_rls(id, body.into_inner(), user_id, dk_ctx.as_ref())
        .await?
    {
        Some(item) => Ok(HttpResponse::Ok().json(ApiResponse::success(item))),
        None => Err(AliothError::NotFound(format!("Entity {} not found", id)).into()),
    }
}

/// 标准删除 handler
pub async fn crud_delete<E, C, U, R, Err>(
    pool: web::Data<sqlx::PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, Err>
where
    E: AliothDbEntity + Serialize,
    C: Send + Sync + 'static,
    U: Send + Sync + 'static,
    R: AliothRepository<E, C, U, Err> + From<sqlx::PgPool>,
    Err: ResponseError
        + std::error::Error
        + From<sqlx::Error>
        + From<AliothError>
        + Send
        + Sync
        + 'static,
{
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".to_string()))?;
    let id = path.into_inner();
    common::permissions::require_resource_access(
        pool.get_ref(),
        user_id,
        &ngac_resource_name::<E>(),
        id,
        "delete",
    )
    .await?;
    // NGAC: clean up Object Attribute (best-effort)
    let _ = sqlx::query(
        "UPDATE isahl_auth.ngac_object_attribute SET deleted_at = NOW(), deleted_by_id = $1 \
         WHERE resource_type = $2 AND fk_resource = $3 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(ngac_resource_name::<E>())
    .bind(id)
    .execute(pool.get_ref())
    .await;
    let repo = R::from(pool.get_ref().clone());
    // 行级可见性预检（NGAC_SPEC visible_ids 语义）：目标行不可见 → NotFound，禁止越权写
    if let Some(visible_ids) = parse_visible_ids(&req) {
        let existing = repo.get_with_rls(id, Some(&visible_ids), None).await?;
        if existing.is_none() {
            return Err(AliothError::NotFound(format!("Entity {} not found", id)).into());
        }
    }
    let dk_ctx = resolve_dk_ctx::<E>(pool.get_ref(), &req).await;
    repo.delete_with_rls(id, user_id, dk_ctx.as_ref()).await?;
    Ok(HttpResponse::NoContent().finish())
}
/// 标准批量删除 handler
pub async fn crud_batch_delete<E, C, U, R, Err>(
    pool: web::Data<sqlx::PgPool>,
    req: HttpRequest,
    body: web::Json<Vec<i64>>,
) -> Result<HttpResponse, Err>
where
    E: AliothDbEntity + Serialize,
    C: Send + Sync + 'static,
    U: Send + Sync + 'static,
    R: AliothRepository<E, C, U, Err> + From<sqlx::PgPool>,
    Err: ResponseError
        + std::error::Error
        + From<sqlx::Error>
        + From<AliothError>
        + Send
        + Sync
        + 'static,
{
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".to_string()))?;
    let repo = R::from(pool.get_ref().clone());
    let ids = body.into_inner();
    // 行级可见性预检（NGAC_SPEC visible_ids 语义）：不可见行剔除，仅删除可见行（禁止越权写）
    let ids = if let Some(visible_ids) = parse_visible_ids(&req) {
        let mut visible: Vec<i64> = Vec::with_capacity(ids.len());
        for id in &ids {
            let existing = repo.get_with_rls(*id, Some(&visible_ids), None).await?;
            if existing.is_some() {
                visible.push(*id);
            }
        }
        visible
    } else {
        ids
    };
    repo.batch_delete(ids, user_id).await?;
    Ok(HttpResponse::NoContent().finish())
}

// ===================================================================
// 引用解析 handlers
// ===================================================================

use crate::generic_repository::GenericRepository;
use crate::reference::HasReferenceJoins;

/// 标准列表（含引用解析）handler
pub async fn crud_list_refs<E>(
    pool: web::Data<sqlx::PgPool>,
    req: HttpRequest,
    query: web::Query<ListQuery>,
) -> Result<HttpResponse, AliothError>
where
    E: AliothDbEntity + HasReferenceJoins + Serialize + Unpin + 'static,
    for<'r> E: sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    let repo = GenericRepository::<E>::new(pool.get_ref().clone());
    let visible_ids = parse_visible_ids(&req);
    let authorized_columns = parse_authorized_columns(&req);
    let result = repo
        .list_refs_with_rls(
            &query,
            visible_ids.as_deref(),
            authorized_columns.as_deref(),
        )
        .await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(result)))
}

/// 标准 get（含引用解析）handler
pub async fn crud_get_refs<E>(
    pool: web::Data<sqlx::PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError>
where
    E: AliothDbEntity + HasReferenceJoins + Serialize + Unpin + 'static,
    for<'r> E: sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    let repo = GenericRepository::<E>::new(pool.get_ref().clone());
    let id = path.into_inner();
    let authorized_columns = parse_authorized_columns(&req);
    match repo.get_refs(id, authorized_columns.as_deref()).await? {
        Some(item) => Ok(HttpResponse::Ok().json(ApiResponse::success(item))),
        None => Err(AliothError::NotFound(format!("Entity {} not found", id))),
    }
}

/// 引用解析路由糖函数
///
/// 在独立 scope 下注册 `GET {path}/refs`（list_refs）和 `GET {path}/refs/{id}`（get_refs）。
/// 与 `crud_routes` 搭配使用（顺序无关）：
///
/// ```rust,ignore
/// cfg.configure(crud_routes::<E, C, U, R, Err>("/products"));
/// cfg.configure(crud_ref_routes::<E, Err>("/products"));
/// ```
///
/// 实现注意：refs 路由必须挂在 `{path}/refs` 独立 scope 下。actix-web 的 scope
/// 无回落语义——首个前缀匹配的 scope 独占请求，若 refs 与 plain 同前缀则互斥：
/// refs 在前则 POST {path} 404，plain 在前则 /{id} 吞噬 "refs"（id 解析失败）。
pub fn crud_ref_routes<E, Err>(path: &str) -> impl FnOnce(&mut web::ServiceConfig) + '_
where
    E: AliothDbEntity + HasReferenceJoins + Serialize + Unpin + 'static,
    for<'r> E: sqlx::FromRow<'r, sqlx::postgres::PgRow>,
    Err: ResponseError
        + std::error::Error
        + From<sqlx::Error>
        + From<AliothError>
        + Send
        + Sync
        + 'static,
{
    let refs_path = format!("{}/refs", path.trim_end_matches('/'));
    move |cfg| {
        cfg.service(
            web::scope(&refs_path)
                .route("", web::get().to(crud_list_refs::<E>))
                .route("/{id}", web::get().to(crud_get_refs::<E>)),
        );
    }
}

// ===================================================================
// 扩展感知 handler — 自动注入应用级逻辑扩展
// ===================================================================

/// 从泛型类型读取实体业务名称
///
/// 读取 `AliothDbEntity::ENTITY_NAME` 常量。
fn entity_name_from_type<E: AliothDbEntity>() -> &'static str {
    E::ENTITY_NAME
}

/// 将 DTO 序列化为表达式引擎变量
fn dto_to_variables<C: Serialize>(dto: &C) -> HashMap<String, Value> {
    serde_json::to_value(dto)
        .ok()
        .and_then(|v| {
            v.as_object()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        })
        .unwrap_or_default()
}

/// 将扩展产生的字段变更合并回 DTO
fn apply_mutations<T>(dto: &mut T, mutations: &HashMap<String, Value>)
where
    T: Serialize + DeserializeOwned,
{
    if mutations.is_empty() {
        return;
    }
    let mut value = match serde_json::to_value(&dto) {
        Ok(v) => v,
        Err(_) => return,
    };
    if let Some(obj) = value.as_object_mut() {
        for (k, v) in mutations {
            obj.insert(k.clone(), v.clone());
        }
    }
    if let Ok(new_dto) = serde_json::from_value(value) {
        *dto = new_dto;
    }
}

/// 将扩展运行时错误转换为 `AliothError`
fn extension_err_to_alioth(e: ExtensionRuntimeError) -> AliothError {
    AliothError::Internal(format!("Extension execution failed: {}", e))
}

/// 扩展感知创建 handler
///
/// 在标准创建前执行 `before_create` 扩展（约束验证 + 业务规则），
/// 在创建后执行 `after_create` 扩展（工作流触发）。
pub async fn crud_create_with_extensions<E, C, U, R, Err>(
    pool: web::Data<sqlx::PgPool>,
    registry: web::Data<AppExtensionRegistry>,
    app_ctx: web::Data<AppContext>,
    req: HttpRequest,
    body: web::Json<C>,
    bus: Option<web::Data<Arc<dyn common::event_bus::DomainEventBus>>>,
) -> Result<HttpResponse, Err>
where
    E: AliothDbEntity + Serialize,
    C: Serialize + DeserializeOwned + Send + Sync + 'static,
    U: Send + Sync + 'static,
    R: AliothRepository<E, C, U, Err> + From<sqlx::PgPool>,
    Err: ResponseError
        + std::error::Error
        + From<sqlx::Error>
        + From<AliothError>
        + Send
        + Sync
        + 'static,
{
    let entity_name = entity_name_from_type::<E>();
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".to_string()))?;

    let mut body = body.into_inner();

    // 1. before_create 扩展
    let mut variables = dto_to_variables(&body);
    let ext_result = registry
        .before_create(&app_ctx.app_code, entity_name, &mut variables)
        .map_err(extension_err_to_alioth)?;

    if !ext_result.all_passed {
        return Err(AliothError::BadRequest(ext_result.blocking_errors.join("; ")).into());
    }
    apply_mutations(&mut body, &ext_result.mutations);

    // 2. 标准创建
    let repo = R::from(pool.get_ref().clone());
    let item = repo.create(body, user_id).await?;

    // 3. after_create 扩展
    let after_vars = dto_to_variables(&item);
    let _ = registry
        .after_create(&app_ctx.app_code, entity_name, &after_vars)
        .map_err(extension_err_to_alioth);

    // 领域事件：实体创建（审批自动触发订阅者的输入通道；best-effort）
    publish_entity_created(
        bus.as_ref().map(|b| b.get_ref()),
        E::table_name(),
        item.id(),
        user_id,
    )
    .await;

    Ok(HttpResponse::Created().json(ApiResponse::success(item)))
}

/// 扩展感知更新 handler
///
/// 在标准更新前执行 `before_update` 扩展，
/// 在更新后执行 `after_update` 扩展。
pub async fn crud_update_with_extensions<E, C, U, R, Err>(
    pool: web::Data<sqlx::PgPool>,
    registry: web::Data<AppExtensionRegistry>,
    app_ctx: web::Data<AppContext>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<U>,
) -> Result<HttpResponse, Err>
where
    E: AliothDbEntity + Serialize,
    C: Send + Sync + 'static,
    U: Serialize + DeserializeOwned + Send + Sync + 'static,
    R: AliothRepository<E, C, U, Err> + From<sqlx::PgPool>,
    Err: ResponseError
        + std::error::Error
        + From<sqlx::Error>
        + From<AliothError>
        + Send
        + Sync
        + 'static,
{
    let entity_name = entity_name_from_type::<E>();
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".to_string()))?;

    let mut body = body.into_inner();
    let id = path.into_inner();

    // 1. before_update 扩展
    let mut variables = dto_to_variables(&body);
    variables.insert("id".to_string(), Value::Number(id.into()));

    // 读取当前实体状态（用于状态机转换验证等）
    let repo_for_fetch = R::from(pool.get_ref().clone());
    let current_variables: HashMap<String, Value> = match repo_for_fetch.get(id).await {
        Ok(Some(entity)) => serde_json::to_value(&entity)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .map(|m| m.into_iter().collect())
            .unwrap_or_default(),
        _ => HashMap::new(),
    };

    let ext_result = registry
        .before_update(
            &app_ctx.app_code,
            entity_name,
            &mut variables,
            &current_variables,
        )
        .map_err(extension_err_to_alioth)?;

    if !ext_result.all_passed {
        return Err(AliothError::BadRequest(ext_result.blocking_errors.join("; ")).into());
    }
    apply_mutations(&mut body, &ext_result.mutations);

    // 2. 标准更新
    let repo = R::from(pool.get_ref().clone());
    match repo.update(id, body, user_id).await? {
        Some(item) => {
            // 3. after_update 扩展
            let after_vars = dto_to_variables(&item);
            let _ = registry
                .after_update(&app_ctx.app_code, entity_name, &after_vars)
                .map_err(extension_err_to_alioth);

            Ok(HttpResponse::Ok().json(ApiResponse::success(item)))
        }
        None => Err(AliothError::NotFound(format!("Entity {} not found", id)).into()),
    }
}

/// 扩展感知删除 handler
///
/// 在标准删除前执行 `before_delete` 扩展，
/// 在删除后执行 `after_delete` 扩展。
pub async fn crud_delete_with_extensions<E, C, U, R, Err>(
    pool: web::Data<sqlx::PgPool>,
    registry: web::Data<AppExtensionRegistry>,
    app_ctx: web::Data<AppContext>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, Err>
where
    E: AliothDbEntity + Serialize,
    C: Send + Sync + 'static,
    U: Send + Sync + 'static,
    R: AliothRepository<E, C, U, Err> + From<sqlx::PgPool>,
    Err: ResponseError
        + std::error::Error
        + From<sqlx::Error>
        + From<AliothError>
        + Send
        + Sync
        + 'static,
{
    let entity_name = entity_name_from_type::<E>();
    let user_id = extract_user_id(&req)
        .ok_or_else(|| AliothError::Unauthorized("Authentication required".to_string()))?;

    let id = path.into_inner();

    // 1. before_delete 扩展
    let mut variables = HashMap::new();
    variables.insert("id".to_string(), Value::Number(id.into()));
    let ext_result = registry
        .before_delete(&app_ctx.app_code, entity_name, &mut variables)
        .map_err(extension_err_to_alioth)?;

    if !ext_result.all_passed {
        return Err(AliothError::BadRequest(ext_result.blocking_errors.join("; ")).into());
    }

    // 2. 标准删除
    let repo = R::from(pool.get_ref().clone());
    let dk_ctx = resolve_dk_ctx::<E>(pool.get_ref(), &req).await;
    repo.delete_with_rls(id, user_id, dk_ctx.as_ref()).await?;

    // 3. after_delete 扩展
    let _ = registry
        .after_delete(&app_ctx.app_code, entity_name, &variables)
        .map_err(extension_err_to_alioth);

    Ok(HttpResponse::NoContent().finish())
}

// ===================================================================
// 糖函数：一键生成 5 条标准路由
// ===================================================================

/// 为 actix-web 生成标准 CRUD 路由配置
///
/// 适用于无需自定义 list 过滤的实体。若模块需要自定义 list 行为
///（如额外查询参数），请使用上述独立 `crud_*` handler 手动组合路由。
///
/// # 示例
///
/// ```rust,ignore
/// pub fn config(cfg: &mut web::ServiceConfig) {
///     cfg.configure(crud_routes::<
///         Product,
///         CreateProductRequest,
///         UpdateProductRequest,
///         ProductRepository,
///         ApiError,
///     >("/inventory/products"));
/// }
/// ```
pub fn crud_routes<E, C, U, R, Err>(path: &str) -> impl FnOnce(&mut web::ServiceConfig) + '_
where
    E: AliothDbEntity + Serialize + 'static,
    C: DeserializeOwned + Send + Sync + 'static,
    U: DeserializeOwned + Send + Sync + 'static,
    R: AliothRepository<E, C, U, Err> + From<sqlx::PgPool> + 'static,
    Err: ResponseError
        + std::error::Error
        + From<sqlx::Error>
        + From<AliothError>
        + Send
        + Sync
        + 'static,
{
    move |cfg| {
        cfg.service(
            web::scope(path)
                .route("", web::get().to(crud_list::<E, C, U, R, Err>))
                .route("", web::post().to(crud_create::<E, C, U, R, Err>))
                .route("/{id}", web::get().to(crud_get::<E, C, U, R, Err>))
                .route("/{id}", web::put().to(crud_update::<E, C, U, R, Err>))
                .route("/{id}", web::delete().to(crud_delete::<E, C, U, R, Err>))
                .route(
                    "/batch",
                    web::delete().to(crud_batch_delete::<E, C, U, R, Err>),
                ),
        );
    }
}

// ===================================================================
// 糖函数：扩展感知路由配置
// ===================================================================

/// 为 actix-web 生成扩展感知 CRUD 路由配置
///
/// 与 `crud_routes` 的区别：create/update/delete handler 会自动调用
/// `AppExtensionRegistry` 执行应用级约束验证和业务规则。
///
/// 实体名称从 `E` 的泛型类型名自动推导（如 `Order`）。
///
/// # 示例
///
/// ```rust,ignore
/// pub fn config(cfg: &mut web::ServiceConfig) {
///     cfg.configure(crud_routes_with_extensions::<
///         Order,
///         CreateOrderRequest,
///         UpdateOrderRequest,
///         OrderRepository,
///         ApiError,
///     >("/orders"));
/// }
/// ```
pub fn crud_routes_with_extensions<E, C, U, R, Err>(
    path: &str,
) -> impl FnOnce(&mut web::ServiceConfig) + '_
where
    E: AliothDbEntity + Serialize + 'static,
    C: Serialize + DeserializeOwned + Send + Sync + 'static,
    U: Serialize + DeserializeOwned + Send + Sync + 'static,
    R: AliothRepository<E, C, U, Err> + From<sqlx::PgPool> + 'static,
    Err: ResponseError
        + std::error::Error
        + From<sqlx::Error>
        + From<AliothError>
        + Send
        + Sync
        + 'static,
{
    move |cfg| {
        cfg.service(
            web::scope(path)
                .route("", web::get().to(crud_list::<E, C, U, R, Err>))
                .route(
                    "",
                    web::post().to(crud_create_with_extensions::<E, C, U, R, Err>),
                )
                .route("/{id}", web::get().to(crud_get::<E, C, U, R, Err>))
                .route(
                    "/{id}",
                    web::put().to(crud_update_with_extensions::<E, C, U, R, Err>),
                )
                .route(
                    "/{id}",
                    web::delete().to(crud_delete_with_extensions::<E, C, U, R, Err>),
                )
                .route(
                    "/batch",
                    web::delete().to(crud_batch_delete::<E, C, U, R, Err>),
                ),
        );
    }
}

// ===================================================================
// 单元测试 — 纯函数（无需数据库）
// ===================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use runtime_engine::ExtensionRuntimeError;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    use std::collections::HashMap;

    #[derive(Debug, Serialize, Deserialize, Clone)]
    struct MockDto {
        name: String,
        age: i32,
        active: bool,
    }

    #[test]
    fn test_dto_to_variables() {
        let dto = MockDto {
            name: "test".into(),
            age: 25,
            active: true,
        };
        let vars = dto_to_variables(&dto);
        assert_eq!(vars.len(), 3);
        assert_eq!(vars["name"], Value::String("test".into()));
        assert_eq!(vars["age"], Value::Number(25.into()));
        assert_eq!(vars["active"], Value::Bool(true));
    }

    #[test]
    fn test_dto_to_variables_empty_dto() {
        #[derive(Debug, Serialize, Deserialize)]
        struct EmptyDto {}
        let dto = EmptyDto {};
        let vars = dto_to_variables(&dto);
        assert!(vars.is_empty());
    }

    #[test]
    fn test_apply_mutations() {
        let mut dto = MockDto {
            name: "original".into(),
            age: 30,
            active: false,
        };
        let mut mutations = HashMap::new();
        mutations.insert("name".into(), Value::String("modified".into()));
        mutations.insert("active".into(), Value::Bool(true));

        apply_mutations(&mut dto, &mutations);
        assert_eq!(dto.name, "modified");
        assert_eq!(dto.age, 30); // unchanged
        assert!(dto.active);
    }

    #[test]
    fn test_apply_mutations_empty_no_change() {
        let mut dto = MockDto {
            name: "original".into(),
            age: 30,
            active: false,
        };
        let mutations = HashMap::new();
        apply_mutations(&mut dto, &mutations);
        assert_eq!(dto.name, "original");
    }

    #[test]
    fn test_apply_mutations_extra_field_ignored() {
        let mut dto = MockDto {
            name: "test".into(),
            age: 10,
            active: true,
        };
        let mut mutations = HashMap::new();
        mutations.insert("nonexistent".into(), Value::String("x".into()));
        apply_mutations(&mut dto, &mutations);
        // extra fields don't exist in struct, so DTO unchanged
        assert_eq!(dto.name, "test");
    }

    #[test]
    fn test_extension_err_to_alioth_internal() {
        let err = ExtensionRuntimeError::EvaluationFailed("test error".into());
        let result = extension_err_to_alioth(err);
        assert!(matches!(result, AliothError::Internal(_)));
        let msg = format!("{}", result);
        assert!(msg.contains("test error"));
    }

    #[test]
    fn test_extract_user_id_no_extension() {
        // 对于没有扩展的请求，应返回 None
        let req = actix_web::test::TestRequest::default().to_http_request();
        let result = extract_user_id(&req);
        assert!(result.is_none());
    }
    #[test]
    fn test_parse_visible_ids_valid() {
        let req = actix_web::test::TestRequest::default()
            .insert_header(("X-Visible-Ids", "1,2,3,5"))
            .to_http_request();
        let ids = parse_visible_ids(&req);
        assert_eq!(ids, Some(vec![1, 2, 3, 5]));
    }

    #[test]
    fn test_parse_visible_ids_spaces() {
        let req = actix_web::test::TestRequest::default()
            .insert_header(("X-Visible-Ids", " 10 , 20 ,30 "))
            .to_http_request();
        let ids = parse_visible_ids(&req);
        assert_eq!(ids, Some(vec![10, 20, 30]));
    }

    #[test]
    fn test_parse_visible_ids_empty() {
        let req = actix_web::test::TestRequest::default()
            .insert_header(("X-Visible-Ids", ""))
            .to_http_request();
        let ids = parse_visible_ids(&req);
        assert_eq!(ids, None);
    }

    #[test]
    fn test_parse_visible_ids_missing() {
        let req = actix_web::test::TestRequest::default().to_http_request();
        let ids = parse_visible_ids(&req);
        assert_eq!(ids, None);
    }

    #[test]
    fn test_parse_visible_ids_invalid() {
        let req = actix_web::test::TestRequest::default()
            .insert_header(("X-Visible-Ids", "abc, def"))
            .to_http_request();
        let ids = parse_visible_ids(&req);
        assert_eq!(ids, None); // all values fail to parse → None
    }

    #[test]
    fn test_parse_visible_ids_none_marker() {
        // 字面量 `none` = 显式空授权（fail-closed）→ Some([])，与列控 `none` 约定对称
        let req = actix_web::test::TestRequest::default()
            .insert_header(("X-Visible-Ids", "none"))
            .to_http_request();
        let ids = parse_visible_ids(&req);
        assert_eq!(ids, Some(vec![]));
    }
}
