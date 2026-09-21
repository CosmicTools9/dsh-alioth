//! 组织管理主体 Handler — WZ org-wz 模块数据源
//!
//! 覆盖：
//! - `GET /subjects` — 主体列表（视角/社会分类/搜索/分页筛选 + 可选 kind 按主体分类过滤 + 状态解析 + 视角标签聚合）
//! - `GET /subjects/{id}/view-tags` / `PUT /subjects/{id}/view-tags` — 交易视角标签读写
//! - `GET /subjects/{id}/accounts` / `POST /subjects/{id}/accounts` / `DELETE /subjects/{id}/accounts/{relId}` — 账户关联读写
//! - `POST /subjects` — 创建主体（notice/code 必填，可选 comments）
//! - `PUT /subjects/{id}` / `DELETE /subjects/{id}` — 更新 / 软删除主体
//!
//! 模型语义（零 DDL）：
//! - 主体统一叶表 `isahl.zc_id_subjects`（社会分类由 tableoid 叶表判定）
//! - 交易视角 = 主体→岗位(`zc_id_subj-post_rr_view`)→关联行标签(`zc_id_relation-post_view_r_tags`)
//!   →`zc_id_tags-post_view` 字典（标签宿主 = 关联行 id，非岗位 id）
//! - 账户关联 = `zc_id_subjects_rr_account`（ref_left=主体 id, ref_right=账户实体 id）
//! - 状态 = `zc_id_lifecycle_r_primary-status` → `zc_id_stus-org`（normal/disabled，黑名单语义）
//! - 状态切换写路径复用 contract-wz `POST /counterparties/{id}/status`，不在此重复实现

use crate::handlers::identities::category_id_by_code;
use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, QueryBuilder};

/// 主体 MDM 编码扩展表 ensure（零 DDL 交付：运行时幂等自愈 → backup-ddl 快照收编）。
/// isahl schema 冻结 + comments 禁嵌 JSON，MDM 编码落 `wz_fssc.subject_mdm`
/// （先例：subject_bank_card / subject_invoice_info）。
///
/// **一码一视角**：主键 `(subject_id, view_tag)`。用户裁决 2026-09-14：「MDM 码按视角不同」
/// （河北兴泰 客户 `F90E000298` / 供应商 `F400001114`；易运科技 两者均 `W90R0`）
/// ⇒ 单列主键 `(subject_id)` 装不下，升为复合主键；旧形态表在此**就地幂等迁移**。
/// AtomicBool 仅作免重复标记——DDL 幂等，并发重入无害。
static SUBJECT_MDM_ENSURED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 按视角存码的键空间顺序（兼作单值 `mdmCode` 兼容字段的优先级：CUST → SUPP → BIZ）。
pub(crate) const MDM_VIEW_TAG_PRIORITY: [&str; 3] = ["VIEW-CUST", "VIEW-SUPP", "VIEW-BIZ"];

pub(crate) async fn ensure_subject_mdm(pool: &PgPool) -> Result<(), ApiError> {
    use std::sync::atomic::Ordering;
    if SUBJECT_MDM_ENSURED.load(Ordering::Relaxed) {
        return Ok(());
    }
    // wz_fssc 为 WZ 扩展 schema（共享内核跨 namespace 复用——AVIC-CAASEC 等
    // 库无该 schema）：先自愈建 schema 再建表，两段均幂等。
    let result = async {
        sqlx::query("CREATE SCHEMA IF NOT EXISTS wz_fssc")
            .execute(pool)
            .await?;
        // 旧形态探测须在建表**之前**（建表后列恒在，探测失真）
        let table_exists: bool = sqlx::query_scalar(
            "SELECT to_regclass('wz_fssc.subject_mdm') IS NOT NULL",
        )
        .fetch_one(pool)
        .await?;
        let has_view_tag: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
              WHERE table_schema = 'wz_fssc' AND table_name = 'subject_mdm' \
                AND column_name = 'view_tag')",
        )
        .fetch_one(pool)
        .await?;

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS wz_fssc.subject_mdm (
                subject_id    bigint NOT NULL,
                view_tag      text NOT NULL DEFAULT '',
                mdm_code      text NOT NULL,
                created_by_id bigint,
                updated_by_id bigint,
                created_at    timestamptz DEFAULT now() NOT NULL,
                updated_at    timestamptz DEFAULT now() NOT NULL,
                PRIMARY KEY (subject_id, view_tag)
            )"#,
        )
        .execute(pool)
        .await?;

        if table_exists && !has_view_tag {
            // ── 迁移 ①：加列（先可空以容旧行）──────────────────────────────
            sqlx::query("ALTER TABLE wz_fssc.subject_mdm ADD COLUMN view_tag text")
                .execute(pool)
                .await?;
            sqlx::query("UPDATE wz_fssc.subject_mdm SET view_tag = '' WHERE view_tag IS NULL")
                .execute(pool)
                .await?;

            // ── 迁移 ②：旧行按视角展开回填（同一码填到该主体拥有的各视角）──
            // 键空间 = zc_id_subj-post_rr_view → relation-post_view_r_tags → tags-post_view
            sqlx::query(
                r#"INSERT INTO wz_fssc.subject_mdm
                       (subject_id, view_tag, mdm_code, created_by_id, updated_by_id, created_at, updated_at)
                   SELECT m.subject_id, vt.code, m.mdm_code, m.created_by_id, m.updated_by_id,
                          m.created_at, m.updated_at
                     FROM wz_fssc.subject_mdm m
                     JOIN "isahl"."zc_id_subj-post_rr_view" e
                       ON e.ref_right = m.subject_id AND e.deleted_at IS NULL
                     JOIN "isahl"."zc_id_relation-post_view_r_tags" r
                       ON r.ref_left = e.id AND r.deleted_at IS NULL
                     JOIN "isahl"."zc_id_tags-post_view" vt
                       ON vt.id = r.ref_right AND vt.deleted_at IS NULL
                    WHERE m.view_tag = ''
                   ON CONFLICT (subject_id, view_tag) DO NOTHING"#,
            )
            .execute(pool)
            .await?;

            // ── 迁移 ③：可判出视角者删兜底行；判不出者保留 view_tag='' 兜底 ──
            sqlx::query(
                r#"DELETE FROM wz_fssc.subject_mdm m
                    WHERE m.view_tag = ''
                      AND EXISTS (
                        SELECT 1 FROM "isahl"."zc_id_subj-post_rr_view" e
                          JOIN "isahl"."zc_id_relation-post_view_r_tags" r
                            ON r.ref_left = e.id AND r.deleted_at IS NULL
                          JOIN "isahl"."zc_id_tags-post_view" vt
                            ON vt.id = r.ref_right AND vt.deleted_at IS NULL
                         WHERE e.ref_right = m.subject_id AND e.deleted_at IS NULL)"#,
            )
            .execute(pool)
            .await?;

            sqlx::query("ALTER TABLE wz_fssc.subject_mdm ALTER COLUMN view_tag SET NOT NULL")
                .execute(pool)
                .await?;
            sqlx::query("ALTER TABLE wz_fssc.subject_mdm ALTER COLUMN view_tag SET DEFAULT ''")
                .execute(pool)
                .await?;
        }

        // ── 迁移 ④：主键升为 (subject_id, view_tag)——仅当仍为单列主键（可重入）──
        let legacy_pk: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_constraint c \
              JOIN pg_class t ON t.oid = c.conrelid \
              JOIN pg_namespace n ON n.oid = t.relnamespace \
              WHERE n.nspname = 'wz_fssc' AND t.relname = 'subject_mdm' AND c.contype = 'p' \
                AND pg_get_constraintdef(c.oid) = 'PRIMARY KEY (subject_id)')",
        )
        .fetch_one(pool)
        .await?;
        if legacy_pk {
            sqlx::query("ALTER TABLE wz_fssc.subject_mdm DROP CONSTRAINT subject_mdm_pkey")
                .execute(pool)
                .await?;
            sqlx::query(
                "ALTER TABLE wz_fssc.subject_mdm ADD CONSTRAINT subject_mdm_pkey \
                 PRIMARY KEY (subject_id, view_tag)",
            )
            .execute(pool)
            .await?;
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => {}
        // 并发首次 ensure 竞态：23505 唯一索引（pg_type/pg_class）、42701 列已存在、
        // 42P07 表已存在——均已由并发请求落定，视为成功
        Err(sqlx::Error::Database(e))
            if matches!(
                e.code().as_deref(),
                Some("23505") | Some("42701") | Some("42P07")
            ) => {}
        Err(e) => return Err(ApiError::from_sqlx(e)),
    }
    SUBJECT_MDM_ENSURED.store(true, Ordering::Relaxed);
    Ok(())
}

/// 主体拥有的交易视角 code 集合（键空间查询，供单值 `mdmCode` 兼容字段判定落码视角）。
pub(crate) async fn subject_view_tags(
    pool: &PgPool,
    subject_id: i64,
) -> Result<Vec<String>, ApiError> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT vt.code FROM \"isahl\".\"zc_id_subj-post_rr_view\" e \
         JOIN \"isahl\".\"zc_id_relation-post_view_r_tags\" r \
           ON r.ref_left = e.id AND r.deleted_at IS NULL \
         JOIN \"isahl\".\"zc_id_tags-post_view\" vt \
           ON vt.id = r.ref_right AND vt.deleted_at IS NULL \
         WHERE e.ref_right = $1 AND e.deleted_at IS NULL",
    )
    .bind(subject_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    Ok(rows)
}

/// 单值 `mdmCode` 兼容字段落码视角：按 CUST → SUPP → BIZ 取该主体**拥有**的首个；
/// 均不拥有（判不出）则 `VIEW-BIZ`。
pub(crate) async fn resolve_mdm_view_tag(
    pool: &PgPool,
    subject_id: i64,
) -> Result<String, ApiError> {
    let owned = subject_view_tags(pool, subject_id).await?;
    Ok(MDM_VIEW_TAG_PRIORITY
        .iter()
        .find(|tag| owned.iter().any(|o| o == *tag))
        .copied()
        .unwrap_or("VIEW-BIZ")
        .to_string())
}

/// 单视角写 MDM 码（空串 = 删该视角行，非空 = upsert）。调用方须先 `ensure_subject_mdm`。
pub(crate) async fn write_subject_mdm(
    pool: &PgPool,
    subject_id: i64,
    view_tag: &str,
    mdm_code: &str,
    user_id: i64,
) -> Result<(), ApiError> {
    let tag = view_tag.trim();
    let trimmed = mdm_code.trim();
    if trimmed.is_empty() {
        sqlx::query("DELETE FROM wz_fssc.subject_mdm WHERE subject_id = $1 AND view_tag = $2")
            .bind(subject_id)
            .bind(tag)
            .execute(pool)
            .await
            .map_err(ApiError::from_sqlx)?;
    } else {
        sqlx::query(
            r#"INSERT INTO wz_fssc.subject_mdm (subject_id, view_tag, mdm_code, created_by_id, updated_by_id)
               VALUES ($1, $2, $3, $4, $4)
               ON CONFLICT (subject_id, view_tag)
               DO UPDATE SET mdm_code = EXCLUDED.mdm_code,
                             updated_by_id = EXCLUDED.updated_by_id,
                             updated_at = now()"#,
        )
        .bind(subject_id)
        .bind(tag)
        .bind(trimmed)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(ApiError::from_sqlx)?;
    }
    Ok(())
}

/// 按视角批量写（映射语义）：值为空串 = 删该视角行，非空 = upsert；
/// **未出现在映射中的视角一律不动**（前端字段隐藏/未改时不误清其它视角）。
pub(crate) async fn write_subject_mdm_codes(
    pool: &PgPool,
    subject_id: i64,
    codes: &std::collections::BTreeMap<String, String>,
    user_id: i64,
) -> Result<(), ApiError> {
    ensure_subject_mdm(pool).await?;
    for (view_tag, code) in codes {
        write_subject_mdm(pool, subject_id, view_tag, code, user_id).await?;
    }
    Ok(())
}

/// 侧表行 → 视角映射（`jsonb_object_agg` 结果解码，键排序稳定）。
fn mdm_code_map(raw: &Option<serde_json::Value>) -> std::collections::BTreeMap<String, String> {
    raw.as_ref()
        .and_then(|v| v.as_object())
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// 单值 `mdmCode` 兼容字段：按 CUST → SUPP → BIZ 取首个非空；
/// 三者皆无时回退任意非空（含判不出视角的 `view_tag=''` 兜底行），全空则 None。
/// 保留原因：旧前端/详情页仍读单值字段，避免断裂。
fn compat_mdm_code(map: &std::collections::BTreeMap<String, String>) -> Option<String> {
    MDM_VIEW_TAG_PRIORITY
        .iter()
        .find_map(|tag| map.get(*tag).filter(|v| !v.is_empty()).cloned())
        .or_else(|| map.values().find(|v| !v.is_empty()).cloned())
}

/// 社会分类 ⇄ 主体叶表**单一映射表**（标签 → 表名、表名 → 标签双向派生，杜绝两份清单漂移）。
///
/// 值口径（2026-09-14 用户裁决「保持现状」）：本表是**社会分类维度值集**（前端选项 / 原型 8 类消费），
/// 非表名展示副本；与 `meta_collections.name` 的差异属同义异词（智体/智能体、群组/组、国家·地区/国家），
/// **既有标签不再按模型名改写**——表名展示面一致性由 `scripts/check/check-table-display-names.ts` 覆盖
/// （本表不在其判定面内）。本轮仅**补齐**此前未收录的主体树节点（模型语义名 → 既有文案风格）：
/// 主体-银行→银行机构、主体-层级→层级、主体-行政机关→行政机关、主体-主权实体→主权实体、主体-岗位→岗位。
const SUBJECT_CATEGORY_TABLES: [(&str, &str); 17] = [
    ("法人", "zc_id_orga-legal"),
    ("非银行法人", "zc_id_orga-non-banking-legal"),
    ("商业银行", "zc_id_bank-commercial"),
    ("中央银行", "zc_id_bank-central"),
    // 开户机构登记表（code=联行号；银行卡 fk_trustee 指向该子树行）
    ("银行机构", "zc_id_subj-bank"),
    ("层级", "zc_id_subj-hierarchy"),
    ("行政机关", "zc_id_subj-ministry"),
    ("主权实体", "zc_id_subj-sovereign"),
    ("组织", "zc_id_subj-org"),
    ("部门", "zc_id_orga-department"),
    ("组", "zc_id_subj-group"),
    ("雇员", "zc_id_subj-employee"),
    ("自然人", "zc_id_empl-natural"),
    ("智能体", "zc_id_empl-agent"),
    ("国家", "zc_id_subj-country"),
    ("超国家", "zc_id_subj-supranational"),
    ("岗位", "zc_id_subj-position"),
];

/// 叶表名（`pg_class.relname`，渲染无关）→ 社会分类标签；未收录（根表 / 关系表）→ 「主体」
fn social_category(leaf: &str) -> &'static str {
    SUBJECT_CATEGORY_TABLES
        .iter()
        .find(|(_, table)| *table == leaf)
        .map(|(label, _)| *label)
        .unwrap_or("主体")
}

/// 社会分类标签（或主体叶表名）→ 该分类的**基表**；未收录 → `None`（调用方 fail-closed 成空集）。
/// 筛选口径 = 基表 ∪ 其全部后代表（继承子树），见 `push_subject_filters`。
fn category_base_table(value: &str) -> Option<&'static str> {
    SUBJECT_CATEGORY_TABLES
        .iter()
        .find(|(label, table)| *label == value || *table == value)
        .map(|(_, table)| *table)
}

#[derive(Debug, Deserialize)]
pub struct SubjectListQuery {
    /// 关系视角 code（如 VIEW-CUST / VIEW-SUPP / VIEW-CPARTY / VIEW-REGULATOR）
    pub view: Option<String>,
    /// 社会分类标签（如 法人 / 自然人 / 商业银行）
    pub category: Option<String>,
    /// 搜索词（匹配 notice / code）
    pub q: Option<String>,
    /// 主体分类简写（customer / carrier / driver，映射 ck_category → SUBJ-* 分类 code 过滤）
    pub kind: Option<String>,
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
    /// 排除系统主体（默认 true——isahl 管理员等 socialCategory='主体' 不进业务列表）
    #[serde(default = "default_true")]
    pub exclude_system: bool,
    /// 排除停用主体（黑名单 disabled；默认 false——管理页显示全部，选择器显式传 true）
    #[serde(default)]
    pub exclude_disabled: bool,
}

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    20
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubjectListItem {
    #[serde(with = "common::serde_zuid")]
    id: i64,
    code: Option<String>,
    notice: Option<String>,
    social_category: String,
    category: Option<String>,
    view_tags: Vec<String>,
    status: String,
    /// 备注（含电话/联系人 JSON；详情页展示依赖，历史缺失致联系电话不可见）
    comments: Option<String>,
    /// MDM 主数据编码（wz_fssc.subject_mdm；**视角 → 码**映射，无记录为空对象）
    mdm_codes: std::collections::BTreeMap<String, String>,
    /// MDM 主数据编码兼容单值（按 CUST → SUPP → BIZ 取首个非空；全空为 null）
    mdm_code: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ViewTagItem {
    #[serde(with = "common::serde_zuid")]
    id: i64,
    code: String,
    notice: String,
}

/// 账户桥行（id / ref_right / 账户名 / 物权分类 id / 物权分类名）
type AccountBridgeRow = (i64, i64, Option<String>, Option<i64>, Option<String>);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountItem {
    #[serde(with = "common::serde_zuid")]
    rel_id: i64,
    #[serde(with = "common::serde_zuid")]
    account_id: i64,
    account_name: Option<String>,
    /// 物权分类（主体对该账户储元持有的物权类型；NULL = 未标注）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    real_rights: Option<i64>,
    real_rights_name: Option<String>,
}

/// POST /subjects 请求体（notice/name 必填，code 可选自动生成，subject_type 落 comments.kind）
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSubjectRequest {
    /// 主体名称（必填；前端契约历史用 name，serde alias 兼容两字段名）
    #[serde(alias = "name")]
    pub notice: String,
    /// 主体编码（前端可不传，空则自动生成 SUBJ-<zuid 后 6 位>）
    #[serde(default)]
    pub code: Option<String>,
    /// 主体类型（业务别名或叶表名，如 zc_id_empl-natural/自然人；落 comments.kind）
    /// rename_all=camelCase 主名接受 subjectType，alias 兼容前端契约 subject_type
    #[serde(default, alias = "subject_type")]
    pub subject_type: Option<String>,
    /// 备注（可选；与 kind 合并为 comments JSON）
    #[serde(default)]
    pub comments: Option<String>,
    /// 岗位 id（可选；有值挂到指定岗位，为空且需写视角标签时自动建默认岗位 POST-AUTO-<id> 挂接）
    #[serde(default)]
    pub position_id: Option<i64>,
    /// 交易视角 code 列表（可选；创建事务内直接落标签关系，与 PUT view-tags 同差量语义）
    #[serde(default)]
    pub view_tags: Option<Vec<String>>,
    /// 联系方式（可选；落「实体↔联系人↔联系方式」链，kind 白名单见 add_entity_contact）
    #[serde(default)]
    pub contacts: Option<Vec<CreateSubjectContact>>,
    /// 证照（可选；落独立表 zc_id_identity + zc_id_entity_rr_identity 关联）
    #[serde(default)]
    pub identities: Option<Vec<CreateSubjectIdentity>>,
    /// 就业状态 code（可选；EMPL-ACTIVE/EMPL-PROBATION/EMPL-RETIRED，字典 zc_id_stus-employ=状态-就业。
    /// 有值时同事务建 zc_id_subj-employee 雇员行 + r_employ-status 就业状态关系——主体单方属性；
    /// 雇佣（双方）走 subj-org_rr_employee 链，不在此路径）
    #[serde(default)]
    pub employ_status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSubjectContact {
    /// 号码（手机/固话）
    pub value: String,
    /// 类型（kind：mobile/phone/emergency 等，落 contacts.comments JSON）
    pub kind: Option<String>,
    /// 是否默认联系方式（entity_rr_contacts.default_contact）
    #[serde(default)]
    pub is_default: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSubjectIdentity {
    /// 证照类型 code（zc_id_cate-identity；缺项自动建字典）
    pub category_code: String,
    /// 证照号（identity 列）
    pub cert_no: String,
    /// 证照名称（dname；缺省用 cert_no）
    pub name: Option<String>,
}

/// PUT /subjects/{id} 请求体（None 字段保持不变）
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSubjectRequest {
    /// 主体名称
    pub notice: Option<String>,
    /// 主体编码
    pub code: Option<String>,
    /// 备注
    pub comments: Option<String>,
    /// MDM 主数据编码（**视角 → 码**映射，wz_fssc.subject_mdm；值为空串 = 删该视角行，
    /// 未出现的视角不动）。与单值 `mdmCode` 同传时以本字段为准（`mdmCode` 仅在其缺省时生效）。
    #[serde(default)]
    pub mdm_codes: Option<std::collections::BTreeMap<String, String>>,
    /// MDM 主数据编码兼容单值（Some(非空)=upsert，Some(空)=清除，None=不动）。
    /// 落码视角 = 该主体拥有的 CUST → SUPP 首个；判不出（两视角皆无）则 VIEW-BIZ。
    pub mdm_code: Option<String>,
}

/// `/subjects` 筛选参数（列表与计数**同一实现**的输入）
struct SubjectFilters<'a> {
    q: &'a str,
    view: &'a str,
    category: &'a str,
    kind: &'a str,
    exclude_system: bool,
    exclude_disabled: bool,
}

/// 计数查询 FROM（`QueryBuilder::new` 的静态起点）——单点常量：探针 `list_count_sql_probe`
/// 据此断言「计数与列表同一起点」，禁再出现第二份副本字面量。
const SUBJECT_COUNT_FROM: &str = "SELECT COUNT(*) FROM \"isahl\".\"zc_id_subjects\" s \
     LEFT JOIN \"isahl\".\"zc_id_cate-subject\" c ON c.id = s.ck_category AND c.deleted_at IS NULL \
     WHERE s.deleted_at IS NULL";

/// 主体列表筛选谓词——列表与计数**共用同一实现**（两份副本曾漂移：计数漏 `exclude_disabled`）。
///
/// 语义逐条在此单点实现：
/// - `q`：`notice` / `code` 模糊匹配；
/// - `category`：社会分类 = **基表 ∪ 其全部后代表**（`pg_catalog.pg_inherits` 递归，禁硬编码父子边）；
///   未收录分类 fail-closed 成空集（旧实现 `_ => zc_id_subjects` 把未知分类静默变成「不过滤」）；
/// - `exclude_system`：排除岗位（`zc_id_subj-position`）与无业务分类的根表行（`zc_id_subjects`）；
/// - `view`：主体经「岗位↔视角主体桥 → 视角标签」持有该关系视角；
/// - `kind`：主体分类 code 简写（customer / carrier / driver）。
fn push_subject_filters(builder: &mut QueryBuilder<Postgres>, f: &SubjectFilters<'_>) {
    // 停用/黑名单主体其他页面不可选——选择器传 exclude_disabled=1；
    // 码集 = `crate::subject_status::BLACKLIST_STATUS_CODES`（与相对方签约拒绝、blacklisted 投影同源）
    if f.exclude_disabled {
        builder.push(
            " AND NOT EXISTS (SELECT 1 FROM \"isahl\".\"zc_id_lifecycle_r_primary-status\" ps \
             JOIN \"isahl\".\"zc_id_stus-org\" ss ON ss.id = ps.ref_right AND ss.deleted_at IS NULL \
             WHERE ps.ref_left = s.id AND ps.deleted_at IS NULL AND ss.code = ANY(",
        );
        builder.push_bind(
            crate::subject_status::BLACKLIST_STATUS_CODES
                .map(|c| c.to_string())
                .to_vec(),
        );
        builder.push("))");
    }
    if !f.q.is_empty() {
        let pat = format!("%{}%", f.q);
        builder.push(" AND (s.notice ILIKE ");
        builder.push_bind(pat.clone());
        builder.push(" OR s.code ILIKE ");
        builder.push_bind(pat);
        builder.push(")");
    }
    if !f.category.is_empty() {
        match category_base_table(f.category) {
            Some(base) => {
                builder.push(concat!(
                    " AND s.tableoid IN (WITH RECURSIVE cat(oid) AS (",
                    " SELECT c.oid FROM pg_class c \
                       JOIN pg_namespace n ON n.oid = c.relnamespace \
                      WHERE n.nspname = 'isahl' AND c.relname = "
                ));
                builder.push_bind(base);
                builder.push(" UNION ALL SELECT ch.oid FROM cat JOIN pg_inherits i ON i.inhparent = cat.oid JOIN pg_class ch ON ch.oid = i.inhrelid JOIN pg_namespace n2 ON n2.oid = ch.relnamespace AND n2.nspname = 'isahl') SELECT oid FROM cat)");
            }
            // 未收录分类 = 语义上不存在该分类 → 空集（fail-closed；禁止静默退化为「不过滤」）
            None => {
                builder.push(" AND FALSE");
            }
        }
    }
    // 排除系统主体（批注 c1960312/95d25697：isahl 管理员等不进业务列表；无业务分类的根表行同排除）。
    // 判定基于 `pg_class.relname`，与连接 search_path 无关。
    if f.exclude_system {
        builder.push(concat!(
            " AND ",
            common::leaf_relname!(s),
            " NOT IN ('zc_id_subjects', 'zc_id_subj-position')"
        ));
    }
    if !f.view.is_empty() {
        builder.push(
            " AND EXISTS (SELECT 1 FROM \"isahl\".\"zc_id_subj-post_rr_view\" e \
             JOIN \"isahl\".\"zc_id_relation-post_view_r_tags\" r ON r.ref_left = e.id AND r.deleted_at IS NULL \
             JOIN \"isahl\".\"zc_id_tags-post_view\" vt ON vt.id = r.ref_right AND vt.deleted_at IS NULL \
             WHERE e.ref_right = s.id AND e.deleted_at IS NULL AND vt.code = ",
        );
        builder.push_bind(f.view.to_string());
        builder.push(")");
    }
    // kind 简写 → 主体分类 code（客户/承运商/司机；未知值原样比对，缺省返回全部）
    if !f.kind.is_empty() {
        let code = match f.kind {
            "customer" => "SUBJ-CUSTOMER",
            "carrier" => "SUBJ-CARRIER",
            "driver" => "SUBJ-DRIVER",
            other => other,
        };
        builder.push(" AND c.code = ");
        builder.push_bind(code.to_string());
    }
}

/// GET /service/isahl-db/subjects — 组织管理主体列表（视角/分类/搜索/分页；可选 kind 按主体分类过滤）
pub async fn list_subjects(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    query: web::Query<SubjectListQuery>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "list").await?;
    ensure_subject_mdm(pool.get_ref()).await?;

    let q = query.q.clone().unwrap_or_default().trim().to_string();
    let view = query.view.clone().unwrap_or_default().trim().to_string();
    let category = query
        .category
        .clone()
        .unwrap_or_default()
        .trim()
        .to_string();
    let kind = query.kind.clone().unwrap_or_default().trim().to_string();
    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;

    let mut builder: QueryBuilder<Postgres> = QueryBuilder::new(concat!(
        "SELECT s.id, s.code, s.notice, ",
        common::leaf_relname!(s),
        " AS leaf, s.comments, \
           COALESCE((SELECT ss.code FROM \"isahl\".\"zc_id_lifecycle_r_primary-status\" ps \
             JOIN \"isahl\".\"zc_id_stus-org\" ss ON ss.id = ps.ref_right AND ss.deleted_at IS NULL \
             WHERE ps.ref_left = s.id AND ps.deleted_at IS NULL LIMIT 1), 'normal') AS status, \
           c.code AS category_code, \
           COALESCE((SELECT json_agg(DISTINCT vt.code) \
             FROM \"isahl\".\"zc_id_subj-post_rr_view\" e \
             JOIN \"isahl\".\"zc_id_relation-post_view_r_tags\" r ON r.ref_left = e.id AND r.deleted_at IS NULL \
             JOIN \"isahl\".\"zc_id_tags-post_view\" vt ON vt.id = r.ref_right AND vt.deleted_at IS NULL \
             WHERE e.ref_right = s.id AND e.deleted_at IS NULL), '[]') AS view_tags, \
           (SELECT jsonb_object_agg(m.view_tag, m.mdm_code) \
              FROM wz_fssc.subject_mdm m WHERE m.subject_id = s.id) AS mdm_codes \
         FROM \"isahl\".\"zc_id_subjects\" s \
         LEFT JOIN \"isahl\".\"zc_id_cate-subject\" c ON c.id = s.ck_category AND c.deleted_at IS NULL \
         WHERE s.deleted_at IS NULL"
    ));
    push_subject_filters(
        &mut builder,
        &SubjectFilters {
            q: &q,
            view: &view,
            category: &category,
            kind: &kind,
            exclude_system: query.exclude_system,
            exclude_disabled: query.exclude_disabled,
        },
    );

    builder.push(" ORDER BY s.notice, s.id LIMIT ");
    builder.push_bind(page_size);
    builder.push(" OFFSET ");
    builder.push_bind(offset);

    #[allow(clippy::type_complexity)] // sqlx 行类型
    let rows: Vec<(
        i64,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        String,
        Option<String>,
        serde_json::Value,
        Option<serde_json::Value>,
    )> = builder
        .build_query_as()
        .fetch_all(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?;

    // 总数（分页页码列表——批注 c65c4382：只显示当前页码）
    // 与列表**同一谓词实现**（`push_subject_filters`）：此前两份副本漂移（计数漏 exclude_disabled）
    let total: i64 = {
        let mut cb: QueryBuilder<Postgres> = QueryBuilder::new(SUBJECT_COUNT_FROM);
        push_subject_filters(
            &mut cb,
            &SubjectFilters {
                q: &q,
                view: &view,
                category: &category,
                kind: &kind,
                exclude_system: query.exclude_system,
                exclude_disabled: query.exclude_disabled,
            },
        );
        cb.build_query_scalar::<i64>()
            .fetch_one(pool.get_ref())
            .await
            .map_err(ApiError::from_sqlx)?
    };

    let items: Vec<SubjectListItem> = rows
        .into_iter()
        .map(
            |(
                id,
                code,
                notice,
                leaf,
                comments,
                status,
                category_code,
                view_tags,
                mdm_codes_raw,
            )| {
                // 父表插入（tableoid=zc_id_subjects）的行：优先读 comments.kind 回退类型标签
                // （create 时 subject_type 落 comments.kind，叶表名或中文别名）
                let base = social_category(&leaf);
                let social = if base == "主体" {
                    comments
                        .as_deref()
                        .and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok())
                        .and_then(|v| {
                            v.get("kind").and_then(|k| k.as_str()).map(|k| {
                                if k.starts_with("zc_id_") {
                                    social_category(k).to_string()
                                } else {
                                    k.to_string()
                                }
                            })
                        })
                        .unwrap_or_else(|| base.to_string())
                } else {
                    base.to_string()
                };
                SubjectListItem {
                    id,
                    code,
                    notice,
                    social_category: social,
                    category: category_code,
                    view_tags: view_tags
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default(),
                    status,
                    comments,
                    mdm_code: compat_mdm_code(&mdm_code_map(&mdm_codes_raw)),
                    mdm_codes: mdm_code_map(&mdm_codes_raw),
                }
            },
        )
        .collect();

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "items": items,
            "total": total,
        }))),
    )
}

/// GET /service/isahl-db/subjects/{id} — 单条主体详情（详情直达；与 list_subjects 字段口径一致，
/// 字段演进需同步 list_subjects 的 SELECT 与 map）
pub async fn get_subject(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "list").await?;
    ensure_subject_mdm(pool.get_ref()).await?;
    let id = path.into_inner();
    #[allow(clippy::type_complexity)] // sqlx 行类型
    let row: Option<(
        i64,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        String,
        Option<String>,
        serde_json::Value,
        Option<serde_json::Value>,
    )> = sqlx::query_as(concat!(
        r#"SELECT s.id, s.code, s.notice, "#,
        common::leaf_relname!(s),
        r#" AS leaf, s.comments,
                  COALESCE((SELECT ss.code FROM "isahl"."zc_id_lifecycle_r_primary-status" ps
                    JOIN "isahl"."zc_id_stus-org" ss ON ss.id = ps.ref_right AND ss.deleted_at IS NULL
                    WHERE ps.ref_left = s.id AND ps.deleted_at IS NULL LIMIT 1), 'normal') AS status,
                  c.code AS category_code,
                  COALESCE((SELECT json_agg(DISTINCT vt.code)
                    FROM "isahl"."zc_id_subj-post_rr_view" e
                    JOIN "isahl"."zc_id_relation-post_view_r_tags" r ON r.ref_left = e.id AND r.deleted_at IS NULL
                    JOIN "isahl"."zc_id_tags-post_view" vt ON vt.id = r.ref_right AND vt.deleted_at IS NULL
                    WHERE e.ref_right = s.id AND e.deleted_at IS NULL), '[]') AS view_tags,
                  (SELECT jsonb_object_agg(m.view_tag, m.mdm_code)
                     FROM wz_fssc.subject_mdm m WHERE m.subject_id = s.id) AS mdm_codes
           FROM "isahl"."zc_id_subjects" s
           LEFT JOIN "isahl"."zc_id_cate-subject" c ON c.id = s.ck_category AND c.deleted_at IS NULL
           WHERE s.deleted_at IS NULL AND s.id = $1
           LIMIT 1"#
    ))
    .bind(id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    let Some((id, code, notice, leaf, comments, status, category_code, view_tags, mdm_codes_raw)) =
        row
    else {
        return Err(ApiError::NotFound(format!("主体不存在: {}", id)));
    };
    // 与 list_subjects 同口径：父表行优先读 comments.kind 回退类型标签
    let base = social_category(&leaf);
    let social = if base == "主体" {
        comments
            .as_deref()
            .and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok())
            .and_then(|v| {
                v.get("kind").and_then(|k| k.as_str()).map(|k| {
                    if k.starts_with("zc_id_") {
                        social_category(k).to_string()
                    } else {
                        k.to_string()
                    }
                })
            })
            .unwrap_or_else(|| base.to_string())
    } else {
        base.to_string()
    };
    let item = SubjectListItem {
        id,
        code,
        notice,
        social_category: social,
        category: category_code,
        view_tags: view_tags
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        status,
        comments,
        mdm_code: compat_mdm_code(&mdm_code_map(&mdm_codes_raw)),
        mdm_codes: mdm_code_map(&mdm_codes_raw),
    };
    Ok(HttpResponse::Ok().json(ApiResponse::success(item)))
}

/// GET /service/isahl-db/subjects/{id}/view-tags — 读交易视角标签
pub async fn get_subject_view_tags(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "read").await?;

    ensure_subject_exists(pool.get_ref(), subject_id).await?;

    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT DISTINCT vt.id, vt.code, vt.notice, vt.o_number \
         FROM \"isahl\".\"zc_id_subj-post_rr_view\" e \
         JOIN \"isahl\".\"zc_id_relation-post_view_r_tags\" r ON r.ref_left = e.id AND r.deleted_at IS NULL \
         JOIN \"isahl\".\"zc_id_tags-post_view\" vt ON vt.id = r.ref_right AND vt.deleted_at IS NULL \
         WHERE e.ref_right = $1 AND e.deleted_at IS NULL \
         ORDER BY vt.o_number, vt.id",
    )
    .bind(subject_id)
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    let items: Vec<ViewTagItem> = rows
        .into_iter()
        .map(|(id, code, notice)| ViewTagItem { id, code, notice })
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

#[derive(Debug, Deserialize)]
pub struct PutViewTagsRequest {
    /// 目标视角 code 列表（如 ["VIEW-CUST", "VIEW-SUPP"]）
    pub view_tags: Vec<String>,
}

/// PUT /service/isahl-db/subjects/{id}/view-tags — 写交易视角标签（差额同步，幂等）
pub async fn put_subject_view_tags(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<PutViewTagsRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "update").await?;

    ensure_subject_exists(pool.get_ref(), subject_id).await?;

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    // 主体名下视角关联行（subj-post_rr_view: ref_left=岗位, ref_right=被视角主体）
    // 标签宿主 = 关联行；同步范围 = 该主体名下**全部存活关联行**（与读路径同范围）。
    // 无关联行且目标标签为空 → 无操作直接成功（基本信息保存不应被门禁阻断）；
    // 无关联行但要写标签 → 自动建默认岗位挂接（缺则自建，不再 400）
    let mut pair_ids = subject_view_pair_ids(&mut tx, subject_id).await?;
    if pair_ids.is_empty() {
        if body.view_tags.is_empty() {
            tx.commit().await.map_err(ApiError::from_sqlx)?;
            return Ok(
                HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                    "id": subject_id.to_string(),
                    "viewTags": body.view_tags,
                }))),
            );
        }
        pair_ids = ensure_subject_view_pairs(&mut tx, subject_id, None, user_id).await?;
    }

    // 逐关联行差量同步视角标签（幂等；未知字典 code → 400）
    for pair_id in &pair_ids {
        sync_view_tags(&mut tx, *pair_id, &body.view_tags, user_id).await?;
    }

    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "id": subject_id.to_string(),
            "viewTags": body.view_tags,
        }))),
    )
}

/// GET /service/isahl-db/subjects/{id}/accounts — 读关联账户
pub async fn list_subject_accounts(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "read").await?;

    ensure_subject_exists(pool.get_ref(), subject_id).await?;

    let rows: Vec<AccountBridgeRow> = sqlx::query_as(
        "SELECT r.id, r.ref_right, a.notice, r.ck_real_rights, rr.notice \
         FROM \"isahl\".\"zc_id_subjects_rr_account\" r \
         LEFT JOIN \"isahl\".\"zc_id_stor-account\" a ON a.id = r.ref_right AND a.deleted_at IS NULL \
         LEFT JOIN \"isahl\".\"zc_id_cate-real_rights\" rr ON rr.id = r.ck_real_rights AND rr.deleted_at IS NULL \
         WHERE r.ref_left = $1 AND r.deleted_at IS NULL \
         ORDER BY r.id",
    )
    .bind(subject_id)
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    let items: Vec<AccountItem> = rows
        .into_iter()
        .map(
            |(rel_id, account_id, account_name, real_rights, real_rights_name)| AccountItem {
                rel_id,
                account_id,
                account_name,
                real_rights,
                real_rights_name,
            },
        )
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

#[derive(Debug, Deserialize)]
pub struct AddAccountRequest {
    /// 账户储元 id（ref_right，zc_id_stor-account 继承链行，如 zc_id_stor-acc-bank / zc_id_stor-acc-cash 叶表行）
    #[serde(with = "common::serde_zuid")]
    pub account_id: i64,
    /// 物权分类 code（`zc_id_cate-real_rights`，如 OWNERSHIP 所有权 / USUFRUCT 用益物权）；
    /// 未传 = 不标注（模型侧非必填）
    #[serde(default)]
    pub real_rights: Option<String>,
}

/// POST /service/isahl-db/subjects/{id}/accounts — 添加账户关联（幂等）
pub async fn add_subject_account(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<AddAccountRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "update").await?;

    ensure_subject_exists(pool.get_ref(), subject_id).await?;
    // 账户储元存在性（zc_id_stor-account 继承链统一可见 acc-* 叶表；主体/储元为 lifecycle 兄弟族互不可见）
    let account_exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM \"isahl\".\"zc_id_stor-account\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(body.account_id)
    .fetch_one(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    if !account_exists {
        return Err(ApiError::NotFound(format!(
            "账户实体不存在: {}",
            body.account_id
        )));
    }

    // 物权分类（可选）：code → 字典 id，fail-fast（未知 code 在此 400，不落到写入后才报错）
    let real_rights_id =
        resolve_real_rights_id_opt(pool.get_ref(), body.real_rights.as_deref()).await?;

    // 已存在 → 返回现有关联（幂等）
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM \"isahl\".\"zc_id_subjects_rr_account\" \
         WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(subject_id)
    .bind(body.account_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    if let Some(rel_id) = existing {
        return Ok(
            HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                "id": rel_id.to_string(),
            }))),
        );
    }

    let rel_id: i64 = sqlx::query_scalar(
        "INSERT INTO \"isahl\".\"zc_id_subjects_rr_account\" \
         (notice, ref_left, ref_right, ck_real_rights, created_by_id, updated_by_id) \
         VALUES ($1, $2, $3, $4, $5, $5) RETURNING id",
    )
    .bind(format!("subject-{} account", subject_id))
    .bind(subject_id)
    .bind(body.account_id)
    .bind(real_rights_id)
    .bind(user_id)
    .fetch_one(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": rel_id.to_string(),
        }))),
    )
}

/// DELETE /service/isahl-db/subjects/{id}/accounts/{relId} — 删除账户关联（软删除）
pub async fn delete_subject_account(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (subject_id, rel_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "update").await?;

    let rows = sqlx::query(
        "UPDATE \"isahl\".\"zc_id_subjects_rr_account\" SET deleted_at = NOW(), deleted_by_id = $3 \
         WHERE id = $1 AND ref_left = $2 AND deleted_at IS NULL",
    )
    .bind(rel_id)
    .bind(subject_id)
    .bind(user_id)
    .execute(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    if rows.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!(
            "账户关联不存在: rel={} subject={}",
            rel_id, subject_id
        )));
    }

    Ok(HttpResponse::NoContent().finish())
}

/// 主体名下存活视角关联行 id（升序）；无则空。
///
/// 视角标签宿主 = 关联行（`zc_id_relation-post_view_r_tags.ref_left`）；
/// 读路径（列表 viewTags / view 筛选 / 详情 / view-tags）与写路径 MUST 同范围。
async fn subject_view_pair_ids(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    subject_id: i64,
) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar(
        "SELECT id FROM \"isahl\".\"zc_id_subj-post_rr_view\" \
         WHERE ref_right = $1 AND deleted_at IS NULL ORDER BY id",
    )
    .bind(subject_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)
}

/// 确保主体已挂视角关联行：返回该主体名下**全部存活关联行 id**（升序）。
/// - 指定岗位：校验存在后幂等挂接 `zc_id_subj-post_rr_view`（ref_left=岗位, ref_right=主体）
/// - 未指定：自动建默认岗位 `POST-AUTO-<subject_id>` 并挂接（与存量回填种子 seed-wz-defaults.sql 同模式）
pub async fn ensure_subject_view_pairs(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    subject_id: i64,
    position_id: Option<i64>,
    user_id: i64,
) -> Result<Vec<i64>, ApiError> {
    if let Some(post_id) = position_id {
        let exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM \"isahl\".\"zc_id_subj-position\" WHERE id = $1 AND deleted_at IS NULL AND _f_ IS NULL",
    )
    .bind(post_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;
        if !exists {
            return Err(ApiError::BadRequest(format!("岗位不存在: {}", post_id)));
        }
        sqlx::query(
            "INSERT INTO \"isahl\".\"zc_id_subj-post_rr_view\" \
         (notice, ref_left, ref_right, created_by_id) \
         SELECT 'auto-link', $1, $2, $3 \
         WHERE NOT EXISTS (SELECT 1 FROM \"isahl\".\"zc_id_subj-post_rr_view\" e \
                           WHERE e.ref_left = $1 AND e.ref_right = $2 AND e.deleted_at IS NULL)",
        )
        .bind(post_id)
        .bind(subject_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        return subject_view_pair_ids(tx, subject_id).await;
    }

    // 无指定岗位：自动建默认岗位 POST-AUTO-<subject_id>（幂等，与种子回填一致）
    // id 显式 gen_next_zuid（与 wz 库 zc_id_subj-position 表默认同生成器；测试库无表默认亦可用）
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve_conn(tx, ("TX", "FJA", "↓_GG"))
            .await
            .map_err(ApiError::from)?;
    let inserted: Option<i64> = sqlx::query_scalar(
    "INSERT INTO \"isahl\".\"zc_id_subj-position\" (id, notice, code, created_by_id, dk_scene, dk_factor, dk_function) \
     SELECT isahl.gen_next_zuid(), '默认岗位', 'POST-AUTO-' || $1::text, $2, $3, $4, $5 \
     WHERE NOT EXISTS (SELECT 1 FROM \"isahl\".\"zc_id_subj-position\" p \
                       WHERE p.code = 'POST-AUTO-' || $1::text AND p.deleted_at IS NULL) \
     RETURNING id",
)
.bind(subject_id)
.bind(user_id)
.bind(dk_scene)
.bind(dk_factor)
.bind(dk_function)
.fetch_optional(&mut **tx)
.await
.map_err(ApiError::from_sqlx)?;
    let post_id = match inserted {
        Some(post_id) => post_id,
        None => sqlx::query_scalar(
            "SELECT id FROM \"isahl\".\"zc_id_subj-position\" \
         WHERE code = 'POST-AUTO-' || $1::text AND deleted_at IS NULL",
        )
        .bind(subject_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(ApiError::from_sqlx)?,
    };
    sqlx::query(
        "INSERT INTO \"isahl\".\"zc_id_subj-post_rr_view\" \
     (notice, ref_left, ref_right, created_by_id) \
     SELECT 'auto-link', $1, $2, $3 \
     WHERE NOT EXISTS (SELECT 1 FROM \"isahl\".\"zc_id_subj-post_rr_view\" e \
                       WHERE e.ref_left = $1 AND e.ref_right = $2 AND e.deleted_at IS NULL)",
    )
    .bind(post_id)
    .bind(subject_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    subject_view_pair_ids(tx, subject_id).await
}

/// 差量同步视角标签（幂等）：关联行不在目标集合的标签关系软删，目标视角 upsert。
/// 视角字典未知 code → 400。
pub async fn sync_view_tags(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    pair_id: i64,
    target_codes: &[String],
    user_id: i64,
) -> Result<(), ApiError> {
    // 目标视角字典 id（未知 code → 400）
    let mut target_ids: Vec<i64> = Vec::new();
    for code in target_codes {
        let dict_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM \"isahl\".\"zc_id_tags-post_view\" WHERE code = $1 AND deleted_at IS NULL",
    )
    .bind(code)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;
        let dict_id = dict_id.ok_or_else(|| {
            ApiError::BadRequest(format!(
                "未知交易视角 code: '{}'（字典 zc_id_tags-post_view）",
                code
            ))
        })?;
        target_ids.push(dict_id);
    }

    // 删除该关联行不在目标集合中的视角标签关联
    sqlx::query(
    "UPDATE \"isahl\".\"zc_id_relation-post_view_r_tags\" SET deleted_at = NOW(), deleted_by_id = $2 \
     WHERE ref_left = $1 AND deleted_at IS NULL \
     AND ref_right != ALL($3)",
)
.bind(pair_id)
.bind(user_id)
.bind(&target_ids)
.execute(&mut **tx)
.await
.map_err(ApiError::from_sqlx)?;

    // 幂等 upsert 目标关联（已存在则跳过；表无 UNIQUE 约束，用 WHERE NOT EXISTS）
    for dict_id in &target_ids {
        sqlx::query(
            "INSERT INTO \"isahl\".\"zc_id_relation-post_view_r_tags\" \
         (notice, ref_left, ref_right, created_by_id, updated_by_id) \
         SELECT $1, $2, $3, $4, $4 \
         WHERE NOT EXISTS (SELECT 1 FROM \"isahl\".\"zc_id_relation-post_view_r_tags\" r \
                           WHERE r.ref_left = $2 AND r.ref_right = $3 AND r.deleted_at IS NULL)",
        )
        .bind(format!("pair-{} view-tag", pair_id))
        .bind(pair_id)
        .bind(dict_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::from_sqlx)?;
    }
    Ok(())
}

/// 主体证照写入单元（`POST /subjects` identities 分支与集成测试共用的唯一实现）。
///
/// 幂等契约（change: fix-subject-identity-write-idempotency）：
/// - `code` = `identity` = 证件号（口径同 `Pre-Proc/WZ/seed/*.sql` 种子判重键）：同一证件号
///   在同库内至多一行活动证照——已存在则**复用其 id**（不新建）；复用边界 = 同一统一社会
///   信用代码属同一法人（同一主体同一证照），非跨主体共享证照。
/// - 桥行 `zc_id_entity_rr_identity` 按 `(ref_left=主体, ref_right=证照)` 活动对唯一
///   （NOT EXISTS 守卫）：重复建档 / 种子重放不叠桥。
#[allow(clippy::too_many_arguments)] // 证照建档字段集（实体+证件+分类+操作者）——参数即领域，无聚合对象可复用
pub async fn write_subject_identity(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    subject_id: i64,
    subject_notice: &str,
    index: usize,
    cert_no: &str,
    dname: &str,
    category_id: i64,
    user_id: i64,
) -> Result<i64, ApiError> {
    let existing: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM "isahl"."zc_id_identity" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(cert_no)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    let identity_id: i64 = match existing {
        // 复用既有活动行（同 code = 同证件号 = 同一法人同一证照）
        Some(id) => id,
        None => {
            // 坐标三元组（§6.12 声明即必须）：Identity = JE/FJA/↑_DA（identity-org
            // repository/ontology_binding.rs coords_for_entity("Identity") 静态绑定），
            // 值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
            let (dk_scene, dk_factor, dk_function) =
                ontology_binding::resolve_conn(tx, ("JE", "FJA", "↑_DA")).await?;
            sqlx::query_scalar(
                r#"INSERT INTO "isahl"."zc_id_identity"
                   (code, notice, identity, dname, ck_category, created_by_id, updated_by_id, dk_scene, dk_factor, dk_function)
                   VALUES ($1, $2, $3, $4, $5, $6, $6, $7, $8, $9) RETURNING id"#,
            )
            .bind(cert_no)
            .bind(format!("{} 证照 {}", subject_notice, index + 1))
            .bind(cert_no)
            .bind(dname)
            .bind(category_id)
            .bind(user_id)
            .bind(dk_scene)
            .bind(dk_factor)
            .bind(dk_function)
            .fetch_one(&mut **tx)
            .await
            .map_err(ApiError::from_sqlx)?
        }
    };

    // 主体↔证照桥行：按 (ref_left, ref_right) 活动对唯一——重复调用不叠桥
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_entity_rr_identity"
           (notice, ref_left, ref_right, created_by_id, updated_by_id)
           SELECT $1, $2, $3, $4, $4
           WHERE NOT EXISTS (
               SELECT 1 FROM "isahl"."zc_id_entity_rr_identity" rr
               WHERE rr.ref_left = $2 AND rr.ref_right = $3 AND rr.deleted_at IS NULL)"#,
    )
    .bind(format!("subject-{} identity", subject_id))
    .bind(subject_id)
    .bind(identity_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    Ok(identity_id)
}

/// POST /service/isahl-db/subjects — 创建主体（notice/code 必填）
pub async fn create_subject(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<CreateSubjectRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "create").await?;

    let notice = body.notice.trim().to_string();
    if notice.is_empty() {
        return Err(ApiError::BadRequest("notice 不能为空".into()));
    }
    // code 可选：空则自动生成 SUBJ-<zuid 后 6 位>（唯一性由 zuid 保证）
    let code = match body.code.as_deref().map(str::trim) {
        Some(c) if !c.is_empty() => {
            // 显式主体编码（企业=统一社会信用代码，国标唯一）：同码活动主体存在则拒绝重复建档（批注 46814bfa）
            let dup: Option<i64> = sqlx::query_scalar(
                    r#"SELECT id FROM "isahl"."zc_id_subjects" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
                )
                .bind(c)
                .fetch_optional(pool.get_ref())
                .await
                .map_err(ApiError::from_sqlx)?;
            if dup.is_some() {
                return Err(ApiError::BadRequest(format!("主体编码已存在: {}", c)));
            }
            c.to_string()
        }
        _ => {
            let z: i64 = sqlx::query_scalar("SELECT isahl.gen_next_zuid()")
                .fetch_one(pool.get_ref())
                .await
                .map_err(ApiError::from_sqlx)?;
            format!("SUBJ-{:06}", z % 1_000_000)
        }
    };
    // comments 为纯文本语义（remove-comments-json-embedding）：仅存请求备注原文
    // （subject_type 不再并入——kind 寄生通道移除）
    let comments: Option<String> = body
        .comments
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string);

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    // 批注（用户指示）：主体数据一律插入对应叶表（不回退父表）——
    // subject_leaf_table fail-fast（2026-08-29 裁决）：未知/中间层类型 400——
    // 旧行为静默回退 subj-group 造成错分类，已废除；
    // 只插父表则业务侧查子表（natural-persons/司机下拉等）看不到，事后补子表会双行
    let sql: &'static str =
        crate::models::subject_leaf_insert_sql(body.subject_type.as_deref().unwrap_or(""))
            .ok_or_else(|| {
                ApiError::BadRequest(format!(
                    "未知或缺失主体类型: {:?}（法人须明确叶表：非银行法人/商业银行）",
                    body.subject_type
                ))
            })?;
    // 表名与 SQL 均为白名单字面量（编译期常量，无 format! 插值 → 无需 AssertSqlSafe）

    let id: i64 = sqlx::query_scalar(sql)
        .bind(&notice)
        .bind(&code)
        .bind(&comments)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;

    // 挂岗 + 视角标签（事务内一次落库，避免创建后 PUT view-tags 无岗位 400）
    // view_tags 为空但指定了岗位 → 仍挂岗（基本信息保存完整落库）；
    // 两者皆空 → 不强制建岗（列表 viewTags 为空即可，后续 PUT 会自建）
    let view_tags = body.view_tags.clone().unwrap_or_default();
    if !view_tags.is_empty() {
        let pair_ids = ensure_subject_view_pairs(&mut tx, id, body.position_id, user_id).await?;
        for pair_id in &pair_ids {
            sync_view_tags(&mut tx, *pair_id, &view_tags, user_id).await?;
        }
    } else if body.position_id.is_some() {
        ensure_subject_view_pairs(&mut tx, id, body.position_id, user_id).await?;
    }

    // 联系方式落「实体↔联系人↔联系方式」本体链（元数据实名，remap-subject-bank-invoice-isahl）：
    // kind → typed 叶表（缺省 telephone），值不落 o_number、kind 不落 comments
    if let Some(contacts) = &body.contacts {
        for (ci, c) in contacts.iter().enumerate() {
            let value = c.value.trim().to_string();
            if value.is_empty() {
                continue;
            }
            add_entity_contact(
                &mut tx,
                id,
                &notice,
                None,
                c.kind.as_deref(),
                &value,
                c.is_default.unwrap_or(ci == 0),
                user_id,
            )
            .await?;
        }
    }
    if let Some(identities) = &body.identities {
        for (ii, ident) in identities.iter().enumerate() {
            let cert_no = ident.cert_no.trim().to_string();
            if cert_no.is_empty() {
                continue;
            }
            let category_id = category_id_by_code(pool.get_ref(), &ident.category_code).await?;
            let dname = ident.name.clone().unwrap_or_else(|| cert_no.clone());
            write_subject_identity(
                &mut tx,
                id,
                &notice,
                ii,
                &cert_no,
                &dname,
                category_id,
                user_id,
            )
            .await?;
        }
    }

    // 就业状态落雇员链（主体-雇员 + 关系-雇员→就业状态；主体单方属性，change: align-org-position-employment-chains）
    if let Some(employ_status) = body
        .employ_status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let status_id: i64 = sqlx::query_scalar(
            r#"SELECT id FROM "isahl"."zc_id_stus-employ" WHERE code = $1 AND deleted_at IS NULL"#,
        )
        .bind(employ_status)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "未知就业状态 code: '{}'（字典 zc_id_stus-employ）",
                employ_status
            ))
        })?;
        // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve_conn(&mut tx, ("ZJ", "LNC", "↓_EH"))
                .await
                .map_err(ApiError::from)?;
        let employee_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_empl-natural"
               (id, notice, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5) RETURNING id"#,
        )
        .bind(format!("{} 雇员", notice))
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_subj-employee_r_employ-status"
               (ref_left, ref_right, status_date, created_by_id)
               VALUES ($1, $2, now(), $3)"#,
        )
        .bind(employee_id)
        .bind(status_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
    }

    // 缺省储元同步建立（本体裁决 2026-09-07）：主体建档即具备资金/财产承载位——
    // 现金账户（账户-现金）+ 财产储位（场所-资产），经 typed 子桥挂主体；
    // 总关系父表 rr_storage 不直写（PG 继承查子不见父）
    // 坐标三元组（§6.12 声明即必须）：SettlementCash=TX/FJA/↓_EV（identity-org
    // repository/ontology_binding.rs coords_for_entity 静态绑定），值经 ontology_binding 解析
    let (cash_dk_scene, cash_dk_factor, cash_dk_function) =
        ontology_binding::resolve(pool.get_ref(), ("TX", "FJA", "↓_EV")).await?;
    // 物权分类（`zc_id_cate-real_rights`）：缺省储元是主体**自带承载位** ⇒ 所有权；
    // code → 字典 id（ZUID 跨库不通用，MUST NOT 硬编码）
    let ownership_right = resolve_real_rights_id(pool.get_ref(), "OWNERSHIP").await?;
    let cash_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_stor-acc-cash" (notice, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
    )
    .bind(format!("{} 现金账户", notice))
    .bind(user_id)
    .bind(cash_dk_scene)
    .bind(cash_dk_factor)
    .bind(cash_dk_function)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_subjects_rr_account"
           (notice, ref_left, ref_right, ck_real_rights, created_by_id, updated_by_id)
           VALUES ($1, $2, $3, $4, $5, $5)"#,
    )
    .bind(format!("subject-{} 账户", id))
    .bind(id)
    .bind(cash_id)
    .bind(ownership_right)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    let place_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_stor-plc-asset" (notice, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
    )
    .bind(format!("{} 财产储位", notice))
    .bind(user_id)
    .bind(cash_dk_scene)
    .bind(cash_dk_factor)
    .bind(cash_dk_function)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_subjects_rr_place"
           (notice, ref_left, ref_right, ck_real_rights, created_by_id, updated_by_id)
           VALUES ($1, $2, $3, $4, $5, $5)"#,
    )
    .bind(format!("subject-{} 储位", id))
    .bind(id)
    .bind(place_id)
    .bind(ownership_right)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    // 批注（修复：此前 edit 误删 commit——事务 drop 自动回滚，INSERT 全丢但返回 201）
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": id.to_string(),
            "code": code,
        }))),
    )
}

/// PUT /service/isahl-db/subjects/{id} — 更新主体（None 字段保持不变）
pub async fn update_subject(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<UpdateSubjectRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", id, "update").await?;
    if let Some(notice) = &body.notice {
        if notice.trim().is_empty() {
            return Err(ApiError::BadRequest("notice 不能为空".into()));
        }
    }
    if let Some(code) = &body.code {
        let trimmed = code.trim();
        if trimmed.is_empty() {
            return Err(ApiError::BadRequest("code 不能为空".into()));
        }
        // code 变更同码查重（排除自身：同值无操作保存不误报；批注 46814bfa）
        let dup: Option<i64> = sqlx::query_scalar(
            r#"SELECT id FROM "isahl"."zc_id_subjects"
                   WHERE code = $1 AND deleted_at IS NULL AND id <> $2 LIMIT 1"#,
        )
        .bind(trimmed)
        .bind(id)
        .fetch_optional(pool.get_ref())
        .await
        .map_err(ApiError::from_sqlx)?;
        if dup.is_some() {
            return Err(ApiError::BadRequest(format!("主体编码已存在: {}", trimmed)));
        }
    }

    let updated: Option<i64> = sqlx::query_scalar(
        r#"UPDATE "isahl"."zc_id_subjects"
           SET notice = COALESCE($2, notice),
               code = COALESCE($3, code),
               comments = COALESCE($4, comments)
           WHERE id = $1 AND deleted_at IS NULL
           RETURNING id"#,
    )
    .bind(id)
    .bind(&body.notice)
    .bind(&body.code)
    .bind(&body.comments)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    match updated {
        Some(id) => {
            // MDM 主数据编码（wz_fssc.subject_mdm）——一码一视角（主键 subject_id × view_tag）：
            // ① `mdmCodes` 映射（准源）：值为空串 = 删该视角行，非空 = upsert，未出现的视角不动
            // ② `mdmCode` 单值（兼容旧前端）：**仅当 `mdmCodes` 缺省时生效**，避免与新映射相竞
            //    （否则被删视角会被「CUST→SUPP 首个非空」回填复活）
            match (&body.mdm_codes, &body.mdm_code) {
                (Some(codes), _) => {
                    write_subject_mdm_codes(pool.get_ref(), id, codes, user_id).await?;
                }
                (None, Some(mdm_code)) => {
                    ensure_subject_mdm(pool.get_ref()).await?;
                    let tag = resolve_mdm_view_tag(pool.get_ref(), id).await?;
                    write_subject_mdm(pool.get_ref(), id, &tag, mdm_code, user_id).await?;
                }
                (None, None) => {}
            }
            Ok(
                HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                    "id": id.to_string(),
                }))),
            )
        }
        None => Err(ApiError::NotFound(format!("主体不存在: {}", id))),
    }
}

/// 主体删除级联计数（`DELETE /subjects/{id}` 响应 `cascaded` 明细；值 = 本次软删行数）
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectDeleteCascade {
    /// `zc_id_entity_rr_identity`（主体↔证照桥）
    pub identity_bridges: u64,
    /// `zc_id_entity_rr_contacts`（主体↔联系人桥）
    pub contact_bridges: u64,
    /// 私有联系人链（值叶表 + `zc_id_contacts_rr_infos` + `zc_id_contacts`）
    pub contact_chain: u64,
    /// `zc_id_subjects_rr_account`（主体↔账户储元桥）
    pub account_bridges: u64,
    /// `zc_id_subjects_rr_place`（主体↔场所储元桥）
    pub place_bridges: u64,
    /// `zc_id_subj-post_rr_view`（岗位↔主体 视角关联行）
    pub view_pairs: u64,
    /// `zc_id_relation-post_view_r_tags`（标签宿主 = 视角关联行 id）
    pub view_tags: u64,
    /// `zc_id_subj-position` 自动默认岗位（`code = POST-AUTO-<主体id>`）
    pub auto_positions: u64,
    /// 任职/成员桥（`zc_id_subj-post_rr_employee` + `-org_rr_employee` + `-group_rr_member`）
    pub memberships: u64,
    /// `zc_id_relation-cooperation_r_evaluation`（挂合作关系桥：先删桥下评估行，再删桥行）
    pub evaluations: u64,
    /// `zc_id_subjects_rr_partner`（合作关系桥行，主体任一端）
    pub partner_bridges: u64,
}

/// 关系行软删 SQL（表名/列名 = 编译期字面量 ⇒ 静态分发，无 `format!` 插值）
macro_rules! soft_delete_relations_sql {
    ($table:literal, $column:literal) => {
        concat!(
            "UPDATE \"isahl\".\"",
            $table,
            "\" SET deleted_at = NOW(), deleted_by_id = $2 WHERE ",
            $column,
            " = $1 AND deleted_at IS NULL"
        )
    };
}

/// 按主体列软删一行表；口径与同文件既有软删一致
/// （`deleted_at = NOW()` + `deleted_by_id`）。SQL 为编译期常量（无注入/无插值面）。
async fn soft_delete_subject_relations(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    sql: &'static str,
    subject_id: i64,
    user_id: i64,
) -> Result<u64, ApiError> {
    Ok(sqlx::query(sql)
        .bind(subject_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::from_sqlx)?
        .rows_affected())
}

/// 主体软删 + 关系行级联（`DELETE /subjects/{id}` 的唯一实现：handler 与集成测试共用）。
///
/// change: cascade-subject-delete-relations（逐表枚举 + 方向见该 change 的 `design.md` §1/§2）：
/// - **单事务**：主体行与全部级联行同一提交边界，任一步失败整体回滚（无半删态）；
/// - **404 语义不变**：主体行 UPDATE 带 `deleted_at IS NULL` 守卫且**先执行**（顺带取得行锁，
///   并发双删串行化），0 行 ⇒ `NotFound`——此刻事务内零写入，drop 回滚；
/// - **级联白名单**：只软删「该主体自身的关系行」——证照桥 / 联系人链 / 账户与储位归属桥 /
///   视角关联行及其标签 / 任职-成员桥 / 合作评价行；MUST NOT 触碰业务单据面（合同、账单、委托、
///   运单、审批、审计等引用主体的行 = 业务历史，读路径按主体 `deleted_at` 过滤即可）与可共享
///   本体行（`zc_id_identity` 证照本体按 `code` 复用、`zc_id_stor-acc-cash`/`zc_id_stor-plc-asset`
///   储元本体可被其他归属桥引用——只删桥，同 `delete_subject_account` 先例）；
/// - **共享守卫**：联系人链仅在无其他活动引用（其他实体桥 / 任职桥 `ref_right`）时软删；
///   `POST-AUTO-<主体id>` 自动岗位仅在无其他主体活动关联行时软删（显式/共享岗位不碰）。
#[allow(clippy::field_reassign_with_default)] // 级联计数器逐段累积（含条件分支）——结构体初始化无法表达
pub async fn delete_subject_cascade(
    pool: &PgPool,
    subject_id: i64,
    user_id: i64,
) -> Result<SubjectDeleteCascade, ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    let deleted = sqlx::query(
        r#"UPDATE "isahl"."zc_id_subjects"
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?
    .rows_affected();
    if deleted == 0 {
        return Err(ApiError::NotFound(format!("主体不存在: {}", subject_id)));
    }

    // ① 视角关联行 id（标签行宿主）——须在关联行软删前取
    let pair_ids: Vec<i64> = sqlx::query_scalar(
        r#"SELECT id FROM "isahl"."zc_id_subj-post_rr_view"
           WHERE ref_right = $1 AND deleted_at IS NULL ORDER BY id"#,
    )
    .bind(subject_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    // ② 私有联系人 = 本主体桥指向、且无其他活动引用的联系人（共享联系人的链不删）
    let own_contact_ids: Vec<i64> = sqlx::query_scalar(
        r#"SELECT ref_right FROM "isahl"."zc_id_entity_rr_contacts"
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    let private_contact_ids: Vec<i64> = if own_contact_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar(
            r#"SELECT c FROM unnest($1::bigint[]) AS c
               WHERE NOT EXISTS (SELECT 1 FROM "isahl"."zc_id_entity_rr_contacts" e
                                 WHERE e.ref_right = c AND e.ref_left <> $2 AND e.deleted_at IS NULL)
                 AND NOT EXISTS (SELECT 1 FROM "isahl"."zc_id_subj-post_rr_employee" pe
                                 WHERE pe.ref_right = c AND pe.deleted_at IS NULL)
                 AND NOT EXISTS (SELECT 1 FROM "isahl"."zc_id_subj-org_rr_employee" oe
                                 WHERE oe.ref_right = c AND oe.deleted_at IS NULL)
                 AND NOT EXISTS (SELECT 1 FROM "isahl"."zc_id_subj-group_rr_member" gm
                                 WHERE gm.ref_right = c AND gm.deleted_at IS NULL)
               ORDER BY c"#,
        )
        .bind(&own_contact_ids)
        .bind(subject_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?
    };
    // ③ 联系方式值行 id（链尾）——须在 `zc_id_contacts_rr_infos` 软删前取
    let info_ids: Vec<i64> = if private_contact_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar(
            r#"SELECT ref_right FROM "isahl"."zc_id_contacts_rr_infos"
               WHERE ref_left = ANY($1) AND deleted_at IS NULL"#,
        )
        .bind(&private_contact_ids)
        .fetch_all(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?
    };

    let mut cascaded = SubjectDeleteCascade::default();

    // ④ 主体属性面关系行（ref_left = 主体）
    cascaded.identity_bridges = soft_delete_subject_relations(
        &mut tx,
        soft_delete_relations_sql!("zc_id_entity_rr_identity", "ref_left"),
        subject_id,
        user_id,
    )
    .await?;
    cascaded.account_bridges = soft_delete_subject_relations(
        &mut tx,
        soft_delete_relations_sql!("zc_id_subjects_rr_account", "ref_left"),
        subject_id,
        user_id,
    )
    .await?;
    cascaded.place_bridges = soft_delete_subject_relations(
        &mut tx,
        soft_delete_relations_sql!("zc_id_subjects_rr_place", "ref_left"),
        subject_id,
        user_id,
    )
    .await?;
    // 合作评价挂桥（change add-supplier-qualification-certificates）：先软删桥下评估行
    // （ref_left = 桥行 id），再软删合作关系桥行（主体任一端）
    let partner_ids: Vec<i64> = sqlx::query_scalar(
        r#"SELECT id FROM "isahl"."zc_id_subjects_rr_partner"
           WHERE (ref_left = $1 OR ref_right = $1) AND deleted_at IS NULL ORDER BY id"#,
    )
    .bind(subject_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !partner_ids.is_empty() {
        cascaded.evaluations = sqlx::query(
            r#"UPDATE "isahl"."zc_id_relation-cooperation_r_evaluation"
               SET deleted_at = NOW(), deleted_by_id = $2
               WHERE ref_left = ANY($1) AND deleted_at IS NULL"#,
        )
        .bind(&partner_ids)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?
        .rows_affected();
    }
    cascaded.partner_bridges = sqlx::query(
        r#"UPDATE "isahl"."zc_id_subjects_rr_partner"
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE (ref_left = $1 OR ref_right = $1) AND deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?
    .rows_affected();

    // ⑤ 任职/成员桥（ref_right = 主体）
    cascaded.memberships = soft_delete_subject_relations(
        &mut tx,
        soft_delete_relations_sql!("zc_id_subj-post_rr_employee", "ref_right"),
        subject_id,
        user_id,
    )
    .await?
        + soft_delete_subject_relations(
            &mut tx,
            soft_delete_relations_sql!("zc_id_subj-org_rr_employee", "ref_right"),
            subject_id,
            user_id,
        )
        .await?
        + soft_delete_subject_relations(
            &mut tx,
            soft_delete_relations_sql!("zc_id_subj-group_rr_member", "ref_right"),
            subject_id,
            user_id,
        )
        .await?;

    // ⑥ 视角链：标签行（宿主 = 关联行 id）→ 关联行 → 自动默认岗位
    if !pair_ids.is_empty() {
        cascaded.view_tags = sqlx::query(
            r#"UPDATE "isahl"."zc_id_relation-post_view_r_tags"
               SET deleted_at = NOW(), deleted_by_id = $2
               WHERE ref_left = ANY($1) AND deleted_at IS NULL"#,
        )
        .bind(&pair_ids)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?
        .rows_affected();
    }
    cascaded.view_pairs = soft_delete_subject_relations(
        &mut tx,
        soft_delete_relations_sql!("zc_id_subj-post_rr_view", "ref_right"),
        subject_id,
        user_id,
    )
    .await?;
    cascaded.auto_positions = sqlx::query(
        r#"UPDATE "isahl"."zc_id_subj-position" p
           SET deleted_at = NOW(), deleted_by_id = $2
           WHERE p.code = 'POST-AUTO-' || $1::text AND p.deleted_at IS NULL
             AND NOT EXISTS (SELECT 1 FROM "isahl"."zc_id_subj-post_rr_view" r
                             WHERE r.ref_left = p.id AND r.deleted_at IS NULL AND r.ref_right <> $1)"#,
    )
    .bind(subject_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?
    .rows_affected();

    // ⑦ 私有联系人链：值行 → rr_infos → 联系人本体（链三段口径同 remove_subject_contact）
    if !info_ids.is_empty() {
        cascaded.contact_chain += sqlx::query(
            r#"UPDATE "isahl"."zc_id_contact_infos" x
               SET deleted_at = NOW(), deleted_by_id = $2, updated_at = NOW()
               WHERE x.id = ANY($1) AND x.deleted_at IS NULL"#,
        )
        .bind(&info_ids)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?
        .rows_affected();
    }
    if !private_contact_ids.is_empty() {
        cascaded.contact_chain += sqlx::query(
            r#"UPDATE "isahl"."zc_id_contacts_rr_infos" ri
               SET deleted_at = NOW(), deleted_by_id = $2, updated_at = NOW()
               WHERE ri.ref_left = ANY($1) AND ri.deleted_at IS NULL"#,
        )
        .bind(&private_contact_ids)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?
        .rows_affected();
        cascaded.contact_chain += sqlx::query(
            r#"UPDATE "isahl"."zc_id_contacts" c
               SET deleted_at = NOW(), deleted_by_id = $2, updated_at = NOW()
               WHERE c.id = ANY($1) AND c.deleted_at IS NULL"#,
        )
        .bind(&private_contact_ids)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?
        .rows_affected();
    }
    cascaded.contact_bridges = soft_delete_subject_relations(
        &mut tx,
        soft_delete_relations_sql!("zc_id_entity_rr_contacts", "ref_left"),
        subject_id,
        user_id,
    )
    .await?;

    tx.commit().await.map_err(ApiError::from_sqlx)?;
    Ok(cascaded)
}

/// DELETE /service/isahl-db/subjects/{id} — 软删除主体（级联该主体自身关系行，单事务）
pub async fn delete_subject(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", id, "delete").await?;

    let cascaded = delete_subject_cascade(pool.get_ref(), id, user_id).await?;
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "deleted": true,
            "cascaded": cascaded,
        }))),
    )
}

/// 主体存在性校验（subjects 继承链统一可见）
pub(crate) async fn ensure_subject_exists(pool: &PgPool, subject_id: i64) -> Result<(), ApiError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM \"isahl\".\"zc_id_subjects\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !exists {
        return Err(ApiError::NotFound(format!("主体不存在: {}", subject_id)));
    }
    Ok(())
}

/// 物权分类 code → 字典行 id（`zc_id_cate-real_rights`，主体↔储元桥 `ck_real_rights`）。
///
/// 按 code 解析而非 id：ZUID 跨库不通用（dev / ns / 重建库的字典行 id 不同），code 才是
/// 模型侧稳定标识。字典无活动行 → 400 点名 code，MUST NOT 静默落 NULL（否则物权类型被悄悄丢弃）。
pub(crate) async fn resolve_real_rights_id(pool: &PgPool, code: &str) -> Result<i64, ApiError> {
    sqlx::query_scalar(
        "SELECT id FROM \"isahl\".\"zc_id_cate-real_rights\" \
         WHERE code = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(code)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from_sqlx)?
    .ok_or_else(|| {
        ApiError::BadRequest(format!(
            "未知物权分类 code: '{}'（字典 zc_id_cate-real_rights）",
            code
        ))
    })
}

/// 可选物权分类解析：空/未传 → None（模型侧非必填）；有值 → 字典 id（未知 code 仍 400）。
pub(crate) async fn resolve_real_rights_id_opt(
    pool: &PgPool,
    code: Option<&str>,
) -> Result<Option<i64>, ApiError> {
    match code.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => resolve_real_rights_id(pool, c).await.map(Some),
        None => Ok(None),
    }
}

// ═══════════════════════════════════════════════════════
// GET /subjects/types — 主体叶表发现（动态）
// ═══════════════════════════════════════════════════════

/// 主体类型项（叶表发现结果）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubjectTypeItem {
    /// 叶表名（create 时作为 subject_type 传入）
    table_name: String,
    /// 叶表名（兼容旧字段）
    code: String,
    /// 社会分类标签（列表筛选 `category` 参数取值；由叶表名派生）
    ///
    /// 客户端**筛选项候选必须由本字段派生**（叶表发现 = 唯一候选源）：接口只返叶表，
    /// 中间层/禁写父表（法人/组织/雇员等）不可能出现，故候选集天然不含不可选集分类。
    category: String,
    /// 是否系统级分类（国家/超国家/银行等，不提供创建入口由前端过滤）
    system: bool,
}

/// GET /service/isahl-db/subjects/types — 动态发现 zc_id_subjects 继承链叶表
///
/// 数据源：pg_inherits 递归（真实 DB 继承关系），非硬编码。
/// 业务名由叶表名派生（Alioth 模型 collection 名在 Meta 侧，WZ 库无元数据）。
pub async fn list_subject_types(
    req: HttpRequest,
    pool: web::Data<PgPool>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "list").await?;

    // 真叶表：pg_inherits 递归 descendants 中**无子表**的表。
    // 中间表（zc_id_orga-legal 法人、zc_id_subj-org 组织、zc_id_subj-employee 雇员）
    // 有子表，非叶表——不可直接插入（tableoid 判定分类会落到父表）。
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"WITH RECURSIVE descendants AS (
               SELECT c.oid, c.relname AS leaf
               FROM pg_class c
               JOIN pg_namespace n ON n.oid = c.relnamespace
               WHERE n.nspname = 'isahl' AND c.relname = 'zc_id_subjects'
               UNION ALL
               SELECT child.oid, child.relname
               FROM pg_inherits i
               JOIN pg_class child ON child.oid = i.inhrelid
               JOIN pg_namespace n ON n.oid = child.relnamespace AND n.nspname = 'isahl'
               JOIN descendants ON descendants.oid = i.inhparent
           ),
           has_children AS (
               SELECT DISTINCT i.inhparent AS oid
               FROM pg_inherits i
               JOIN pg_class child ON child.oid = i.inhrelid
               JOIN pg_namespace n ON n.oid = child.relnamespace
               WHERE n.nspname = 'isahl'
           )
           SELECT DISTINCT d.leaf FROM descendants d
           WHERE d.leaf <> 'zc_id_subjects'
             AND d.oid NOT IN (SELECT oid FROM has_children)
           ORDER BY d.leaf"#,
    )
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    // 系统级分类（不提供常规创建入口；央行/国家/超国家/主权/部委/层级/银行账户）。
    // 注意：商业银行（zc_id_bank-commercial）继承法人（zc_id_orga-legal），是企业类叶表，
    // 提供创建入口；中央银行（zc_id_bank-central）继承部委，属系统级。
    const SYSTEM_LEAVES: [&str; 7] = [
        "zc_id_bank-central",
        "zc_id_subj-country",
        "zc_id_subj-supranational",
        "zc_id_subj-sovereign",
        "zc_id_subj-ministry",
        "zc_id_subj-bank",
        "zc_id_subj-hierarchy",
    ];

    let items: Vec<SubjectTypeItem> = rows
        .into_iter()
        .map(|(leaf,)| {
            let table_name = leaf.clone();
            SubjectTypeItem {
                system: SYSTEM_LEAVES.iter().any(|p| leaf.starts_with(p)),
                category: social_category(&table_name).to_string(),
                code: leaf,
                table_name,
            }
        })
        .collect();

    Ok(HttpResponse::Ok().json(ApiResponse::success(items)))
}

/// 校验 subject_type 是合法主体类型（防注入与非主体表）
/// - 叶表名（zc_id_ 前缀）→ 校验 ∈ subjects 继承链**且为真叶表**（无子表）
/// - 业务别名（法人/雇员/组 等）→ subject_leaf_table 白名单映射即合法
#[allow(dead_code)] // 主体叶表校验（防注入），创建路径接线待定
async fn ensure_subject_leaf(pool: &PgPool, subject_type: &str) -> Result<(), ApiError> {
    if subject_type.starts_with("zc_id_") {
        // 真叶表：descendants 中无子表的表。中间表（orga-legal 等）不可插入。
        let exists: bool = sqlx::query_scalar(
            r#"WITH RECURSIVE descendants AS (
                   SELECT c.oid, c.relname AS leaf
                   FROM pg_class c
                   JOIN pg_namespace n ON n.oid = c.relnamespace
                   WHERE n.nspname = 'isahl' AND c.relname = 'zc_id_subjects'
                   UNION ALL
                   SELECT child.oid, child.relname
                   FROM pg_inherits i
                   JOIN pg_class child ON child.oid = i.inhrelid
                   JOIN pg_namespace n ON n.oid = child.relnamespace AND n.nspname = 'isahl'
                   JOIN descendants ON descendants.oid = i.inhparent
               ),
               has_children AS (
                   SELECT DISTINCT i.inhparent AS oid
                   FROM pg_inherits i
                   JOIN pg_class child ON child.oid = i.inhrelid
                   JOIN pg_namespace n ON n.oid = child.relnamespace
                   WHERE n.nspname = 'isahl'
               )
               SELECT EXISTS (
                   SELECT 1 FROM descendants d
                   WHERE d.leaf = $1 AND d.oid NOT IN (SELECT oid FROM has_children)
               )"#,
        )
        .bind(subject_type)
        .fetch_one(pool)
        .await
        .map_err(ApiError::from_sqlx)?;
        if !exists {
            return Err(ApiError::BadRequest(format!(
                "主体类型不是 zc_id_subjects 叶表: {}",
                subject_type
            )));
        }
        Ok(())
    } else {
        // 业务别名：subject_leaf_table 白名单覆盖即合法；未知 → 400（fail-fast）
        if crate::models::subject_leaf_table(subject_type).is_none() {
            return Err(ApiError::BadRequest(format!(
                "未知主体类型: {}",
                subject_type
            )));
        }
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════
// POST /subjects — 分类感知主体创建
// ═══════════════════════════════════════════════════════

/// 分类感知创建请求体。
/// 扩展信息通过 Alioth 关联关系承载（零 DDL）：
/// - view_tags    → 主体→岗位(`zc_id_subj-post_rr_view`)→**关联行标签**(`zc_id_relation-post_view_r_tags.ref_left`=关联行 id)→`zc_id_tags-post_view`
/// - employ_status→ `zc_id_subj-employee_r_employ-status`（就业状态，ref_right→stus-employ 字典；雇佣双方链为 subj-org_rr_employee）
/// - employer_id  → `zc_id_subj-org_rr_employee`（所属组织：ref_left=组织, ref_right=雇员）
/// - position_id  → `zc_id_subj-post_rr_view`（视角锚点：ref_left=岗位, ref_right=被视角主体；任职关系不在此路径，由 entity-binding 经 post_rr_employee 承载）
/// - place_id     → `zc_id_subjects_rr_place`（地址：ref_left=主体, ref_right=place）
/// - member_ids   → `zc_id_subj-group_rr_member`（组成员：ref_left=组, ref_right=成员）
///
/// 注册 org-wz 主体管理路由
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/subjects")
            .route(web::get().to(list_subjects))
            .route(web::post().to(create_subject)),
    )
    .service(
        // 注意：/subjects/types 必须注册在 /subjects/{id} 之前，
        // 否则 "types" 被 {id} 路由匹配（GET 无 → 405 Method Not Allowed）。
        web::resource("/subjects/types").route(web::get().to(list_subject_types)),
    )
    .service(
        web::resource("/subjects/{id}")
            .route(web::get().to(get_subject))
            .route(web::put().to(update_subject))
            .route(web::delete().to(delete_subject)),
    )
    .service(
        web::resource("/subjects/{id}/view-tags")
            .route(web::get().to(get_subject_view_tags))
            .route(web::put().to(put_subject_view_tags)),
    )
    .service(
        web::resource("/subjects/{id}/accounts")
            .route(web::get().to(list_subject_accounts))
            .route(web::post().to(add_subject_account)),
    )
    .service(
        web::resource("/subjects/{id}/accounts/{relId}")
            .route(web::delete().to(delete_subject_account)),
    )
    .service(
        // 联系方式链（实体↔联系人↔联系方式）：读/增/删，供综合管理编辑闭环
        web::resource("/subjects/{id}/contacts")
            .route(web::get().to(list_subject_contact_chain))
            .route(web::post().to(add_subject_contact)),
    )
    .service(
        web::resource("/subjects/{id}/contacts/{contactId}")
            .route(web::delete().to(remove_subject_contact)),
    );
}

/// 联系方式叶表 INSERT（编译期固化：表名是字面量，正文单一来源）
macro_rules! info_insert_sql {
    ($table:literal) => {
        concat!(
            "INSERT INTO \"isahl\".\"",
            $table,
            "\" (id, notice, created_by_id) \
             VALUES (isahl.gen_next_zuid(), $1, $2) RETURNING id"
        )
    };
}

/// kind → typed 联系方式叶表静态 SQL（闭式白名单；缺省 telephone）
fn contact_info_sql(kind: Option<&str>) -> &'static str {
    match kind.map(str::trim).filter(|k| !k.is_empty()) {
        Some("email") => info_insert_sql!("zc_id_info-email"),
        Some("im") => info_insert_sql!("zc_id_info-im"),
        Some("isahl") => info_insert_sql!("zc_id_info-isahl"),
        Some("postal") | Some("address") => info_insert_sql!("zc_id_info-postal"),
        Some("zipcode") | Some("zip") => info_insert_sql!("zc_id_info-zipcode"),
        _ => info_insert_sql!("zc_id_info-telephone"),
    }
}

/// 向实体（主体=实体子表行）追加一条联系方式（实体↔联系人↔联系方式链，同事务）
#[allow(clippy::too_many_arguments)] // 联系方式字段集（实体+联系人+类别+值+默认+操作者）——参数即领域
pub async fn add_entity_contact(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    entity_id: i64,
    display_name: &str,
    contact_name: Option<&str>,
    kind: Option<&str>,
    value: &str,
    is_default: bool,
    user_id: i64,
) -> Result<i64, ApiError> {
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve_conn(tx, ("TX", "FJA", "↓_GG"))
            .await
            .map_err(ApiError::from)?;
    let contact_row = sqlx::query_scalar::<_, i64>(
    r#"INSERT INTO "isahl"."zc_id_contacts" (id, code, notice, created_by_id, dk_scene, dk_factor, dk_function)
       VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6) RETURNING id"#,
)
.bind(format!("CT-ENT-{}", entity_id))
.bind(contact_name.unwrap_or(&format!("{} 联系方式", display_name)))
.bind(user_id)
.bind(dk_scene)
.bind(dk_factor)
.bind(dk_function)
.fetch_one(&mut **tx)
.await
.map_err(ApiError::from_sqlx)?;
    let info_sql = contact_info_sql(kind);
    let info_id: i64 = sqlx::query_scalar(info_sql)
        .bind(value)
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(ApiError::from_sqlx)?;
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_contacts_rr_infos"
       (notice, ref_left, ref_right, default_info, created_by_id)
       VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(format!("{} 联系方式", display_name))
    .bind(contact_row)
    .bind(info_id)
    .bind(is_default)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_entity_rr_contacts"
       (id, code, notice, ref_left, ref_right, default_contact, created_by_id)
       VALUES (isahl.gen_next_uid(251), $1, $2, $3, $4, $5, $6)"#,
    )
    .bind(format!("REL-CT-{}", entity_id))
    .bind(format!("{} 联系方式", display_name))
    .bind(entity_id)
    .bind(contact_row)
    .bind(is_default)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    Ok(contact_row)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SubjectContactItem {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    /// 联系人显示名（zc_id_contacts.notice）
    pub name: String,
    /// 类型（kind：telephone/email/im/postal/zipcode/isahl，由值行 tableoid 派生）
    pub kind: String,
    /// 值（typed 叶表 notice）
    pub value: String,
    /// 是否默认联系方式（rr_infos.default_info）
    pub is_default: bool,
}

/// GET /subjects/{id}/contacts — 联系方式链只读列表
pub async fn list_subject_contact_chain(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "read").await?;
    ensure_subject_exists(pool.get_ref(), subject_id).await?;

    let rows: Vec<SubjectContactItem> = sqlx::query_as(
        r#"SELECT c.id, COALESCE(c.notice, '') AS name,
                  CASE WHEN p.id IS NOT NULL THEN 'postal'
                       WHEN z.id IS NOT NULL THEN 'zipcode'
                       WHEN e.id IS NOT NULL THEN 'email'
                       WHEN im.id IS NOT NULL THEN 'im'
                       WHEN isa.id IS NOT NULL THEN 'isahl'
                       WHEN t.id IS NOT NULL THEN 'telephone'
                       ELSE 'telephone' END AS kind,
                  COALESCE(i.notice, '') AS value,
                  COALESCE(ri.default_info, false) AS is_default
           FROM "isahl"."zc_id_entity_rr_contacts" rc
           JOIN "isahl"."zc_id_contacts" c ON c.id = rc.ref_right AND c.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_contacts_rr_infos" ri ON ri.ref_left = c.id AND ri.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_info-telephone" t ON t.id = ri.ref_right AND t.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_info-email" e ON e.id = ri.ref_right AND e.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_info-im" im ON im.id = ri.ref_right AND im.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_info-isahl" isa ON isa.id = ri.ref_right AND isa.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_info-postal" p ON p.id = ri.ref_right AND p.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_info-zipcode" z ON z.id = ri.ref_right AND z.deleted_at IS NULL
           LEFT JOIN LATERAL (
               SELECT x.notice FROM "isahl"."zc_id_contact_infos" x
               WHERE x.id = ri.ref_right AND x.deleted_at IS NULL LIMIT 1
           ) i ON ri.ref_right IS NOT NULL
           WHERE rc.ref_left = $1 AND rc.deleted_at IS NULL
           ORDER BY rc.id"#,
    )
    .bind(subject_id)
    .fetch_all(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(rows)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddSubjectContactRequest {
    /// 联系人显示名（可空，缺省「{主体名} 联系方式」）
    pub name: Option<String>,
    /// 类型（telephone/email/im/postal/zipcode/isahl；缺省 telephone）
    pub kind: Option<String>,
    /// 值（必填）
    pub value: String,
    #[serde(default)]
    pub is_default: bool,
}

/// POST /subjects/{id}/contacts — 追加一条联系方式（同事务写链）
pub async fn add_subject_contact(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<AddSubjectContactRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "update").await?;
    if body.value.trim().is_empty() {
        return Err(ApiError::BadRequest("联系方式值不能为空".into()));
    }
    ensure_subject_exists(pool.get_ref(), subject_id).await?;

    let display_name: String = sqlx::query_scalar(
        r#"SELECT COALESCE(notice, '') FROM "isahl"."zc_id_subjects" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .fetch_one(pool.get_ref())
    .await
    .map_err(ApiError::from_sqlx)?;

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;
    let contact_id = add_entity_contact(
        &mut tx,
        subject_id,
        &display_name,
        body.name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        body.kind.as_deref(),
        body.value.trim(),
        body.is_default,
        user_id,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(
        HttpResponse::Created().json(ApiResponse::success(serde_json::json!({
            "id": contact_id.to_string(),
        }))),
    )
}

/// DELETE /subjects/{id}/contacts/{contactId} — 软删一条联系方式（链三段同事务）
pub async fn remove_subject_contact(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (subject_id, contact_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "update").await?;

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;
    let linked = sqlx::query(
        r#"UPDATE "isahl"."zc_id_entity_rr_contacts" rc
           SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
           WHERE rc.ref_left = $1 AND rc.ref_right = $3 AND rc.deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .bind(user_id)
    .bind(contact_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?
    .rows_affected();
    if linked == 0 {
        return Err(ApiError::NotFound(format!(
            "联系方式不存在: {}",
            contact_id
        )));
    }
    // 值行软删（先取 id——rr_infos 软删前）
    let info_ids: Vec<i64> = sqlx::query_scalar(
        r#"SELECT ref_right FROM "isahl"."zc_id_contacts_rr_infos"
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(contact_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !info_ids.is_empty() {
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_contact_infos" x
               SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
               WHERE x.id = ANY($1) AND x.deleted_at IS NULL"#,
        )
        .bind(&info_ids)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
    }
    sqlx::query(
        r#"UPDATE "isahl"."zc_id_contacts_rr_infos" ri
           SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
           WHERE ri.ref_left = $1 AND ri.deleted_at IS NULL"#,
    )
    .bind(contact_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    sqlx::query(
        r#"UPDATE "isahl"."zc_id_contacts" c
           SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
           WHERE c.id = $1 AND c.deleted_at IS NULL"#,
    )
    .bind(contact_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(HttpResponse::NoContent().finish())
}
#[cfg(test)]
mod list_count_sql_probe {
    use super::*;

    /// 计数 SQL 与列表谓词同源（`push_subject_filters`）——防「两份副本漂移」回归：
    /// ① 计数 FROM 与列表同一起点（尾部 `WHERE s.deleted_at IS NULL`）；
    /// ② `exclude_disabled` 子句必须由共享谓词下发（此前计数副本漏该子句）。
    ///
    /// 移植说明：原作（远端 `tmp_list_count_sql_probe`）引用其分支的 `SUBJECT_LIST_FROM` /
    /// `push_subject_list_filters`——本分支对应符号 = `SUBJECT_COUNT_FROM` /
    /// `push_subject_filters`（裁定：保留验证意图，移植到本分支符号）。
    #[test]
    fn probe_sql_text() {
        assert!(
            SUBJECT_COUNT_FROM.ends_with("WHERE s.deleted_at IS NULL"),
            "count FROM 尾部异常"
        );
        let mut b: QueryBuilder<Postgres> = QueryBuilder::new("");
        push_subject_filters(
            &mut b,
            &SubjectFilters {
                q: "",
                view: "",
                category: "",
                kind: "",
                exclude_system: true,
                exclude_disabled: true,
            },
        );
        assert!(
            b.sql().as_str().contains("ss.code = ANY("),
            "exclude_disabled 子句缺失"
        );
    }
}
