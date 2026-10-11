use actix_web::{web, HttpRequest, HttpResponse};
use sqlx::PgPool;

use crate::ngac::pip::{Pip, PostgresPip};

/// List accessible resource IDs for row-level filtering (RLS).
///
/// Returns `visible_ids: None` for admin users (meaning "all resources"),
/// or a filtered list of resource IDs for regular users.
pub async fn list_resource_access(
    req_http: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<ngac_contract::PdpListRequest>,
) -> HttpResponse {
    let req = body.into_inner();
    // tighten-pdp-decision-surface A：调用者主体必须等于决策主体
    if let Err(resp) = super::enforce_decision_subject(&req_http, req.user_id) {
        return resp;
    }

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
    // 分级（密级）上限（NGAC_SPEC §2.7）：**先于 bootstrap 全量短路**——白名单资源类型上即便处于
    // bootstrap（零 association）MUST NOT 以 `None`（全量）返回高于密级的行（与 Framework 内层门/
    // `decide_access` 的「上限先于 bootstrap」同序）。
    if !has_policies.0 {
        if let Some(resp) =
            ceiling_list_response(pool.get_ref(), req.user_id, &req.resource_type).await
        {
            return resp;
        }
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
        Ok(attrs)
            if ngac_host::attr_names_include_admin(attrs.iter().map(|a| a.o_name.as_str())) =>
        {
            // 分级（密级）上限对 admin 同样生效（NGAC_SPEC §2.7，与 prohibition 同族）：白名单资源类型
            // MUST NOT 用 `visible_ids = None`（全量不过滤）规避上限——否则 `decide` 拒绝而列表仍返回
            // 该行数据（读取旁路）。命中白名单 ⇒ 显式可见集；非白名单 ⇒ 维持 §6.2 全量语义（None）。
            // 实现单源 = `ceiling_list_response`（bootstrap 分支同源）。
            if let Some(resp) =
                ceiling_list_response(pool.get_ref(), req.user_id, &req.resource_type).await
            {
                return resp;
            }
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

/// 上限前置响应：白名单资源类型 ⇒ `Some(过滤后的 id 集)`；非白名单 ⇒ `None`（调用方维持既有全量语义）。
///
/// 用于**「全量不过滤」两条短路**（bootstrap 零 association 与 admin 豁免）之前：二者都会返回
/// `visible_ids = None`（不过滤），若先短路则白名单类型上的高密级行会被整行读回（读取旁路）。
async fn ceiling_list_response(
    pool: &PgPool,
    user_id: i64,
    resource_type: &str,
) -> Option<HttpResponse> {
    match ceiling_only_visible_ids(pool, user_id, resource_type).await {
        Ok(Some(ids)) => Some(HttpResponse::Ok().json(ngac_contract::PdpListResponse {
            permitted: true,
            reason: format!("Label ceiling: {} rows within clearance", ids.len()),
            visible_ids: Some(ids),
        })),
        Ok(None) => None,
        Err(e) => {
            // 降级（§2.7）：基础设施不可用 ⇒ 不介入 + 告警，维持既有全量语义
            log::warn!("label ceiling unavailable, skipping: {e}");
            None
        }
    }
}

/// 密级白名单资源类型上的**可见 id 集**（NGAC_SPEC §2.7）——供「全量不过滤」短路前置使用。
///
/// 返回 `Ok(None)` = 该资源类型不在白名单内 ⇒ 调用方维持 §6.2 的「admin 全量不过滤」语义；
/// 返回 `Ok(Some(ids))` = 白名单内 ⇒ 只能看到标签等级 ≤ 自身 clearance 的行（**缺 clearance
/// ⇒ rank 0 ⇒ 只见无标签行**，fail-closed）。成本：仅白名单类型才会枚举该类型行（零数据面下
/// 不发生；opt-in 语义）。
async fn ceiling_only_visible_ids(
    pool: &PgPool,
    user_id: i64,
    resource_type: &str,
) -> sqlx::Result<Option<Vec<i64>>> {
    let whitelisted: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM isahl_auth.org_policy_class c \
          WHERE c.deleted_at IS NULL AND c.state = 'active' AND c.scope IS NOT NULL \
            AND c.scope -> 'resource_types' @> to_jsonb($1::text))",
    )
    .bind(resource_type)
    .fetch_one(pool)
    .await?;
    if !whitelisted {
        return Ok(None);
    }
    let subj_rank: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(l.rank) FROM isahl_auth.ngac_user_attribute ua \
           JOIN isahl_auth.org_policy_label l \
             ON l.code = (ua.property ->> 'clearance') AND l.deleted_at IS NULL AND l.is_active \
          WHERE ua.deleted_at IS NULL \
            AND ua.id IN (SELECT ua_id FROM isahl_auth.effective_ua_ids($1))",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT oa.fk_resource FROM isahl_auth.ngac_object_attribute oa \
           LEFT JOIN isahl_auth.org_policy_label l \
             ON l.code = (oa.property ->> 'label') AND l.deleted_at IS NULL AND l.is_active \
          WHERE oa.resource_type = $1 AND oa.deleted_at IS NULL AND oa.fk_resource IS NOT NULL \
            AND (l.rank IS NULL OR l.rank <= COALESCE($2, 0)) \
          ORDER BY oa.fk_resource",
    )
    .bind(resource_type)
    .bind(subj_rank)
    .fetch_all(pool)
    .await?;
    Ok(Some(ids))
}

/// NGAC 列级授权——返回用户对某资源类型可访问的列集合（DTO 字段名）。
///
/// 语义：遍历 user_attrs × resource_type 的 collection OA（fk_resource=0）的 association，
/// 收集 `read:*`（通配 → 返回 `["*"]`）与 `read:{col}`（具体列）动作。bootstrap 阶段
/// （无策略）或 admin 全通配时返回 `["*"]`（无列级限制）。
pub async fn list_column_access(
    req_http: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<ngac_contract::PdpColumnsRequest>,
) -> HttpResponse {
    let req = body.into_inner();
    // tighten-pdp-decision-surface A：调用者主体必须等于决策主体
    if let Err(resp) = super::enforce_decision_subject(&req_http, req.user_id) {
        return resp;
    }

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
    // 分级（密级）上限（NGAC_SPEC §2.7）：白名单资源类型上 clearance < 标签 ⇒ 列级授权整体不可见
    // （否则 `decide` 拒绝该资源而列名仍外泄，构成元数据旁路）。**先于 bootstrap 全通配短路**：
    // 白名单已存在即平台非未配置态（与 `decide_access`/RLS 的「上限先于 bootstrap」同序）。
    match ngac_host::label_clearance_denies(pool.get_ref(), &req.resource_type, 0, req.user_id)
        .await
    {
        Ok(true) => {
            return HttpResponse::Ok().json(ngac_contract::PdpColumnsResponse {
                permitted: true,
                reason: "Denied by label ceiling: subject clearance below resource label"
                    .to_string(),
                columns: vec![],
            });
        }
        Ok(false) => {}
        Err(e) => log::warn!("column label ceiling unavailable, skipping: {e}"),
    }

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
        WITH user_attrs AS (
            -- 判定链 PIP 单源（直派 ∪ 认知派生 ∪ 委托派生 ∪ 祖先闭包 + 时效，闭包已在函数内展开）；
            -- 此前本处手写闭包（漏委托/认知派生），与决策面口径不一致。
            SELECT ua_id, 0 as depth FROM isahl_auth.effective_ua_ids($1)
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
