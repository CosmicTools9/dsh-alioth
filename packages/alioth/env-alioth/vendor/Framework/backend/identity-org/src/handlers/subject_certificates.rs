//! 主体持有证书（物权语义应用，add-subject-certificate-title）
//!
//! 主体-证书/授权的关联 = 「主体持有证书」的物权语义：
//! - `POST /subjects/{subject_id}/certificates` — 证书取得：创建证书实体（纸质
//!   `zc_id_prod-certificate` / 数字化 `zc_id_prod-digital_cert-sales`）+ 同事务编制
//!   初始物权凭证（`create_title_voucher_tx` 单边 IN）→ `mv_title_ownership` 刷新后
//!   自动生成 (主体, 证书) 关联项
//! - `GET /subjects/{subject_id}/certificates` — 持有证书查询：mv_title_ownership ×
//!   证书族（prod 父表查询 + tableoid 叶表判定），净属权 > 0 即持有中（吊销/注销 =
//!   出库凭证净减，归零即不再持有）
//!
//! 类列形态 1（§4.3.3）：证书与凭证的 dk_* 均经 `ontology_binding::resolve_conn`
//! 解析（坐标 JC / GID / ↓_LA 授权许可·实现实例），_f_/_t_ 由触发器派生。
//! Handler 为薄壳（auth + 调核心函数），核心逻辑可被集成测试直驱。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// 物权坐标：场景 JC / 要素 GID / 职能 ↓_LA（授权许可，实现·实例）
const TITLE_COORDS: (&str, &str, &str) = ("JC", "GID", "↓_LA");

/// 证书族叶（模型无通用证书叶——prod-certificate 为抽象父；取得按类型叶路由，
/// digital_cert 为数字化载体，其余为纸质类型证书）
const CERT_KINDS: &[(&str, &str)] = &[
    ("digital", "zc_id_prod-digital_cert-sales"),
    ("type", "zc_id_prod-type_cert-sales"),
    ("air", "zc_id_prod-air_cert-sales"),
    ("diploma", "zc_id_prod-diploma-sales"),
    ("marriage", "zc_id_prod-marriage_cert-sales"),
];

#[derive(Debug, Deserialize)]
pub struct AcquireCertificateRequest {
    /// 证书名称（notice）
    pub name: String,
    /// 业务编号（code，幂等锚点）
    pub code: String,
    /// 证书类型编码（zc_id_cate-certification.code，如 CERT-ID-CARD / cert-tc）
    pub category_code: String,
    /// 证书族叶：digital 数字化证书 / type 型号合格证 / air 适航证 / diploma 文凭 / marriage 结婚证
    pub kind: String,
    /// 取得份数（默认 1）
    #[serde(default = "one")]
    pub qty: f64,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Serialize)]
pub struct SubjectCertificate {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    /// 叶表名（持有物形态）
    pub kind: String,
    /// 载体：paper / digital
    pub medium: String,
    pub name: String,
    pub code: String,
    /// 证书类型名（cate-certification.notice）
    pub category: Option<String>,
    /// 净持有份数（Σincome − Σoutgo）
    pub net_qty: f64,
    /// 物权凭证笔数
    pub voucher_count: i64,
    /// 首次取得时间
    pub acquired_at: chrono::DateTime<chrono::Utc>,
}

/// 物权视图就绪 + 刷新（幂等自愈：缺失则按内嵌 DDL 创建 + 初始 REFRESH；
/// 已就绪则补一次刷新——ensure_mv_title_ownership 内置全部分支）
async fn refresh_title_mv(pool: &PgPool) -> Result<(), ApiError> {
    trigger_registry::stock_materialization::ensure_mv_title_ownership(pool)
        .await
        .map_err(|e| ApiError::Internal(format!("mv_title_ownership 自愈失败: {e}")))
}

/// 证书取得核心：证书实体 + 同事务初始物权凭证 + 视图刷新
pub async fn acquire(
    pool: &PgPool,
    subject_id: i64,
    body: &AcquireCertificateRequest,
    user_id: i64,
) -> Result<SubjectCertificate, ApiError> {
    let Some((_, leaf)) = CERT_KINDS.iter().find(|(k, _)| *k == body.kind) else {
        return Err(ApiError::BadRequest(format!(
            "kind 必须为 {}，实际 {}",
            CERT_KINDS
                .iter()
                .map(|(k, _)| *k)
                .collect::<Vec<_>>()
                .join("/"),
            body.kind
        )));
    };
    let leaf = leaf.to_string();
    if body.qty <= 0.0 {
        return Err(ApiError::BadRequest("qty 必须为正数".to_string()));
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

    // 证书类型解析（cate-certification code → id，fail-closed）
    let category_id: i64 = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_cate-certification"
           WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(&body.category_code)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?
    .ok_or_else(|| {
        ApiError::BadRequest(format!(
            "证书类型 {} 不存在（zc_id_cate-certification）",
            body.category_code
        ))
    })?;

    // dk 坐标解析（形态 1 派生源）
    let dk = ontology_binding::resolve_conn(&mut *tx, TITLE_COORDS)
        .await
        .map_err(ApiError::from_sqlx)?;

    // 业务键预检（code 无库级唯一索引——fail-closed 拒绝重复取得）
    let dup_sql = format!(
        r#"SELECT EXISTS(SELECT 1 FROM isahl."{leaf}" WHERE code = $1 AND deleted_at IS NULL)"#
    );
    let dup: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(dup_sql.as_str()))
        .bind(&body.code)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?;
    if dup {
        return Err(ApiError::BadRequest(format!(
            "证书编号 {} 已存在（{leaf}，不得重复取得）",
            body.code
        )));
    }

    // 证书实体落表（按 kind 路由证书族叶——每叶字面量 INSERT，叶表/类契约静态可见）
    let cert_id: i64 = match leaf.as_str() {
        "zc_id_prod-digital_cert-sales" => sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_prod-digital_cert-sales"
                   (id, code, notice, ck_category, dk_scene, dk_factor, dk_function, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7) RETURNING id"#,
        )
        .bind(&body.code)
        .bind(&body.name)
        .bind(category_id)
        .bind(dk.0)
        .bind(dk.1)
        .bind(dk.2)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?,
        "zc_id_prod-type_cert-sales" => sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_prod-type_cert-sales"
                   (id, code, notice, ck_category, dk_scene, dk_factor, dk_function, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7) RETURNING id"#,
        )
        .bind(&body.code)
        .bind(&body.name)
        .bind(category_id)
        .bind(dk.0)
        .bind(dk.1)
        .bind(dk.2)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?,
        "zc_id_prod-air_cert-sales" => sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_prod-air_cert-sales"
                   (id, code, notice, ck_category, dk_scene, dk_factor, dk_function, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7) RETURNING id"#,
        )
        .bind(&body.code)
        .bind(&body.name)
        .bind(category_id)
        .bind(dk.0)
        .bind(dk.1)
        .bind(dk.2)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?,
        "zc_id_prod-diploma-sales" => sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_prod-diploma-sales"
                   (id, code, notice, ck_category, dk_scene, dk_factor, dk_function, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7) RETURNING id"#,
        )
        .bind(&body.code)
        .bind(&body.name)
        .bind(category_id)
        .bind(dk.0)
        .bind(dk.1)
        .bind(dk.2)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from_sqlx)?,
        _ => {
            // marriage（marriage_cert-sales）
            sqlx::query_scalar(
                r#"INSERT INTO isahl."zc_id_prod-marriage_cert-sales"
                   (id, code, notice, ck_category, dk_scene, dk_factor, dk_function, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7) RETURNING id"#,
            )
            .bind(&body.code)
            .bind(&body.name)
            .bind(category_id)
            .bind(dk.0)
            .bind(dk.1)
            .bind(dk.2)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::from_sqlx)?
        }
    };

    // 初始物权凭证（同事务；幂等：同 voucher code 已存在则跳过）
    let voucher_code = format!("TTL-SUBJ{subject_id}-CERT{}", body.code);
    trigger_registry::stock_materialization::create_title_voucher_tx(
        &mut *tx,
        subject_id,
        cert_id,
        &voucher_code,
        body.qty,
        dk,
        user_id,
    )
    .await
    .map_err(|e| ApiError::Internal(format!("初始物权凭证编制失败: {e}")))?;

    tx.commit().await.map_err(ApiError::from_sqlx)?;
    refresh_title_mv(pool).await?;

    Ok(SubjectCertificate {
        id: cert_id,
        kind: leaf.clone(),
        medium: if leaf.contains("digital") {
            "digital".to_string()
        } else {
            "paper".to_string()
        },
        name: body.name.clone(),
        code: body.code.clone(),
        category: Some(body.category_code.clone()),
        net_qty: body.qty,
        voucher_count: 1,
        acquired_at: chrono::Utc::now(),
    })
}

/// 持有证书查询核心：mv_title_ownership × 证书族（纸质/数字化聚合）
pub async fn held(pool: &PgPool, subject_id: i64) -> Result<Vec<SubjectCertificate>, ApiError> {
    let certs: Vec<SubjectCertificate> = sqlx::query_as::<
        _,
        (
            i64,
            String,
            String,
            String,
            Option<String>,
            f64,
            i64,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"SELECT * FROM (
             SELECT t.id, t.tableoid::regclass::text AS kind, t.notice AS name, t.code,
                    cc.notice AS category, o.net_qty::float8, o.voucher_count, o.first_voucher_at
             FROM isahl."zc_id_prod-certificate" t
             JOIN isahl.mv_title_ownership o ON o.production_id = t.id AND o.subject_id = $1 AND o.net_qty > 0
             LEFT JOIN isahl."zc_id_cate-certification" cc ON cc.id = t.ck_category
             WHERE t.deleted_at IS NULL
             UNION ALL
             SELECT t.id, t.tableoid::regclass::text, t.notice, t.code, cc.notice,
                    o.net_qty::float8, o.voucher_count, o.first_voucher_at
             FROM isahl."zc_id_prod-digital_cert-sales" t
             JOIN isahl.mv_title_ownership o ON o.production_id = t.id AND o.subject_id = $1 AND o.net_qty > 0
             LEFT JOIN isahl."zc_id_cate-certification" cc ON cc.id = t.ck_category
             WHERE t.deleted_at IS NULL
             UNION ALL
             SELECT t.id, t.tableoid::regclass::text, t.notice, t.code, cc.notice,
                    o.net_qty::float8, o.voucher_count, o.first_voucher_at
             FROM isahl."zc_id_prod-type_cert-sales" t
             JOIN isahl.mv_title_ownership o ON o.production_id = t.id AND o.subject_id = $1 AND o.net_qty > 0
             LEFT JOIN isahl."zc_id_cate-certification" cc ON cc.id = t.ck_category
             WHERE t.deleted_at IS NULL
             UNION ALL
             SELECT t.id, t.tableoid::regclass::text, t.notice, t.code, cc.notice,
                    o.net_qty::float8, o.voucher_count, o.first_voucher_at
             FROM isahl."zc_id_prod-air_cert-sales" t
             JOIN isahl.mv_title_ownership o ON o.production_id = t.id AND o.subject_id = $1 AND o.net_qty > 0
             LEFT JOIN isahl."zc_id_cate-certification" cc ON cc.id = t.ck_category
             WHERE t.deleted_at IS NULL
             UNION ALL
             SELECT t.id, t.tableoid::regclass::text, t.notice, t.code, cc.notice,
                    o.net_qty::float8, o.voucher_count, o.first_voucher_at
             FROM isahl."zc_id_prod-diploma-sales" t
             JOIN isahl.mv_title_ownership o ON o.production_id = t.id AND o.subject_id = $1 AND o.net_qty > 0
             LEFT JOIN isahl."zc_id_cate-certification" cc ON cc.id = t.ck_category
             WHERE t.deleted_at IS NULL
             UNION ALL
             SELECT t.id, t.tableoid::regclass::text, t.notice, t.code, cc.notice,
                    o.net_qty::float8, o.voucher_count, o.first_voucher_at
             FROM isahl."zc_id_prod-marriage_cert-sales" t
             JOIN isahl.mv_title_ownership o ON o.production_id = t.id AND o.subject_id = $1 AND o.net_qty > 0
             LEFT JOIN isahl."zc_id_cate-certification" cc ON cc.id = t.ck_category
             WHERE t.deleted_at IS NULL
             UNION ALL
             SELECT t.id, t.tableoid::regclass::text, t.notice, t.code, NULL::text,
                    o.net_qty::float8, o.voucher_count, o.first_voucher_at
             FROM isahl."zc_id_prod-license" t
             JOIN isahl.mv_title_ownership o ON o.production_id = t.id AND o.subject_id = $1 AND o.net_qty > 0
             WHERE t.deleted_at IS NULL
             UNION ALL
             SELECT t.id, t.tableoid::regclass::text, t.notice, t.code, NULL::text,
                    o.net_qty::float8, o.voucher_count, o.first_voucher_at
             FROM isahl."zc_id_prod-license-purchase" t
             JOIN isahl.mv_title_ownership o ON o.production_id = t.id AND o.subject_id = $1 AND o.net_qty > 0
             WHERE t.deleted_at IS NULL
             UNION ALL
             SELECT t.id, t.tableoid::regclass::text, t.notice, t.code, NULL::text,
                    o.net_qty::float8, o.voucher_count, o.first_voucher_at
             FROM isahl."zc_id_prod-license-sales" t
             JOIN isahl.mv_title_ownership o ON o.production_id = t.id AND o.subject_id = $1 AND o.net_qty > 0
             WHERE t.deleted_at IS NULL
           ) held
           ORDER BY first_voucher_at DESC"#,
    )
    .bind(subject_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from_sqlx)?
    .into_iter()
    .map(
        |(id, kind, name, code, category, net_qty, voucher_count, acquired_at)| SubjectCertificate {
            id,
            medium: if kind.contains("digital") {
                "digital".into()
            } else {
                "paper".into()
            },
            kind,
            name,
            code,
            category,
            net_qty,
            voucher_count,
            acquired_at,
        },
    )
    .collect();
    Ok(certs)
}

/// 证书取得（POST /subjects/{subject_id}/certificates）
pub async fn acquire_certificate(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<AcquireCertificateRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "create").await?;
    let cert = acquire(pool.get_ref(), subject_id, &body, user_id).await?;
    Ok(HttpResponse::Created().json(ApiResponse::success(cert)))
}

/// 持有证书查询（GET /subjects/{subject_id}/certificates）
pub async fn list_certificates(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", 0, "list").await?;
    let certs = held(pool.get_ref(), subject_id).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(certs)))
}

/// 证书物权路由（挂载于主体域——全 ns 壳可挂）
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/subjects/{subject_id}/certificates")
            .route(web::get().to(list_certificates))
            .route(web::post().to(acquire_certificate)),
    );
}
