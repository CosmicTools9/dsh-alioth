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
use crate::reference;
use crate::repository::AliothRepository;
use common::{AliothError, ApiResponse};
use runtime_engine::{AppContext, AppExtensionRegistry, ExtensionResult, ExtensionRuntimeError};

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
    registry: Option<web::Data<AppExtensionRegistry>>,
    app_ctx: Option<web::Data<AppContext>>,
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
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
    let keys = entity_keys::<E>();
    let scope = ext_scope(&app_ctx);
    // 扩展钩子在**线载荷**上运行（DTO 无需实现 `Serialize`：全库 226 个 `Create*Request`
    // 多数只 derive `Deserialize`）；mutations 合并后再反序列化为 DTO。
    let mut payload = body.into_inner();
    // 0. before_create 扩展（约束 → 初始状态 → `onCreate` 规则；阻断 ⇒ 400）
    ext_before_create(ext_reg(&registry), &keys, scope, &mut payload)?;
    let dto: C = deserialize_payload(payload)?;
    let repo = R::from(pool.get_ref().clone());
    let dk_ctx = resolve_dk_ctx::<E>(pool.get_ref(), &req).await;
    let item = repo.create_with_rls(dto, user_id, dk_ctx.as_ref()).await?;
    let item_id = item.id();
    // NGAC: create Object Attribute for this resource (best-effort)
    register_created_resource_ngac::<E>(pool.get_ref(), item_id, user_id).await;
    // 扩展 after_create（`afterCreate`/`always` 规则 + 工作流触发标记；best-effort，留痕）
    ext_after_create(ext_reg(&registry), &keys, scope, &item);
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
    registry: Option<web::Data<AppExtensionRegistry>>,
    app_ctx: Option<web::Data<AppContext>>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<serde_json::Value>,
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
    let keys = entity_keys::<E>();
    let scope = ext_scope(&app_ctx);
    let mut payload = body.into_inner();
    // 0. before_update 扩展（约束 → 状态机 guard → `onUpdate` 规则；阻断 ⇒ 400）
    //    状态机 guard 的求值上下文 = **当前值 ⊕ 新值**（新值覆盖）：故先取存量行。
    let current_variables: HashMap<String, Value> = match repo.get(id).await {
        Ok(Some(entity)) => serde_json::to_value(&entity)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .map(|m| m.into_iter().collect())
            .unwrap_or_default(),
        _ => HashMap::new(),
    };
    ext_before_update(
        ext_reg(&registry),
        &keys,
        scope,
        &mut payload,
        &current_variables,
    )?;
    let dto: U = deserialize_payload(payload)?;
    let dk_ctx = resolve_dk_ctx::<E>(pool.get_ref(), &req).await;
    match repo
        .update_with_rls(id, dto, user_id, dk_ctx.as_ref())
        .await?
    {
        Some(item) => {
            // 扩展 after_update（`afterUpdate`/`always` 规则 + 工作流触发标记；best-effort，留痕）
            ext_after_update(ext_reg(&registry), &keys, scope, &item);
            Ok(HttpResponse::Ok().json(ApiResponse::success(item)))
        }
        None => Err(AliothError::NotFound(format!("Entity {} not found", id)).into()),
    }
}

/// 标准删除 handler
pub async fn crud_delete<E, C, U, R, Err>(
    pool: web::Data<sqlx::PgPool>,
    registry: Option<web::Data<AppExtensionRegistry>>,
    app_ctx: Option<web::Data<AppContext>>,
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
    let keys = entity_keys::<E>();
    let scope = ext_scope(&app_ctx);
    let repo = R::from(pool.get_ref().clone());
    // 行级可见性预检（NGAC_SPEC visible_ids 语义）：目标行不可见 → NotFound，禁止越权写。
    // 装配了扩展时**同时**取回存量行作删除期钩子上下文（可见性分支本就要读行，一次读取两用）。
    let existing = match parse_visible_ids(&req) {
        Some(visible_ids) => {
            let row = repo.get_with_rls(id, Some(&visible_ids), None).await?;
            if row.is_none() {
                return Err(AliothError::NotFound(format!("Entity {} not found", id)).into());
            }
            row
        }
        None if ext_reg(&registry).is_some() => repo.get(id).await?,
        None => None,
    };
    // 0. before_delete 扩展（约束 → `onDelete` 规则；阻断 ⇒ 400）
    //    上下文 = 存量行 ⊕ `id`：约束是**行状态谓词**；只给 `id` 时引用其他字段的 Error 级约束
    //    会因变量缺失而求值失败（求值失败按 level 处理 ⇒ Error 即阻断）⇒ 该实体删除恒阻断
    //    （锚定用例：`runtime-engine` `extension_pipeline::delete_hook_context_must_carry_existing_row`）。
    //    行不存在 ⇒ 无目标行可校验（DB 侧 no-op）⇒ 跳过钩子，避免把「无行」误判为违规。
    if let Some(row) = existing.as_ref() {
        let mut variables = delete_variables(Some(row), id);
        ext_before_delete(ext_reg(&registry), &keys, scope, &mut variables)?;
    }
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
    let dk_ctx = resolve_dk_ctx::<E>(pool.get_ref(), &req).await;
    repo.delete_with_rls(id, user_id, dk_ctx.as_ref()).await?;
    // 扩展 after_delete（`afterDelete`/`always` 规则 + 工作流触发标记；best-effort，留痕）
    ext_after_delete(ext_reg(&registry), &keys, scope, id);
    Ok(HttpResponse::NoContent().finish())
}
/// 标准批量删除 handler
pub async fn crud_batch_delete<E, C, U, R, Err>(
    pool: web::Data<sqlx::PgPool>,
    registry: Option<web::Data<AppExtensionRegistry>>,
    app_ctx: Option<web::Data<AppContext>>,
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
    let keys = entity_keys::<E>();
    let scope = ext_scope(&app_ctx);
    let reg = ext_reg(&registry);
    // 0. before_delete 扩展（约束 → `onDelete` 规则）：**逐行**执行，与单行删除同语义；
    //    任一阻断 ⇒ 400 且**一行不删**（钩子面原子：不得出现「前几行通过、后几行被拒但都已删」）。
    //    上下文 = 存量行 ⊕ `id`（约束是行状态谓词；只给 `id` 会让引用其他字段的 Error 级约束
    //    求值失败 ⇒ 恒阻断）。行仅在装配了扩展时读取。
    if let Some(registry) = reg {
        for id in &ids {
            // 行不存在 ⇒ 无目标行可校验（DB 侧本身 no-op）：跳过钩子。
            // 否则「行状态谓词」类约束会因变量缺失求值失败而被判违规 ⇒ 幂等批删被误 400。
            let Some(existing) = repo.get(*id).await? else {
                continue;
            };
            let mut variables = delete_variables(Some(&existing), *id);
            ext_before_delete(Some(registry), &keys, scope, &mut variables)?;
        }
    }
    let deleted_ids = ids.clone();
    repo.batch_delete(ids, user_id).await?;
    // 扩展 after_delete（`afterDelete`/`always` 规则 + 工作流触发标记）：逐行 best-effort 留痕
    for id in deleted_ids {
        ext_after_delete(reg, &keys, scope, id);
    }
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
                .route("/{id:\\d+}", web::get().to(crud_get_refs::<E>)),
        );
    }
}

// ===================================================================
// 扩展感知 handler — 自动注入应用级逻辑扩展
// ===================================================================

/// 将 DTO 序列化为表达式引擎变量
pub(crate) fn dto_to_variables<C: Serialize>(dto: &C) -> HashMap<String, Value> {
    serde_json::to_value(dto)
        .ok()
        .and_then(|v| {
            v.as_object()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        })
        .unwrap_or_default()
}

/// 删除期钩子上下文 = **存量行** ⊕ `id`（唯一构造点）
///
/// 约束是**行状态谓词**：只给 `id` 时，引用其他字段的 Error 级约束会因变量缺失求值失败
/// （求值失败按 `level` 处理 ⇒ Error 即阻断）⇒ 该实体删除恒阻断。
/// 锚定用例：`runtime-engine` `extension_pipeline::delete_hook_context_must_carry_existing_row`。
pub(crate) fn delete_variables<C: Serialize>(row: Option<&C>, id: i64) -> HashMap<String, Value> {
    let mut variables = row.map(dto_to_variables).unwrap_or_default();
    variables.insert("id".to_string(), Value::Number(id.into()));
    variables
}

/// 将扩展运行时错误转换为 `AliothError`
fn extension_err_to_alioth(e: ExtensionRuntimeError) -> AliothError {
    AliothError::Internal(format!("Extension execution failed: {}", e))
}

// ===================================================================
// 扩展钩子执行面 — 标准路由内联（无独立工厂）
// ===================================================================

/// 实体候选键：业务名（`Entity::ENTITY_NAME`，历史声明口径）+ 物理表名去 schema/引号
/// （2026-09-22 起 `extensions/*.yaml` 的口径）。两种口径 MUST 同时可解析——
/// 禁「改 YAML 迁就运行时」的单向收口；逐键尝试，任一键命中即执行。
pub(crate) fn entity_keys<E: AliothDbEntity>() -> [&'static str; 2] {
    [E::ENTITY_NAME, bare_table_name(E::table_name())]
}

/// `isahl."zc_id_x"` → `zc_id_x`（去 schema 限定与双引号；`table_name()` 的两种既有形态）
fn bare_table_name(table_name: &str) -> &str {
    table_name
        .rsplit('.')
        .next()
        .unwrap_or(table_name)
        .trim_matches('"')
}

/// 装配态注册表（`web::Data<T>` 双层 Deref：`Data` → `Arc` → `T`）；未装配 ⇒ None（直通）。
fn ext_reg(registry: &Option<web::Data<AppExtensionRegistry>>) -> Option<&AppExtensionRegistry> {
    registry.as_ref().map(|data| &***data)
}

/// 扩展作用域提示：`AppContext` 已装配 ⇒ 其 `app_code`（精确优先命中）；
/// 未装配 ⇒ 空串 = 不做精确提示，注册表按实体名跨 app 解析（见 `AppExtensionRegistry::resolve_app`）。
fn ext_scope(app_ctx: &Option<web::Data<AppContext>>) -> &str {
    app_ctx.as_ref().map(|c| c.app_code.as_str()).unwrap_or("")
}

/// 钩子结果留痕（best-effort 面：不阻断主操作，但 MUST 可见——静默 = 生产不可见）
fn log_ext_outcome(
    stage: &str,
    entity: &str,
    outcome: Result<ExtensionResult, ExtensionRuntimeError>,
) {
    match outcome {
        Ok(res) if !res.all_passed => common::telemetry::warn!(
            "{stage} 扩展未全部通过（{entity} @ {}）：{}",
            res.evaluations
                .first()
                .map(|e| e.evaluated_at.as_str())
                .unwrap_or("-"),
            res.blocking_errors.join("; ")
        ),
        Err(e) => common::telemetry::error!("{stage} 扩展执行失败（{entity}）：{e}"),
        Ok(_) => {}
    }
}

/// JSON 对象 → 表达式变量（非对象载荷 ⇒ 空集，与 `dto_to_variables` 同语义）
fn json_variables(payload: &serde_json::Value) -> HashMap<String, Value> {
    payload
        .as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

/// 扩展 mutations 合并回线载荷
fn apply_json_mutations(payload: &mut serde_json::Value, mutations: &HashMap<String, Value>) {
    if mutations.is_empty() {
        return;
    }
    if let Some(obj) = payload.as_object_mut() {
        for (k, v) in mutations {
            obj.insert(k.clone(), v.clone());
        }
    }
}

/// 线载荷 → DTO（DTO 自带约束——`deny_unknown_fields` / `rename_all` 等——照常生效）。
/// 失败 ⇒ 400（与 actix `Json` 提取器的反序列化失败同语义；报文由本项目错误类型统一）。
fn deserialize_payload<T: DeserializeOwned>(payload: serde_json::Value) -> Result<T, AliothError> {
    serde_json::from_value(payload)
        .map_err(|e| AliothError::BadRequest(format!("请求体与 DTO 不匹配: {e}")))
}

/// 创建前扩展（约束 → 初始状态 → `onCreate` 规则）：阻断 ⇒ 400；mutations 回写线载荷。
pub(crate) fn ext_before_create(
    registry: Option<&AppExtensionRegistry>,
    keys: &[&str],
    scope: &str,
    payload: &mut serde_json::Value,
) -> Result<(), AliothError> {
    let Some(registry) = registry else {
        return Ok(());
    };
    for key in keys {
        let mut variables = json_variables(payload);
        let result = registry
            .before_create(scope, key, &mut variables)
            .map_err(extension_err_to_alioth)?;
        if !result.all_passed {
            return Err(AliothError::BadRequest(result.blocking_errors.join("; ")));
        }
        apply_json_mutations(payload, &result.mutations);
    }
    Ok(())
}

/// 创建后扩展（`afterCreate`/`always` 规则 + 工作流触发标记）：best-effort。
pub(crate) fn ext_after_create(
    registry: Option<&AppExtensionRegistry>,
    keys: &[&str],
    scope: &str,
    item: &impl Serialize,
) {
    let Some(registry) = registry else {
        return;
    };
    for key in keys {
        let variables = dto_to_variables(item);
        log_ext_outcome(
            "after_create",
            key,
            registry.after_create(scope, key, &variables),
        );
    }
}

/// 更新前扩展（约束 → 状态机 guard → `onUpdate` 规则）：阻断 ⇒ 400；mutations 回写线载荷。
pub(crate) fn ext_before_update(
    registry: Option<&AppExtensionRegistry>,
    keys: &[&str],
    scope: &str,
    payload: &mut serde_json::Value,
    current: &HashMap<String, Value>,
) -> Result<(), AliothError> {
    let Some(registry) = registry else {
        return Ok(());
    };
    for key in keys {
        let mut variables = json_variables(payload);
        if let Some(id) = current.get("id") {
            variables.insert("id".to_string(), id.clone());
        }
        let result = registry
            .before_update(scope, key, &mut variables, current)
            .map_err(extension_err_to_alioth)?;
        if !result.all_passed {
            return Err(AliothError::BadRequest(result.blocking_errors.join("; ")));
        }
        apply_json_mutations(payload, &result.mutations);
    }
    Ok(())
}

/// 更新后扩展（`afterUpdate`/`always` 规则 + 工作流触发标记）：best-effort。
pub(crate) fn ext_after_update(
    registry: Option<&AppExtensionRegistry>,
    keys: &[&str],
    scope: &str,
    item: &impl Serialize,
) {
    let Some(registry) = registry else {
        return;
    };
    for key in keys {
        let variables = dto_to_variables(item);
        log_ext_outcome(
            "after_update",
            key,
            registry.after_update(scope, key, &variables),
        );
    }
}

/// 删除前扩展（约束 → `onDelete` 规则）：阻断 ⇒ 400。
pub(crate) fn ext_before_delete(
    registry: Option<&AppExtensionRegistry>,
    keys: &[&str],
    scope: &str,
    variables: &mut HashMap<String, Value>,
) -> Result<(), AliothError> {
    let Some(registry) = registry else {
        return Ok(());
    };
    for key in keys {
        let result = registry
            .before_delete(scope, key, variables)
            .map_err(extension_err_to_alioth)?;
        if !result.all_passed {
            return Err(AliothError::BadRequest(result.blocking_errors.join("; ")));
        }
    }
    Ok(())
}

/// 删除后扩展（`afterDelete`/`always` 规则 + 工作流触发标记）：best-effort。
pub(crate) fn ext_after_delete(
    registry: Option<&AppExtensionRegistry>,
    keys: &[&str],
    scope: &str,
    id: i64,
) {
    let Some(registry) = registry else {
        return;
    };
    for key in keys {
        let mut variables = HashMap::new();
        variables.insert("id".to_string(), Value::Number(id.into()));
        log_ext_outcome(
            "after_delete",
            key,
            registry.after_delete(scope, key, &variables),
        );
    }
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
// ===================================================================
// 引用解析感知路由：写入/单条读响应携带 `_refs`
// ===================================================================
/// 把行级引用解析结果补进响应 JSON（仅当 `_refs` 缺失时；best-effort，失败不影响主响应）。
///
/// 背景：页面「新建/编辑后把响应直接入本地列表镜像」，镜像行缺 `_refs` 时引用列瞬时渲染为空
/// （刷新后由列表读径补齐）。手写 repository 的 `get`/`create`/`update` 返回体不带引用后缀，
/// 故由本包装补一次；模板与 `/refs`、列表读径同源（`reference::build_refs_select_suffix`）。
async fn fill_refs<E>(pool: &sqlx::PgPool, id: i64, body: &mut serde_json::Value)
where
    E: AliothDbEntity + reference::HasReferenceJoins,
{
    if !body.get("_refs").map(|v| v.is_null()).unwrap_or(false) {
        return;
    }
    if let Ok(Some(refs)) = reference::fetch_refs_json::<E>(pool, id).await {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("_refs".to_string(), refs);
        }
    }
}

/// 单条获取（响应携带 `_refs`）
pub async fn crud_get_with_refs<E, C, U, R, Err>(
    pool: web::Data<sqlx::PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, Err>
where
    E: AliothDbEntity + reference::HasReferenceJoins + Serialize,
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
        Some(item) => {
            let mut body = serde_json::to_value(&item).unwrap_or(serde_json::Value::Null);
            fill_refs::<E>(pool.get_ref(), id, &mut body).await;
            Ok(HttpResponse::Ok().json(ApiResponse::success(body)))
        }
        None => Err(AliothError::NotFound(format!("Entity {} not found", id)).into()),
    }
}

/// 标准创建（响应携带 `_refs`；其余语义与 `crud_create` 一致：NGAC 注册 + EntityCreated 事件）
pub async fn crud_create_with_refs<E, C, U, R, Err>(
    pool: web::Data<sqlx::PgPool>,
    registry: Option<web::Data<AppExtensionRegistry>>,
    app_ctx: Option<web::Data<AppContext>>,
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
    bus: Option<web::Data<Arc<dyn common::event_bus::DomainEventBus>>>,
) -> Result<HttpResponse, Err>
where
    E: AliothDbEntity + reference::HasReferenceJoins + Serialize,
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
    let keys = entity_keys::<E>();
    let scope = ext_scope(&app_ctx);
    // 扩展钩子在**线载荷**上运行（DTO 无需实现 `Serialize`）
    let mut payload_in = body.into_inner();
    // 0. before_create 扩展（约束 → 初始状态 → `onCreate` 规则；阻断 ⇒ 400）
    ext_before_create(ext_reg(&registry), &keys, scope, &mut payload_in)?;
    let dto: C = deserialize_payload(payload_in)?;
    let repo = R::from(pool.get_ref().clone());
    let dk_ctx = resolve_dk_ctx::<E>(pool.get_ref(), &req).await;
    let item = repo.create_with_rls(dto, user_id, dk_ctx.as_ref()).await?;
    let item_id = item.id();
    register_created_resource_ngac::<E>(pool.get_ref(), item_id, user_id).await;
    // 扩展 after_create（best-effort，留痕）
    ext_after_create(ext_reg(&registry), &keys, scope, &item);
    publish_entity_created(
        bus.as_ref().map(|b| b.get_ref()),
        E::table_name(),
        item_id,
        user_id,
    )
    .await;
    let mut payload = serde_json::to_value(&item).unwrap_or(serde_json::Value::Null);
    fill_refs::<E>(pool.get_ref(), item_id, &mut payload).await;
    Ok(HttpResponse::Created().json(payload))
}

/// 标准更新（响应携带 `_refs`；其余语义与 `crud_update` 一致）
pub async fn crud_update_with_refs<E, C, U, R, Err>(
    pool: web::Data<sqlx::PgPool>,
    registry: Option<web::Data<AppExtensionRegistry>>,
    app_ctx: Option<web::Data<AppContext>>,
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<serde_json::Value>,
) -> Result<HttpResponse, Err>
where
    E: AliothDbEntity + reference::HasReferenceJoins + Serialize,
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
    common::permissions::require_resource_access(
        pool.get_ref(),
        user_id,
        &ngac_resource_name::<E>(),
        id,
        "update",
    )
    .await?;
    if let Some(visible_ids) = parse_visible_ids(&req) {
        let existing = repo.get_with_rls(id, Some(&visible_ids), None).await?;
        if existing.is_none() {
            return Err(AliothError::NotFound(format!("Entity {} not found", id)).into());
        }
    }
    let keys = entity_keys::<E>();
    let scope = ext_scope(&app_ctx);
    let mut payload = body.into_inner();
    // 0. before_update 扩展（约束 → 状态机 guard → `onUpdate` 规则；阻断 ⇒ 400）
    let current_variables: HashMap<String, Value> = match repo.get(id).await {
        Ok(Some(entity)) => serde_json::to_value(&entity)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .map(|m| m.into_iter().collect())
            .unwrap_or_default(),
        _ => HashMap::new(),
    };
    ext_before_update(
        ext_reg(&registry),
        &keys,
        scope,
        &mut payload,
        &current_variables,
    )?;
    let dto: U = deserialize_payload(payload)?;
    let dk_ctx = resolve_dk_ctx::<E>(pool.get_ref(), &req).await;
    match repo
        .update_with_rls(id, dto, user_id, dk_ctx.as_ref())
        .await?
    {
        Some(item) => {
            // 扩展 after_update（best-effort，留痕）
            ext_after_update(ext_reg(&registry), &keys, scope, &item);
            let mut payload = serde_json::to_value(&item).unwrap_or(serde_json::Value::Null);
            fill_refs::<E>(pool.get_ref(), id, &mut payload).await;
            Ok(HttpResponse::Ok().json(ApiResponse::success(payload)))
        }
        None => Err(AliothError::NotFound(format!("Entity {} not found", id)).into()),
    }
}

/// 为 actix-web 生成「引用解析感知」CRUD 路由配置
///
/// 与 `crud_routes` 的唯一区别：**单条 GET / create / update 的响应体在 `_refs` 缺失时会补一次
/// 引用解析**（模板与 `/refs`、列表读径同源）。用于「写入响应直接入前端本地列表镜像」的页面——
/// 镜像行缺 `_refs` 会导致引用列（如「变更日期」「健康状态」）新建后瞬时为空、刷新才恢复。
///
/// 仅对实现了 `HasReferenceJoins` 的实体可用（该 bound 无法加到 `crud_create`/`crud_update` 上：
/// 全库仍有未声明引用的实体，加 bound 会破坏其编译）。
pub fn crud_routes_with_refs<E, C, U, R, Err>(
    path: &str,
) -> impl FnOnce(&mut web::ServiceConfig) + '_
where
    E: AliothDbEntity + reference::HasReferenceJoins + Serialize + 'static,
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
                .route("", web::post().to(crud_create_with_refs::<E, C, U, R, Err>))
                .route(
                    "/{id:\\d+}",
                    web::get().to(crud_get_with_refs::<E, C, U, R, Err>),
                )
                .route(
                    "/{id:\\d+}",
                    web::put().to(crud_update_with_refs::<E, C, U, R, Err>),
                )
                .route(
                    "/{id:\\d+}",
                    web::delete().to(crud_delete::<E, C, U, R, Err>),
                )
                .route(
                    "/batch",
                    web::delete().to(crud_batch_delete::<E, C, U, R, Err>),
                ),
        );
    }
}

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
                .route("/{id:\\d+}", web::get().to(crud_get::<E, C, U, R, Err>))
                .route("/{id:\\d+}", web::put().to(crud_update::<E, C, U, R, Err>))
                .route(
                    "/{id:\\d+}",
                    web::delete().to(crud_delete::<E, C, U, R, Err>),
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
