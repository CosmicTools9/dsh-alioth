//! 主体银行卡（银行账户）管理
//!
//! 路由：/service/isahl-db/subjects/{id}/bank-cards
//! 存储：isahl."zc_id_stor-acc-bank"（账户-银行，交易媒介类目；模型中心元数据实名）
//! 归属：isahl."zc_id_subjects_rr_account"（关联-主体↔账户；subjects.account m2n 引用桥）
//!
//! 字段映射（用户裁决 2026-08-29，change: remap-subject-bank-invoice-isahl）：
//! - 账户名（户名） name   → name
//! - 账户号         code   → code
//! - 账户（显示）   notice → `{户名}:{账号掩码}`（如 张三:1234***7890，写侧物化）
//! - 开户机构       fk_trustee → zc_id_subjects 主体行（银行主体；可空）
//! - 归属主体       rr_account 桥（ref_left=主体, ref_right=账户行）
//!
//! 「默认卡」「联行号」「MDM 编码」概念已抹除——isahl 模型无此语义。

use actix_web::{web, HttpRequest, HttpResponse};
use common::context::require_auth;
use common::data::ApiResponse;
use common::permissions::require_resource_access;
use common::AliothError as ApiError;
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/subjects/{id}/bank-cards")
            .route(web::get().to(list_subject_bank_cards))
            .route(web::post().to(create_subject_bank_card)),
    )
    .service(
        web::resource("/subjects/{id}/bank-cards/{cardId}")
            .route(web::put().to(update_subject_bank_card))
            .route(web::delete().to(delete_subject_bank_card)),
    );
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SubjectBankCard {
    #[serde(with = "i64_string")]
    pub id: i64,
    #[serde(with = "i64_string")]
    pub subject_id: i64,
    /// 账户名/户名（zc_id_stor-acc-bank.name）
    pub name: String,
    /// 账户号（zc_id_stor-acc-bank.code，同主体内唯一）
    pub account: String,
    /// 账户显示串（zc_id_stor-acc-bank.notice，`户名:1234***7890` 掩码物化）
    pub masked: String,
    /// 开户机构名称（fk_trustee → zc_id_subj-bank.notice 派生）
    pub bank_name: Option<String>,
    /// 开户机构主体 id（fk_trustee）
    #[serde(with = "i64_string")]
    pub trustee_id: i64,
    /// 联行号（开户机构行 zc_id_subj-bank.code 派生）
    pub bank_line_no: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertBankCardRequest {
    /// 账户名/户名（必填）
    pub name: String,
    /// 账户号（必填，同主体内唯一）
    pub account: String,
    /// 开户机构主体 id（zc_id_subjects 行；必填）
    #[serde(with = "common::serde_zuid")]
    pub trustee_id: i64,
}

mod i64_string {
    use serde::{Deserialize, Serializer};

    pub fn serialize<S: Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        let s = String::deserialize(d)?;
        s.parse::<i64>().map_err(serde::de::Error::custom)
    }
}

/// 账号掩码：长度 > 8 时前 4 + `***` + 后 4，否则原样
fn mask_account(account: &str) -> String {
    let chars: Vec<char> = account.chars().collect();
    if chars.len() > 8 {
        format!(
            "{}***{}",
            chars[..4].iter().collect::<String>(),
            chars[chars.len() - 4..].iter().collect::<String>()
        )
    } else {
        account.to_string()
    }
}

/// notice 物化：`{户名}:{账号掩码}`
fn build_notice(name: &str, account: &str) -> String {
    format!("{}:{}", name.trim(), mask_account(account.trim()))
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

/// 开户机构存在性校验（未删除的主体行）
async fn ensure_trustee(pool: &PgPool, trustee_id: i64) -> Result<(), ApiError> {
    let exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(SELECT 1 FROM "isahl"."zc_id_subjects" WHERE id = $1 AND deleted_at IS NULL)"#,
    )
    .bind(trustee_id)
    .fetch_one(pool)
    .await?;
    if !exists {
        return Err(ApiError::NotFound(format!(
            "开户机构主体不存在: {}",
            trustee_id
        )));
    }
    Ok(())
}

/// 同主体账号唯一校验（桥关联的未软删账户行间；exclude_card_id 更新时排除自身）
async fn account_conflict(
    pool: &PgPool,
    subject_id: i64,
    account: &str,
    exclude_card_id: i64,
) -> Result<bool, ApiError> {
    let exists = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM "isahl"."zc_id_subjects_rr_account" r
            JOIN "isahl"."zc_id_stor-acc-bank" b ON b.id = r.ref_right AND b.deleted_at IS NULL
            WHERE r.ref_left = $1 AND r.deleted_at IS NULL
              AND b.code = $2 AND b.id <> $3
        )
        "#,
    )
    .bind(subject_id)
    .bind(account)
    .bind(exclude_card_id)
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

const BANK_CARD_FIELDS: &str = "b.id, r.ref_left AS subject_id, COALESCE(b.name, '') AS name, \
     COALESCE(b.code, '') AS account, COALESCE(b.notice, '') AS masked, \
     tb.notice AS bank_name, b.fk_trustee AS trustee_id, tb.code AS bank_line_no";

const BANK_CARD_FROM: &str = "\"isahl\".\"zc_id_stor-acc-bank\" b \
     JOIN \"isahl\".\"zc_id_subjects_rr_account\" r ON r.ref_right = b.id AND r.deleted_at IS NULL \
     LEFT JOIN \"isahl\".\"zc_id_subjects\" tb ON tb.id = b.fk_trustee AND tb.deleted_at IS NULL";

pub async fn list_subject_bank_cards(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "read").await?;

    let sql = format!(
        "SELECT {} FROM {} \
         WHERE r.ref_left = $1 AND b.deleted_at IS NULL \
         ORDER BY b.id DESC",
        BANK_CARD_FIELDS, BANK_CARD_FROM
    );
    let rows = sqlx::query_as::<_, SubjectBankCard>(AssertSqlSafe(sql.as_str()))
        .bind(subject_id)
        .fetch_all(pool.get_ref())
        .await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(rows)))
}

/// 单事务：账户行 INSERT + 归属桥 INSERT（原子落库）
pub async fn create_subject_bank_card(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<i64>,
    body: web::Json<UpsertBankCardRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let subject_id = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "create").await?;

    if body.name.trim().is_empty() {
        return Err(ApiError::BadRequest("账户名不能为空".into()));
    }
    if body.account.trim().is_empty() {
        return Err(ApiError::BadRequest("账户号不能为空".into()));
    }
    ensure_trustee(pool.get_ref(), body.trustee_id).await?;
    ensure_subject(pool.get_ref(), subject_id).await?;
    if account_conflict(pool.get_ref(), subject_id, &body.account, 0).await? {
        return Err(ApiError::BadRequest(format!(
            "主体下已存在账户号 {}，请确认",
            body.account
        )));
    }

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;
    let notice = build_notice(&body.name, &body.account);
    let card_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_stor-acc-bank" (name, code, notice, fk_trustee, created_by_id, updated_by_id)
           VALUES ($1, $2, $3, $4, $5, $5) RETURNING id"#,
    )
    .bind(body.name.trim())
    .bind(body.account.trim())
    .bind(&notice)
    .bind(body.trustee_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_subjects_rr_account" (notice, ref_left, ref_right, created_by_id, updated_by_id)
           VALUES ($1, $2, $3, $4, $4)"#,
    )
    .bind(format!("subject-{} bank-card", subject_id))
    .bind(subject_id)
    .bind(card_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    let sql = format!(
        "SELECT {} FROM {} WHERE b.id = $1 AND b.deleted_at IS NULL",
        BANK_CARD_FIELDS, BANK_CARD_FROM
    );
    let row = sqlx::query_as::<_, SubjectBankCard>(AssertSqlSafe(sql.as_str()))
        .bind(card_id)
        .fetch_one(pool.get_ref())
        .await?;
    Ok(HttpResponse::Created().json(ApiResponse::success(row)))
}

/// 单事务：账户行 UPDATE + notice 掩码重物化（归属桥不变）
pub async fn update_subject_bank_card(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
    body: web::Json<UpsertBankCardRequest>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (subject_id, card_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "update").await?;

    if body.name.trim().is_empty() {
        return Err(ApiError::BadRequest("账户名不能为空".into()));
    }
    if body.account.trim().is_empty() {
        return Err(ApiError::BadRequest("账户号不能为空".into()));
    }
    ensure_trustee(pool.get_ref(), body.trustee_id).await?;
    if account_conflict(pool.get_ref(), subject_id, &body.account, card_id).await? {
        return Err(ApiError::BadRequest(format!(
            "主体下已存在账户号 {}，请确认",
            body.account
        )));
    }

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;
    let notice = build_notice(&body.name, &body.account);
    let updated: Option<i64> = sqlx::query_scalar(
        r#"UPDATE "isahl"."zc_id_stor-acc-bank" b
           SET name = $1, code = $2, notice = $3, fk_trustee = $4, updated_at = now(), updated_by_id = $5
           FROM "isahl"."zc_id_subjects_rr_account" r
           WHERE b.id = r.ref_right AND r.deleted_at IS NULL
             AND b.id = $6 AND r.ref_left = $7 AND b.deleted_at IS NULL
           RETURNING b.id"#,
    )
    .bind(body.name.trim())
    .bind(body.account.trim())
    .bind(&notice)
    .bind(body.trustee_id)
    .bind(user_id)
    .bind(card_id)
    .bind(subject_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;

    let Some(_) = updated else {
        return Err(ApiError::NotFound(format!("银行卡不存在: {}", card_id)));
    };
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    let sql = format!(
        "SELECT {} FROM {} WHERE b.id = $1 AND b.deleted_at IS NULL",
        BANK_CARD_FIELDS, BANK_CARD_FROM
    );
    let row = sqlx::query_as::<_, SubjectBankCard>(AssertSqlSafe(sql.as_str()))
        .bind(card_id)
        .fetch_one(pool.get_ref())
        .await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(row)))
}

/// 软删：账户行 + 归属桥同时软删（同主体守卫在桥上）
pub async fn delete_subject_bank_card(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(i64, i64)>,
) -> Result<HttpResponse, ApiError> {
    let user_id = require_auth(&req)?;
    let (subject_id, card_id) = path.into_inner();
    require_resource_access(pool.get_ref(), user_id, "identities", subject_id, "delete").await?;

    let mut tx = pool.begin().await.map_err(ApiError::from_sqlx)?;
    let bridge = sqlx::query(
        r#"UPDATE "isahl"."zc_id_subjects_rr_account" r
           SET deleted_at = now(), deleted_by_id = $2, updated_at = now()
           WHERE r.ref_left = $1 AND r.ref_right = $3 AND r.deleted_at IS NULL"#,
    )
    .bind(subject_id)
    .bind(user_id)
    .bind(card_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?
    .rows_affected();

    if bridge == 0 {
        return Err(ApiError::NotFound(format!("银行卡不存在: {}", card_id)));
    }
    sqlx::query(
        r#"UPDATE "isahl"."zc_id_stor-acc-bank" b
           SET deleted_at = now(), deleted_by_id = $1, updated_at = now()
           FROM "isahl"."zc_id_subjects_rr_account" r
           WHERE b.id = r.ref_right AND b.id = $2 AND b.deleted_at IS NULL"#,
    )
    .bind(user_id)
    .bind(card_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from_sqlx)?;
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    Ok(HttpResponse::NoContent().finish())
}
