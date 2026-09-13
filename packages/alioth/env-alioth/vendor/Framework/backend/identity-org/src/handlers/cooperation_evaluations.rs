//! 供应商合作评价（cooperation_r_evaluation 桥消费，wire-supplier-qualification-chain）
//!
//! B3 资质证照链首个写/读谓词消费方（resolve-carrier-scene-data-gaps 决策卡）：
//! `zc_id_subjects ←(ref_left) zc_id_relation-cooperation_r_evaluation(ref_right)→ zc_id_leve-qualification`
//!
//! - `POST /subjects/{subject_id}/cooperation-evaluations` — 登记合作评价：合作类型
//!   （cate-cooperation）+ 资质等级（leve-qualification）fail-closed 解析；同
//!   (主体, 合作类型) 旧未删评价自动软删（调级语义——历史行保留可溯）
//! - `GET /subjects/{subject_id}/cooperation-evaluations` — 当前评价读谓词：桥 ×
//!   类型字典 × 等级字典 JOIN（未删行 = 当前等级）
//!
//! 桥表非类契约表（无 `_f_`/`_t_`，非 lifecycle 子树）——INSERT 无类列要求；
//! 未删唯一索引 `(ref_left, ref_right)` 保证同主体同等级单行。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Debug, Deserialize)]
pub struct CreateCooperationEvaluationRequest {
    /// 合作类型编码（zc_id_cate-cooperation.code，如 COOP-TRANSPORT）
    pub cooperation_code: String,
    /// 资质等级编码（zc_id_leve-qualification.code，如 QUAL-A）
    pub qualification_code: String,
    /// 评价说明（可选，落 notice）
    pub remark: Option<String>,
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
}

/// 登记合作评价核心（handler 为薄壳）
pub async fn create(
    pool: &PgPool,
    subject_id: i64,
    body: &CreateCooperationEvaluationRequest,
    user_id: i64,
) -> Result<CooperationEvaluation, ApiError> {
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

    // 调级：同 (主体, 合作类型) 旧未删评价软删（历史保留可溯）
    sqlx::query(
        r#"UPDATE isahl."zc_id_relation-cooperation_r_evaluation"
           SET deleted_at = NOW(), deleted_by_id = $3
           WHERE ref_left = $1 AND ck_cooperation = $2 AND deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .bind(coop_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    // 桥行落库（ref_left=主体 / ref_right=等级 / ck_cooperation=类型；id 走表默认）
    let eval_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_relation-cooperation_r_evaluation"
           (notice, ref_left, ref_right, ck_cooperation, created_by_id)
           VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
    )
    .bind(&body.remark)
    .bind(subject_id)
    .bind(level_id)
    .bind(coop_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    let evaluated_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        r#"SELECT created_at FROM isahl."zc_id_relation-cooperation_r_evaluation" WHERE id = $1"#,
    )
    .bind(eval_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(CooperationEvaluation {
        id: eval_id,
        cooperation_code: body.cooperation_code.clone(),
        cooperation_name: coop_name,
        qualification_code: body.qualification_code.clone(),
        qualification_name: level_name,
        level_value,
        remark: body.remark.clone(),
        evaluated_at,
    })
}

/// 当前评价读谓词核心：桥 × 类型 × 等级 JOIN（未删 = 当前）
pub async fn current(
    pool: &PgPool,
    subject_id: i64,
) -> Result<Vec<CooperationEvaluation>, ApiError> {
    let rows: Vec<CooperationEvaluation> = sqlx::query_as::<
        _,
        (
            i64,
            String,
            String,
            String,
            String,
            f64,
            Option<String>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"SELECT e.id, cc.code, cc.notice, lq.code, lq.notice, lq.lv_value::float8,
                  e.notice, e.created_at
           FROM isahl."zc_id_relation-cooperation_r_evaluation" e
           JOIN isahl."zc_id_cate-cooperation" cc ON cc.id = e.ck_cooperation
           JOIN isahl."zc_id_leve-qualification" lq ON lq.id = e.ref_right
           WHERE e.ref_left = $1 AND e.deleted_at IS NULL
           ORDER BY e.created_at DESC"#,
    )
    .bind(subject_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from_sqlx)?
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
        )| {
            CooperationEvaluation {
                id,
                cooperation_code,
                cooperation_name,
                qualification_code,
                qualification_name,
                level_value,
                remark,
                evaluated_at,
            }
        },
    )
    .collect();
    Ok(rows)
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
