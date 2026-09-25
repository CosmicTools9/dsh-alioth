//! 通讯录薄包装 Handler — 复用 Framework/contacts ContactsService
//!
//! 端点（挂载在 /service/isahl-db）：
//! - `GET /contacts`       — 联系人列表（分页）
//! - `POST /contacts`      — 创建联系人
//! - `GET /contacts/{id}`  — 获取单个联系人
//! - `PUT /contacts/{id}`  — 更新联系人
//! - `DELETE /contacts/{id}` — 删除联系人（软删除）
//!
//! DTO：`{id, name, code, comments, infos:[{kind,value,is_default}]}`
//! L2 语义命名：notice→name（由 Framework contacts 提供）
//!
//! 员工管理读径（显式参数，**缺省即现状**）：`GET /contacts?owner=natural`
//! **以自然人为全集**——主表 = `zc_id_empl-natural`（与主体列表页「自然人」同集），
//! LEFT JOIN 该自然人的联系人链（联系方式）、证据信息、所属主体与**交易视角**；无联系人行的自然人**仍然出现**
//! （联系方式列空 ⇒ 前端显 `—`）。DTO 另填 `ownerName`（所属主体）+ `ownerId`（自然人 id）
//! + `contactId`（该自然人的联系人行 id，无则省略）+ `identities:[{type,no}]`（证件信息）
//! + `viewTags:[{code,notice}]`（交易视角标签，与主体列表页同源同键）。
//! 可选 `view=<code>` 只在本读径内做视角筛选（按岗位侧视角标签，`pv.ref_right` = 被看待主体）。
//! 不带 `owner` 时 SQL 与 DTO 均与改造前逐字等价（新字段 `skip_serializing_if` 省略，
//! 缺省读径不走本分支的任何代码）。
//!
//! 复用策略（REUSE_FIRST）：create/update/delete 委托 `ContactsService` 公开方法；
//! list 因需 q 关键字过滤（ContactsService 无 keyword 支持，crud raw_filter 不支持
//! 参数绑定）改由 handler 侧 sqlx QueryBuilder 参数化查询，行映射复用 get 同款
//! `SELECT_FIELDS` + `build_refs_select_suffix`（不重写聚合解析）；
//! code/comments 随请求体直传 Framework 模型（Create/UpdateContactRequest），同事务落列，
//! 响应始终从真实行映射。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use crud::entity::AliothDbEntity;
use crud::reference::build_refs_select_suffix;
use framework_contacts::{
    models::{
        ContactInfoInput, ContactInfoValue, ContactsEntity, CreateContactRequest,
        UpdateContactRequest,
    },
    ContactsService,
};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};

/// 证件信息项（员工管理读径 `owner=natural` 时填充：证件类型 + 证件号）
#[derive(Debug, Serialize)]
struct ContactIdentityDto {
    #[serde(rename = "type")]
    kind: String,
    no: String,
}

/// 交易视角标签项（员工管理读径 `owner=natural` 时填充：视角 `code` + 字典展示名 `notice`）
#[derive(Debug, Serialize)]
struct ContactViewTagDto {
    code: String,
    notice: String,
}

/// 前端 DTO：通讯录联系人（camelCase）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactDto {
    #[serde(with = "common::serde_zuid")]
    id: i64,
    name: String,
    code: String,
    comments: String,
    infos: Vec<ContactInfoValue>,
    /// 所属主体名（`owner=natural` 时填充）。
    /// `skip_serializing_if` ⇒ 缺省请求不带该键，返回与改造前**逐字等价**。
    #[serde(skip_serializing_if = "Option::is_none")]
    owner_name: Option<String>,
    /// 归属实体 id（= 自然人主体 id；`owner=natural` 时填充，缺省省略）。
    /// 证件编辑（`/subjects/{id}/identities`）与所属主体（`/organizations/{id}/employees`）
    /// 的寻址键——前端从本字段取主体 id，不用名称反查。
    #[serde(
        with = "common::serde_zuid::opt",
        skip_serializing_if = "Option::is_none"
    )]
    owner_id: Option<i64>,
    /// 该自然人的**联系人行 id**（`zc_id_contacts`；`owner=natural` 且自然人已挂联系人行时填充，
    /// 缺省读径/无联系人行时省略）。写面（`PUT/DELETE /contacts/{id}`）按联系人行寻址，
    /// 列表行的 `id` 是**自然人 id**，故写操作取本字段。
    #[serde(
        with = "common::serde_zuid::opt",
        skip_serializing_if = "Option::is_none"
    )]
    contact_id: Option<i64>,
    /// 证件信息（`owner=natural` 时填充；缺省同样省略）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    identities: Vec<ContactIdentityDto>,
    /// 交易视角标签（`owner=natural` 时填充；缺省读径恒为空 ⇒ 该键不出现，返回逐字不变）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    view_tags: Vec<ContactViewTagDto>,
}

/// 单条查询行（id/name/code/comments + refs 后缀；`owner=natural` 时附带归属/证件列）
#[derive(sqlx::FromRow)]
struct ContactRow {
    id: i64,
    #[sqlx(rename = "notice")]
    name: Option<String>,
    #[sqlx(default)]
    code: Option<String>,
    #[sqlx(default)]
    comments: Option<String>,
    #[sqlx(default)]
    _refs: Option<serde_json::Value>,
    /// 所属主体名（仅 `owner=natural` 查询携带该列；缺省列不存在 → 默认 None）
    #[sqlx(default)]
    owner_name: Option<String>,
    /// 归属实体 id（仅 `owner=natural` 查询携带该列；缺省列不存在 → 默认 None）
    #[sqlx(default)]
    owner_id: Option<i64>,
    /// 证件 JSONB 数组（仅 `owner=natural` 查询携带该列）
    #[sqlx(default)]
    identities: Option<serde_json::Value>,
    /// 该自然人的联系人行 id（仅 `owner=natural` 查询携带该列；无联系人行 ⇒ 列值为 NULL）
    #[sqlx(default)]
    contact_id: Option<i64>,
    /// 交易视角标签 JSONB 数组（仅 `owner=natural` 查询携带该列）
    #[sqlx(default)]
    view_tags: Option<serde_json::Value>,
}

/// 从证件 JSONB 数组（`[{"type","no"}]`）映射 DTO 列表
fn parse_identities(value: &Option<serde_json::Value>) -> Vec<ContactIdentityDto> {
    let Some(arr) = value.as_ref().and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let no = item.get("no").and_then(|v| v.as_str()).unwrap_or("");
            if no.is_empty() {
                return None;
            }
            Some(ContactIdentityDto {
                kind: item
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                no: no.to_string(),
            })
        })
        .collect()
}

/// 从视角标签 JSONB 数组（`[{"code","notice"}]`）映射 DTO 列表
fn parse_view_tags(value: &Option<serde_json::Value>) -> Vec<ContactViewTagDto> {
    let Some(arr) = value.as_ref().and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let code = item.get("code").and_then(|v| v.as_str()).unwrap_or("");
            if code.is_empty() {
                return None;
            }
            Some(ContactViewTagDto {
                code: code.to_string(),
                notice: item
                    .get("notice")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

/// 从真实行映射 ContactDto（code/comments 取行值，不再硬编码空串）
fn contact_row_to_dto(row: ContactRow) -> ContactDto {
    let infos = parse_contact_infos_from_refs(&row._refs);
    ContactDto {
        id: row.id,
        name: row.name.unwrap_or_default(),
        code: row.code.unwrap_or_default(),
        comments: row.comments.unwrap_or_default(),
        infos,
        owner_name: row.owner_name,
        owner_id: row.owner_id,
        contact_id: row.contact_id,
        identities: parse_identities(&row.identities),
        view_tags: parse_view_tags(&row.view_tags),
    }
}

/// `owner=natural`（员工管理）**主表 = 自然人全集**（`zc_id_empl-natural`，与主体列表页
/// 「自然人」同集——批注 30141b82：本页应有 9 条，与主体页一致）。
///
/// 联系人链（`zc_id_entity_rr_contacts` → `zc_id_contacts`）经 `LEFT JOIN LATERAL` 挂载：
/// 每个自然人**恒占一行**（无联系人行 ⇒ `e` 侧全 NULL → 联系方式列空 ⇒ 前端显 `—`），
/// 且按 `c.id` 取首条（现状 1 自然人对 1 联系人行；多行时不产生行乘）。
/// `e` 别名同时供 `ContactRow._refs`（`build_refs_select_suffix::<ContactsEntity>` 的
/// 联系人 → 联系方式叶子子查询，相关键为 `e.id`）复用——`e.id` 为 NULL 时各子查询为空 ⇒
/// `_refs = {}` ⇒ `infos = []`。
const NATURAL_FROM: &str = concat!(
    r#"FROM isahl."zc_id_empl-natural" AS en"#,
    r#" LEFT JOIN LATERAL (SELECT c.id FROM isahl."zc_id_entity_rr_contacts" dpr"#,
    r#"   JOIN isahl.zc_id_contacts c ON c.id = dpr.ref_right AND c.deleted_at IS NULL"#,
    r#"  WHERE dpr.ref_left = en.id AND dpr.deleted_at IS NULL"#,
    r#"  ORDER BY c.id LIMIT 1) e ON TRUE"#,
);

/// `owner=natural` 的行过滤：自然人全集（未删、有名称）——**无联系人行的自然人也返回**。
const NATURAL_FILTER: &str =
    r#" WHERE en.deleted_at IS NULL AND en.notice IS NOT NULL AND en.notice != ''"#;

/// `owner=natural` 的额外 SELECT 列（所属主体 + 自然人 id + 证件信息 + 联系人行 id）。
///
/// - 所属主体：自然人经雇佣桥 `zc_id_subj-org_rr_employee`（ref_right = 自然人）→
///   `zc_id_subjects.notice`；未挂主体时回退自然人自身名称（`zc_id_entity.notice`）。
/// - 证件信息：自然人 id 经 `zc_id_entity_rr_identity` → `zc_id_identity`
///   （`identity` = 证件号，`ck_category` → `zc_id_cate-identity.notice` = 证件类型），
///   与门户司机读径（OpenActivity supplier.rs `list_my_drivers`）同源同口径。
const NATURAL_OWNER_SELECT: &str = concat!(
    r#", (SELECT COALESCE("#,
    r#"(SELECT su.notice FROM isahl."zc_id_subj-org_rr_employee" ore"#,
    r#"   JOIN isahl.zc_id_subjects su ON su.id = ore.ref_left AND su.deleted_at IS NULL"#,
    r#"  WHERE ore.ref_right = en.id AND ore.deleted_at IS NULL LIMIT 1),"#,
    r#"(SELECT se.notice FROM isahl.zc_id_entity se WHERE se.id = en.id AND se.deleted_at IS NULL))) AS owner_name"#,
    // 自然人自身就是主体（`zc_id_empl-natural.id` = 主体 id）⇒ ownerId = 行 id；
    // 证件/所属主体写入面（/subjects/{id}/identities、/organizations/{id}/employees）按主体 id 寻址。
    r#", en.id AS owner_id"#,
    r#", (SELECT COALESCE(jsonb_agg(jsonb_build_object('type', c.notice, 'no', i.identity)"#,
    r#"        ORDER BY r.created_at DESC, r.id DESC), '[]'::jsonb)"#,
    r#"  FROM isahl."zc_id_entity_rr_identity" r"#,
    r#"  JOIN isahl."zc_id_identity" i ON i.id = r.ref_right AND i.deleted_at IS NULL"#,
    r#"  LEFT JOIN isahl."zc_id_cate-identity" c ON c.id = i.ck_category AND c.deleted_at IS NULL"#,
    r#" WHERE r.deleted_at IS NULL AND r.ref_left = en.id) AS identities"#,
    // 该自然人的联系人行 id（无 ⇒ NULL）：写面（PUT/DELETE /contacts/{id}）按联系人行寻址。
    r#", e.id AS contact_id"#,
);

/// `owner=natural` 的**交易视角** SELECT 列（`viewTags`）。
///
/// 来源链与主体列表页（`subjects.rs` `get_subject_view_tags` / 列表 `view_tags` 列）
/// **同源同键**：视角挂在**岗位**（`zc_id_subj-post_rr_view.ref_left`）上，被看待主体在
/// `ref_right` —— 本页主体即自然人 ⇒ `e.ref_right = en.id`（`zc_id_empl-natural.id` = 主体 id）。
/// 字典展示名取 `zc_id_tags-post_view.notice`；无标签 ⇒ `[]`（前端显 `—`）。
/// 同一自然人可能经多个岗位挂同一字典行 ⇒ 子查询内 `SELECT DISTINCT`（按字典行 id 去重，
/// 与 `subjects.rs` 列表的 `json_agg(DISTINCT vt.code)` 同效、且保留 `o_number` 排序）。
const NATURAL_VIEW_SELECT: &str = concat!(
    r#", COALESCE((SELECT jsonb_agg(jsonb_build_object('code', t.code, 'notice', t.notice)"#,
    r#"        ORDER BY t.o_number, t.id)"#,
    r#"  FROM (SELECT DISTINCT vt.id, vt.code, vt.notice, vt.o_number"#,
    r#"          FROM isahl."zc_id_subj-post_rr_view" pv"#,
    r#"          JOIN isahl."zc_id_relation-post_view_r_tags" pvr ON pvr.ref_left = pv.id AND pvr.deleted_at IS NULL"#,
    r#"          JOIN isahl."zc_id_tags-post_view" vt ON vt.id = pvr.ref_right AND vt.deleted_at IS NULL"#,
    r#"         WHERE pv.ref_right = en.id AND pv.deleted_at IS NULL) t), '[]'::jsonb) AS view_tags"#,
);

/// `owner=natural` 搜索：**所属主体**（与 `NATURAL_OWNER_SELECT` 的 COALESCE 同源——
/// 组织名优先、回退自然人自身名称）。标量表达式，无尾随闭合。
const NATURAL_SUBJECT_MATCH: &str = concat!(
    r#" OR (SELECT COALESCE("#,
    r#"(SELECT su.notice FROM isahl."zc_id_subj-org_rr_employee" ore"#,
    r#"   JOIN isahl.zc_id_subjects su ON su.id = ore.ref_left AND su.deleted_at IS NULL"#,
    r#"  WHERE ore.ref_right = en.id AND ore.deleted_at IS NULL LIMIT 1),"#,
    r#"(SELECT se.notice FROM isahl.zc_id_entity se WHERE se.id = en.id AND se.deleted_at IS NULL))) ILIKE "#,
);

/// `owner=natural` 搜索：**证据信息**（证件类型 `cate-identity.notice` 或证件号 `identity`；
/// 归属解析与 `NATURAL_OWNER_SELECT` 的 identities 子查询同源）。
/// `COALESCE(...) || ' ' || COALESCE(...)` 把两项并成一串 ⇒ 只需一个绑定、尾随 `)` 单一闭合。
const NATURAL_IDENTITY_MATCH: &str = concat!(
    r#" OR EXISTS (SELECT 1 FROM isahl."zc_id_entity_rr_identity" r"#,
    r#"   JOIN isahl.zc_id_identity i ON i.id = r.ref_right AND i.deleted_at IS NULL"#,
    r#"   LEFT JOIN isahl."zc_id_cate-identity" c ON c.id = i.ck_category AND c.deleted_at IS NULL"#,
    r#"  WHERE r.deleted_at IS NULL AND r.ref_left = en.id"#,
    r#"    AND COALESCE(i.identity, '') || ' ' || COALESCE(c.notice, '') ILIKE "#,
);

/// `owner=natural` 搜索：**联系方式**（「联系方式」列 = 该自然人的联系人链
/// `zc_id_entity_rr_contacts` → `zc_id_contacts` → `zc_id_contacts_rr_infos` → 信息叶表 `notice`）。
/// 七张叶表无共同父表可查，逐叶 UNION ALL（各自带 `deleted_at` 过滤）。
const NATURAL_INFO_MATCH: &str = concat!(
    r#" OR EXISTS (SELECT 1 FROM isahl."zc_id_entity_rr_contacts" dpr"#,
    r#"   JOIN isahl.zc_id_contacts cc ON cc.id = dpr.ref_right AND cc.deleted_at IS NULL"#,
    r#"   JOIN isahl."zc_id_contacts_rr_infos" ri ON ri.ref_left = cc.id AND ri.deleted_at IS NULL"#,
    r#"   JOIN ("#,
    r#"SELECT id, notice, deleted_at FROM isahl."zc_id_info-email" UNION ALL "#,
    r#"SELECT id, notice, deleted_at FROM isahl."zc_id_info-im" UNION ALL "#,
    r#"SELECT id, notice, deleted_at FROM isahl."zc_id_info-isahl" UNION ALL "#,
    r#"SELECT id, notice, deleted_at FROM isahl."zc_id_info-postal" UNION ALL "#,
    r#"SELECT id, notice, deleted_at FROM isahl."zc_id_info-telephone" UNION ALL "#,
    r#"SELECT id, notice, deleted_at FROM isahl."zc_id_info-url" UNION ALL "#,
    r#"SELECT id, notice, deleted_at FROM isahl."zc_id_info-zipcode") iv ON iv.id = ri.ref_right"#,
    r#"  WHERE dpr.ref_left = en.id AND dpr.deleted_at IS NULL AND iv.deleted_at IS NULL AND iv.notice ILIKE "#,
);

/// `owner=natural` 搜索：**交易视角**（视角 code 或字典展示名；来源链与 `NATURAL_VIEW_SELECT`
/// 同源同键）。
const NATURAL_VIEW_MATCH: &str = concat!(
    r#" OR EXISTS (SELECT 1 FROM isahl."zc_id_subj-post_rr_view" pv"#,
    r#"   JOIN isahl."zc_id_relation-post_view_r_tags" pvr ON pvr.ref_left = pv.id AND pvr.deleted_at IS NULL"#,
    r#"   JOIN isahl."zc_id_tags-post_view" vt ON vt.id = pvr.ref_right AND vt.deleted_at IS NULL"#,
    r#"  WHERE pv.ref_right = en.id AND pv.deleted_at IS NULL"#,
    r#"    AND COALESCE(vt.code, '') || ' ' || COALESCE(vt.notice, '') ILIKE "#,
);

/// 上述各段谓词的统一收尾：闭合 `EXISTS (`。
const KEYWORD_PREDICATE_TAIL: &str = ")";

/// 是否走员工（自然人）读径——仅显式 `owner=natural` 生效，其它值/缺省均保持原行为
fn natural_owner_enabled(owner: &Option<String>) -> bool {
    owner.as_deref().map(str::trim) == Some("natural")
}

/// 按 id 读取**联系人行**并映射 DTO（create/update/get 缺省读径共用）。
async fn fetch_contact_dto(pool: &PgPool, id: i64) -> Result<Option<ContactDto>, ApiError> {
    let refs_suffix = build_refs_select_suffix::<ContactsEntity>();
    let sql = format!(
        "SELECT e.id, e.notice AS notice, e.code AS code, e.comments AS comments {} FROM {} AS e WHERE e.id = $1 AND e.deleted_at IS NULL",
        refs_suffix,
        ContactsEntity::table_name()
    );

    let row: Option<ContactRow> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::from_sqlx)?;

    Ok(row.map(contact_row_to_dto))
}

/// 按 id 读取**自然人**（员工管理读径 `owner=natural`）并映射 DTO。
///
/// 行 id = 自然人 id（`zc_id_empl-natural.id` = 主体 id），与列表同源同结构；
/// 名称/编码/备注取自然人自身行值，联系方式取该自然人联系人链的 `_refs`（无联系人行 ⇒ 空）。
async fn fetch_natural_dto(pool: &PgPool, id: i64) -> Result<Option<ContactDto>, ApiError> {
    let refs_suffix = build_refs_select_suffix::<ContactsEntity>();
    let sql = format!(
        "SELECT en.id AS id, en.notice AS notice, en.code AS code, en.comments AS comments {}{}{} {}{} AND en.id = $1",
        refs_suffix,
        NATURAL_OWNER_SELECT,
        NATURAL_VIEW_SELECT,
        NATURAL_FROM,
        NATURAL_FILTER
    );

    let row: Option<ContactRow> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::from_sqlx)?;

    Ok(row.map(contact_row_to_dto))
}

/// ContactsService 返回 String 错误 → 统一映射为 Database 错误
fn map_service_err(e: String) -> ApiError {
    ApiError::Database(e)
}

/// 分页查询参数（snake_case 契约：page/page_size/q，对齐 subjects.rs SubjectListQuery）
#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    #[serde(default = "default_page")]
    page: i64,
    #[serde(default = "default_page_size")]
    page_size: i64,
    /// 搜索词（缺省读径匹配 notice / code；`owner=natural` 另覆盖备注/所属主体/证据信息/联系方式/交易视角）
    q: Option<String>,
    /// 显式读径选择：`natural` ⇒ 员工管理（**以自然人为全集** + 所属主体/证件/联系人行 id/交易视角）。
    /// 缺省/其它值 ⇒ 与改造前逐字等价。
    owner: Option<String>,
    /// 交易视角筛选（仅 `owner=natural` 读径生效；缺省/空串不过滤）。
    view: Option<String>,
}

/// 单条读径的读径选择参数（`owner=natural` 时详情同源返回所属主体/证件）
#[derive(Debug, Deserialize)]
pub struct OwnerQuery {
    owner: Option<String>,
}

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    20
}

/// 向查询构建器追加关键字过滤条件（参数化绑定，防注入；与 subjects.rs 同范式）。
///
/// 缺省（联系人）读径：匹配 `notice`（名称）/`code`（编码）。
fn push_keyword_filter(builder: &mut sqlx::QueryBuilder<sqlx::Postgres>, q: &Option<String>) {
    let kw = q.as_deref().map(str::trim).unwrap_or("");
    if kw.is_empty() {
        return;
    }
    let pat = format!("%{}%", kw);
    builder.push(" AND (e.notice ILIKE ");
    builder.push_bind(pat.clone());
    builder.push(" OR e.code ILIKE ");
    builder.push_bind(pat);
    builder.push(")");
}

/// `owner=natural`（员工管理）读径的关键字过滤：覆盖本页**每一列**——
/// 名称 / 编码 / 备注（自然人自身行值）+ 所属主体 / 证据信息 / 联系方式 / 交易视角（子查询谓词）。
///
/// 与前端 `ContactListPage.contactSearchText` 同集，保证服务端取回的页内行不会被本地筛选
/// 二次剔除（批注 fb9de670「搜索的内容多些」）。仅 `owner=natural` 分支使用——
/// 缺省读径的 SQL 逐字不变。
fn push_natural_keyword_filter(
    builder: &mut sqlx::QueryBuilder<sqlx::Postgres>,
    q: &Option<String>,
) {
    let kw = q.as_deref().map(str::trim).unwrap_or("");
    if kw.is_empty() {
        return;
    }
    let pat = format!("%{}%", kw);
    builder.push(" AND (en.notice ILIKE ");
    builder.push_bind(pat.clone());
    builder.push(" OR en.code ILIKE ");
    builder.push_bind(pat.clone());
    // 备注（本页「备注」列）——自然人自身列，无需子查询
    builder.push(" OR en.comments ILIKE ");
    builder.push_bind(pat.clone());
    // 所属主体（标量表达式，无尾随闭合）
    builder.push(NATURAL_SUBJECT_MATCH);
    builder.push_bind(pat.clone());
    // 证据信息 / 联系方式 / 交易视角（各一 EXISTS 子查询谓词，由 KEYWORD_PREDICATE_TAIL 闭合）
    for predicate in [
        NATURAL_IDENTITY_MATCH,
        NATURAL_INFO_MATCH,
        NATURAL_VIEW_MATCH,
    ] {
        builder.push(predicate);
        builder.push_bind(pat.clone());
        builder.push(KEYWORD_PREDICATE_TAIL);
    }
    builder.push(")");
}

/// `owner=natural`（员工管理）读径的**视角筛选**（`view=<code>`，缺省/空串不过滤）。
///
/// 谓词与 `subjects.rs` 列表的 `view` 过滤同源同键（`vt.code = $`）：视角挂在岗位
/// （`zc_id_subj-post_rr_view.ref_left`）、被看待主体在 `ref_right` ⇒ `pv.ref_right = en.id`。
/// 仅 `owner=natural` 分支使用——缺省读径收不到该参数，SQL 逐字不变。
fn push_natural_view_filter(
    builder: &mut sqlx::QueryBuilder<sqlx::Postgres>,
    view: &Option<String>,
) {
    let code = view.as_deref().map(str::trim).unwrap_or("");
    if code.is_empty() {
        return;
    }
    builder.push(
        " AND EXISTS (SELECT 1 FROM isahl.\"zc_id_subj-post_rr_view\" pv \
           JOIN isahl.\"zc_id_relation-post_view_r_tags\" pvr ON pvr.ref_left = pv.id AND pvr.deleted_at IS NULL \
           JOIN isahl.\"zc_id_tags-post_view\" vt ON vt.id = pvr.ref_right AND vt.deleted_at IS NULL \
          WHERE pv.ref_right = en.id AND pv.deleted_at IS NULL AND vt.code = ",
    );
    builder.push_bind(code.to_string());
    builder.push(")");
}

/// GET /contacts — 联系人列表（分页 + q 关键字过滤）
pub async fn list_contacts(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    query: web::Query<PaginationQuery>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "contacts", 0, "list").await?;

    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;

    // 员工管理读径（显式 `owner=natural`）：**以自然人为全集**（主表 `zc_id_empl-natural`），
    // 与缺省读径完全分离——缺省读径不走此分支的任何代码，SQL/DTO 与改造前逐字等价。
    if natural_owner_enabled(&query.owner) {
        return list_natural_contacts(
            pool.get_ref(),
            page,
            page_size,
            offset,
            &query.q,
            &query.view,
        )
        .await;
    }

    // 行级查询：SELECT_FIELDS 同语义（id/notice/code/comments + _refs 后缀），
    // q 过滤走参数化绑定（ContactsService 无 keyword 支持，raw_filter 不支持绑定参数）。
    let refs_suffix = build_refs_select_suffix::<ContactsEntity>();
    let mut builder: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(format!(
        "SELECT e.id, e.notice AS notice, e.code AS code, e.comments AS comments {} FROM {} AS e WHERE e.deleted_at IS NULL AND e.notice IS NOT NULL AND e.notice != ''",
        refs_suffix,
        ContactsEntity::table_name()
    ));
    push_keyword_filter(&mut builder, &query.q);
    builder.push(" ORDER BY e.id LIMIT ");
    builder.push_bind(page_size);
    builder.push(" OFFSET ");
    builder.push_bind(offset);
    let rows: Vec<ContactRow> = builder
        .build_query_as()
        .fetch_all(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?;

    // 同条件计数（total 与过滤结果一致）
    let mut counter: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(format!(
        "SELECT COUNT(*) FROM {} AS e WHERE e.deleted_at IS NULL AND e.notice IS NOT NULL AND e.notice != ''",
        ContactsEntity::table_name()
    ));
    push_keyword_filter(&mut counter, &query.q);
    let (total,): (i64,) = counter
        .build_query_as()
        .fetch_one(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?;

    let items: Vec<ContactDto> = rows.into_iter().map(contact_row_to_dto).collect();

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "items": items,
            "total": total,
            "page": page,
            "page_size": page_size,
        }))),
    )
}

/// 员工管理读径（`owner=natural`）：自然人全集分页列表 + 同条件计数。
///
/// 主表 = `zc_id_empl-natural`（LEFT JOIN 其联系人链，见 `NATURAL_FROM`）⇒
/// 返回**全部自然人**（含无联系人行者），与主体列表页「自然人」集合逐行一致。
async fn list_natural_contacts(
    pool: &PgPool,
    page: i64,
    page_size: i64,
    offset: i64,
    q: &Option<String>,
    view: &Option<String>,
) -> Result<HttpResponse, ApiError> {
    let refs_suffix = build_refs_select_suffix::<ContactsEntity>();
    let mut builder: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(format!(
        "SELECT en.id AS id, en.notice AS notice, en.code AS code, en.comments AS comments {}{}{} {}{}",
        refs_suffix, NATURAL_OWNER_SELECT, NATURAL_VIEW_SELECT, NATURAL_FROM, NATURAL_FILTER
    ));
    push_natural_keyword_filter(&mut builder, q);
    push_natural_view_filter(&mut builder, view);
    builder.push(" ORDER BY en.id LIMIT ");
    builder.push_bind(page_size);
    builder.push(" OFFSET ");
    builder.push_bind(offset);
    let rows: Vec<ContactRow> = builder
        .build_query_as()
        .fetch_all(pool)
        .await
        .map_err(ApiError::from_sqlx)?;

    // 同条件计数（total 与过滤结果一致；过滤谓词与行查询复用同一函数）
    let mut counter: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(format!(
        "SELECT COUNT(*) {}{}",
        NATURAL_FROM, NATURAL_FILTER
    ));
    push_natural_keyword_filter(&mut counter, q);
    push_natural_view_filter(&mut counter, view);
    let (total,): (i64,) = counter
        .build_query_as()
        .fetch_one(pool)
        .await
        .map_err(ApiError::from_sqlx)?;

    let items: Vec<ContactDto> = rows.into_iter().map(contact_row_to_dto).collect();

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "items": items,
            "total": total,
            "page": page,
            "page_size": page_size,
        }))),
    )
}

/// POST /contacts — 创建联系人
pub async fn create_contact(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<CreateContactInput>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "contacts", 0, "create").await?;

    let req_inner = CreateContactRequest {
        name: body.name.clone(),
        code: body.code.clone(),
        comments: body.comments.clone(),
        infos: body.infos.clone(),
    };

    let contact = ContactsService::create_contact(pool.get_ref(), req_inner)
        .await
        .map_err(map_service_err)?;

    let dto = fetch_contact_dto(pool.get_ref(), contact.id)
        .await?
        .ok_or_else(|| ApiError::Internal("created contact not found".into()))?;

    Ok(HttpResponse::Created().json(ApiResponse::success(dto)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateContactInput {
    name: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    comments: Option<String>,
    #[serde(default)]
    infos: Vec<ContactInfoInput>,
}

/// GET /contacts/{id}
pub async fn get_contact(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    query: web::Query<OwnerQuery>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "contacts", id, "read").await?;

    // `owner=natural`（员工管理）：按**自然人 id** 读取（与列表同源同结构）；
    // 缺省读径按联系人行 id 读取，SQL 与改造前逐字等价。
    let dto = if natural_owner_enabled(&query.owner) {
        fetch_natural_dto(pool.get_ref(), id).await?
    } else {
        fetch_contact_dto(pool.get_ref(), id).await?
    };
    match dto {
        Some(dto) => Ok(HttpResponse::Ok().json(ApiResponse::success(dto))),
        None => Err(ApiError::NotFound("Contact not found".into())),
    }
}

/// 从 `_refs` JSONB 解析联系方式数组（与 Framework service.rs entity_to_info 同逻辑）
fn parse_contact_infos_from_refs(_refs: &Option<serde_json::Value>) -> Vec<ContactInfoValue> {
    let mut infos = Vec::new();
    let Some(refs) = _refs else {
        return infos;
    };
    let kind_keys = ["email", "phone", "im", "isahl", "postal", "zipcode"];
    for kind in kind_keys {
        if let Some(arr) = refs.get(kind).and_then(|v| v.as_array()) {
            for item in arr {
                let value = item.get("notice").and_then(|v| v.as_str()).unwrap_or("");
                let is_default = item
                    .get("is_default")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                infos.push(ContactInfoValue {
                    kind: kind.to_string(),
                    value: value.to_string(),
                    is_default,
                });
            }
        }
    }
    infos
}

/// PUT /contacts/{id}
pub async fn update_contact(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<UpdateContactInput>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "contacts", id, "update").await?;

    let req_inner = UpdateContactRequest {
        name: body.name.clone(),
        code: body.code.clone(),
        comments: body.comments.clone(),
        infos: body.infos.clone(),
    };

    let contact = ContactsService::update_contact(pool.get_ref(), id, req_inner, user_id)
        .await
        .map_err(map_service_err)?;

    if contact.is_none() {
        return Err(ApiError::NotFound("Contact not found".into()));
    }

    let dto = fetch_contact_dto(pool.get_ref(), id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Contact not found".into()))?;

    Ok(HttpResponse::Ok().json(ApiResponse::success(dto)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateContactInput {
    name: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    comments: Option<String>,
    #[serde(default)]
    infos: Vec<ContactInfoInput>,
}

/// DELETE /contacts/{id}
pub async fn delete_contact(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "contacts", id, "delete").await?;

    let deleted = ContactsService::delete_contact(pool.get_ref(), id, user_id)
        .await
        .map_err(map_service_err)?;

    if !deleted {
        return Err(ApiError::NotFound("Contact not found".into()));
    }
    Ok(HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({ "deleted": true }))))
}

/// 注册通讯录路由
pub fn register(cfg: &mut actix_web::web::ServiceConfig) {
    cfg.service(
        web::resource("/contacts")
            .route(web::get().to(list_contacts))
            .route(web::post().to(create_contact)),
    )
    .service(
        web::resource("/contacts/{id}")
            .route(web::get().to(get_contact))
            .route(web::put().to(update_contact))
            .route(web::delete().to(delete_contact)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contact_row_to_dto_maps_real_code_comments() {
        // 纯逻辑：验证行映射（code/comments 取行值，不再硬编码空串）
        let row = ContactRow {
            id: 1,
            name: Some("张三".into()),
            code: Some("CT-001".into()),
            comments: Some("核心客户".into()),
            _refs: Some(serde_json::json!({
                "email": [{ "notice": "zhang@example.com", "is_default": true }],
            })),
            owner_name: None,
            owner_id: None,
            identities: None,
            contact_id: None,
            view_tags: None,
        };
        let dto = contact_row_to_dto(row);
        assert_eq!(dto.id, 1);
        assert_eq!(dto.name, "张三");
        assert_eq!(dto.code, "CT-001");
        assert_eq!(dto.comments, "核心客户");
        assert_eq!(dto.infos.len(), 1);
        assert_eq!(dto.infos[0].kind, "email");
        assert_eq!(dto.infos[0].value, "zhang@example.com");
        assert!(dto.infos[0].is_default);
        // 缺省读径（无 owner 列）⇒ 新字段省略，序列化与改造前逐字等价
        assert!(dto.owner_name.is_none());
        assert!(dto.owner_id.is_none());
        assert!(dto.identities.is_empty());
        assert!(dto.view_tags.is_empty());
        let json = serde_json::to_value(&dto).unwrap();
        assert!(json.get("ownerName").is_none());
        assert!(json.get("ownerId").is_none());
        assert!(json.get("contactId").is_none());
        assert!(json.get("identities").is_none());
        assert!(json.get("viewTags").is_none());
    }

    #[test]
    fn test_contact_row_to_dto_natural_owner_and_identities() {
        // 员工管理读径（自然人为全集）：所属主体 + 自然人 id + 联系人行 id + 证件信息随行映射
        let row = ContactRow {
            id: 7,
            name: Some("孙大勇 (司机) 电话".into()),
            code: Some("CT-WZ-BIZ-DRV-03".into()),
            comments: None,
            _refs: None,
            owner_name: Some("中铁物流有限公司".into()),
            owner_id: Some(343022366001683),
            identities: Some(serde_json::json!([
                { "type": "身份证", "no": "150203197908110019" },
            ])),
            contact_id: Some(343022366007830),
            view_tags: Some(serde_json::json!([
                { "code": "VIEW-EMPLOYEE", "notice": "雇员" },
            ])),
        };
        let dto = contact_row_to_dto(row);
        assert_eq!(dto.owner_name.as_deref(), Some("中铁物流有限公司"));
        assert_eq!(dto.identities.len(), 1);
        assert_eq!(dto.identities[0].kind, "身份证");
        assert_eq!(dto.identities[0].no, "150203197908110019");
        assert_eq!(dto.view_tags.len(), 1);
        assert_eq!(dto.view_tags[0].code, "VIEW-EMPLOYEE");
        assert_eq!(dto.view_tags[0].notice, "雇员");
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["ownerName"], "中铁物流有限公司");
        // 归属实体 id 字符串化（serde_zuid ⇒ 前端可直接用于 /subjects/{id}/identities 寻址）
        assert_eq!(json["ownerId"], "343022366001683");
        // 联系人行 id（写面寻址）：列表行 id 是自然人 id，写操作取 contactId
        assert_eq!(json["contactId"], "343022366007830");
        assert_eq!(json["identities"][0]["type"], "身份证");
        assert_eq!(json["identities"][0]["no"], "150203197908110019");
        assert_eq!(json["viewTags"][0]["code"], "VIEW-EMPLOYEE");
        assert_eq!(json["viewTags"][0]["notice"], "雇员");
    }

    #[test]
    fn test_contact_row_to_dto_without_infos_and_nullable() {
        let row = ContactRow {
            id: 2,
            name: Some("李四".into()),
            code: None,
            comments: None,
            _refs: None,
            owner_name: None,
            owner_id: None,
            identities: None,
            contact_id: None,
            view_tags: None,
        };
        let dto = contact_row_to_dto(row);
        assert_eq!(dto.id, 2);
        assert_eq!(dto.name, "李四");
        assert_eq!(dto.code, "");
        assert_eq!(dto.comments, "");
        assert!(dto.infos.is_empty());
    }

    #[test]
    fn test_parse_identities_skips_blank_no() {
        let value = serde_json::json!([
            { "type": "身份证", "no": "110101199001011234" },
            { "type": "护照", "no": "" },
            { "no": "X-1" },
        ]);
        let identities = parse_identities(&Some(value));
        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].kind, "身份证");
        assert_eq!(identities[0].no, "110101199001011234");
        assert_eq!(identities[1].kind, "");
        assert_eq!(identities[1].no, "X-1");
        assert!(parse_identities(&None).is_empty());
    }

    #[test]
    fn test_parse_view_tags_skips_blank_code() {
        let value = serde_json::json!([
            { "code": "VIEW-EMPLOYEE", "notice": "雇员" },
            { "code": "", "notice": "空码" },
            { "notice": "无码" },
        ]);
        let tags = parse_view_tags(&Some(value));
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].code, "VIEW-EMPLOYEE");
        assert_eq!(tags[0].notice, "雇员");
        // 无标签 ⇒ 空（DTO 侧 `skip_serializing_if` ⇒ 缺省读径不带 viewTags 键）
        assert!(parse_view_tags(&None).is_empty());
        assert!(parse_view_tags(&Some(serde_json::json!([]))).is_empty());
    }

    #[test]
    fn test_natural_owner_enabled_only_explicit_param() {
        // 只有显式 `owner=natural` 才切到员工（自然人全集）读径
        // 显式 `owner=natural` 才切换读径（含前后空白）；缺省/其它值保持原行为
        assert!(natural_owner_enabled(&Some("natural".into())));
        assert!(natural_owner_enabled(&Some(" natural ".into())));
        assert!(!natural_owner_enabled(&None));
        assert!(!natural_owner_enabled(&Some(String::new())));
        assert!(!natural_owner_enabled(&Some("company".into())));
    }

    #[test]
    fn test_parse_contact_infos_from_refs() {
        let refs = serde_json::json!({
            "email": [{ "notice": "a@b.com", "is_default": true }],
            "phone": [{ "notice": "13800000000", "is_default": false }],
        });
        let infos = parse_contact_infos_from_refs(&Some(refs));
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].kind, "email");
        assert_eq!(infos[0].value, "a@b.com");
        assert!(infos[0].is_default);
        assert_eq!(infos[1].kind, "phone");
        assert!(!infos[1].is_default);
    }

    #[test]
    fn test_parse_contact_infos_empty() {
        let infos = parse_contact_infos_from_refs(&None);
        assert!(infos.is_empty());
    }
}
