use actix_web::{web, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::AssertSqlSafe;
use sqlx::FromRow;
use sqlx::PgPool;

use crate::auth::AuthState;

// B-0 consolidate-ngac-cognition-source：认知/委托派生链唯一实现上提 common
// （NGAC_SPEC §2.2.3/§2.2.4），本模块与 /auth/me 矩阵、permissions.rs、Gateway
// resolve_user_permissions 一律消费 common 常量/函数，禁止本地副本。
use common::ngac_org::{ensure_cognition_uas, COGNITION_CTE, DELEGATED_CTE};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NgacUserAttribute {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    #[serde(with = "common::serde_zuid::seq")]
    pub ancestor_ids: Vec<i64>,
    #[serde(with = "common::serde_zuid::seq")]
    pub children_ids: Vec<i64>,
    pub property: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NgacObjectAttribute {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    #[serde(with = "common::serde_zuid::seq")]
    pub ancestor_ids: Vec<i64>,
    #[serde(with = "common::serde_zuid::seq")]
    pub children_ids: Vec<i64>,
    pub resource_type: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_resource: Option<i64>,
    pub resource_identifier: Option<String>,
    pub property: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NgacUserRrAttribute {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub created_by_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub updated_by_id: Option<i64>,
    pub o_name: Option<String>,
    #[serde(with = "common::serde_zuid")]
    pub fk_user: i64,
    #[serde(with = "common::serde_zuid")]
    pub fk_user_attribute: i64,
    pub assigned_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub conditions: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct AssignUserAttributeRequest {
    #[serde(with = "common::serde_zuid")]
    pub fk_user_attribute: i64,
    pub conditions: Option<serde_json::Value>,
    pub expires_at: Option<DateTime<Utc>>,
}

// Type alias for backward compatibility
pub type NgacUserAttributeAssignment = NgacUserRrAttribute;

#[derive(Debug, Deserialize)]
pub struct CreateNgacUserAttributeRequest {
    pub o_name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    #[serde(with = "common::serde_zuid::seq")]
    pub ancestor_ids: Vec<i64>,
    pub property: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct AttributeResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[async_trait::async_trait]
pub trait Pip: Send + Sync {
    async fn get_user_attributes(&self, fk_user: i64) -> sqlx::Result<Vec<NgacUserAttribute>>;
    async fn get_object_attributes(
        &self,
        resource_type: &str,
        fk_resource: i64,
    ) -> sqlx::Result<Vec<NgacObjectAttribute>>;
    async fn assign_user_attribute(
        &self,
        fk_user: i64,
        fk_user_attribute: i64,
        conditions: Option<serde_json::Value>,
    ) -> sqlx::Result<i64>;
    async fn remove_user_attribute(&self, fk_user: i64, fk_user_attribute: i64)
        -> sqlx::Result<()>;
    async fn get_all_user_attributes_with_inheritance(
        &self,
        fk_user: i64,
    ) -> sqlx::Result<Vec<NgacUserAttribute>>;
    async fn get_all_object_attributes_with_inheritance(
        &self,
        resource_type: &str,
        fk_resource: i64,
    ) -> sqlx::Result<Vec<NgacObjectAttribute>>;
    /// Get resource IDs accessible by a user for a given resource type and action.
    async fn get_accessible_resource_ids(
        &self,
        fk_user: i64,
        resource_type: &str,
        action: &str,
    ) -> sqlx::Result<Vec<i64>>;
}

#[derive(Clone)]
pub struct PostgresPip {
    pool: PgPool,
}

impl PostgresPip {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    async fn get_inherited_user_attributes(
        &self,
        fk_user: i64,
    ) -> sqlx::Result<Vec<NgacUserAttribute>> {
        // 认知派生 UA 先物化（幂等），再参与有效 UA 集解析（design D1/D2）
        ensure_cognition_uas(&self.pool, fk_user).await;
        let sql = format!(
            r#"
            WITH RECURSIVE {COGNITION_CTE},
            {DELEGATED_CTE},
            user_attr_inheritance AS (
                SELECT ua.id, ua.o_name, ua.fk_policy_class, ua.ancestor_ids, ua.children_ids, ua.property, ua.created_at
                FROM isahl_auth.ngac_user_attribute ua
                INNER JOIN isahl_auth.ngac_user_rr_attribute ura ON ua.id = ura.fk_user_attribute
                WHERE ura.fk_user = $1
                  AND ura.deleted_at IS NULL
                  AND (ura.expires_at IS NULL OR ura.expires_at > NOW())
                  AND ua.deleted_at IS NULL

                UNION ALL

                -- 认知派生 UA（岗位 position:{{code}} / 视角标签 view:{{code}}）
                SELECT ua.id, ua.o_name, ua.fk_policy_class, ua.ancestor_ids, ua.children_ids, ua.property, ua.created_at
                FROM isahl_auth.ngac_user_attribute ua
                INNER JOIN cognition_ua_names cn ON cn.o_name = ua.o_name
                WHERE ua.deleted_at IS NULL

                UNION ALL

                -- 委托派生 UA（add-ngac-delegation D2：active + 时间窗内）
                SELECT ua.id, ua.o_name, ua.fk_policy_class, ua.ancestor_ids, ua.children_ids, ua.property, ua.created_at
                FROM isahl_auth.ngac_user_attribute ua
                INNER JOIN delegated_ua du ON du.id = ua.id
                WHERE ua.deleted_at IS NULL

                UNION ALL

                SELECT parent.id, parent.o_name, parent.fk_policy_class, parent.ancestor_ids, parent.children_ids, parent.property, parent.created_at
                FROM isahl_auth.ngac_user_attribute parent
                INNER JOIN user_attr_inheritance child ON parent.id = ANY(child.ancestor_ids)
                WHERE parent.deleted_at IS NULL
            )
            SELECT DISTINCT id, o_name, fk_policy_class, ancestor_ids, children_ids, property, created_at
            FROM user_attr_inheritance
            ORDER BY id
            "#,
            COGNITION_CTE = COGNITION_CTE,
            DELEGATED_CTE = DELEGATED_CTE
        );
        let attributes = sqlx::query_as::<_, NgacUserAttribute>(AssertSqlSafe(sql.as_str()))
            .bind(fk_user)
            .fetch_all(&self.pool)
            .await?;

        Ok(attributes)
    }

    async fn get_inherited_object_attributes(
        &self,
        resource_type: &str,
        fk_resource: i64,
    ) -> sqlx::Result<Vec<NgacObjectAttribute>> {
        let attributes = sqlx::query_as::<_, NgacObjectAttribute>(
            r#"
            WITH RECURSIVE object_attr_inheritance AS (
                SELECT id, o_name, fk_policy_class, ancestor_ids, children_ids, resource_type, fk_resource, resource_identifier, property, created_at
                FROM isahl_auth.ngac_object_attribute
                WHERE resource_type = $1 AND fk_resource = $2
                  AND deleted_at IS NULL

                UNION ALL

                SELECT ua.id, ua.o_name, ua.fk_policy_class, ua.ancestor_ids, ua.children_ids, ua.resource_type, ua.fk_resource, ua.resource_identifier, ua.property, ua.created_at
                FROM isahl_auth.ngac_object_attribute ua
                INNER JOIN object_attr_inheritance oai ON ua.id = ANY(oai.ancestor_ids)
                WHERE ua.deleted_at IS NULL
            )
            SELECT DISTINCT id, o_name, fk_policy_class, ancestor_ids, children_ids, resource_type, fk_resource, resource_identifier, property, created_at
            FROM object_attr_inheritance
            ORDER BY id
            "#
        )
        .bind(resource_type)
        .bind(fk_resource)
        .fetch_all(&self.pool)
        .await?;

        Ok(attributes)
    }
}

#[async_trait::async_trait]
impl Pip for PostgresPip {
    async fn get_user_attributes(&self, fk_user: i64) -> sqlx::Result<Vec<NgacUserAttribute>> {
        sqlx::query_as::<_, NgacUserAttribute>(
            r#"
            SELECT ua.id, ua.o_name, ua.fk_policy_class, ua.ancestor_ids, ua.children_ids, ua.property, ua.created_at
            FROM isahl_auth.ngac_user_attribute ua
            INNER JOIN isahl_auth.ngac_user_rr_attribute ura ON ua.id = ura.fk_user_attribute
            WHERE ura.fk_user = $1
              AND ura.deleted_at IS NULL
              AND (ura.expires_at IS NULL OR ura.expires_at > NOW())
            "#
        )
        .bind(fk_user)
        .fetch_all(&self.pool)
        .await
    }

    async fn get_object_attributes(
        &self,
        resource_type: &str,
        fk_resource: i64,
    ) -> sqlx::Result<Vec<NgacObjectAttribute>> {
        sqlx::query_as::<_, NgacObjectAttribute>(
            "SELECT id, o_name, fk_policy_class, ancestor_ids, children_ids, resource_type, fk_resource, resource_identifier, property, created_at FROM isahl_auth.ngac_object_attribute WHERE resource_type = $1 AND fk_resource = $2 AND deleted_at IS NULL"
        )
        .bind(resource_type)
        .bind(fk_resource)
        .fetch_all(&self.pool)
        .await
    }

    async fn assign_user_attribute(
        &self,
        fk_user: i64,
        fk_user_attribute: i64,
        conditions: Option<serde_json::Value>,
    ) -> sqlx::Result<i64> {
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, conditions) VALUES ($1, $2, $3) RETURNING id"
        )
        .bind(fk_user)
        .bind(fk_user_attribute)
        .bind(&conditions)
        .fetch_one(&self.pool)
        .await
    }

    async fn remove_user_attribute(
        &self,
        fk_user: i64,
        fk_user_attribute: i64,
    ) -> sqlx::Result<()> {
        sqlx::query(
            "DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1 AND fk_user_attribute = $2"
        )
        .bind(fk_user)
        .bind(fk_user_attribute)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_all_user_attributes_with_inheritance(
        &self,
        fk_user: i64,
    ) -> sqlx::Result<Vec<NgacUserAttribute>> {
        self.get_inherited_user_attributes(fk_user).await
    }

    async fn get_all_object_attributes_with_inheritance(
        &self,
        resource_type: &str,
        fk_resource: i64,
    ) -> sqlx::Result<Vec<NgacObjectAttribute>> {
        self.get_inherited_object_attributes(resource_type, fk_resource)
            .await
    }

    /// RLS 可见行解析（deny-overrides 同源化，fix-ngac-decision-consistency D2）：
    ///
    /// 两段式——SQL 收敛候选超集（用户有效 UA 闭包 + 闭包内任一 association 触及的
    /// 行 OA），Rust 侧对每行跑 `Pdp::evaluate_pair_in`（UA 闭包 × 行 OA 闭包，
    /// deny-overrides + conditions fail-closed），与单条 decide 严格同一语义；
    /// prohibition 命中的行 MUST NOT 出现在结果。禁止第二份授权逻辑。
    async fn get_accessible_resource_ids(
        &self,
        fk_user: i64,
        resource_type: &str,
        action: &str,
    ) -> sqlx::Result<Vec<i64>> {
        // 认知派生 UA 先物化（幂等），再参与可见 ID 解析（design D1/D2/D3）
        ensure_cognition_uas(&self.pool, fk_user).await;

        // 用户有效 UA 闭包（直接指派 ∪ 认知派生 ∪ 委托派生 ∪ 祖先闭包）：id + 名
        let ua_sql = format!(
            r#"
            WITH RECURSIVE {COGNITION_CTE},
            {DELEGATED_CTE},
            user_attrs AS (
                SELECT fk_user_attribute AS ua_id, 0 AS depth
                FROM isahl_auth.ngac_user_rr_attribute
                WHERE fk_user = $1
                  AND deleted_at IS NULL
                  AND (expires_at IS NULL OR expires_at > NOW())
                UNION ALL
                SELECT ua.id, 0
                FROM isahl_auth.ngac_user_attribute ua
                INNER JOIN cognition_ua_names cn ON cn.o_name = ua.o_name
                WHERE ua.deleted_at IS NULL
                UNION ALL
                SELECT du.id, 0
                FROM delegated_ua du
                UNION ALL
                SELECT unnest(ua.ancestor_ids)::BIGINT, depth + 1
                FROM isahl_auth.ngac_user_attribute ua
                INNER JOIN user_attrs AS ua_cte ON ua.id = ua_cte.ua_id
                WHERE depth < 10
            )
            SELECT DISTINCT ua.id, ua.o_name
            FROM user_attrs c
            JOIN isahl_auth.ngac_user_attribute ua ON ua.id = c.ua_id
            WHERE ua.deleted_at IS NULL
            "#,
            COGNITION_CTE = COGNITION_CTE,
            DELEGATED_CTE = DELEGATED_CTE
        );
        let user_attrs: Vec<(i64, String)> = sqlx::query_as(AssertSqlSafe(ua_sql.as_str()))
            .bind(fk_user)
            .fetch_all(&self.pool)
            .await?;

        // 候选行 OA 超集：行 OA 闭包（自身 ∪ ancestor_ids）内存在用户 UA 闭包
        // 的任一 association（rights/conditions 不限——Rust 侧精确求值收敛）
        let cand_sql = format!(
            r#"
            WITH RECURSIVE {COGNITION_CTE},
            {DELEGATED_CTE},
            user_attrs AS (
                SELECT fk_user_attribute AS ua_id, 0 AS depth
                FROM isahl_auth.ngac_user_rr_attribute
                WHERE fk_user = $1
                  AND deleted_at IS NULL
                  AND (expires_at IS NULL OR expires_at > NOW())
                UNION ALL
                SELECT ua.id, 0
                FROM isahl_auth.ngac_user_attribute ua
                INNER JOIN cognition_ua_names cn ON cn.o_name = ua.o_name
                WHERE ua.deleted_at IS NULL
                UNION ALL
                SELECT du.id, 0
                FROM delegated_ua du
                UNION ALL
                SELECT unnest(ua.ancestor_ids)::BIGINT, depth + 1
                FROM isahl_auth.ngac_user_attribute ua
                INNER JOIN user_attrs AS ua_cte ON ua.id = ua_cte.ua_id
                WHERE depth < 10
            )
            SELECT DISTINCT oa.id, oa.fk_resource
            FROM isahl_auth.ngac_object_attribute oa
            WHERE oa.resource_type = $2
              AND oa.deleted_at IS NULL
              AND EXISTS (
                  SELECT 1
                  FROM isahl_auth.ngac_association a
                  JOIN user_attrs c ON a.fk_user_attribute = c.ua_id
                  WHERE a.deleted_at IS NULL
                    AND a.fk_object_attribute = ANY(oa.id || COALESCE(oa.ancestor_ids, '{{}}'::bigint[]))
              )
            "#,
            COGNITION_CTE = COGNITION_CTE,
            DELEGATED_CTE = DELEGATED_CTE
        );
        let candidates: Vec<(i64, i64)> = sqlx::query_as(AssertSqlSafe(cand_sql.as_str()))
            .bind(fk_user)
            .bind(resource_type)
            .fetch_all(&self.pool)
            .await?;

        // 该 resource_type 全部 OA（闭包解析与 conditions 的 object_attr_in 求值用）
        let oas: Vec<(i64, String, Vec<i64>)> = sqlx::query_as(
            r#"
            SELECT id, o_name, COALESCE(ancestor_ids, '{}'::bigint[])
            FROM isahl_auth.ngac_object_attribute
            WHERE resource_type = $1 AND deleted_at IS NULL
            "#,
        )
        .bind(resource_type)
        .fetch_all(&self.pool)
        .await?;

        // 策略图确保加载（版本信号驱动；失败 fail-closed 上抛）
        let pdp = crate::ngac::pdp::Pdp::global();
        pdp.ensure_policy_loaded(self)
            .await
            .map_err(|e| sqlx::Error::Protocol(format!("policy load failed: {e}")))?;
        let pg = pdp.policy_graph();

        let oa_index: std::collections::HashMap<i64, (String, Vec<i64>)> = oas
            .into_iter()
            .map(|(id, name, ancestors)| (id, (name, ancestors)))
            .collect();
        let ua_ids: Vec<i64> = user_attrs.iter().map(|(id, _)| *id).collect();
        let ua_names: Vec<String> = user_attrs.iter().map(|(_, n)| n.clone()).collect();
        let now = Utc::now();

        let mut visible: Vec<i64> = Vec::new();
        for (oa_id, fk_resource) in candidates {
            // 行 OA 祖先闭包（含自身；ancestor_ids 为直接父边，迭代求可达集）
            let mut closure: Vec<i64> = Vec::new();
            let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
            let mut stack = vec![oa_id];
            while let Some(id) = stack.pop() {
                if !seen.insert(id) {
                    continue;
                }
                closure.push(id);
                if let Some((_, ancestors)) = oa_index.get(&id) {
                    stack.extend(ancestors.iter().copied());
                }
            }
            let ctx = crate::ngac::pdp::ConditionContext {
                now,
                user_ua_names: ua_names.clone(),
                oa_closure_names: closure
                    .iter()
                    .filter_map(|id| oa_index.get(id).map(|(n, _)| n.clone()))
                    .collect(),
            };
            // deny-overrides：任一对 Deny 即排除（早停）；Permit 记录后继续。
            let mut saw_permit = false;
            let mut denied = false;
            'row: for &ua in &ua_ids {
                for &oa in &closure {
                    match pdp.evaluate_pair_in(&pg, ua, oa, action, &ctx).0 {
                        crate::ngac::pdp::Decision::Deny => {
                            denied = true;
                            break 'row;
                        }
                        crate::ngac::pdp::Decision::Permit => saw_permit = true,
                        crate::ngac::pdp::Decision::NotApplicable => {}
                    }
                }
            }
            if !denied && saw_permit {
                visible.push(fk_resource);
            }
        }
        visible.sort_unstable();
        visible.dedup();
        Ok(visible)
    }
}

pub type NgacPip = PostgresPip;

/// 指派审计上下文（收拢 actor/session/ip，add-ngac-access-request D3）。
#[derive(Debug, Clone, Default)]
pub struct AuditContext {
    pub actor: i64,
    pub session_id: Option<String>,
    pub ip_address: Option<String>,
}

/// UA 指派 + 同事务审计（add-ngac-access-request D3 提取）——唯一实现，
/// `set_user_attribute` 与 access-request approve 共用；禁止第二份实现。
/// 语义与 `set_user_attribute` 完全一致：INSERT（含 expires_at）→ 行镜像 →
/// 审计 insert/user_assignment；失败由调用方事务整体回滚。
pub async fn assign_ua_with_audit_tx(
    tx: &mut sqlx::PgConnection,
    fk_user: i64,
    fk_user_attribute: i64,
    conditions: Option<serde_json::Value>,
    expires_at: Option<DateTime<Utc>>,
    audit: &AuditContext,
) -> sqlx::Result<i64> {
    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, conditions, expires_at) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(fk_user)
    .bind(fk_user_attribute)
    .bind(&conditions)
    .bind(expires_at)
    .fetch_one(&mut *tx)
    .await?;
    let new_values =
        crate::ngac::audit_writer::user_rr_mirror_tx(&mut *tx, fk_user, fk_user_attribute).await?;
    crate::ngac::audit_writer::write_audit_tx(
        &mut *tx,
        &crate::ngac::audit_writer::AuditRecord {
            action: "insert",
            entity_type: "user_assignment",
            entity_id: id,
            old_values: None,
            new_values,
            actor: audit.actor,
            session_id: audit.session_id.clone(),
            ip_address: audit.ip_address.clone(),
        },
    )
    .await?;
    Ok(id)
}

pub async fn get_user_attributes(pool: web::Data<PgPool>, path: web::Path<i64>) -> HttpResponse {
    let fk_user = path.into_inner();

    let pip = PostgresPip::new(pool.get_ref().clone());

    match pip.get_user_attributes(fk_user).await {
        Ok(attributes) => HttpResponse::Ok().json(attributes),
        Err(e) => {
            log::error!("Failed to get user attributes: {}", e);
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Failed to get user attributes: {}", e),
            })
        }
    }
}

pub async fn set_user_attribute(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    path: web::Path<i64>,
    body: web::Json<AssignUserAttributeRequest>,
) -> HttpResponse {
    let fk_user = path.into_inner();
    let assign_req = body.into_inner();

    // 审计 actor/session 取自 JWT（外层 RequireAuth + NgacPep 已校验；
    // 本 handler 不强制 admin UA——sso_admin OA 决策为边界）
    let claims =
        match crate::auth::jwt::validate_access_token(&req, &state.verification_keys()).await {
            Ok(c) => c,
            Err(e) => {
                log::error!("set_user_attribute: token validation failed: {}", e);
                return HttpResponse::Unauthorized().json(ErrorResponse {
                    error: "Invalid or missing authentication token".to_string(),
                });
            }
        };
    let actor: i64 = claims.sub.parse().unwrap_or(0);
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };
    let ip = crate::ngac::audit_writer::client_ip(&req);

    // 同事务审计（change add-ngac-audit-trail-view D1/W-2）
    let mut tx = match pool.get_ref().begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("set_user_attribute tx begin error: {}", e);
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Failed to assign user attribute: {}", e),
            });
        }
    };
    let result: sqlx::Result<i64> = async {
        let id = assign_ua_with_audit_tx(
            &mut tx,
            fk_user,
            assign_req.fk_user_attribute,
            assign_req.conditions,
            assign_req.expires_at,
            &AuditContext {
                actor,
                session_id,
                ip_address: ip,
            },
        )
        .await?;
        tx.commit().await?;
        sqlx::Result::Ok(id)
    }
    .await;

    match result {
        Ok(id) => HttpResponse::Created().json(AttributeResponse {
            id,
            message: "User attribute assigned successfully".to_string(),
        }),
        Err(e) => {
            log::error!("Failed to assign user attribute: {}", e);
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Failed to assign user attribute: {}", e),
            })
        }
    }
}

pub async fn get_object_attributes(
    pool: web::Data<PgPool>,
    path: web::Path<(String, i64)>,
) -> HttpResponse {
    let (resource_type, fk_resource) = path.into_inner();

    let pip = PostgresPip::new(pool.get_ref().clone());

    match pip.get_object_attributes(&resource_type, fk_resource).await {
        Ok(attributes) => HttpResponse::Ok().json(attributes),
        Err(e) => {
            log::error!("Failed to get object attributes: {}", e);
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Failed to get object attributes: {}", e),
            })
        }
    }
}

/// 注册 PIP 管理面路由（挂载于 `/api/admin/ngac/pip`，见 lib.rs）。
///
/// 安全：PIP 写端点（用户属性指派）MUST 仅经 `/api/admin/*` 管理面访问——
/// 外层 `RequireAuth`（JWT）+ `NgacPep`（sso_admin OA 决策）双重保护。
/// 不再挂载于 `/api/ngac` noauth 前缀（SECURITY_SPEC §3.4 豁免最小化）。
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/users/{user_id}/attributes",
        web::get().to(get_user_attributes),
    )
    .route(
        "/users/{user_id}/attributes",
        web::post().to(set_user_attribute),
    )
    .route(
        "/objects/{resource_type}/{resource_id}/attributes",
        web::get().to(get_object_attributes),
    );
}
