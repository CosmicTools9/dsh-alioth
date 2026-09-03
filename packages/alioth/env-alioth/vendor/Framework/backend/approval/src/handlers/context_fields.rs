//! 上下文字段端点 — `GET /approval-flows/context-fields?table=<leaf>`
//!
//! 流程设计器节点行为编制（判断条件、执行分支）的字段来源：
//! 流程创建时已绑定输入范畴（fk_context → task/event/approve 三域叶表），
//! 节点是 operation，不存在手工增删字段——本端点按绑定叶表返回自动识别的
//! 有意义业务字段。
//!
//! 运行时可见性约束（2026-08-27 裁决）：isahl_meta 在 app 运行时不可见——
//! 字段元数据编译期绑定于 `crate::context_meta`（AUTO-GENERATED，
//! `bun scripts/generate-context-fields.ts` 从 DB 元数据重建，模型升级后重跑），
//! 运行时零 catalog 查询。
//!
//! table 参数 MUST 命中三域叶表白名单（即 context_meta 在册叶表），非法返回 400。
//! 纯只读、无 DB 访问。

use actix_web::{web, HttpResponse};
use common::error::AliothError;
use common::ApiResponse;
use serde::Deserialize;

use crate::context_meta::{context_fields_of, ContextFieldMeta};

#[derive(Debug, Deserialize)]
pub struct ContextFieldsQuery {
    pub table: String,
}

pub async fn context_fields(
    query: web::Query<ContextFieldsQuery>,
) -> Result<HttpResponse, AliothError> {
    let table = query.table.trim();
    if table.is_empty() {
        return Err(AliothError::BadRequest("table 参数必填".to_string()));
    }

    let fields: &'static [ContextFieldMeta] = context_fields_of(table).ok_or_else(|| {
        AliothError::BadRequest(format!(
            "table 不在上下文三域（task/event/approve）叶表白名单内: {table}"
        ))
    })?;

    Ok(HttpResponse::Ok().json(ApiResponse::success(fields)))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/approval-flows/context-fields",
        web::get().to(context_fields),
    );
}
