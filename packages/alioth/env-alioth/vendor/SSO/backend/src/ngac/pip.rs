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
    /// jsonb NULLABLE（DB 真值）——按 Option 解码（NULL ⇒ None，勿拒整行）。
    #[serde(default)]
    pub property: Option<serde_json::Value>,
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
    /// jsonb NULLABLE（DB 真值）——按 Option 解码（NULL ⇒ None，勿拒整行）。
    #[serde(default)]
    pub property: Option<serde_json::Value>,
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

                -- 通配对象属性（resource_type = '*'）：对全部资源类型生效。与 Framework
                -- `permissions::require_resource_access` 的 resource_attrs 同义（M16 判定链对齐）。
                SELECT id, o_name, fk_policy_class, ancestor_ids, children_ids, resource_type, fk_resource, resource_identifier, property, created_at
                FROM isahl_auth.ngac_object_attribute
                WHERE resource_type = '*' AND deleted_at IS NULL

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

    /// 资源受限有效 UA 解析（NGAC_SPEC §2.2 属权派生 + §2.5 同源义务）。
    ///
    /// 语义 = [`Self::get_inherited_user_attributes`]（直派 ∪ 认知派生 ∪ 委托派生 ∪ 祖先闭包）
    /// **∪ 属权派生**——属权按 `(fk_user, resource_type, fk_resource)` 从
    /// `ngac_host::ownership_cte` 取四路属性 id（owner / benefit / permit / access），
    /// 并展开其祖先闭包（与全局闭包同口径）。
    ///
    /// 为什么必须**资源受限**：属权 UA 的取值随资源行变化（`r.id = $3`），不属于用户全局
    /// 有效 UA 集 ⇒ MUST NOT 并入 `effective_ua_ids`（全局函数）或
    /// [`Self::get_inherited_user_attributes`]；只能由**有资源上下文**的判定点（decide /
    /// explain / list 候选超集 / review / impact）叠加。SQL 单源 = `ngac_org::ownership_cte`，
    /// 消费方 MUST NOT 复制。
    pub async fn get_resource_scoped_user_attributes(
        &self,
        fk_user: i64,
        resource_type: &str,
        fk_resource: i64,
    ) -> sqlx::Result<Vec<NgacUserAttribute>> {
        // 认知派生 UA 先物化（幂等），与全局入口同序
        ensure_cognition_uas(&self.pool, fk_user).await;
        let mut attrs = self.get_inherited_user_attributes(fk_user).await?;
        let sql = format!(
            r#"
            WITH RECURSIVE {ownership},
            ownership_tree AS (
                SELECT ua.id, ua.o_name, ua.fk_policy_class, ua.ancestor_ids, ua.children_ids, ua.property, ua.created_at
                FROM isahl_auth.ngac_user_attribute ua
                WHERE ua.deleted_at IS NULL
                  AND ua.id IN (SELECT ua_id FROM ownership_ua)

                UNION ALL

                SELECT parent.id, parent.o_name, parent.fk_policy_class, parent.ancestor_ids, parent.children_ids, parent.property, parent.created_at
                FROM isahl_auth.ngac_user_attribute parent
                INNER JOIN ownership_tree child ON parent.id = ANY(child.ancestor_ids)
                WHERE parent.deleted_at IS NULL
            )
            SELECT DISTINCT id, o_name, fk_policy_class, ancestor_ids, children_ids, property, created_at
            FROM ownership_tree
            ORDER BY id
            "#,
            ownership = ngac_host::ownership_cte()
        );
        let extra: Vec<NgacUserAttribute> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(fk_user)
            .bind(resource_type)
            .bind(fk_resource)
            .fetch_all(&self.pool)
            .await?;
        let mut seen: std::collections::HashSet<i64> = attrs.iter().map(|a| a.id).collect();
        for a in extra {
            if seen.insert(a.id) {
                attrs.push(a);
            }
        }
        Ok(attrs)
    }

    /// 用户有效 UA 名集（wire-obligations-into-pep-audit 响应义务求值上下文）：
    /// 委托同 impl 私有 `get_inherited_user_attributes`（判定链同源）——pub 薄转发，
    /// 非第二实现。
    pub async fn inherited_ua_names(&self, fk_user: i64) -> sqlx::Result<Vec<String>> {
        Ok(self
            .get_inherited_user_attributes(fk_user)
            .await?
            .into_iter()
            .map(|ua| ua.o_name)
            .collect())
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
        // 属权派生（资源受限）：把 `ownership_cte()` 的 $2/$3 绑定到外层行表达式后嵌入 LATERAL
        // 子查询（单源用法二，见 `ngac_host::ownership_cte` 文档；禁复制 SQL）。
        // LATERAL 结构 = `WITH {ownership_cte()} SELECT (ua_ids 子查询), (ua_names 子查询),
        // ({CREATOR_DERIVED_EXISTS_OA}) AS creator_ua`——creator 判定与 own_res 同层求值
        // （视图单次枚举；MUST NOT 移回外层 WHERE，见该常量文档）。
        // 属权 UA 随资源行变化 ⇒ MUST 逐行解析，MUST NOT 并入全局 `effective_ua_ids`。
        let ownership_row_cte = ngac_host::ownership_cte()
            .replace("$2", "oa.resource_type")
            .replace("$3", "oa.fk_resource");
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
            SELECT DISTINCT oa.id, oa.fk_resource,
                   COALESCE(own.ua_ids, '{{}}'::bigint[]) AS own_ua_ids,
                   COALESCE(own.ua_names, '{{}}'::text[]) AS own_ua_names,
                   (oa.property ->> 'label') AS label_code
            FROM isahl_auth.ngac_object_attribute oa
            LEFT JOIN LATERAL (
                WITH {ownership_row_cte}
                SELECT
                    (SELECT array_agg(DISTINCT ua.id)
                       FROM ownership_ua ou
                       INNER JOIN isahl_auth.ngac_user_attribute ua
                               ON ua.id = ou.ua_id AND ua.deleted_at IS NULL) AS ua_ids,
                    (SELECT array_agg(DISTINCT ua.o_name)
                       FROM ownership_ua ou
                       INNER JOIN isahl_auth.ngac_user_attribute ua
                               ON ua.id = ou.ua_id AND ua.deleted_at IS NULL) AS ua_names,
                    ({creator_exists}) AS creator_ua
            ) own ON TRUE
            WHERE oa.resource_type = $2
              AND oa.deleted_at IS NULL
              AND (EXISTS (
                  SELECT 1
                  FROM isahl_auth.ngac_association a
                  WHERE a.deleted_at IS NULL
                    AND a.fk_object_attribute = ANY(oa.id || COALESCE(oa.ancestor_ids, '{{}}'::bigint[]))
                    AND (a.fk_user_attribute IN (SELECT c.ua_id FROM user_attrs c)
                         OR a.fk_user_attribute = ANY(COALESCE(own.ua_ids, '{{}}'::bigint[])))
              )
              OR own.creator_ua)
            "#,
            COGNITION_CTE = COGNITION_CTE,
            DELEGATED_CTE = DELEGATED_CTE,
            ownership_row_cte = ownership_row_cte,
            creator_exists = ngac_host::CREATOR_DERIVED_EXISTS_OA
        );
        let candidates: Vec<(i64, i64, Vec<i64>, Vec<String>, Option<String>)> =
            sqlx::query_as(AssertSqlSafe(cand_sql.as_str()))
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

        // 索引化求值原语（indexed-evaluation-primitive）：一次建索引、逐行求值，
        // 替代原先逐行自写的 deny-overrides 合并循环（唯一实现 = ngac-decision crate）。
        let indexed = ngac_decision::IndexedPolicyGraph::from(&pg);

        let oa_index: std::collections::HashMap<i64, (String, Vec<i64>)> = oas
            .into_iter()
            .map(|(id, name, ancestors)| (id, (name, ancestors)))
            .collect();
        let ua_ids: Vec<i64> = user_attrs.iter().map(|(id, _)| *id).collect();
        let ua_names: Vec<String> = user_attrs.iter().map(|(_, n)| n.clone()).collect();
        let now = Utc::now();

        // 分级（密级）上限预取（NGAC_SPEC §2.7）：白名单 / 标签等级 / UA clearance 等级。
        // 逐行 N+1 不可接受 ⇒ 一次取全、逐行按同一规则（`ngac_org::label_clearance_denies_rule`）
        // 收敛。非白名单 ⇒ 完全不查（零成本）。表缺失（ns 库未 ensure）⇒ 视为无数据 + 告警。
        let label_whitelisted: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM isahl_auth.org_policy_class c \
              WHERE c.deleted_at IS NULL AND c.state = 'active' AND c.scope IS NOT NULL \
                AND c.scope -> 'resource_types' @> to_jsonb($1::text))",
        )
        .bind(resource_type)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| {
            log::warn!("label ceiling unavailable (whitelist query), skipping: {e}");
            false
        });
        let label_ranks: std::collections::HashMap<String, i32> = if label_whitelisted {
            sqlx::query_as::<_, (String, i32)>(
                "SELECT code, rank FROM isahl_auth.org_policy_label WHERE deleted_at IS NULL AND is_active",
            )
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect()
        } else {
            std::collections::HashMap::new()
        };
        let ua_clearance: std::collections::HashMap<i64, i32> = if label_whitelisted {
            sqlx::query_as::<_, (i64, i32)>(
                "SELECT ua.id, l.rank FROM isahl_auth.ngac_user_attribute ua \
                 JOIN isahl_auth.org_policy_label l \
                   ON l.code = (ua.property ->> 'clearance') AND l.deleted_at IS NULL AND l.is_active \
                 WHERE ua.deleted_at IS NULL",
            )
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect()
        } else {
            std::collections::HashMap::new()
        };

        // 创建者派生资源集（单查预取；逐行成员判定，避免 N+1）——同源实现 =
        // common::ngac_org::creator_derived_resource_ids（与 decide/内层门同一谓词族）。
        let creator_derived: std::collections::HashSet<i64> =
            ngac_host::creator_derived_resource_ids(&self.pool, fk_user)
                .await
                .unwrap_or_default()
                .into_iter()
                .collect();
        let mut visible: Vec<i64> = Vec::new();
        for (oa_id, fk_resource, own_ua_ids, own_ua_names, label_code) in candidates {
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
            // 属权派生（资源受限）并入本行求值：UA id 集与条件求值的 UA 名集
            // （`user_attr_in` 需含属权派生名，§2.4 一致性义务）。
            let mut row_ua_ids: Vec<i64> = ua_ids.clone();
            row_ua_ids.extend(own_ua_ids.iter().copied());
            let mut row_ua_names: Vec<String> = ua_names.clone();
            row_ua_names.extend(own_ua_names.iter().cloned());
            // 分级（密级）上限（NGAC_SPEC §2.7）：白名单资源上 clearance < 标签等级 ⇒ 该行不可见。
            // 规则实现单源 = `common::ngac_org::label_clearance_denies_rule`（与单条 decide 同规则）。
            // **主体 clearance 只取全局有效 UA 集**（密级 = 主体属性）：MUST NOT 并入本行的
            // 属权派生 UA（`own_ua_ids`，资源级授权）——后者是「对该资源的操作权」，不是主体的密级。
            if label_whitelisted {
                let res_rank = label_code
                    .as_deref()
                    .and_then(|c| label_ranks.get(c))
                    .copied();
                let subj_rank = ua_ids
                    .iter()
                    .filter_map(|id| ua_clearance.get(id))
                    .copied()
                    .max();
                if ngac_host::label_clearance_denies_rule(true, res_rank, subj_rank) {
                    continue;
                }
            }
            let ctx = crate::ngac::pdp::ConditionContext {
                now,
                user_ua_names: row_ua_names.clone(),
                oa_closure_names: closure
                    .iter()
                    .filter_map(|id| oa_index.get(id).map(|(n, _)| n.clone()))
                    .collect(),
                // 切片 B：RLS 列表与真决策**同源消费** `{"rhai": …}` 条件（逐行资源上下文，
                // 主体 = 本请求用户）——否则「列表不可见而详情可读」的分叉。
                rhai_eval: Some(ngac_host::rhai_condition_evaluator(
                    fk_user,
                    format!("{resource_type}:{fk_resource}"),
                    action.to_string(),
                )),
            };
            // deny-overrides 合并走唯一实现（ngac-decision 索引原语；不构建轨迹 + Deny 短路）。
            let pair_decision =
                indexed.evaluate_decision(&row_ua_ids, &closure, action, &ctx, false);
            let denied = pair_decision == ngac_decision::Decision::Deny;
            let saw_permit = pair_decision == ngac_decision::Decision::Permit;
            // 派生命中同样受 deny-overrides（denied 即排除）与密级上限（上方 continue）约束；
            // 与单条 decide 同序：物理 Permit OR 派生命中。
            if !denied && (saw_permit || creator_derived.contains(&fk_resource)) {
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
/// 指派级 `conditions` 的写径校验（唯一实现 = [`validate_assignment_conditions`]）由调用方
/// 在进入本函数前执行（HTTP 面 ⇒ 400）。
///
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

/// 指派级 `conditions` 写径校验（**唯一实现**；NGAC_SPEC §2.4 子集）。
///
/// 键集 MUST ⊆ §2.4 合法键集（`ngac_decision::CONDITION_KEYS`，经
/// `ngac_host::validate_condition_keys`），且 MUST 仅含**解析点可求值**的键：
/// `isahl_auth.effective_ua_ids` 在该点无资源/动作上下文，故仅 `not_before` / `not_after`
/// 可求值；其余键（`rhai` / `user_attr_in` / `object_attr_in`）在此不可求值 ⇒ 接受即
/// 「写入成功却静默失效」（解析期失败封闭 ⇒ 该 UA 被丢弃）⇒ 写径 MUST 显式拒绝。
/// 两侧（本校验 + 闭包 SQL 谓词）合并即指派级条件的**键集契约**。
pub fn validate_assignment_conditions(conditions: &serde_json::Value) -> Result<(), String> {
    ngac_host::validate_condition_keys(conditions)?;
    if let Some(obj) = conditions.as_object() {
        for k in obj.keys() {
            if k != "not_before" && k != "not_after" {
                return Err(format!(
                    "指派级 conditions 仅支持时间窗键（not_before / not_after）；`{k}` 在有效 UA 解析点不可求值（NGAC_SPEC §2.4 子集）"
                ));
            }
        }
    }
    Ok(())
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
    // 审计署名的主体 id：无法解析自然人主体（服务令牌 `client:*` / 畸形 sub）即拒绝该写操作，
    // MUST NOT 以 actor=0 写审计行（判定唯一实现 = common::context::natural_subject_id）。
    let Some(actor) = common::context::natural_subject_id(&claims.sub) else {
        log::warn!(
            "set_user_attribute: sub 非自然人主体 id，拒绝写操作: {}",
            claims.sub
        );
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "Subject is not a natural-person principal".to_string(),
        });
    };
    let session_id = if claims.sid.is_empty() {
        None
    } else {
        Some(claims.sid)
    };
    let ip = crate::ngac::audit_writer::client_ip(&req);

    // 指派级 `conditions` 写径校验（NGAC_SPEC §2.4 子集；`wire-ngac-zero-consumer-faces`）：
    // 有效 UA 解析点无资源/动作上下文 ⇒ 仅时间窗键可求值；其余键（含 `rhai`）MUST 在此拒绝
    // （400），否则 = 静默失效条件（解析期失败封闭 ⇒ 该 UA 被静默丢弃）。
    if let Some(c) = assign_req.conditions.as_ref() {
        if let Err(e) = validate_assignment_conditions(c) {
            return HttpResponse::BadRequest().json(ErrorResponse { error: e });
        }
    }

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
