//! 图校验端点 — `POST /approval-flows/validate`
//!
//! 发布级图校验（fix-flow-designer-runtime-chain D8）：与 publish 共用
//! `validate_graph`（节点提取 + 类型白名单 + 边提取），保存前预检与发布
//! 同判据。body = 流程图 JSON（{nodes, edges} 或节点数组），合法返回
//! `{valid: true}`，非法返回 400 + 错误明细。纯只读、无 DB 访问。

use actix_web::{web, HttpResponse};
use common::error::AliothError;
use common::ApiResponse;
use serde_json::Value;

use super::publish::validate_graph;

pub async fn validate_flow(body: web::Json<Value>) -> Result<HttpResponse, AliothError> {
    let parsed = body.into_inner();
    let (_nodes, _edges) = validate_graph(&parsed)?;
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "valid": true,
        }))),
    )
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route("/approval-flows/validate", web::post().to(validate_flow));
}
