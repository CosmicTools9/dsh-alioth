//! 主体开票信息——主体引用面组合投影
//!
//! 路由：/service/isahl-db/subjects/{id}/invoice-info（只读，单对象）
//!
//! 本体映射（change: remap-subject-bank-invoice-isahl，元数据实证）：
//! - 公司名 company_name ← 主体行 notice（发票抬头即主体本身，发票 fk_sender/fk_recipient 指主体）
//! - 税号 tax_no ← `zc_id_identity`（BUSINESS_LICENSE 营业执照，经 zc_id_entity_rr_identity 桥）证照号
//! - 电话 tel ← contacts 链路 `zc_id_info-telephone.notice`
//! - 开户行/账号 ← 最近一张 `zc_id_subj-bank`（comments / o_number）
//! - 地址 addr ← 恒 null（`zc_id_subjects_rr_place` 实名「主体↔储位」，非通讯地址，模型无落点）
//!
//! 无独立持久化实体、无写端点——六件套各归其主（主体编辑/证照/联系人/银行卡）。
//! 「默认开票信息」概念已随默认卡一并抹除。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::Serialize;
use sqlx::PgPool;

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/subjects/{id}/invoice-info").route(web::get().to(get_subject_invoice_info)),
    );
}

#[derive(Debug, Serialize)]
pub struct SubjectInvoiceInfo {
    /// 主体 id（投影归属，zuid 字符串）
    #[serde(with = "i64_string")]
    pub id: i64,
    #[serde(with = "i64_string")]
    pub subject_id: i64,
    /// 公司名称（主体 notice）
    pub company_name: String,
    /// 税号（BUSINESS_LICENSE 证照号）
    pub tax_no: Option<String>,
    /// 地址（模型无落点，恒 null；deprecated）
    pub addr: Option<String>,
    /// 电话（contacts 链路 telephone）
    pub tel: Option<String>,
    /// 开户银行名称（最近 subj-bank 行 comments）
    pub bank_name: Option<String>,
    /// 银行账号（最近 subj-bank 行 o_number）
    pub account: Option<String>,
}

mod i64_string {
    use serde::Serializer;

    pub fn serialize<S: Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }
}

async fn ensure_subject(pool: &PgPool, subject_id: i64) -> Result<(), ApiError> {
    let exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(SELECT 1 FROM "isahl"."zc_id_subjects" WHERE id = $1 AND deleted_at IS NULL)"#,
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await?;
    if !exists {
        return Err(ApiError::NotFound(format!("主体不存在: {}", subject_id)));
    }
    Ok(())
}

/// 最新营业执照证照号（税号载体）
async fn latest_tax_no(pool: &PgPool, subject_id: i64) -> Result<Option<String>, ApiError> {
    let v: Option<String> = sqlx::query_scalar(
        r#"SELECT i.identity FROM "isahl"."zc_id_entity_rr_identity" r
           JOIN "isahl"."zc_id_identity" i ON i.id = r.ref_right AND i.deleted_at IS NULL
           JOIN "isahl"."zc_id_cate-identity" c ON c.id = i.ck_category AND c.deleted_at IS NULL
           WHERE r.ref_left = $1 AND r.deleted_at IS NULL AND c.code = 'BUSINESS_LICENSE'
           ORDER BY i.id DESC LIMIT 1"#,
    )
    .bind(subject_id)
    .fetch_optional(pool)
    .await?;
    Ok(v)
}

/// 最新电话（contacts 链路：主体 → 联系人 → 联系方式 → 电话叶表）
async fn latest_tel(pool: &PgPool, subject_id: i64) -> Result<Option<String>, ApiError> {
    let v: Option<String> = sqlx::query_scalar(
        r#"SELECT t.notice FROM "isahl"."zc_id_entity_rr_contacts" rc
           JOIN "isahl"."zc_id_contacts" ct ON ct.id = rc.ref_right AND ct.deleted_at IS NULL
           JOIN "isahl"."zc_id_contacts_rr_infos" ri ON ri.ref_left = ct.id AND ri.deleted_at IS NULL
           JOIN "isahl"."zc_id_info-telephone" t ON t.id = ri.ref_right AND t.deleted_at IS NULL
           WHERE rc.ref_left = $1 AND rc.deleted_at IS NULL
           ORDER BY t.id DESC LIMIT 1"#,
    )
    .bind(subject_id)
    .fetch_optional(pool)
    .await?;
    Ok(v)
}
/// 最近银行账户（经 rr_account 桥：账号=code，开户机构=fk_trustee 主体名）
async fn latest_bank(
    pool: &PgPool,
    subject_id: i64,
) -> Result<Option<(Option<String>, Option<String>)>, ApiError> {
    let v: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"SELECT tb.notice, b.code FROM "isahl"."zc_id_stor-acc-bank" b
           JOIN "isahl"."zc_id_subjects_rr_account" r ON r.ref_right = b.id AND r.deleted_at IS NULL
           LEFT JOIN "isahl"."zc_id_subjects" tb ON tb.id = b.fk_trustee AND tb.deleted_at IS NULL
           WHERE r.ref_left = $1 AND b.deleted_at IS NULL
           ORDER BY b.id DESC LIMIT 1"#,
    )
    .bind(subject_id)
    .fetch_optional(pool)
    .await?;
    Ok(v)
}

pub async fn get_subject_invoice_info(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "read").await?;
    ensure_subject(pool.get_ref(), subject_id).await?;

    let company_name: Option<String> = sqlx::query_scalar(
        r#"SELECT notice FROM "isahl"."zc_id_subjects" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .fetch_one(pool.get_ref())
    .await?;

    let tax_no = latest_tax_no(pool.get_ref(), subject_id).await?;
    let tel = latest_tel(pool.get_ref(), subject_id).await?;
    let bank = latest_bank(pool.get_ref(), subject_id).await?;
    let (bank_name, account) = bank.unwrap_or((None, None));

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(SubjectInvoiceInfo {
            id: subject_id,
            subject_id,
            company_name: company_name.unwrap_or_default(),
            tax_no,
            addr: None,
            tel,
            bank_name,
            account,
        })),
    )
}
