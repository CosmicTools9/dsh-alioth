//! cc 收件箱（A6 完整接线）：GET /approval-cc/mine
//!
//! 返回「抄送给我」的 cc 节点门禁记录：gate 实例（cc 节点到达时创建）→ 载体
//! timeline.recipients（结构化数组解析命中当前用户；legacy 文本仅 user: 直配支持）。
//! 幂等只读；NGAC 校验 auth（读自己抄送无需额外权限）。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context;
use common::error::AliothError as ApiError;
use common::ApiResponse;
use sqlx::PgPool;

/// 抄送我的列表
pub async fn cc_mine(pool: web::Data<PgPool>, req: HttpRequest) -> Result<HttpResponse, ApiError> {
    let user_id = context::require_auth(&req)?;
    // 当前用户可解析身份（name/username 用于 engineer 匹配）
    let me: Option<(String, String)> = sqlx::query_as::<_, (String, String)>(
        r#"SELECT COALESCE(u.username,''), COALESCE(u.name,'') FROM isahl_auth.auth_users u
           WHERE u.id = $1 AND u.is_active = TRUE"#,
    )
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    // 我所属 UA（role 匹配）
    let my_roles: Vec<String> = sqlx::query_scalar(
        r#"SELECT ua.o_name FROM isahl_auth.ngac_user_attribute ua
           JOIN isahl_auth.ngac_user_rr_attribute rel
             ON rel.fk_user_attribute = ua.id AND rel.deleted_at IS NULL
           WHERE rel.fk_user = $1 AND ua.deleted_at IS NULL"#,
    )
    .bind(user_id)
    .fetch_all(pool.get_ref())
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;

    // cc 节点 gate 实例 + 载体（recipients 物化于载体 timeline）
    let rows: Vec<(i64, String, i64, serde_json::Value, chrono::NaiveDateTime)> =
        sqlx::query_as::<_, (i64, String, i64, serde_json::Value, chrono::NaiveDateTime)>(
            r#"SELECT g.id, COALESCE(g.notice, ''), ea.id, ea.timeline->'recipients', g.created_at
             FROM isahl."zc_id_oper-gate" g
             JOIN isahl."zc_id_operation_rr_event" ge
               ON ge.ref_left = g.id AND ge.deleted_at IS NULL
             JOIN isahl."zc_id_even-approve" ea ON ea.id = ge.ref_right AND ea.deleted_at IS NULL
             JOIN isahl."zc_id_operation_rr_event" oe
               ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
             JOIN isahl."zc_id_operation" o ON o.id = oe.ref_left AND o.deleted_at IS NULL
             JOIN isahl."zc_id_cate-proc_op" c
               ON c.id = o."ck_cate-proc_op" AND c.deleted_at IS NULL
            WHERE c.code = 'cc'
              AND ea.timeline ? 'recipients'
              AND g.deleted_at IS NULL
            ORDER BY g.created_at DESC
            LIMIT 200"#,
        )
        .fetch_all(pool.get_ref())
        .await
        .map_err(|e| ApiError::Database(e.to_string()))?;

    let mut mine: Vec<serde_json::Value> = Vec::new();
    for (gate_id, notice, carrier, recipients, created_at) in rows {
        let recips = recipients.clone();
        let hit = match &recips {
            serde_json::Value::Array(items) => {
                let mut hit = false;
                for it in items {
                    let kind = it.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                    let id = it.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    if kind != "role" {
                        if let Some((un, nm)) = &me {
                            if id == un || id == nm {
                                hit = true;
                            }
                        }
                        if !hit {
                            let emp: Option<i64> = sqlx::query_scalar(
                                r#"SELECT e.fk_user FROM isahl."zc_id_subj-employee" e
                                   WHERE e.deleted_at IS NULL
                                     AND (e.id::text = $1 OR e.notice = $1 OR e.code = $1)
                                   LIMIT 1"#,
                            )
                            .bind(id)
                            .fetch_optional(pool.get_ref())
                            .await
                            .map_err(|e| ApiError::Database(e.to_string()))?
                            .flatten();
                            if emp == Some(user_id) {
                                hit = true;
                            }
                        }
                    } else if my_roles.iter().any(|r| r == id) {
                        hit = true;
                    }
                }
                hit
            }
            serde_json::Value::String(txt) => {
                let wanted = format!("user:{}", user_id);
                txt.split(',').any(|part| part.trim() == wanted)
            }
            _ => false,
        };
        if hit {
            mine.push(serde_json::json!({
                "gate_id": gate_id.to_string(),
                "node": notice,
                "carrier_id": carrier.to_string(),
                "recipients": recips,
                "created_at": created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            }));
        }
    }

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "items": mine,
        }))),
    )
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route("/approval-cc/mine", web::get().to(cc_mine));
}
