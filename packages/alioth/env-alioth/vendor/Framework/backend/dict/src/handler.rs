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
use common::audit::{record_audit_event_with_metadata, Decision};
use common::context::{extract_user_email, extract_username, require_auth};
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use crud::schema_repository::SchemaRepository;
use serde::Deserialize;
use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool, Row};

/// NGAC 资源名：dict（综合管理「描述信息」字典管理）
const NGAC_RESOURCE: &str = "dict";

/// 字典表白名单：cate/tags/unit 叶表 + 基表 + 模型级标准值域字典。防 URL 参数任意表读写。
/// 用 starts_with/== 前缀匹配（非正则，NO_REGEX_FOR_PARSING 合规）。
/// 注：`zc_id_tags_poi`（下划线变体，DB 实测存在）一并收录。
/// unit 族（计量单位字典 zc_id_unit-*，fix-vehicle-unit-binding-add-volume）：
/// 载重/体积等单位下拉经 /dict/leaf/{table} 直读，与 cate/tags 同级受控。
/// 模型级标准值域（2026-09-07，add-dict-data-locale-rendering）：国家/时区/政体
/// 字典同入 dict 管理面——locale 回显（en 精确键）覆盖主出口。
fn is_allowed_dict_table(table: &str) -> bool {
    table == "zc_id_category"
        || table == "zc_id_tags"
        || table == "zc_id_tags_poi"
        || table == "zc_id_cons-goods-tags"
        || table == "zc_id_unit"
        || table == "zc_id_subj-country"
        || table == "zc_id_cons-timezone-cate"
        || table == "zc_id_cons-polity-cate"
        // 类目字典（cate-*）与标签字典同样可维护：/dict/tables 左栏已列出，
        // 若不在此白名单，点击条目列表即 400「非字典表」。
        || table.starts_with("zc_id_cate-")
        // 共识族字典（cons-*：容器类目=车型 / 职能类目 / 要素类目 / 时区 / 政体 …）同款族前缀放行——
        // 原先只显式列 3 张（goods-tags / timezone-cate / polity-cate），车型字典等漏在清单外
        || table.starts_with("zc_id_cons-")
        || table.starts_with("zc_id_unit-")
        || table.starts_with("zc_id_tags-")
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
    /// 表列元数据（列级 schema 单源 `crate::schema`；前端动态列渲染依据）
    columns: Vec<DictColumnOut>,
    /// **模型级种子权威面**：本表 ∈ 库内 `isahl.gf_seed_tables()`（模型声明集）。
    /// true ⇒ 前端 MUST 提示「改动须纳入模型版本」：`namespace-db.sh seed`（默认 align）对该表
    /// `TRUNCATE TABLE ONLY` 后重放基线，未纳入基线的运行期行会被覆盖
    /// （用户裁决 2026-09-30：权威档 = 模型种子单一权威 + 机制化并入）。
    /// 判定函数缺席（部分 ns 壳库）/返回空集 ⇒ 回退 true——白名单表构造上即模型级 `isahl` 字典表。
    model_seeded: bool,
}

/// 列元数据（/dict/tables payload；列级 schema 产物逐列投影）。
/// `type` / `title` 一律来自产物——前后端 MUST NOT 各自硬编码列名或列中文名。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DictColumnOut {
    name: String,
    /// 模型元数据 data_type（`isahl_meta.meta_fields.data_type`）
    #[serde(rename = "type")]
    data_type: String,
    /// 列中文名（`meta_fields.title`；模型未填标题 ⇒ 空串）
    title: String,
    editable: bool,
}

/// GET /dict/tables — 字典表清单（cate/tags + 基表，附 meta_collections 中文名 + 列 schema）
pub async fn list_dict_tables(pool: web::Data<PgPool>) -> Result<HttpResponse, ApiError> {
    let rows: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT c.relname
        FROM pg_catalog.pg_class c
        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace AND n.nspname = 'isahl'
        WHERE c.relkind = 'r'
          AND (c.relname LIKE 'zc_id_cate-%' OR c.relname LIKE 'zc_id_tags-%'
               OR c.relname LIKE 'zc_id_unit-%' OR c.relname LIKE 'zc_id_cons-%'
               OR c.relname IN ('zc_id_category', 'zc_id_tags', 'zc_id_cons-goods-tags', 'zc_id_unit'))
        ORDER BY c.relname
        "#,
    )
    .fetch_all(pool.get_ref())
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;

    // 模型级种子声明集（权威档判定面）：探针 + 集合查询；缺席/空集 ⇒ 回退 true（见字段注释）。
    let (declared_available, declared_tables): (bool, std::collections::HashSet<String>) =
        if sqlx::query_scalar::<_, bool>(
            "SELECT to_regprocedure('isahl.gf_seed_tables()') IS NOT NULL",
        )
        .fetch_one(pool.get_ref())
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?
        {
            let set: std::collections::HashSet<String> =
                sqlx::query_scalar::<_, String>("SELECT table_name FROM isahl.gf_seed_tables()")
                    .fetch_all(pool.get_ref())
                    .await
                    .map_err(|e| ApiError::Database(e.to_string()))?
                    .into_iter()
                    .collect();
            let available = !set.is_empty();
            (available, set)
        } else {
            (false, std::collections::HashSet::new())
        };

    // 中文名权威 = 模型中心 `isahl_meta.meta_collections`（本端点自述契约，见文件头）。
    // 该表仅存在于 Meta 侧库（aliothstudio_dev / _test）；ns 壳库（wz/avic/se/cosmic）
    // 有 `isahl_meta` schema 但**无此表** ⇒ 静态 SQL 直 JOIN 缺表即 500（历史事故），
    // 故先做存在性探针，存在时才补查 name；缺席 → 回退静态短名表。
    let meta_names: std::collections::HashMap<String, String> = if sqlx::query_scalar::<_, bool>(
        "SELECT to_regclass('isahl_meta.meta_collections') IS NOT NULL",
    )
    .fetch_one(pool.get_ref())
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    {
        sqlx::query_as::<_, (String, String)>(
            "SELECT table_name, name FROM isahl_meta.meta_collections \
             WHERE table_name = ANY($1) AND name IS NOT NULL AND name <> ''",
        )
        .bind(&rows)
        .fetch_all(pool.get_ref())
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?
        .into_iter()
        .collect()
    } else {
        std::collections::HashMap::new()
    };

    // 静态短名表 = **回退**（仅服务无 meta 表的 ns 壳库）。MUST NOT 当作第二真相源：
    // 表在 meta 中有行时一律以 meta 的 name 为准（历史漂移：`zc_id_tags-r-type`
    // 此处曾写「关系类型」，模型中心实为「标签-容器分类」）。
    // cate 45 + tags 20 + unit 32 + cons 15 + 基表（zc_id_category / zc_id_cons-goods-tags / zc_id_tags / zc_id_unit）。
    fn display_name(table: &str) -> &str {
        // 值口径 = 模型语义核：`isahl_meta.meta_collections.name` 去掉首段族前缀
        //（类目- / 标签- / 单位- / 实现- / 共识-），保留 `/通用` `/自持` 后缀。
        // 唯一例外 `zc_id_tags-post_view`：模型名「标签-岗位视角」尚未随 2026-09-14
        //「关系视角」裁决更新（归模型中心人工通道），此处保留裁决值。
        match table {
            "zc_id_cate-acc-title" => "会计科目",
            "zc_id_cate-agent" => "智体",
            "zc_id_cate-alert" => "提示",
            "zc_id_cate-approve" => "审批类型",
            "zc_id_cate-approve_role" => "审批角色",
            "zc_id_cate-auth" => "认证方式",
            "zc_id_cate-bom-item" => "BOM单项",
            "zc_id_cate-certification" => "证书",
            "zc_id_cate-clause" => "条款类型",
            "zc_id_cate-contact_role" => "合约角色",
            "zc_id_cate-contacts" => "联系人分组",
            "zc_id_cate-cooperation" => "合作关系",
            "zc_id_cate-defect" => "缺陷类型",
            "zc_id_cate-department" => "部门",
            "zc_id_cate-employment" => "雇佣关系",
            "zc_id_cate-file" => "文件类型",
            "zc_id_cate-group" => "群组",
            "zc_id_cate-group_member" => "组员职能",
            "zc_id_cate-identity" => "身份",
            "zc_id_cate-inspection" => "质检",
            "zc_id_cate-inv-title" => "库存科目",
            "zc_id_cate-inv-title-cm" => "库存科目/通用",
            "zc_id_cate-inv-title-ns" => "库存科目/自持",
            "zc_id_cate-inve-trasnfer" => "移库动作",
            "zc_id_cate-log" => "日志类型",
            "zc_id_cate-maintain" => "维修",
            "zc_id_cate-modify" => "变更",
            "zc_id_cate-op_standard" => "操作项标准",
            "zc_id_cate-ope-title" => "经营科目",
            "zc_id_cate-ope-title-cm" => "经营科目/通用",
            "zc_id_cate-ope-title-ns" => "经营科目/自持",
            "zc_id_cate-org_system" => "经营制度",
            "zc_id_cate-organization" => "组织",
            "zc_id_cate-position" => "岗位",
            "zc_id_cate-proc_op" => "流程节点",
            "zc_id_cate-process" => "流程处理",
            "zc_id_cate-project" => "项目分类",
            "zc_id_cate-real_rights" => "物权分类",
            "zc_id_cate-seal" => "铅封",
            "zc_id_cate-society" => "社会",
            "zc_id_cate-sto-title" => "储元科目",
            "zc_id_cate-subject" => "主体",
            "zc_id_cate-tax-title" => "税收科目",
            "zc_id_cate-testing" => "测试项目",
            "zc_id_cate-tracking" => "追踪类型",
            "zc_id_cate-traffic" => "交通",
            "zc_id_cate-training" => "培训",
            "zc_id_cate-tsp" => "运单类目",
            "zc_id_cate-tsp-title" => "承运科目",
            "zc_id_cate-tsp-title-cm" => "承运科目/通用",
            "zc_id_cate-tsp-title-ns" => "承运科目/自持",
            "zc_id_cate-ver_branch" => "版控分支",
            "zc_id_cate-warehouse" => "仓储类型",
            "zc_id_cate-wh-title" => "工时科目",
            "zc_id_category" => "类目",
            "zc_id_cons-goods-tags" => "商品货描",
            // 共识族（cons-*）回退短名 = 模型集合名去族前缀（共识-／类目-／标签-），
            // 与 `isahl_meta.meta_collections.name` 同源；车型字典（容器类目）在本族内
            "zc_id_cons-consanguinity-cate" => "亲属/血缘类目",
            "zc_id_cons-cron-cate" => "周期类目",
            "zc_id_cons-date-tags" => "日期/时间-标签",
            "zc_id_cons-ethnic_group-cate" => "民族",
            "zc_id_cons-factor-cate" => "要素类目",
            "zc_id_cons-function-cate" => "职能类目",
            "zc_id_cons-industry-cate" => "行业类目",
            "zc_id_cons-license-cate" => "执照类目",
            "zc_id_cons-packing-cate" => "包装类目",
            "zc_id_cons-polity-cate" => "政体-类目",
            "zc_id_cons-r-type-cate" => "容器类目",
            "zc_id_cons-timezone-cate" => "时区-类目",
            "zc_id_cons-ts_concomitant-tags" => "时空伴随-标签",
            "zc_id_cons-zone-tags" => "地理分区-标签",
            "zc_id_tags" => "标签",
            "zc_id_tags-baseline" => "基线",
            "zc_id_tags-batch" => "批次",
            "zc_id_tags-bom_item" => "BOM单项",
            "zc_id_tags-channel" => "渠道",
            "zc_id_tags-contacts" => "联系人标签",
            "zc_id_tags-event" => "事件",
            "zc_id_tags-finance" => "财务单项",
            "zc_id_tags-hscode" => "HSCODE",
            "zc_id_tags-info_title" => "联系方式_标题",
            "zc_id_tags-milestone" => "里程碑",
            "zc_id_tags-parties" => "参与方",
            "zc_id_tags-plan_action" => "计划动作",
            "zc_id_tags-post_view" => "关系视角",
            "zc_id_tags-project" => "业务项目",
            "zc_id_tags-r-type" => "容器分类",
            "zc_id_tags-r-type-alias" => "容器类型别名",
            "zc_id_tags-skill" => "技能",
            "zc_id_tags-version" => "版本",
            "zc_id_tags-warehousing" => "仓储特性",
            "zc_id_unit" => "单位",
            "zc_id_unit-angle" => "角度",
            "zc_id_unit-area" => "面积",
            "zc_id_unit-common" => "日常",
            "zc_id_unit-container" => "容器",
            "zc_id_unit-currency" => "货币",
            "zc_id_unit-current" => "电流",
            "zc_id_unit-data" => "数据",
            "zc_id_unit-density" => "密度",
            "zc_id_unit-display" => "成像",
            "zc_id_unit-distance" => "距离",
            "zc_id_unit-duration" => "时长",
            "zc_id_unit-energy" => "能量",
            "zc_id_unit-frequency" => "频率",
            "zc_id_unit-geo" => "经纬",
            "zc_id_unit-illuminance" => "照度",
            "zc_id_unit-intensity" => "强度",
            "zc_id_unit-luminance" => "亮度",
            "zc_id_unit-magnetic_field_strength" => "磁场强度",
            "zc_id_unit-magnetic_flux" => "磁通量",
            "zc_id_unit-power" => "功率",
            "zc_id_unit-pressure" => "压强",
            "zc_id_unit-price" => "价格",
            "zc_id_unit-pricing" => "计价",
            "zc_id_unit-radiation" => "辐射强度",
            "zc_id_unit-speed" => "速度",
            "zc_id_unit-stress" => "力度",
            "zc_id_unit-temperature" => "温度",
            "zc_id_unit-voltage" => "电压",
            "zc_id_unit-volume" => "体积",
            "zc_id_unit-weight" => "重量",
            "zc_id_unit-working" => "工时",
            _ => table,
        }
    }

    let mut out: Vec<DictTableRow> = Vec::with_capacity(rows.len());
    for table in rows {
        let group = if table == "zc_id_category"
            || table.starts_with("zc_id_cate-")
            || table == "zc_id_cons-goods-tags"
            // 共识族：命名后缀即族属（`-cate` → 类目、`-tags` → 标签），其余值沿用下支
            || (table.starts_with("zc_id_cons-") && !table.ends_with("-tags"))
        {
            "cate"
        } else if table == "zc_id_unit" || table.starts_with("zc_id_unit-") {
            "unit"
        } else {
            "tags"
        };
        // 列元数据 = 列级 schema 单源（静态产物，无 DB 往返）
        let columns = schema_columns_of(&table);
        out.push(DictTableRow {
            table: table.clone(),
            name: meta_names
                .get(&table)
                .cloned()
                .unwrap_or_else(|| display_name(&table).to_string()),
            group: group.to_string(),
            read_only: is_base_dict_table(&table),
            columns,
            model_seeded: if declared_available {
                declared_tables.contains(&table)
            } else {
                true
            },
        });
    }

    // 顶层 schemaVersion（= 产物 model_version）：前端据此检测「产物 vs 服务端」失配
    // （值域办公室 e1；data 形态 = { schemaVersion, tables }，见 change design §接口契约）。
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "schemaVersion": crate::schema::model_version(),
            "tables": out,
        }))),
    )
}

/// GET /dict/leaf/{table} — 字典条目列表
pub async fn list_dict_entries(
    path: web::Path<String>,
    query: web::Query<DictListQuery>,
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    let table = path.into_inner();
    if !is_allowed_dict_table(&table) {
        return Err(ApiError::BadRequest(format!("非字典表: {}", table)));
    }
    let en = crate::locale::accept_is_en(
        req.headers()
            .get("accept-language")
            .and_then(|v| v.to_str().ok()),
    );
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
            // 数据值 locale 回显（DATA_I18N_SPEC）：en 请求覆盖 notice 显示名
            if en {
                localize_item_notice(&table, &mut item);
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
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    let ItemPath { table, id } = path.into_inner();
    if !is_allowed_dict_table(&table) {
        return Err(ApiError::BadRequest(format!("非字典表: {}", table)));
    }
    let en = crate::locale::accept_is_en(
        req.headers()
            .get("accept-language")
            .and_then(|v| v.to_str().ok()),
    );
    let repo = SchemaRepository::new(pool.get_ref().clone());
    match repo.get(&table, id).await? {
        Some(mut v) => {
            // 同 list：zuid id 转字符串防前端精度丢失
            if let Some(id_v) = v.get_mut("id") {
                if let Some(n) = id_v.as_i64() {
                    *id_v = serde_json::Value::String(n.to_string());
                }
            }
            // 数据值 locale 回显（DATA_I18N_SPEC）
            if en {
                localize_item_notice(&table, &mut v);
            }
            Ok(HttpResponse::Ok().json(ApiResponse::success(v)))
        }
        None => Err(ApiError::NotFound("not_found".into())),
    }
}

/// 按请求 locale 覆盖条目 `notice`（code 取条目 code 列，无 code 行跳过覆盖）。
fn localize_item_notice(table: &str, item: &mut serde_json::Value) {
    let code = item.get("code").and_then(|c| c.as_str());
    let localized = crate::locale::localize_notice(
        table,
        code,
        item.get("notice").and_then(|n| n.as_str()),
        true,
    );
    if let (Some(localized), Some(notice_v)) = (localized, item.get_mut("notice")) {
        *notice_v = serde_json::Value::String(localized);
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

/// 表列元数据（**唯一来源** = 列级 schema 产物 `crate::schema`；表不在产物内 ⇒ 空列集）。
fn schema_columns_of(table: &str) -> Vec<DictColumnOut> {
    crate::schema::table_columns(table)
        .unwrap_or(&[])
        .iter()
        .map(|c| DictColumnOut {
            name: c.name.to_string(),
            data_type: c.data_type.to_string(),
            title: c.title.to_string(),
            editable: c.editable,
        })
        .collect()
}

/// 写径可写列 = 列级 schema 的 editable 列 ∩ 既有白名单 ∩ 实表列（**不放宽**：
/// schema 未标 editable 的列——含值域家族外表——一律不可写；`cols.is_empty()` 即 400 fail-loud）。
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
    Ok(crate::schema::editable_columns(table)
        .unwrap_or(&[])
        .iter()
        .filter(|c| DICT_WRITABLE_COLUMNS.iter().any(|w| *w == **c))
        .filter(|c| present.contains(**c))
        .map(|c| (**c).to_string())
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
        let bound = crud::bind_json::coerce(
            &v,
            col_types
                .get(*c)
                .map(crud::column_types::ColumnMeta::data_type),
        );
        q = crud::bind_json::apply_query_as(q, bound);
    }
    let (new_id,) = q
        .fetch_one(pool.get_ref())
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
    // NGAC_SPEC §7.3：CRUD 创建行必须注册 object_attribute + 创建者 association，
    // 否则新行对创建者 RLS 不可见/不可编辑。schema_routes 不触发，此处手动注册。
    register_dict_resource_ngac(pool.get_ref(), new_id, user_id).await;
    // 留痕（office-audit-trace）：create 记新值摘要（仅实际写入的列）。
    let new_values = serde_json::Value::Object(
        cols.iter()
            .filter_map(|c| data.get(*c).map(|v| ((*c).to_string(), v.clone())))
            .collect(),
    );
    audit_value_domain_write(
        pool.get_ref(),
        user_id,
        &req,
        &table,
        new_id,
        "dict.create",
        None,
        Some(&new_values),
    )
    .await;
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
    // 前值摘要（留痕用）：行不存在时 UPDATE 无命中 ⇒ 404，审计不落（不产生成功事件）。
    let repo = SchemaRepository::new(pool.get_ref().clone());
    let before = repo.get(&table, id).await?;
    let mut q = sqlx::query_as::<_, (i64,)>(AssertSqlSafe(sql.as_str()));
    let col_types = crud::column_types::resolve(pool.get_ref(), &table).await;
    for c in &cols {
        let v = data.get(*c).cloned().unwrap_or(Value::Null);
        let bound = crud::bind_json::coerce(
            &v,
            col_types
                .get(*c)
                .map(crud::column_types::ColumnMeta::data_type),
        );
        q = crud::bind_json::apply_query_as(q, bound);
    }
    q = q.bind(user_id).bind(id);
    let row: Option<(i64,)> = q
        .fetch_optional(pool.get_ref())
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;
    match row {
        Some(_) => match repo.get(&table, id).await? {
            Some(v) => {
                // 留痕：update 记前后值摘要（before = 更新前镜像；after = 更新后镜像）
                audit_value_domain_write(
                    pool.get_ref(),
                    user_id,
                    &req,
                    &table,
                    id,
                    "dict.update",
                    before.as_ref(),
                    Some(&v),
                )
                .await;
                Ok(HttpResponse::Ok().json(ApiResponse::success(v)))
            }
            None => Err(ApiError::NotFound("not_found".into())),
        },
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
    // 被删行摘要（留痕用）：行不存在时 delete 亦 404，审计不落。
    let before = repo.get(&table, id).await?;
    repo.delete(&table, id, user_id).await?;
    audit_value_domain_write(
        pool.get_ref(),
        user_id,
        &req,
        &table,
        id,
        "dict.delete",
        before.as_ref(),
        None,
    )
    .await;
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "id": id.to_string(),
            "table": table,
            "deleted": true,
        }))),
    )
}

/// 值域写入留痕（`value-domain-office :: office-audit-trace`）：
/// 经 `common::audit::record_audit_event_with_metadata` 落 `isahl_audit.audit_events`，
/// 事件含 actor（user_id/username/email）、表名、行 id、前后值摘要。
///
/// 仅在写入**成功后**调用（被拒路径在调用点之前 return ⇒ MUST NOT 落成功事件）；
/// 非阻断语义（与 record_audit_event_* 一致）：留痕失败仅 warn，不回滚业务写入。
// 审计事件字段即参数面（actor/定位/操作/前后值），与 llm::service 同判据豁免打包
#[allow(clippy::too_many_arguments)]
async fn audit_value_domain_write(
    pool: &PgPool,
    user_id: i64,
    req: &HttpRequest,
    table: &str,
    row_id: i64,
    operation: &str,
    before: Option<&Value>,
    after: Option<&Value>,
) {
    let username = extract_username(req).unwrap_or_default();
    let subject = common::audit::resolve_subject(&username, user_id);
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "actor".to_string(),
        serde_json::json!({
            "user_id": user_id.to_string(),
            "username": username,
            "email": extract_user_email(req),
        }),
    );
    metadata.insert("table".to_string(), serde_json::json!(table));
    // zuid（> 2^53）以字符串入 metadata，避免 JSON number 精度丢失（与列表出口同口径）
    metadata.insert("id".to_string(), serde_json::json!(row_id.to_string()));
    if let Some(b) = before {
        metadata.insert("before".to_string(), b.clone());
    }
    if let Some(a) = after {
        metadata.insert("after".to_string(), a.clone());
    }
    if let Err(e) = record_audit_event_with_metadata(
        pool,
        user_id,
        &subject,
        &format!("dict:{}:{}", table, row_id),
        operation,
        &Decision::Permit,
        Value::Object(metadata),
    )
    .await
    {
        common::telemetry::warn!(
            "dict 写入留痕失败（{} {} {}）：{}",
            operation,
            table,
            row_id,
            e
        );
    }
}

/// 行级 NGAC 注册（**授权写入单源** = `common::ngac_grants`；best-effort）。
///
/// 对齐 `crud::handler::register_created_resource_ngac` 语义（**锚点单行**；创建者授权由派生层
/// 判定，derive-creator-grants-from-lifecycle）。失败有日志不阻断。
async fn register_dict_resource_ngac(pool: &PgPool, item_id: i64, user_id: i64) {
    if let Err(e) = common::ngac_grants::ensure_resource_oa(
        pool,
        &common::ngac_grants::ResourceOaSpec {
            o_name: format!("{NGAC_RESOURCE}-{item_id}"),
            resource_type: NGAC_RESOURCE,
            resource_id: item_id,
            resource_identifier: None,
            created_by: Some(user_id),
            policy_class_name: None,
        },
    )
    .await
    {
        common::telemetry::warn!("dict NGAC 锚点写入失败（{NGAC_RESOURCE} {item_id}）: {e}");
    }
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
