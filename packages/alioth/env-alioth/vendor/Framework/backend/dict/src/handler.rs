//! 描述信息（分类/标签字典）Handler — 综合管理「描述信息」Block
//!
//! 端点（挂载在 /service/isahl-db/dict）：
//! - `GET /dict/tables`                    — 字典表清单（zc_id_cate-*/zc_id_tags-*/基表，附 meta_collections 中文名）
//! - `GET /dict/leaf/{table}`              — 字典条目列表（分页 page/page_size）
//! - `GET /dict/leaf/{table}/{id}`         — 字典条目详情
//! - `POST /dict/leaf/{table}`             — 新增字典条目
//! - `PUT /dict/leaf/{table}/{id}`         — 更新字典条目
//! - `DELETE /dict/leaf/{table}/{id}`      — 软删字典条目
//!
//! 覆盖字典族（DB 实测 2026-08-14，共 66 张）：
//! - `zc_id_cate-*`（类目叶表，~44）+ 基表 `zc_id_category`
//! - `zc_id_tags-*`（标签叶表，~20）+ 基表 `zc_id_tags`
//!   叶表全部属于 `isahl_meta.gf_query_leafs('zc_id_object')` 集合（写入路径可用）；
//!   基表非 leaf（只读——列表可看，禁止新增/编辑/删除）。
//!
//! 复用策略（REUSE_FIRST）：
//! - 动态表 CRUD 委托 `crud::SchemaRepository`（list/get/create/update/delete，
//!   列白名单过滤 + 软删 + created_by_id 注入）——不重写 SQL。
//! - 表名白名单 + NGAC `require_resource_access` 在 handler 层包装：
//!   `crud::schema_routes` 裸挂无 allowlist 且 create 不注册 NGAC object_attribute
//!   （NGAC_SPEC §7.3：CRUD 创建行须自动注册，否则新行对创建者 RLS 不可见），
//!   故自定义包装而非直接 schema_routes（设计决策，见 change design §3.2）。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use crud::schema_repository::SchemaRepository;
use serde::Deserialize;
use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool, Row};

/// NGAC 资源名：dict（综合管理「描述信息」字典管理）
const NGAC_RESOURCE: &str = "dict";

/// 字典表白名单：cate/tags/unit 叶表 + 基表。防 URL 参数任意表读写。
/// 用 starts_with/== 前缀匹配（非正则，NO_REGEX_FOR_PARSING 合规）。
/// 注：`zc_id_tags_poi`（下划线变体，DB 实测存在）一并收录。
/// unit 族（计量单位字典 zc_id_unit-*，fix-vehicle-unit-binding-add-volume）：
/// 载重/体积等单位下拉经 /dict/leaf/{table} 直读，与 cate/tags 同级受控。
fn is_allowed_dict_table(table: &str) -> bool {
    table == "zc_id_category"
        || table == "zc_id_tags"
        || table == "zc_id_tags_poi"
        || table == "zc_id_cons-goods-tags"
        || table == "zc_id_unit"
        || table.starts_with("zc_id_tags-")
        // 类目字典（cate-*）与标签字典同样可维护：/dict/tables 左栏已列出，
        // 若不在此白名单，点击条目列表即 400「非字典表」。
        || table.starts_with("zc_id_cate-")
        || table.starts_with("zc_id_unit-")
}

/// 基表（非 leaf，只读）：zc_id_category / zc_id_tags / zc_id_unit
fn is_base_dict_table(table: &str) -> bool {
    table == "zc_id_category" || table == "zc_id_tags" || table == "zc_id_unit"
}
#[derive(Debug, Deserialize)]
pub struct DictListQuery {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
}

fn default_page() -> i64 {
    1
}

fn default_page_size() -> i64 {
    20
}

#[derive(Debug, Deserialize)]
pub struct ItemPath {
    pub table: String,
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
}

/// 字典表清单行（/dict/tables）
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DictTableRow {
    table: String,
    name: String,
    group: String,
    /// 是否只读基表（zc_id_category/zc_id_tags，非 leaf）
    read_only: bool,
    /// 表用户列（前端动态列渲染依据；取 DICT_WRITABLE_COLUMNS ∩ 表实际列）
    columns: Vec<String>,
}

/// GET /dict/tables — 字典表清单（cate/tags + 基表，附 meta_collections 中文名 + 可写列）
pub async fn list_dict_tables(pool: web::Data<PgPool>) -> Result<HttpResponse, ApiError> {
    let rows: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT c.relname
        FROM pg_catalog.pg_class c
        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace AND n.nspname = 'isahl'
        WHERE c.relkind = 'r'
          AND (c.relname LIKE 'zc_id_cate-%' OR c.relname LIKE 'zc_id_tags-%'
               OR c.relname LIKE 'zc_id_unit-%'
               OR c.relname IN ('zc_id_category', 'zc_id_tags', 'zc_id_cons-goods-tags', 'zc_id_unit'))
        ORDER BY c.relname
        "#,
    )
    .fetch_all(pool.get_ref())
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;

    // 字典表中文名映射（无 isahl_meta.meta_collections 依赖——WZ 库无该 schema，
    // 原 LEFT JOIN 导致 /dict/tables 500；缺省回退表名）。cate 45 + tags 20 +
    // unit 32 + 基表（zc_id_category / zc_id_cons-goods-tags / zc_id_tags / zc_id_unit）。
    fn display_name(table: &str) -> &str {
        match table {
            "zc_id_cate-acc-title" => "会计科目",
            "zc_id_cate-agent" => "代理类型",
            "zc_id_cate-approve" => "审批类型",
            "zc_id_cate-approve_role" => "审批岗位",
            "zc_id_cate-auth" => "认证类型",
            "zc_id_cate-bom-item" => "BOM 物料项",
            "zc_id_cate-certification" => "资质证照",
            "zc_id_cate-clause" => "条款类型",
            "zc_id_cate-contacts" => "联系人类型",
            "zc_id_cate-defect" => "缺陷类型",
            "zc_id_cate-department" => "部门类型",
            "zc_id_tags-milestone" => "里程碑标签",
            "zc_id_cate-tracking" => "追踪类型",
            "zc_id_cate-modify" => "变更类型",
            "zc_id_cate-log" => "日志类型",
            "zc_id_cate-alert" => "告警类型",
            "zc_id_cate-employment" => "雇佣类型",
            "zc_id_cate-file" => "文件类型",
            "zc_id_cate-group" => "组类型",
            "zc_id_cate-group_member" => "组成员类型",
            "zc_id_cate-identity" => "身份类型",
            "zc_id_cate-inspection" => "检验类型",
            "zc_id_cate-inv-title" => "发票抬头",
            "zc_id_cate-inv-title-ns" => "发票抬头（命名空间）",
            "zc_id_cate-inve-trasnfer" => "资产转移类型",
            "zc_id_cate-maintain" => "维护类型",
            "zc_id_cate-op_standard" => "作业标准",
            "zc_id_cate-ope-title" => "单据抬头",
            "zc_id_cate-ope-title-ns" => "单据抬头（命名空间）",
            "zc_id_cate-org_system" => "组织体系",
            "zc_id_cate-organization" => "组织类型",
            "zc_id_cate-position" => "岗位类型",
            "zc_id_cate-proc_op" => "流程操作类型",
            "zc_id_cate-process" => "流程类型",
            "zc_id_cate-project" => "项目类型",
            "zc_id_cate-society" => "社会类型",
            "zc_id_cate-sto-title" => "存储抬头",
            "zc_id_cate-subject" => "主体类型",
            "zc_id_cate-tax-title" => "税目",
            "zc_id_cate-testing" => "测试类型",
            "zc_id_cate-traffic" => "交通类型",
            "zc_id_cate-training" => "培训类型",
            "zc_id_cate-tsp" => "运输服务商类型",
            "zc_id_cate-tsp-title" => "TSP 抬头",
            "zc_id_cate-tsp-title-ns" => "TSP 抬头（命名空间）",
            "zc_id_cate-ver_branch" => "版本分支",
            "zc_id_cate-warehouse" => "仓库类型",
            "zc_id_cate-wh-title" => "仓库抬头",
            "zc_id_category" => "类目",
            "zc_id_cons-goods-tags" => "货物描述",
            "zc_id_tags" => "标签",
            "zc_id_tags-baseline" => "基线标签",
            "zc_id_tags-batch" => "批次标签",
            "zc_id_tags-bom_item" => "BOM 物料标签",
            "zc_id_tags-channel" => "渠道标签",
            "zc_id_tags-contacts" => "联系人标签",
            "zc_id_tags-event" => "事件标签",
            "zc_id_tags-finance" => "财务标签",
            "zc_id_tags-hscode" => "海关编码",
            "zc_id_tags-info_title" => "信息标题标签",
            "zc_id_tags-parties" => "当事人标签",
            "zc_id_tags-plan_action" => "计划动作标签",
            "zc_id_tags-post_view" => "岗位视图标签",
            "zc_id_tags-project" => "项目标签",
            "zc_id_tags-r-type" => "关系类型",
            "zc_id_tags-r-type-alias" => "关系类型别名",
            "zc_id_tags-skill" => "技能标签",
            "zc_id_tags-version" => "版本标签",
            "zc_id_tags-warehousing" => "仓储标签",
            "zc_id_unit" => "单位（组织）",
            "zc_id_unit-angle" => "单位-角度",
            "zc_id_unit-area" => "单位-面积",
            "zc_id_unit-common" => "单位-通用",
            "zc_id_unit-container" => "单位-容器",
            "zc_id_unit-currency" => "单位-货币",
            "zc_id_unit-current" => "单位-电流",
            "zc_id_unit-data" => "单位-数据",
            "zc_id_unit-density" => "单位-密度",
            "zc_id_unit-display" => "单位-显示",
            "zc_id_unit-distance" => "单位-距离",
            "zc_id_unit-duration" => "单位-时长",
            "zc_id_unit-energy" => "单位-能量",
            "zc_id_unit-frequency" => "单位-频率",
            "zc_id_unit-geo" => "单位-地理",
            "zc_id_unit-intensity" => "单位-强度",
            "zc_id_unit-luminance" => "单位-亮度",
            "zc_id_unit-magnetic_field_strength" => "单位-磁场强度",
            "zc_id_unit-magnetic_flux" => "单位-磁通量",
            "zc_id_unit-power" => "单位-功率",
            "zc_id_unit-pressure" => "单位-压力",
            "zc_id_unit-price" => "单位-价格",
            "zc_id_unit-pricing" => "单位-计价",
            "zc_id_unit-radiation" => "单位-辐射",
            "zc_id_unit-speed" => "单位-速度",
            "zc_id_unit-stress" => "单位-应力",
            "zc_id_unit-temperature" => "单位-温度",
            "zc_id_unit-voltage" => "单位-电压",
            "zc_id_unit-volume" => "单位-体积",
            "zc_id_unit-weight" => "单位-重量",
            "zc_id_unit-working" => "单位-工时",
            _ => table,
        }
    }

    let mut out: Vec<DictTableRow> = Vec::with_capacity(rows.len());
    for table in rows {
        let group = if table == "zc_id_category"
            || table.starts_with("zc_id_cate-")
            || table == "zc_id_cons-goods-tags"
        {
            "cate"
        } else if table == "zc_id_unit" || table.starts_with("zc_id_unit-") {
            "unit"
        } else {
            "tags"
        };
        let columns = dict_columns_for(pool.get_ref(), &table).await?;
        out.push(DictTableRow {
            table: table.clone(),
            name: display_name(&table).to_string(),
            group: group.to_string(),
            read_only: is_base_dict_table(&table),
            columns,
        });
    }

    Ok(HttpResponse::Ok().json(ApiResponse::success(out)))
}

/// GET /dict/leaf/{table} — 字典条目列表
pub async fn list_dict_entries(
    path: web::Path<String>,
    query: web::Query<DictListQuery>,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    let table = path.into_inner();
    if !is_allowed_dict_table(&table) {
        return Err(ApiError::BadRequest(format!("非字典表: {}", table)));
    }
    let repo = SchemaRepository::new(pool.get_ref().clone());
    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 500);
    let data = repo.list(&table, page, page_size).await?;
    // zuid（> 2^53）JSON number 会丢精度：id 一律转字符串，前端回传路径参数不失真
    let items: Vec<serde_json::Value> = data
        .into_iter()
        .map(|mut item| {
            if let Some(v) = item.get_mut("id") {
                if let Some(n) = v.as_i64() {
                    *v = serde_json::Value::String(n.to_string());
                }
            }
            item
        })
        .collect();
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "items": items,
            "total": items.len() as i64,
            "page": page,
            "page_size": page_size,
        }))),
    )
}

/// GET /dict/leaf/{table}/{id} — 字典条目详情
pub async fn get_dict_entry(
    path: web::Path<ItemPath>,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    let ItemPath { table, id } = path.into_inner();
    if !is_allowed_dict_table(&table) {
        return Err(ApiError::BadRequest(format!("非字典表: {}", table)));
    }
    let repo = SchemaRepository::new(pool.get_ref().clone());
    match repo.get(&table, id).await? {
        Some(mut v) => {
            // 同 list：zuid id 转字符串防前端精度丢失
            if let Some(id_v) = v.get_mut("id") {
                if let Some(n) = id_v.as_i64() {
                    *id_v = serde_json::Value::String(n.to_string());
                }
            }
            Ok(HttpResponse::Ok().json(ApiResponse::success(v)))
        }
        None => Err(ApiError::NotFound("not_found".into())),
    }
}

/// 字典表用户可写列（通用 CRUD 引擎保护 `notice`——Alioth 业务语义是系统维护；
/// 字典表 notice=名称 须用户写，故 dict handler 用本白名单 + 参数化 SQL 直写）。
/// 按表实际列存在性过滤（66 表列不一：cate 族有 enable/c_sort_，tags 族有
/// v_group/t_sort_/v_filter）。
const DICT_WRITABLE_COLUMNS: &[&str] = &[
    "notice", "code", "o_number", "comments", "enable", "t_color_", "c_sort_", "v_group",
    "t_sort_", "v_filter",
];

/// 查询表实际存在的列（白名单 ∩ 表列）
async fn dict_columns_for(pool: &PgPool, table: &str) -> Result<Vec<String>, ApiError> {
    let rows = sqlx::query(
        r#"SELECT column_name FROM information_schema.columns
           WHERE table_schema='isahl' AND table_name=$1"#,
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    let present: std::collections::HashSet<String> =
        rows.iter().map(|r| r.get::<String, _>(0)).collect();
    Ok(DICT_WRITABLE_COLUMNS
        .iter()
        .filter(|c| present.contains(**c))
        .map(|s| s.to_string())
        .collect())
}

/// POST /dict/leaf/{table} — 新增字典条目（基表只读拒绝；create 后注册 NGAC OA）
pub async fn create_dict_entry(
    path: web::Path<String>,
    body: web::Json<Value>,
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    let table = path.into_inner();
    if !is_allowed_dict_table(&table) {
        return Err(ApiError::BadRequest(format!("非字典表: {}", table)));
    }
    if is_base_dict_table(&table) {
        return Err(ApiError::BadRequest(format!(
            "基表只读，禁止写入: {}",
            table
        )));
    }
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, NGAC_RESOURCE, 0, "create").await?;

    let data = body.into_inner();
    let writable = dict_columns_for(pool.get_ref(), &table).await?;
    let mut data = data;
    // 批注 ff3c3ab9/3a6183d9：编码自动生成——code 未填时生成（DICT-<表后缀>-<序号>）
    if data
        .get("code")
        .is_none_or(|v| v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty()))
        && writable.iter().any(|c| c == "code")
    {
        let seq: i64 = sqlx::query_scalar("SELECT nextval('isahl.uid_seq')")
            .fetch_one(pool.get_ref())
            .await
            .map_err(ApiError::from)?;
        let suffix = table
            .trim_start_matches("zc_id_cate-")
            .trim_start_matches("zc_id_tags-");
        data["code"] = serde_json::Value::String(format!("DICT-{}-{}", suffix, seq % 100000));
    }
    let cols: Vec<&str> = writable
        .iter()
        .filter(|c| data.get(*c).is_some() && !data[*c].is_null())
        .map(|s| s.as_str())
        .collect();
    if cols.is_empty() {
        return Err(ApiError::BadRequest("no writable columns".into()));
    }
    let placeholders: Vec<String> = (1..=cols.len()).map(|i| format!("${}", i)).collect();
    let sql = format!(
        r#"INSERT INTO isahl."{table}" ({cols})
           VALUES ({ph})
           RETURNING id"#,
        table = table,
        cols = cols
            .iter()
            .map(|c| format!("\"{}\"", c))
            .collect::<Vec<_>>()
            .join(", "),
        ph = placeholders.join(", "),
    );
    let col_types = crud::column_types::resolve(pool.get_ref(), &table).await;
    let mut q = sqlx::query_as::<_, (i64,)>(AssertSqlSafe(sql.as_str()));
    for c in &cols {
        let v = data.get(*c).cloned().unwrap_or(Value::Null);
        let bound = crud::bind_json::coerce(&v, col_types.get(*c).map(String::as_str));
        q = crud::bind_json::apply_query_as(q, bound);
    }
    let (new_id,) = q
        .fetch_one(pool.get_ref())
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
    // NGAC_SPEC §7.3：CRUD 创建行必须注册 object_attribute + 创建者 association，
    // 否则新行对创建者 RLS 不可见/不可编辑。schema_routes 不触发，此处手动注册。
    register_dict_resource_ngac(pool.get_ref(), new_id, user_id).await;
    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": new_id.to_string(),
            "table": table,
        }))),
    )
}

/// PUT /dict/leaf/{table}/{id} — 更新字典条目（基表只读拒绝；COALESCE 语义）
pub async fn update_dict_entry(
    path: web::Path<ItemPath>,
    body: web::Json<Value>,
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    let ItemPath { table, id } = path.into_inner();
    if !is_allowed_dict_table(&table) {
        return Err(ApiError::BadRequest(format!("非字典表: {}", table)));
    }
    if is_base_dict_table(&table) {
        return Err(ApiError::BadRequest(format!(
            "基表只读，禁止写入: {}",
            table
        )));
    }
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, NGAC_RESOURCE, id, "update").await?;

    let data = body.into_inner();
    let writable = dict_columns_for(pool.get_ref(), &table).await?;
    let cols: Vec<&str> = writable
        .iter()
        .filter(|c| data.get(*c).is_some() && !data[*c].is_null())
        .map(|s| s.as_str())
        .collect();
    if cols.is_empty() {
        return Err(ApiError::BadRequest("no writable columns".into()));
    }
    let set_clause: Vec<String> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| format!("\"{}\" = ${}", c, i + 1))
        .collect();
    let n = cols.len();
    let sql = format!(
        r#"UPDATE isahl."{table}" SET {set}, updated_at = NOW(), updated_by_id = ${uid}
           WHERE id = ${id} AND deleted_at IS NULL RETURNING id"#,
        table = table,
        set = set_clause.join(", "),
        uid = n + 1,
        id = n + 2,
    );
    let mut q = sqlx::query_as::<_, (i64,)>(AssertSqlSafe(sql.as_str()));
    let col_types = crud::column_types::resolve(pool.get_ref(), &table).await;
    for c in &cols {
        let v = data.get(*c).cloned().unwrap_or(Value::Null);
        let bound = crud::bind_json::coerce(&v, col_types.get(*c).map(String::as_str));
        q = crud::bind_json::apply_query_as(q, bound);
    }
    q = q.bind(user_id).bind(id);
    let row: Option<(i64,)> = q
        .fetch_optional(pool.get_ref())
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
    match row {
        Some(_) => {
            let repo = SchemaRepository::new(pool.get_ref().clone());
            match repo.get(&table, id).await? {
                Some(v) => Ok(HttpResponse::Ok().json(ApiResponse::success(v))),
                None => Err(ApiError::NotFound("not_found".into())),
            }
        }
        None => Err(ApiError::NotFound("not_found".into())),
    }
}

/// POST /dict/leaf/{table} — 新增字典条目（基表只读拒绝；create 后注册 NGAC OA）
pub async fn delete_dict_entry(
    path: web::Path<ItemPath>,
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    let ItemPath { table, id } = path.into_inner();
    if !is_allowed_dict_table(&table) {
        return Err(ApiError::BadRequest(format!("非字典表: {}", table)));
    }
    if is_base_dict_table(&table) {
        return Err(ApiError::BadRequest(format!(
            "基表只读，禁止写入: {}",
            table
        )));
    }
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, NGAC_RESOURCE, id, "delete").await?;
    let repo = SchemaRepository::new(pool.get_ref().clone());
    repo.delete(&table, id, user_id).await?;
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "id": id.to_string(),
            "table": table,
            "deleted": true,
        }))),
    )
}

/// NGAC 注册：为新建字典条目创建 object_attribute + 创建者 full-CRUD association。
/// 对齐 `crud::handler::register_created_resource_ngac` 语义（泛型版本要求
/// `E: AliothDbEntity`，字典表为动态表名，此处按 resource_type='dict' 手动等价实现）。
async fn register_dict_resource_ngac(pool: &PgPool, item_id: i64, user_id: i64) {
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_object_attribute \
         (o_name, fk_policy_class, resource_type, fk_resource, created_by_id) \
         VALUES ($1, (SELECT id FROM isahl_auth.ngac_policy_class LIMIT 1), $2, $3, $4) \
         ON CONFLICT(resource_type, fk_resource) DO NOTHING",
    )
    .bind(format!("{}-{}", NGAC_RESOURCE, item_id))
    .bind(NGAC_RESOURCE)
    .bind(item_id)
    .bind(user_id)
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "INSERT INTO isahl_auth.ngac_association \
         (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at) \
         SELECT rr.fk_user_attribute, oa.id, \
                ARRAY(SELECT id FROM isahl_auth.ngac_access_right \
                      WHERE o_name IN ('read','write','delete','update','create')), \
                oa.fk_policy_class, NOW() \
         FROM isahl_auth.ngac_user_rr_attribute rr \
         JOIN isahl_auth.ngac_object_attribute oa \
           ON oa.resource_type = $2 AND oa.fk_resource = $3 AND oa.deleted_at IS NULL \
         WHERE rr.fk_user = $1 AND rr.deleted_at IS NULL \
           AND NOT EXISTS ( \
               SELECT 1 FROM isahl_auth.ngac_association a2 \
               WHERE a2.fk_user_attribute = rr.fk_user_attribute AND a2.fk_object_attribute = oa.id \
                 AND a2.deleted_at IS NULL) \
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(NGAC_RESOURCE)
    .bind(item_id)
    .execute(pool)
    .await;
}

/// 注册 dict 路由到 /service/isahl-db scope。
/// 用 web::resource（非 scope 内多段 route）——actix scope 对多段路径参数
/// 支持不稳（/leaf/{table}/{id} 曾 404），resource 级路径参数合法
/// （NGAC_SPEC §7.2 actix 路由注册顺序铁律）。
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/dict/tables").route(web::get().to(list_dict_tables)))
        .service(
            // 注意：同一路径的多个 method 必须合并在一个 resource 内——
            // actix 对同路径拆分多个 .service 只保留首个注册的 method（POST 会 405）。
            web::resource("/dict/leaf/{table}")
                .route(web::get().to(list_dict_entries))
                .route(web::post().to(create_dict_entry)),
        )
        .service(
            web::resource("/dict/leaf/{table}/{id}")
                .route(web::get().to(get_dict_entry))
                .route(web::put().to(update_dict_entry))
                .route(web::delete().to(delete_dict_entry)),
        );
}
