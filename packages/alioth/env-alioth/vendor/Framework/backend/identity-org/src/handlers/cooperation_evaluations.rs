//! 供应商合作评价（cooperation_r_evaluation，挂合作关系桥）
//!
//! 本体链正本（用户 2026-09-16 给出，change `add-supplier-qualification-certificates`）：
//! `zc_id_subjects ←zc_id_subjects_rr_partner→ zc_id_subjects`（合作关系桥）
//! ←(ref_left) `zc_id_relation-cooperation_r_evaluation`(ref_right)→ `zc_id_leve-qualification`
//! （评估挂桥）；`ck_cooperation → zc_id_cate-cooperation`（合作类型）；
//! `ak_attachment → zc_id_attachment`（证明材料 = 证照实体，仅已批证照可挂）。
//!
//! - `POST /subjects/{subject_id}/cooperation-evaluations` 语义：**评估方（调用者锚定主体，
//!   占位规范化 → 平台运营主体）× 被评估主体** 找或建合作关系桥（幂等，`qk_period` NULL），
//!   评估行 `ref_left = 桥行 id`——MUST NOT 直挂主体（旧实现，两库 0 行零迁移改写）。
//! - 同 (桥, 合作类型) 旧未删评价自动软删（调级语义）；模型未删唯一键 `(ref_left, ref_right)`
//!   = 每合作关系**同等级仅一条**——同桥异类型复用同等级显式 400（预检，不裸撞唯一键）。
//! - `ak_attachment` 挂已批证照（`zc_id_prod-certificate` 子树，`fk_subj-provider` = 被评估主体，
//!   状态桥 `cert-state-approved`）；未批/异属/不存在证照 fail-visible 400。
//!   列未扩散的库（测试库快照落后）拒收证照参数（400 列未扩散），读径降级空证照。
//!
//! 桥表非类契约表（无 `_f_`/`_t_`，非 lifecycle 子树）——INSERT 无类列要求。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::OnceLock;

#[derive(Debug, Deserialize)]
pub struct CreateCooperationEvaluationRequest {
    /// 合作类型编码（zc_id_cate-cooperation.code，如 COOP-TRANSPORT）
    pub cooperation_code: String,
    /// 资质等级编码（zc_id_leve-qualification.code，如 QUAL-A）
    pub qualification_code: String,
    /// 评价说明（可选，落 notice）
    pub remark: Option<String>,
    /// 证明材料证照行 id（可选；仅 `cert-state-approved` 且 `fk_subj-provider` = 被评估主体）
    #[serde(with = "common::serde_zuid::opt_seq", default)]
    pub certificate_ids: Option<Vec<i64>>,
}

/// 证照摘要（评估行 `ak_attachment` 解析；id 字符串化）
#[derive(Debug, Clone, Serialize)]
pub struct CertificateBrief {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    /// 证照类别（zc_id_cate-certification.notice）
    pub category_name: Option<String>,
    /// 证号（o_number）
    pub cert_no: Option<String>,
    /// 发证机关（notice）
    pub issuer: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CooperationEvaluation {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub cooperation_code: String,
    pub cooperation_name: String,
    pub qualification_code: String,
    pub qualification_name: String,
    /// 等级值（leve-qualification.lv_value，越高越优）
    pub level_value: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
    pub evaluated_at: chrono::DateTime<chrono::Utc>,
    /// 证明材料证照（评估行 ak_attachment 解析；空 = 未挂或列未扩散降级）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub certificates: Vec<CertificateBrief>,
}

/// 评估表 `ak_attachment` 列探测（进程内单次缓存；运行期 DB 查询结果，故用 OnceLock）：
/// 测试库模型快照可能落后（该列 2026-09-14 后扩散），缺列时读径降级空证照、
/// 写径拒收证照参数——不 42703 裸崩。
async fn eval_attachment_ready(pool: &PgPool) -> bool {
    static READY: OnceLock<bool> = OnceLock::new();
    if let Some(v) = READY.get() {
        return *v;
    }
    let has: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (SELECT 1 FROM information_schema.columns
            WHERE table_schema = 'isahl'
              AND table_name = 'zc_id_relation-cooperation_r_evaluation'
              AND column_name = 'ak_attachment')"#,
    )
    .fetch_one(pool)
    .await
    .unwrap_or(false);
    *READY.get_or_init(|| has)
}

/// 找或建合作关系桥（ref_left = 评估方主体 / ref_right = 被评估主体 / qk_period NULL；
/// 幂等：命中未删唯一键 `(ref_left, ref_right, COALESCE(qk_period,-1))` 复用既有行）。
async fn find_or_create_partner_bridge(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    anchor_org: i64,
    subject_id: i64,
    user_id: i64,
) -> Result<i64, ApiError> {
    let sql = r#"WITH ins AS (
            INSERT INTO isahl."zc_id_subjects_rr_partner"
                (notice, ref_left, ref_right, created_by_id)
            VALUES ('合作评价自动建桥', $1, $2, $3)
            ON CONFLICT (ref_left, ref_right, COALESCE(qk_period, '-1'::integer::bigint))
                WHERE deleted_at IS NULL
                DO NOTHING
            RETURNING id
        )
        SELECT id FROM ins
        UNION ALL
        SELECT id FROM isahl."zc_id_subjects_rr_partner"
         WHERE ref_left = $1 AND ref_right = $2 AND qk_period IS NULL AND deleted_at IS NULL
         LIMIT 1"#;
    sqlx::query_scalar(sql)
        .bind(anchor_org)
        .bind(subject_id)
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(ApiError::from_sqlx)
}

/// 证照可挂性校验：活跃 + `fk_subj-provider` = 被评估主体 + 状态桥 `cert-state-approved`。
/// 返回不符 id 集（空 = 全部可挂）。
async fn inadmissible_certificates(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    cert_ids: &[i64],
    subject_id: i64,
) -> Result<Vec<i64>, ApiError> {
    let admitted: Vec<i64> = sqlx::query_scalar(
        r#"SELECT c.id FROM isahl."zc_id_prod-certificate" c
           WHERE c.id = ANY($1) AND c.deleted_at IS NULL AND c."fk_subj-provider" = $2
             AND EXISTS (
                 SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ps
                 JOIN isahl."zc_id_stus-certification" sc
                   ON sc.id = ps.ref_right AND sc.code = 'cert-state-approved'
                  AND sc.deleted_at IS NULL
                 WHERE ps.ref_left = c.id AND ps.deleted_at IS NULL)
           ORDER BY c.id"#,
    )
    .bind(cert_ids)
    .bind(subject_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    let bad: Vec<i64> = cert_ids
        .iter()
        .copied()
        .filter(|id| !admitted.contains(id))
        .collect();
    Ok(bad)
}

/// 评估读谓词行（桥两跳 + 证照 id 集）
type EvalRow = (
    i64,
    String,
    String,
    String,
    String,
    f64,
    Option<String>,
    chrono::DateTime<chrono::Utc>,
    Option<Vec<i64>>,
);

/// 证照摘要行（id / 类别 / 证号 / 发证机关）
type CertBriefRow = (i64, Option<String>, Option<String>, Option<String>);

/// 评估行 `ak_attachment` 证照摘要解析（读径共用；缺行/软删行自然落空）。
async fn certificate_briefs(
    pool: &PgPool,
    cert_ids: &[i64],
) -> Result<Vec<CertificateBrief>, ApiError> {
    if cert_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<CertBriefRow> = sqlx::query_as(
        r#"SELECT c.id, cc.notice, c.o_number, c.notice
           FROM isahl."zc_id_prod-certificate" c
           LEFT JOIN isahl."zc_id_cate-certification" cc
                  ON cc.id = c."ck_cate-cert" AND cc.deleted_at IS NULL
           WHERE c.id = ANY($1) AND c.deleted_at IS NULL
           ORDER BY c.id"#,
    )
    .bind(cert_ids)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|(id, category_name, cert_no, issuer)| CertificateBrief {
            id,
            category_name,
            cert_no,
            issuer,
        })
        .collect())
}

/// 登记合作评价（handler 壳）：评估方 = 调用者锚定主体（账号 → 绑定主体 → 占位规范化 →
/// 平台运营主体；未绑定 fail-visible `OPERATOR_ORG_UNBOUND`）。
pub async fn create(
    pool: &PgPool,
    subject_id: i64,
    body: &CreateCooperationEvaluationRequest,
    user_id: i64,
) -> Result<CooperationEvaluation, ApiError> {
    let mut conn = pool.acquire().await.map_err(ApiError::from_sqlx)?;
    let actor = common::actor_identity::resolve_actor_identity(&mut conn, user_id).await?;
    drop(conn);
    create_anchored(pool, subject_id, actor.subject_id, body, user_id).await
}

/// 登记合作评价核心（挂桥）：`anchor_org` × `subject_id` 找或建合作关系桥 → 评估行落桥。
/// 测试直驱本函数（不经 actor 解析，fixture 自持主体对）。
pub async fn create_anchored(
    pool: &PgPool,
    subject_id: i64,
    anchor_org: i64,
    body: &CreateCooperationEvaluationRequest,
    user_id: i64,
) -> Result<CooperationEvaluation, ApiError> {
    if anchor_org == subject_id {
        return Err(ApiError::BadRequest(
            "评估方与被评估主体相同：合作关系桥禁止自指（zc_id_lifecycle_rr_non_self）".into(),
        ));
    }

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;

    // 主体存在性（fail-closed）
    let subject_exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(SELECT 1 FROM isahl."zc_id_subjects"
           WHERE id = $1 AND deleted_at IS NULL)"#,
    )
    .bind(subject_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    if !subject_exists {
        return Err(ApiError::NotFound(format!("主体 {subject_id} 不存在")));
    }

    // 合作类型解析（cate-cooperation，fail-closed）
    let coop: Option<(i64, String)> = sqlx::query_as(
        r#"SELECT id, notice FROM isahl."zc_id_cate-cooperation"
           WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(&body.cooperation_code)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    let Some((coop_id, coop_name)) = coop else {
        return Err(ApiError::BadRequest(format!(
            "合作类型 {} 不存在（zc_id_cate-cooperation）",
            body.cooperation_code
        )));
    };

    // 资质等级解析（leve-qualification，fail-closed）
    let level: Option<(i64, String, f64)> = sqlx::query_as(
        r#"SELECT id, notice, lv_value::float8 FROM isahl."zc_id_leve-qualification"
           WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(&body.qualification_code)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    let Some((level_id, level_name, level_value)) = level else {
        return Err(ApiError::BadRequest(format!(
            "资质等级 {} 不存在（zc_id_leve-qualification）",
            body.qualification_code
        )));
    };

    // 找或建合作关系桥（ref_left=评估方 / ref_right=被评估主体）
    let bridge_id = find_or_create_partner_bridge(&mut tx, anchor_org, subject_id, user_id).await?;

    // 证明材料（可选）：去重排序；列未扩散拒收（400）；未批/异属/不存在证照 fail-visible
    let mut cert_ids: Vec<i64> = body.certificate_ids.clone().unwrap_or_default();
    cert_ids.sort();
    cert_ids.dedup();
    if !cert_ids.is_empty() {
        if !eval_attachment_ready(pool).await {
            return Err(ApiError::BadRequest(
                "评估表 ak_attachment 列未扩散（模型快照落后）：暂不能挂证明材料证照".into(),
            ));
        }
        let bad = inadmissible_certificates(&mut tx, &cert_ids, subject_id).await?;
        if !bad.is_empty() {
            let listed = bad
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ApiError::BadRequest(format!(
                "证照 {listed} 不可挂：须为已批（cert-state-approved）且属主为被评估主体的证照行"
            )));
        }
    }

    // 模型唯一键 (桥, 等级) 预检：每合作关系同等级仅一条——异类型复用同等级显式 400
    let clash: Option<String> = sqlx::query_scalar(
        r#"SELECT cc.code FROM isahl."zc_id_relation-cooperation_r_evaluation" e
           JOIN isahl."zc_id_cate-cooperation" cc ON cc.id = e.ck_cooperation
           WHERE e.ref_left = $1 AND e.ref_right = $2 AND e.deleted_at IS NULL
           LIMIT 1"#,
    )
    .bind(bridge_id)
    .bind(level_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    if let Some(existing_code) = clash {
        if existing_code != body.cooperation_code {
            return Err(ApiError::BadRequest(format!(
                "该合作关系已有类型 {existing_code} 的同等级评价：模型唯一键 (桥, 等级) 每合作关系同等级仅一条"
            )));
        }
    }

    // 调级：同 (桥, 合作类型) 旧未删评价软删（历史保留可溯）
    sqlx::query(
        r#"UPDATE isahl."zc_id_relation-cooperation_r_evaluation"
           SET deleted_at = NOW(), deleted_by_id = $3
           WHERE ref_left = $1 AND ck_cooperation = $2 AND deleted_at IS NULL"#,
    )
    .bind(bridge_id)
    .bind(coop_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    // 评估行落桥（ref_left=桥行 / ref_right=等级 / ck_cooperation=类型；id 走表默认）
    let (eval_id, evaluated_at): (i64, chrono::DateTime<chrono::Utc>) = if cert_ids.is_empty() {
        sqlx::query_as(
            r#"INSERT INTO isahl."zc_id_relation-cooperation_r_evaluation"
               (notice, ref_left, ref_right, ck_cooperation, created_by_id)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id, created_at"#,
        )
        .bind(&body.remark)
        .bind(bridge_id)
        .bind(level_id)
        .bind(coop_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
    } else {
        sqlx::query_as(
            r#"INSERT INTO isahl."zc_id_relation-cooperation_r_evaluation"
               (notice, ref_left, ref_right, ck_cooperation, ak_attachment, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id, created_at"#,
        )
        .bind(&body.remark)
        .bind(bridge_id)
        .bind(level_id)
        .bind(coop_id)
        .bind(&cert_ids)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
    }
    .map_err(ApiError::from_sqlx)?;

    tx.commit().await.map_err(ApiError::from_sqlx)?;

    let certificates = certificate_briefs(pool, &cert_ids).await?;

    Ok(CooperationEvaluation {
        id: eval_id,
        cooperation_code: body.cooperation_code.clone(),
        cooperation_name: coop_name,
        qualification_code: body.qualification_code.clone(),
        qualification_name: level_name,
        level_value,
        remark: body.remark.clone(),
        evaluated_at,
        certificates,
    })
}

/// 当前评价读谓词核心：桥两跳（主体任一端）× 类型 × 等级 JOIN（未删 = 当前）+ 证照解析。
pub async fn current(
    pool: &PgPool,
    subject_id: i64,
) -> Result<Vec<CooperationEvaluation>, ApiError> {
    // 缺列库（测试库快照落后）走无 ak_attachment 形态——列名在计划期解析，
    // CASE WHEN 分支无法规避 42703，必须两条 SQL。
    let rows: Vec<EvalRow> = if eval_attachment_ready(pool).await {
        sqlx::query_as(
            r#"SELECT e.id, cc.code, cc.notice, lq.code, lq.notice, lq.lv_value::float8,
                      e.notice, e.created_at, e.ak_attachment
               FROM isahl."zc_id_subjects_rr_partner" b
               JOIN isahl."zc_id_relation-cooperation_r_evaluation" e
                 ON e.ref_left = b.id AND e.deleted_at IS NULL
               JOIN isahl."zc_id_cate-cooperation" cc
                 ON cc.id = e.ck_cooperation AND cc.deleted_at IS NULL
               JOIN isahl."zc_id_leve-qualification" lq
                 ON lq.id = e.ref_right AND lq.deleted_at IS NULL
               WHERE b.deleted_at IS NULL AND (b.ref_left = $1 OR b.ref_right = $1)
               ORDER BY e.created_at DESC"#,
        )
    } else {
        sqlx::query_as(
            r#"SELECT e.id, cc.code, cc.notice, lq.code, lq.notice, lq.lv_value::float8,
                      e.notice, e.created_at, NULL::bigint[]
               FROM isahl."zc_id_subjects_rr_partner" b
               JOIN isahl."zc_id_relation-cooperation_r_evaluation" e
                 ON e.ref_left = b.id AND e.deleted_at IS NULL
               JOIN isahl."zc_id_cate-cooperation" cc
                 ON cc.id = e.ck_cooperation AND cc.deleted_at IS NULL
               JOIN isahl."zc_id_leve-qualification" lq
                 ON lq.id = e.ref_right AND lq.deleted_at IS NULL
               WHERE b.deleted_at IS NULL AND (b.ref_left = $1 OR b.ref_right = $1)
               ORDER BY e.created_at DESC"#,
        )
    }
    .bind(subject_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from_sqlx)?;

    // 证照摘要批量解析（一次查询，按评估行分组回填）
    let mut cert_id_set: std::collections::BTreeSet<i64> = Default::default();
    for row in rows.iter() {
        if let Some(ids) = &row.8 {
            cert_id_set.extend(ids.iter().copied());
        }
    }
    let all_ids: Vec<i64> = cert_id_set.into_iter().collect();
    let brief_map: std::collections::HashMap<i64, CertificateBrief> =
        certificate_briefs(pool, &all_ids)
            .await?
            .into_iter()
            .map(|b| (b.id, b))
            .collect();

    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                cooperation_code,
                cooperation_name,
                qualification_code,
                qualification_name,
                level_value,
                remark,
                evaluated_at,
                cert_ids,
            )| {
                let certificates = cert_ids
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|cid| brief_map.get(cid).cloned())
                    .collect();
                CooperationEvaluation {
                    id,
                    cooperation_code,
                    cooperation_name,
                    qualification_code,
                    qualification_name,
                    level_value,
                    remark,
                    evaluated_at,
                    certificates,
                }
            },
        )
        .collect())
}

/// 登记合作评价（POST /subjects/{subject_id}/cooperation-evaluations）
pub async fn create_evaluation(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<CreateCooperationEvaluationRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "create").await?;
    let eval = create(pool.get_ref(), subject_id, &body, user_id).await?;
    Ok(HttpResponse::Created().json(ApiResponse::success(eval)))
}

/// 当前评价查询（GET /subjects/{subject_id}/cooperation-evaluations）
pub async fn list_evaluations(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "list").await?;
    let rows = current(pool.get_ref(), subject_id).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(rows)))
}

/// 合作评价路由（挂载于主体域——全 ns 壳可挂）
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/subjects/{subject_id}/cooperation-evaluations")
            .route(web::get().to(list_evaluations))
            .route(web::post().to(create_evaluation)),
    );
}
