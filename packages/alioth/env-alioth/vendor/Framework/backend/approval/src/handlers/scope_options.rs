//! 流程范畴选项端点 — `GET /approval-flows/scope-options`
//!
//! 为「新建流程」对话框提供两棵树：
//! - branches：zc_id_process 的叶表分支（流程自身范畴，一级分类）
//! - domains：流程输入范畴（fk_context 可选值）三域——task（任务）/
//!   event（非审批事件）/ approve（审批事件），叶表项附业务概念与范畴定义行 scopeId
//!
//! 运行时可见性约束（2026-08-27 裁决）：isahl_meta 在 app 运行时不可见——
//! 继承结构与业务概念编译期绑定于 `crate::context_meta`（AUTO-GENERATED，
//! `bun scripts/generate-context-fields.ts` 重建，模型升级后重跑），
//! 运行时零 catalog 查询。仅范畴定义行 scope_id（`zc_id_proc-context`
//! 业务数据，可运行时增删）保留 DB 读取。

use actix_web::{web, HttpResponse};
use common::error::AliothError;
use common::ApiResponse;
use serde::Serialize;
use sqlx::PgPool;

use crate::context_meta::{SCOPE_BRANCHES, SCOPE_DOMAINS};

#[derive(Debug, Serialize)]
pub struct BranchOption {
    pub table: String,
    pub concept: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ContextItem {
    pub table: String,
    pub concept: Option<String>,
    /// 范畴定义行 zuid（_t_='scope-definition'）；未种子化时 null
    #[serde(with = "common::serde_zuid::opt")]
    pub scope_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ContextDomain {
    /// task / event / approve（前端 i18n 负责领域显示名）
    pub key: &'static str,
    pub items: Vec<ContextItem>,
}

#[derive(Debug, Serialize)]
pub struct ScopeOptions {
    pub branches: Vec<BranchOption>,
    pub domains: Vec<ContextDomain>,
    /// 终端节点语义实体叶表（2026-08-29 裁决）：end 的 statement 叶表与
    /// task 驱动 start 的 task 叶表（设计器 Inspector 下拉数据源）
    pub terminal_leaves: TerminalLeafOptions,
}

#[derive(Debug, Serialize)]
pub struct TerminalLeafOptions {
    /// statement 族真叶表（end 节点结论承载落表选项）
    pub statement: Vec<BranchOption>,
    /// task 族真叶表（start 节点 task 驱动选项）
    pub task: Vec<BranchOption>,
    /// event 族真叶表（start 节点 event 驱动的「事件类型」选项）
    pub event: Vec<BranchOption>,
}

#[derive(serde::Deserialize)]
pub struct SceneQuery {
    /// 场景码（如 AVIC=JE）：按场景过滤 end 结论承载叶表——
    /// 仅保留该场景存在活跃行的 statement 叶表（跨 ns 共享叶表的
    /// namespace 归属由行级 dk_scene 表达；缺省不过滤返回全模型叶表）
    pub scene: Option<String>,
}
pub async fn scope_options(
    pool: web::Data<PgPool>,
    query: web::Query<SceneQuery>,
) -> Result<HttpResponse, AliothError> {
    // 范畴定义行（父表读聚合 + tableoid 派生叶表归属，避免动态叶表查询）——
    // 业务数据，唯一保留的运行时 DB 读取
    let scope_rows: Vec<(String, i64)> = sqlx::query_as(
        r#"SELECT replace(tableoid::regclass::text, '"', '') AS leaf, id
           FROM isahl."zc_id_proc-context"
           WHERE _t_ = 'scope-definition' AND deleted_at IS NULL"#,
    )
    .fetch_all(pool.get_ref())
    .await?;
    let scope_of = |table: &str| -> Option<i64> {
        scope_rows
            .iter()
            .find(|(leaf, _)| leaf == table)
            .map(|(_, id)| *id)
    };

    let branches = SCOPE_BRANCHES
        .iter()
        .map(|b| BranchOption {
            table: b.table.to_string(),
            concept: b.concept.map(str::to_string),
        })
        .collect::<Vec<_>>();

    let domains = SCOPE_DOMAINS
        .iter()
        .map(|d| ContextDomain {
            key: d.key,
            items: d
                .items
                .iter()
                .map(|it| ContextItem {
                    table: it.table.to_string(),
                    concept: it.concept.map(str::to_string),
                    scope_id: scope_of(it.table),
                })
                .collect(),
        })
        .collect::<Vec<_>>();

    // scene 过滤：statement 叶表仅保留该场景存在活跃行的表（一次聚合查询；
    // 跨 ns 共享叶表的归属由行级 dk_scene 表达）
    let mut statement = crate::context_meta::STATEMENT_LEAVES
        .iter()
        .map(|i| BranchOption {
            table: i.table.to_string(),
            concept: i.concept.map(str::to_string),
        })
        .collect::<Vec<_>>();
    if let Some(scene) = query.scene.as_deref() {
        let populated: Vec<String> = sqlx::query_scalar(
            r#"SELECT replace(tableoid::regclass::text, '"', '') AS leaf
               FROM isahl."zc_id_statement"
               WHERE deleted_at IS NULL
                 AND dk_scene = (SELECT id FROM isahl."zc_id_scene" WHERE code = $1 AND deleted_at IS NULL LIMIT 1)
               GROUP BY 1"#,
        )
        .bind(scene)
        .fetch_all(pool.get_ref())
        .await?;
        statement.retain(|o| populated.iter().any(|t| t == &o.table));
    }

    let terminal_leaves = TerminalLeafOptions {
        statement,
        task: crate::context_meta::TASK_LEAVES
            .iter()
            .map(|i| BranchOption {
                table: i.table.to_string(),
                concept: i.concept.map(str::to_string),
            })
            .collect(),
        event: crate::context_meta::EVENT_LEAVES
            .iter()
            .map(|i| BranchOption {
                table: i.table.to_string(),
                concept: i.concept.map(str::to_string),
            })
            .collect(),
    };

    Ok(HttpResponse::Ok().json(ApiResponse::success(ScopeOptions {
        branches,
        domains,
        terminal_leaves,
    })))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/approval-flows/scope-options",
        web::get().to(scope_options),
    );
}
