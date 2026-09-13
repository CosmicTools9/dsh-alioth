//! 图校验端点 — `POST /approval-flows/validate`
//!
//! 发布级图校验（fix-flow-designer-runtime-chain D8 + add-approval-flow-static-scan）：
//! 与 publish 共用 `validate_graph`（节点提取 + 类型白名单 + 边提取）与静态缺陷
//! 扫描器 `scan_graph`（分级发现），保存前预检与发布同判据。body = 流程图 JSON
//! （{nodes, edges} 或节点数组）。
//!
//! 分级语义：扫描 errors（结构性硬错）→ HTTP 400（ErrorResponse，
//! details.errors = 全量阻断明细）；仅 warnings（疑似形态）→ 200
//! `{valid: true, warnings: [...]}`。纯只读（④ 空审批人 DB 实查经只读连接）。

use actix_web::{web, HttpResponse};
use common::error::{AliothError, ErrorResponse};
use common::ApiResponse;
use serde_json::Value;

use super::publish::validate_graph;

pub async fn validate_flow(
    pool: web::Data<sqlx::PgPool>,
    body: web::Json<Value>,
) -> Result<HttpResponse, AliothError> {
    let parsed = body.into_inner();
    let (nodes, edges) = validate_graph(&parsed)?;

    // ④ 需要 DB（岗位/员工解析）——acquire 失败降级为结构检查（None → 仅配置
    // 存在性），不阻断 validate 主路径；结构检查不依赖 DB。
    let mut conn = match pool.acquire().await {
        Ok(c) => Some(c),
        Err(e) => {
            common::telemetry::warn!(
                "approval validate: acquire db conn failed — {} （空审批人实查降级）",
                e
            );
            None
        }
    };
    let report = crate::scan::scan_graph(nodes, edges, conn.as_deref_mut()).await;

    if !report.errors.is_empty() {
        let first = &report.errors[0];
        let total = report.errors.len();
        let message = if total > 1 {
            format!("{}（共 {} 项静态缺陷，发布将被阻断）", first.message, total)
        } else {
            first.message.clone()
        };
        return Ok(HttpResponse::BadRequest().json(ErrorResponse {
            code: "VALIDATION_ERROR".to_string(),
            message,
            details: Some(serde_json::json!({ "errors": report.errors })),
        }));
    }

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "valid": true,
            "warnings": report.warnings,
        }))),
    )
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route("/approval-flows/validate", web::post().to(validate_flow));
}
