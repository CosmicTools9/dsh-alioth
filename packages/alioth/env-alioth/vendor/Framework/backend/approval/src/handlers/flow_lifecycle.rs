//! 流程生命周期类端点 — 设计·实例 / 实现·范例 分离（flow-lifecycle-split）
//!
//! `_f_`/`_t_` 是 `dk_function.code` 前缀派生的自动分类列（ALIOTH_ONTOLOGY_SPEC
//! §4.3.1 强不变量，trigger `LifecycleBizTemplate` 落值，业务层禁写）：
//! - 设计·实例 = `↑_*` 前缀（流程定义页卡片数据源）
//! - 实现·范例 = `↓.*` 前缀（流程模板页卡片数据源；tpl_id → 设计·实例行）
//! - 实现·实例 = `↓_*` 前缀（运行期执行行，由发起路径物化，见 advance.rs）
use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use common::error::AliothError;
use common::{ApiResponse, PaginatedResponse};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

/// 设计·实例（`_f_='设计' AND (_t_='实例' OR _t_ IS NULL)`，function 前缀 `↑_`）——
/// `_t_ IS NULL` 为设计·模板行（历史 monitor ↓_BA 共享行遗留形态——如 GTPL-*；
/// 门禁审批流行 GT-PLAN/GT-DESIGN/FLOW-STD/FLOW-URGENT 已收敛为 设计·实例，
/// 见 seed-avic-caasec-business.sh「类订正」，设计器列表两者皆可见）
pub(crate) const CLASS_DESIGN_INSTANCE: &str = "design-instance";
/// 实现·范例（`_f_='实现' AND (_t_='范例' OR _t_ IS NULL)`，function 前缀 `↓.`）
pub(crate) const CLASS_IMPL_EXEMPLAR: &str = "impl-exemplar";

/// 类谓词（静态文本，编译期常量；`_f_`/`_t_` 为派生文本列，直接字面量过滤）
pub(crate) fn class_predicate(class: &str) -> Option<(&'static str, &'static str)> {
    match class {
        CLASS_DESIGN_INSTANCE => Some(("设计", "实例")),
        CLASS_IMPL_EXEMPLAR => Some(("实现", "范例")),
        _ => None,
    }
}

#[derive(Debug, FromRow, Serialize)]
pub struct LifecycleFlowRow {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub code: Option<String>,
    pub t_color_: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_context: Option<i64>,
    /// 实际落位叶表（tableoid 派生，如 zc_id_proc-approve）
    pub branch: Option<String>,
    pub context_concept: Option<String>,
    pub context_leaf: Option<String>,
    /// 生命周期主状态（_r_primary-status 桥派生：draft/published/deprecated）
    #[sqlx(default)]
    pub status: Option<String>,
    /// 模板锚点：范例行 → 设计·实例 id；设计/执行行为 NULL
    #[serde(with = "common::serde_zuid::opt")]
    pub tpl_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    /// 设计图 JSON 信封（FlowGraphPayload；画布打开数据源，缺失时画布空）
    pub meta: Option<serde_json::Value>,
    /// 生命周期类属（_t_ 派生文本：'实例'/'范例'；NULL=设计·模板行遗留形态——
    /// 可见于列表但不可 generate-template/initiate；门禁审批流行已收敛为实例）
    pub class_t: Option<String>,
}

const LIFECYCLE_SELECT: &str =
    "e.id, e.notice AS name, e.code, e.t_color_, e.comments, e.meta, e.fk_context, \
     replace(e.tableoid::regclass::text, '\"', '') AS branch, \
     (SELECT c.notice FROM isahl.\"zc_id_proc-context\" c \
      WHERE c.id = e.fk_context AND c.deleted_at IS NULL) AS context_concept, \
     (SELECT replace(c.tableoid::regclass::text, '\"', '') FROM isahl.\"zc_id_proc-context\" c \
      WHERE c.id = e.fk_context AND c.deleted_at IS NULL) AS context_leaf, \
     (SELECT s.code FROM isahl.\"zc_id_lifecycle_r_primary-status\" ls \
      JOIN isahl.\"zc_id_stus-process\" s ON s.id = ls.ref_right \
      WHERE ls.ref_left = e.id AND ls.deleted_at IS NULL) AS status, \
     e.tpl_id, e.created_at, e.updated_at, e._t_ AS class_t";

const LIFECYCLE_FROM: &str = "FROM isahl.zc_id_process e";

#[derive(Debug, Deserialize)]
pub struct LifecycleListQuery {
    #[serde(default, with = "common::serde_zuid::opt")]
    pub page: Option<i64>,
    #[serde(alias = "pageSize", default, with = "common::serde_zuid::opt")]
    pub page_size: Option<i64>,
    /// name/code 模糊搜索（% 通配符剥离，防注入式宽匹配）
    #[serde(default)]
    pub search: Option<String>,
}

/// GET /approval-flows/lifecycle/{class} — 按生命周期类列流程卡片数据。
/// 注册于 CRUD scope 之前（scope-options 同款先例），避免 {class} 被吞为 {id}。
pub async fn list_by_class(
    pool: web::Data<PgPool>,
    path: web::Path<String>,
    query: web::Query<LifecycleListQuery>,
) -> Result<HttpResponse, AliothError> {
    let class = path.into_inner();
    let (f, t) = class_predicate(&class).ok_or_else(|| AliothError::Validation {
        field: "class".into(),
        message: format!(
            "非法生命周期类 '{class}'——合法值：{CLASS_DESIGN_INSTANCE}（设计·实例）/ \
             {CLASS_IMPL_EXEMPLAR}（实现·范例）"
        ),
    })?;

    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(100).clamp(1, 200);
    let offset = (page - 1) * page_size;
    let needle = query
        .search
        .as_deref()
        .map(|s| format!("%{}%", s.trim().replace('%', "")))
        .filter(|p| p != "%%");

    // _t_ 放宽：匹配类值或未设置（模板行 _t_=NULL 属对应 _f_ 类可见范围，
    // 如 design-instance 须含标准审批流程模板——FLOW-STD/FLOW-URGENT/GT-PLAN/GT-DESIGN）
    let where_sql = "WHERE e.deleted_at IS NULL AND e._f_ = $1 AND (e._t_ = $2 OR e._t_ IS NULL) \
                     AND ($3::text IS NULL OR e.notice ILIKE $3 OR e.code ILIKE $3)";

    let list_sql = format!(
        "SELECT {LIFECYCLE_SELECT} {LIFECYCLE_FROM} {where_sql} \
         ORDER BY e.created_at DESC LIMIT $4 OFFSET $5"
    );
    let items: Vec<LifecycleFlowRow> =
        sqlx::query_as::<_, LifecycleFlowRow>(sqlx::AssertSqlSafe(list_sql.as_str()))
            .bind(f)
            .bind(t)
            .bind(needle.as_deref())
            .bind(page_size)
            .bind(offset)
            .fetch_all(pool.get_ref())
            .await
            .map_err(AliothError::from)?;

    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(
        format!("SELECT COUNT(*) {LIFECYCLE_FROM} {where_sql}").as_str(),
    ))
    .bind(f)
    .bind(t)
    .bind(needle.as_deref())
    .fetch_one(pool.get_ref())
    .await
    .map_err(AliothError::from)?;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(PaginatedResponse::new(
            items, total, page, page_size,
        ))),
    )
}

#[derive(Debug, Serialize)]
pub struct GenerateTemplateResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub code: Option<String>,
    /// 实际落位叶表
    pub branch: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub tpl_id: Option<i64>,
}

/// POST /approval-flows/{id}/generate-template — 设计·实例 → 实现·范例。
///
/// 校验链（全 400 fail-closed）：
/// 1. 行在册且为设计·实例（`_f_='设计' AND _t_='实例'`）
/// 2. function 码 `↑_` 前缀（类不变量的码侧对齐；↓. 对应码必须字典在册）
/// 3. 生命周期主状态 = published（「有效」态；zc_id_stus-process 仅
///    draft/published/deprecated 三档）
/// 4. 幂等守卫：同 tpl_id 且同 notice 的在册范例已存在 → 400（防连点重复克隆）
///
/// 克隆：同叶表落 实现·范例 行——notice/code/comments/t_color_/fk_context/
/// dk_scene/dk_factor 原样复制，dk_function 换 `↓.{suffix}` 码，tpl_id → 设计行。
pub async fn generate_template(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<i64>,
) -> Result<HttpResponse, AliothError> {
    // 写端点对齐 initiate/publish：必须携带已认证会话
    let _ = common::context::require_auth(&req)?;
    let design_id = path.into_inner();
    let pool = pool.get_ref();

    // 1. 设计·实例行在册（含 function 码与落位叶表）
    let row: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        r#"SELECT replace(tableoid::regclass::text, '"', ''), notice,
                  (SELECT f.code FROM isahl.zc_id_function f
                   WHERE f.id = e.dk_function AND f.deleted_at IS NULL LIMIT 1)
           FROM isahl.zc_id_process e
           WHERE e.id = $1 AND e.deleted_at IS NULL
             AND e._f_ = '设计' AND e._t_ = '实例'"#,
    )
    .bind(design_id)
    .fetch_optional(pool)
    .await
    .map_err(AliothError::from)?;
    let Some((branch, _notice, fn_code)) = row else {
        return Err(AliothError::Validation {
            field: "flow".into(),
            message: format!("流程 {design_id} 不存在或不是设计·实例（_f_=设计/_t_=实例）"),
        });
    };

    // 2. function 码 → 实现·范例码：`↑_*` 前缀换 `↓.{suffix}`；非 ↑_ 前缀
    //    （含 NULL/字典缺码）置 NULL——类契约由 _f_/_t_ 字面量承载（见 clone_sql），
    //    dk 码侧仅做一致性装饰，不阻塞克隆
    let exemplar_fn_id: Option<i64> = match fn_code.as_deref().filter(|c| c.starts_with("↑_")) {
        Some(code) => {
            let exemplar_fn_code = format!("↓.{}", &code["↑_".len()..]);
            sqlx::query_scalar(
                r#"SELECT id FROM isahl.zc_id_function
                   WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
            )
            .bind(&exemplar_fn_code)
            .fetch_optional(pool)
            .await
            .map_err(AliothError::from)?
            .flatten()
        }
        None => None,
    };

    // 3. 「有效」态（published）
    let published: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
             SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
             JOIN isahl."zc_id_stus-process" s ON s.id = ls.ref_right
             WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL AND s.code = 'published'
           )"#,
    )
    .bind(design_id)
    .fetch_one(pool)
    .await
    .map_err(AliothError::from)?;
    if !published {
        return Err(AliothError::Validation {
            field: "status".into(),
            message: format!("流程 {design_id} 未处于有效态（published）——不可生成模板"),
        });
    }

    // 4. 幂等守卫：同源同名范例已存在
    let duplicate: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
             SELECT 1 FROM isahl.zc_id_process e
             WHERE e.tpl_id = $1 AND e.deleted_at IS NULL
               AND e._f_ = '实现' AND e._t_ = '范例'
               AND e.notice = (SELECT notice FROM isahl.zc_id_process WHERE id = $1)
           )"#,
    )
    .bind(design_id)
    .fetch_one(pool)
    .await
    .map_err(AliothError::from)?;
    if duplicate {
        return Err(AliothError::Validation {
            field: "tpl_id".into(),
            message: format!("设计流程 {design_id} 已存在同名实现·范例——勿重复生成"),
        });
    }

    // 5. 同叶表克隆（静态 match 分发，禁 format! 动态表名；对齐 repositories::create）
    let insert_sql = match branch.as_str() {
        "zc_id_proc-approve" => clone_sql("isahl.\"zc_id_proc-approve\""),
        "zc_id_proc-cicd" => clone_sql("isahl.\"zc_id_proc-cicd\""),
        "zc_id_proc-loading" => clone_sql("isahl.\"zc_id_proc-loading\""),
        "zc_id_proc-make" => clone_sql("isahl.\"zc_id_proc-make\""),
        "zc_id_proc-project" => clone_sql("isahl.\"zc_id_proc-project\""),
        "zc_id_proc-purchase" => clone_sql("isahl.\"zc_id_proc-purchase\""),
        "zc_id_proc-service" => clone_sql("isahl.\"zc_id_proc-service\""),
        other => {
            return Err(AliothError::Validation {
                field: "branch".into(),
                message: format!("未知流程叶表分支 '{other}'——不可克隆"),
            });
        }
    };
    let created: (i64, String, Option<String>) =
        sqlx::query_as(sqlx::AssertSqlSafe(insert_sql.as_str()))
            .bind(design_id)
            .bind(exemplar_fn_id)
            // 静态 SQL（表名编译期常量 + 参数化值），AssertSqlSafe 声明已审计
            .fetch_one(pool)
            .await
            .map_err(AliothError::from)?;

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(GenerateTemplateResponse {
            id: created.0,
            name: created.1,
            code: created.2,
            branch: Some(branch),
            tpl_id: Some(design_id),
        })),
    )
}

/// 克隆 INSERT：notice/code/comments/t_color_/fk_context/dk_scene/dk_factor 原样，
/// dk_function → 实现·范例码 id（$2），tpl_id → 设计行（$1）。
fn clone_sql(table: &str) -> String {
    format!(
        r#"INSERT INTO {table}
           (notice, code, comments, t_color_, meta, mermaid, fk_context, dk_scene, dk_factor, dk_function,
            tpl_id, created_by_id, _f_, _t_)
           SELECT notice, code, comments, t_color_, meta, mermaid, fk_context, dk_scene, dk_factor, $2,
                  $1,
                  (SELECT created_by_id FROM isahl.zc_id_process WHERE id = $1),
                  '实现', '范例'
           FROM isahl.zc_id_process WHERE id = $1
           RETURNING id, notice, code"#
    )
}
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/approval-flows/lifecycle/{class}",
        web::get().to(list_by_class),
    )
    .route(
        "/approval-flows/{id}/generate-template",
        web::post().to(generate_template),
    );
}
