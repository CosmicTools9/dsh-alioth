//! Enriched ApprovalInstance Handler
//! GET /service/approval/approval-instances/enriched
//! Registered BEFORE CRUD /{id} to avoid catch-all conflict.

use actix_web::{web, HttpRequest, HttpResponse};
use common::error::AliothError;
use common::search::like_pattern;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

#[derive(Debug, Deserialize, Default)]
pub struct EnrichedListQuery {
    #[serde(default, with = "common::serde_zuid::opt")]
    pub page: Option<i64>,
    #[serde(alias = "pageSize", default, with = "common::serde_zuid::opt")]
    pub page_size: Option<i64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    /// 模糊搜索：节点名（实例 notice）/ 申请人姓名 ILIKE
    #[serde(default)]
    pub q: Option<String>,
    /// 实例 code 白名单（逗号分隔，trim 后去空项，上限 20 项）。
    /// 用途：管理端「注册审批队列」限定认证域 code（注册双通道），避免拉全量后本地过滤。
    #[serde(default)]
    pub codes: Option<String>,
}

/// `codes` 白名单上限（超出即 400——静默截断会让调用方以为过滤生效）。
const CODES_MAX: usize = 20;

/// 解析 `codes` 查询参数：逗号分隔 → trim → 去空项；超限或全空 → Validation。
fn parse_codes(raw: Option<&str>) -> Result<Option<Vec<String>>, AliothError> {
    let Some(raw) = raw else { return Ok(None) };
    let items: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if items.is_empty() {
        return Ok(None);
    }
    if items.len() > CODES_MAX {
        return Err(AliothError::Validation {
            field: "codes".into(),
            message: format!("too many codes: {} > {}", items.len(), CODES_MAX),
        });
    }
    Ok(Some(items))
}

#[derive(Debug, FromRow)]
#[allow(dead_code)]
struct EnrichedRow {
    pub id: i64,
    // 列可空（实例 notice 允许 NULL）：Option 解码防御，避免共享库历史 NULL 行
    // 触发整表 500（2026-08-26 实证：存量 NULL notice 行）
    pub node_name: Option<String>,
    pub code: Option<String>,
    pub fk_approve: Option<i64>,
    pub fk_subject: Option<i64>,
    /// 重提链来源实例 id（`meta.original_id` 派生；NULL = 非重提）
    pub original_id: Option<i64>,
    pub comments: Option<String>,
    #[allow(dead_code)]
    pub lk_urgent: Option<i64>,
    pub priority_label: Option<String>,
    pub sla_hours: Option<i64>,
    pub timeline: Option<serde_json::Value>,
    pub applicant_name: Option<String>,
    pub derived_status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub total_count: i64,
}

#[derive(Debug, Serialize)]
pub struct EnrichedItem {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    // 与 EnrichedRow 对齐：实例 notice 可空，响应返回 null 而非 500
    pub node_name: Option<String>,
    pub code: Option<String>,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_approve: Option<i64>,
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_subject: Option<i64>,
    /// 重提链：再申请来源实例 id（zuid ⇒ JSON 字符串；无链为 null）
    #[serde(default)]
    #[serde(with = "common::serde_zuid::opt")]
    pub original_id: Option<i64>,
    pub comments: Option<String>,
    pub applicant: Option<String>,
    pub status: String,
    pub result: String,
    pub priority: Option<String>,
    /// SLA 时长（小时，**非 id 语义** ⇒ 线上为数字，与 `approval-analytics-api` 的
    /// `slaHours: 24` 同形；ID_JSON_PRECISION 只覆盖 id 语义键，原 serde_zuid 属过施加）
    pub sla: Option<i64>,
    pub timeline: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize)]
struct EnrichedListResponse {
    pub items: Vec<EnrichedItem>,
    // 计数/页码非 id 语义 ⇒ 线上为数字（前端 `PagedResult.total/page/pageSize: number` 同形；
    // 原 serde_zuid 属过施加，唯一受害面是「声明 number 而实际收到 string」）
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

pub async fn list_enriched(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    query: web::Query<EnrichedListQuery>,
) -> Result<HttpResponse, AliothError> {
    // Validate params——scope 三态：todo（默认，我的待办：处理人=我或岗位命中我，
    // 持 admin UA 的用户按管理视角全量可见）/ my-request（我发起的）/ all（全量，管理视角）
    let scope_key = query.scope.as_deref().unwrap_or("todo").to_string();
    match scope_key.as_str() {
        "todo" | "my-request" | "all" => {}
        other => {
            return Err(AliothError::Validation {
                field: "scope".into(),
                message: format!("invalid scope: {other}"),
            })
        }
    }
    let user_id = common::context::require_auth(&req)?;
    // `scope=all` = 管理视角全量（跨 operator 可见）——MUST 持 admin UA，否则 403。
    // 判据唯一实现 = common::permissions::is_admin_user（与 require_resource_access 的
    // admin 兜底同源）；fail-closed：判定失败即拒绝，不回落任何「策略缺失即放行」分支。
    if scope_key == "all" && !common::permissions::is_admin_user(pool.get_ref(), user_id).await? {
        return Err(AliothError::Forbidden(
            "scope=all 仅限 admin：非管理员不可读取全量审批实例".into(),
        ));
    }
    let codes = parse_codes(query.codes.as_deref())?;
    if let Some(s) = &query.status {
        if ![
            "pending",
            "approved",
            "rejected",
            "withdrawn",
            "cancelled",
            "abstained",
        ]
        .contains(&s.as_str())
        {
            return Err(AliothError::Validation {
                field: "status".into(),
                message: format!("invalid status: {s}"),
            });
        }
    }

    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1) * page_size;
    let q_pattern = query
        .q
        .as_deref()
        .map(|q| like_pattern(q.trim()))
        .filter(|p| p != "%%");

    // Single CTE: derive status in SQL, apply ALL filters, paginate, count via window function
    // 任职账号集合片段注入（唯一实现 = common::position_incumbent_accounts_sql!()）
    let sql = concat!(
        r#"
        WITH src AS (
            -- 申请人 id 归一：`fk_subject` 在本表的既有契约是**用户 id**（写入侧存调用者 user_id：
            -- advance::create_approval_instances 的 actx.initiator；既有消费侧 analytics/notify/remind
            -- 亦按 auth_users 解析）；早期/种子实例该列为 NULL 且均为首实例（fk_previous IS NULL）⇒
            -- 以 created_by_id 兜底（首实例的 created_by_id 即发起人；下游实例的 created_by_id 是
            -- 上一节点触发者，不可当申请人，故仅在首实例兜底）。
            SELECT i.*,
                   COALESCE(i.fk_subject,
                            CASE WHEN i.fk_previous IS NULL THEN i.created_by_id END) AS applicant_uid
            FROM isahl."zc_id_oper-approve" i
        ),
        base AS (
            SELECT i.id, i.notice AS node_name, i.code,
                   (SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
                    WHERE oe.ref_left = i.id AND oe.deleted_at IS NULL
                      AND EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_event oe2
                                  JOIN isahl.zc_id_process_rr_operation rro2
                                    ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL
                                  WHERE oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL)
                    ORDER BY oe.created_at LIMIT 1) AS fk_approve,
                   i.fk_subject, i.comments,
                   -- 重提链：创建时写入的 meta.original_id（「再申请」入口；免 DDL 扩展槽）。
                   -- 加数字形态守卫 ⇒ 手工写入的非数字值退化为 NULL 而非整表 500。
                   CASE WHEN i.meta->>'original_id' ~ '^[0-9]+$'
                        THEN (i.meta->>'original_id')::bigint END AS original_id,
                   ev.lk_urgent, ev.timeline,
                   COALESCE(e.notice, u.display_name, u.username) AS applicant_name,
                   lu.notice AS priority_label,
                   sd.mark::bigint AS sla_hours,
                   COALESCE(st.code, 'pending') AS derived_status,
                   -- 置顶分组：注册 / 认证 / 授权的相关审批恒定优先（用户裁决 2026-09-27：
                   -- 「我的审批的首个记录就应该是用户注册、认证、授权的相关审批」）。
                   -- 判据 = 业务实例行（`_t_='实例'`，同 advance.rs 的模板/接线行判别口径）
                   -- ∪ 实例 code（注册双通道写入的 code）∪ 实例事件所属流程为三条认证域流程
                   -- （经既有归属桥链：实例 →操作↔事件桥→ 事件 →归属桥→ 流程，与同查询
                   -- `fk_approve` 的解析同构）。**不含**流程节点模板行（`_t_='范例'`/NULL）
                   -- ——否则模型种子物化的 N1/N2/N3 节点会被误置顶。
                   CASE WHEN i._t_ = '实例'
                             AND (
                               i.code IN ('user-register-approval', 'external-subject-register-approval')
                               OR EXISTS (
                                    SELECT 1
                                    FROM isahl.zc_id_operation_rr_event ev4
                                    JOIN isahl.zc_id_process_rr_operation rro4
                                      ON rro4.ref_right = ev4.ref_left AND rro4.deleted_at IS NULL
                                    JOIN isahl.zc_id_process p4
                                      ON p4.id = rro4.ref_left AND p4.deleted_at IS NULL
                                    WHERE ev4.deleted_at IS NULL
                                      AND p4.code IN ('FLOW-USER-VERIFY', 'FLOW-AUTHORIZATION', 'FLOW-EXTERNAL-SUBJECT')
                                      AND ev4.ref_right = (SELECT oe4.ref_right
                                                             FROM isahl.zc_id_operation_rr_event oe4
                                                            WHERE oe4.ref_left = i.id AND oe4.deleted_at IS NULL
                                                            ORDER BY oe4.created_at LIMIT 1)
                                  )
                             )
                        THEN 0 ELSE 1 END AS sort_group,
                   i.created_at, i.updated_at
            FROM src i
            -- 员工主体行按 fk_user（用户 id → 员工）解析；兼容少量历史行直接存员工 id
            LEFT JOIN LATERAL (
                SELECT ee.notice
                FROM isahl."zc_id_subj-employee" ee
                WHERE ee.deleted_at IS NULL
                  AND (ee.fk_user = i.applicant_uid OR ee.id = i.applicant_uid)
                ORDER BY (ee.fk_user = i.applicant_uid) DESC
                LIMIT 1
            ) e ON true
            -- 无员工主体绑定时回退账号显示名（auth_users 主键唯一，无笛卡尔展开）
            LEFT JOIN isahl_auth.auth_users u ON u.id = i.applicant_uid
            LEFT JOIN isahl."zc_id_even-approve" ev ON ev.id = (SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
                    WHERE oe.ref_left = i.id AND oe.deleted_at IS NULL
                      AND EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_event oe2
                                  JOIN isahl.zc_id_process_rr_operation rro2
                                    ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL
                                  WHERE oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL)
                    ORDER BY oe.created_at LIMIT 1) AND ev.deleted_at IS NULL
            LEFT JOIN isahl."zc_id_leve-urgent" lu ON lu.id = ev.lk_urgent
            LEFT JOIN isahl."zc_id_scal-duration" sd ON sd.id = ev.qk_sla
            LEFT JOIN LATERAL (
                SELECT s.code FROM isahl."zc_id_lifecycle_r_primary-status" ls
                JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
                WHERE ls.ref_left = i.id AND ls.deleted_at IS NULL
                ORDER BY ls.id DESC LIMIT 1
            ) st ON true
            WHERE i.deleted_at IS NULL
              AND (
                $2::text = 'all'
                OR ($2::text = 'my-request' AND i.fk_subject = $3)
                OR ($2::text = 'todo' AND (
                  i.fk_operator = $3
                  OR EXISTS (
                    SELECT 1
                    FROM isahl.zc_id_operation_rr_event ie2
                    JOIN isahl."zc_id_even-approve" ea2
                      ON ea2.id = ie2.ref_right AND ea2.deleted_at IS NULL
                    JOIN isahl.zc_id_operation_rr_event oe2
                      ON oe2.ref_right = ea2.id AND oe2.deleted_at IS NULL
                      AND EXISTS (SELECT 1 FROM isahl.zc_id_process_rr_operation rro2
                                  WHERE rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL)
                    JOIN (
                      SELECT ref_left, ref_right FROM isahl."zc_id_operation_rr_approve" WHERE deleted_at IS NULL
                      UNION ALL
                      SELECT ref_left, ref_right FROM isahl."zc_id_operation_rr_review" WHERE deleted_at IS NULL
                      UNION ALL
                      SELECT ref_left, ref_right FROM isahl."zc_id_operation_rr_post" WHERE deleted_at IS NULL
                    ) br2 ON br2.ref_left = oe2.ref_left
                    -- 「我是该节点岗位审批人」判据 = 岗位任职账号集合（标量 ∪ 任职桥派生；
                    -- 唯一实现 = common 片段）——只认标量会漏掉仅挂桥任职的审批人
                    WHERE ie2.ref_left = i.id AND ie2.deleted_at IS NULL
                      AND EXISTS (
                        SELECT 1 FROM ("#,
        common::position_incumbent_accounts_sql!(),
        r#"
                        ) inc
                        WHERE inc.pos_id = br2.ref_right AND inc.uid = $3
                      )
                  )
                  OR isahl_auth.is_admin($3)
                ))
              )
        )
        SELECT *, COUNT(*) OVER() AS total_count
        FROM base
        WHERE ($1::text IS NULL OR derived_status = $1)
          AND ($4::text IS NULL OR node_name ILIKE $4 OR applicant_name ILIKE $4)
          -- $7: 实例 code 白名单（管理端认证域队列用；NULL = 不过滤）
          AND ($7::text[] IS NULL OR code = ANY($7::text[]))
        ORDER BY sort_group, created_at DESC
        LIMIT $5 OFFSET $6
        "#
    );
    let rows: Vec<EnrichedRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(query.status.as_deref()) // $1: status filter (None = no filter)
        .bind(&scope_key) // $2: scope key (todo/my-request/all)
        .bind(user_id) // $3: user_id for scope filter
        .bind(q_pattern.as_deref()) // $4: q ILIKE pattern (None = no filter)
        .bind(page_size) // $5: limit
        .bind(offset) // $6: offset
        .bind(codes.as_deref()) // $7: instance code whitelist (None = no filter)
        .fetch_all(pool.get_ref())
        .await
        .map_err(|e| AliothError::Database(e.to_string()))?;

    let total = rows.first().map(|r| r.total_count).unwrap_or(0);

    let items = rows
        .into_iter()
        .map(|r| {
            let result = match r.derived_status.as_str() {
                "approved" => "approved",
                "rejected" => "rejected",
                _ => "pending",
            };
            EnrichedItem {
                id: r.id,
                node_name: r.node_name,
                code: r.code,
                fk_approve: r.fk_approve,
                fk_subject: r.fk_subject,
                original_id: r.original_id,
                comments: r.comments,
                applicant: r.applicant_name,
                status: r.derived_status,
                result: result.to_string(),
                sla: r.sla_hours,
                timeline: r.timeline,
                priority: r.priority_label,
                created_at: r.created_at,
                updated_at: r.updated_at,
            }
        })
        .collect();

    Ok(HttpResponse::Ok().json(EnrichedListResponse {
        items,
        total,
        page,
        page_size,
    }))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route("/approval-instances/enriched", web::get().to(list_enriched));
}
