use actix_web::{web, HttpResponse};
use sqlx::PgPool;

use crate::ngac::pip::{Pip, PostgresPip};

/// List accessible resource IDs for row-level filtering (RLS).
///
/// Returns `visible_ids: None` for admin users (meaning "all resources"),
/// or a filtered list of resource IDs for regular users.
pub async fn list_resource_access(
    pool: web::Data<PgPool>,
    body: web::Json<ngac_contract::PdpListRequest>,
) -> HttpResponse {
    let req = body.into_inner();

    // Bootstrap guard: check if any associations exist
    let has_policies: (bool,) = match sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.ngac_association WHERE deleted_at IS NULL)",
    )
    .fetch_one(pool.get_ref())
    .await
    {
        Ok(r) => r,
        Err(e) => {
            log::error!("Failed to check bootstrap status: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "permitted": false,
                "reason": format!("Database error: {}", e)
            }));
        }
    };

    // Bootstrap phase: no policies configured, return all resources
    if !has_policies.0 {
        return HttpResponse::Ok().json(ngac_contract::PdpListResponse {
            permitted: true,
            reason: "Bootstrap phase — no policies configured, all resources visible".to_string(),
            visible_ids: None,
        });
    }
    // Admin 豁免：拥有 admin user_attribute（含继承）的用户看到全部资源。
    // 与 NGAC_SPEC §6.2「visible_ids = None → admin 用户，不过滤」一致，
    // 避免 admin 被 association 过滤导致列表端点 403（WZ logi-tasks 等页面加载失败根因）。
    let pip = PostgresPip::new(pool.get_ref().clone());
    match pip
        .get_all_user_attributes_with_inheritance(req.user_id)
        .await
    {
        Ok(attrs) if attrs.iter().any(|a| a.o_name == "admin") => {
            return HttpResponse::Ok().json(ngac_contract::PdpListResponse {
                permitted: true,
                reason: "Admin user — all resources visible".to_string(),
                visible_ids: None,
            });
        }
        Ok(_) => {}
        Err(e) => {
            log::error!("Failed to get user attributes for admin check: {}", e);
            // 不因 admin 检查失败而放行——继续走 association 判定（fail-closed）
        }
    }

    // Get visible resource IDs via PIP
    match pip
        .get_accessible_resource_ids(req.user_id, &req.resource_type, &req.action)
        .await
    {
        Ok(ids) => {
            if ids.is_empty() {
                HttpResponse::Ok().json(ngac_contract::PdpListResponse {
                    permitted: false,
                    reason: "No accessible resources found".to_string(),
                    visible_ids: Some(vec![]),
                })
            } else {
                HttpResponse::Ok().json(ngac_contract::PdpListResponse {
                    permitted: true,
                    reason: format!("{} accessible resources", ids.len()),
                    visible_ids: Some(ids),
                })
            }
        }
        Err(e) => {
            log::error!("Failed to get accessible resource IDs: {}", e);
            HttpResponse::InternalServerError().json(ngac_contract::PdpListResponse {
                permitted: false,
                reason: format!("Database error: {}", e),
                visible_ids: None,
            })
        }
    }
}

/// NGAC 列级授权——返回用户对某资源类型可访问的列集合（DTO 字段名）。
///
/// 语义：遍历 user_attrs × resource_type 的 collection OA（fk_resource=0）的 association，
/// 收集 `read:*`（通配 → 返回 `["*"]`）与 `read:{col}`（具体列）动作。bootstrap 阶段
/// （无策略）或 admin 全通配时返回 `["*"]`（无列级限制）。
pub async fn list_column_access(
    pool: web::Data<PgPool>,
    body: web::Json<ngac_contract::PdpColumnsRequest>,
) -> HttpResponse {
    let req = body.into_inner();

    // Bootstrap guard: 无策略时放行（无列级限制）
    let has_policies: (bool,) = match sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM isahl_auth.ngac_association WHERE deleted_at IS NULL)",
    )
    .fetch_one(pool.get_ref())
    .await
    {
        Ok(r) => r,
        Err(e) => {
            log::error!("Failed to check bootstrap status: {}", e);
            return HttpResponse::InternalServerError().json(ngac_contract::PdpColumnsResponse {
                permitted: false,
                reason: format!("Database error: {}", e),
                columns: vec![],
            });
        }
    };
    if !has_policies.0 {
        return HttpResponse::Ok().json(ngac_contract::PdpColumnsResponse {
            permitted: true,
            reason: "Bootstrap phase — no policies, all columns visible".to_string(),
            columns: vec!["*".to_string()],
        });
    }

    // 收集授权列：user 的有效 UA 经 association → collection OA（fk_resource=0）→ read:* / read:{col}
    let cols: Vec<String> = match sqlx::query_scalar(
        r#"
        WITH RECURSIVE user_attrs AS (
            SELECT fk_user_attribute AS ua_id, 0 AS depth
            FROM isahl_auth.ngac_user_rr_attribute
            WHERE fk_user = $1 AND deleted_at IS NULL AND (expires_at IS NULL OR expires_at > NOW())
            UNION ALL
            SELECT unnest(ua.ancestor_ids)::BIGINT AS ua_id, depth + 1
            FROM isahl_auth.ngac_user_attribute ua
            INNER JOIN user_attrs AS c ON ua.id = c.ua_id
            WHERE c.depth < 10 AND ua.deleted_at IS NULL
        )
        SELECT DISTINCT ar.o_name
        FROM isahl_auth.ngac_association a
        INNER JOIN user_attrs AS ua ON a.fk_user_attribute = ua.ua_id
        INNER JOIN isahl_auth.ngac_object_attribute oa ON a.fk_object_attribute = oa.id
        INNER JOIN isahl_auth.ngac_access_right ar ON ar.id = ANY(a.ak_access_rights)
        WHERE oa.resource_type = $2 AND oa.fk_resource = 0 AND oa.deleted_at IS NULL
          AND a.deleted_at IS NULL
          AND (ar.o_name = 'read:*' OR ar.o_name LIKE 'read:%')
        "#,
    )
    .bind(req.user_id)
    .bind(&req.resource_type)
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(cols) => cols,
        Err(e) => {
            log::error!("Failed to get authorized columns: {}", e);
            return HttpResponse::InternalServerError().json(ngac_contract::PdpColumnsResponse {
                permitted: false,
                reason: format!("Database error: {}", e),
                columns: vec![],
            });
        }
    };

    // read:* 通配 → 全部列（无列级限制）
    if cols.iter().any(|c| c == "read:*") {
        return HttpResponse::Ok().json(ngac_contract::PdpColumnsResponse {
            permitted: true,
            reason: "Wildcard column access granted".to_string(),
            columns: vec!["*".to_string()],
        });
    }

    // read:{col} → 提取列名
    let columns: Vec<String> = cols
        .iter()
        .filter_map(|c| c.strip_prefix("read:"))
        .map(|c| c.to_string())
        .collect();

    HttpResponse::Ok().json(ngac_contract::PdpColumnsResponse {
        permitted: true,
        reason: format!("{} authorized columns", columns.len()),
        columns,
    })
}
