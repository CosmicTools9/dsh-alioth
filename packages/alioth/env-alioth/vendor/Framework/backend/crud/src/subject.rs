//! 共享 Subject Repository —— 非银行法人主体 (zc_id_orga-non-banking-legal)
//!
//! 为 clients / vendors 等模块提供共享的 CRUD seam。
//! 模块通过实现 `SubjectCreateFields` / `SubjectUpdateFields` trait
//! 将自身的 DTO 接入共享 repository。

use crate::entity::{AliothDbEntity, Identifiable};
use crate::generic_repository::GenericRepository;
use crate::pagination::{ListQuery, PaginatedResponse};
use crate::repository::AliothRepository;
use crate::trigger;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::AliothError;
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, FromRow, PgPool};

const TABLE_NAME: &str = "zc_id_orga-non-banking-legal";

// ─── 共享实体模型 ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Subject {
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    #[serde(with = "common::serde_zuid::opt")]
    pub created_by_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub updated_by_id: Option<i64>,
    #[sqlx(rename = "notice")]
    pub name: Option<String>,
    pub t_color_: Option<String>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub code: Option<String>,
    pub public: Option<bool>,
    #[sqlx(rename = "comments")]
    pub description: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub d_count: Option<i64>,
    pub ak_dimensions: Option<Vec<String>>,
    #[sqlx(rename = "_f_")]
    pub _f_: Option<String>,
    #[sqlx(rename = "_t_")]
    pub _t_: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub dk_scene: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub dk_factor: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub dk_function: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub tpl_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sk_currency: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sk_unit: Option<i64>,
    pub paths: Option<serde_json::Value>,
    #[serde(with = "common::serde_zuid::opt")]
    pub lk_structure: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_country: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_user: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub deleted_by_id: Option<i64>,
}

impl Identifiable for Subject {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for Subject {
    const ENTITY_NAME: &'static str = "Subject";
    fn table_name() -> &'static str {
        r#"isahl."zc_id_orga-non-banking-legal""#
    }
    const SELECT_FIELDS: &'static str = "created_at, updated_at, id, created_by_id, updated_by_id, notice, t_color_, deleted_at, code, public, comments, d_count, ak_dimensions, \"_f_\", \"_t_\", dk_scene, dk_factor, dk_function, tpl_id, sk_currency, ck_category, sk_unit, paths, lk_structure, fk_country, fk_user, deleted_by_id";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = true;
}

// ─── 共享 DTO ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSubjectRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    pub public: Option<bool>,
    pub description: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_country: Option<i64>,
    pub paths: Option<serde_json::Value>,
    #[serde(with = "common::serde_zuid::opt")]
    pub lk_structure: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sk_currency: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sk_unit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_user: Option<i64>,
    /// 本体坐标，由 handler 层从 code 查表转换后注入
    #[serde(default, skip_serializing)]
    #[serde(with = "common::serde_zuid::opt")]
    pub dk_scene: Option<i64>,
    #[serde(default, skip_serializing)]
    #[serde(with = "common::serde_zuid::opt")]
    pub dk_factor: Option<i64>,
    #[serde(default, skip_serializing)]
    #[serde(with = "common::serde_zuid::opt")]
    pub dk_function: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSubjectRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    pub public: Option<bool>,
    pub description: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_country: Option<i64>,
    pub paths: Option<serde_json::Value>,
    #[serde(with = "common::serde_zuid::opt")]
    pub lk_structure: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sk_currency: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub ck_category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub sk_unit: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_user: Option<i64>,
}

// ─── 字段访问 trait ─────────────────────────────────────────────

/// Create 请求 DTO 的字段访问 seam
pub trait SubjectCreateFields: Send + Sync + 'static {
    fn name(&self) -> &Option<String>;
    fn code(&self) -> &Option<String>;
    fn public(&self) -> Option<bool>;
    fn description(&self) -> &Option<String>;
    fn fk_country(&self) -> Option<i64>;
    fn paths(&self) -> &Option<serde_json::Value>;
    fn lk_structure(&self) -> Option<i64>;
    fn sk_currency(&self) -> Option<i64>;
    fn ck_category(&self) -> Option<i64>;
    fn sk_unit(&self) -> Option<i64>;
    fn fk_user(&self) -> Option<i64>;
    /// 本体坐标访问方法（handler 层 resolve code→ID 后注入）
    fn dk_scene(&self) -> Option<i64> {
        None
    }
    fn dk_factor(&self) -> Option<i64> {
        None
    }
    fn dk_function(&self) -> Option<i64> {
        None
    }
}

/// Update 请求 DTO 的字段访问 seam
pub trait SubjectUpdateFields: Send + Sync + 'static {
    fn name(&self) -> &Option<String>;
    fn code(&self) -> &Option<String>;
    fn public(&self) -> Option<bool>;
    fn description(&self) -> &Option<String>;
    fn fk_country(&self) -> Option<i64>;
    fn paths(&self) -> &Option<serde_json::Value>;
    fn lk_structure(&self) -> Option<i64>;
    fn sk_currency(&self) -> Option<i64>;
    fn ck_category(&self) -> Option<i64>;
    fn sk_unit(&self) -> Option<i64>;
    fn fk_user(&self) -> Option<i64>;
}

// ─── 为共享 DTO 实现字段访问 trait ─────────────────────────────

impl SubjectCreateFields for CreateSubjectRequest {
    fn name(&self) -> &Option<String> {
        &self.name
    }
    fn code(&self) -> &Option<String> {
        &self.code
    }
    fn public(&self) -> Option<bool> {
        self.public
    }
    fn description(&self) -> &Option<String> {
        &self.description
    }
    fn fk_country(&self) -> Option<i64> {
        self.fk_country
    }
    fn paths(&self) -> &Option<serde_json::Value> {
        &self.paths
    }
    fn lk_structure(&self) -> Option<i64> {
        self.lk_structure
    }
    fn sk_currency(&self) -> Option<i64> {
        self.sk_currency
    }
    fn ck_category(&self) -> Option<i64> {
        self.ck_category
    }
    fn sk_unit(&self) -> Option<i64> {
        self.sk_unit
    }
    fn fk_user(&self) -> Option<i64> {
        self.fk_user
    }
    fn dk_scene(&self) -> Option<i64> {
        self.dk_scene
    }
    fn dk_factor(&self) -> Option<i64> {
        self.dk_factor
    }
    fn dk_function(&self) -> Option<i64> {
        self.dk_function
    }
}

impl SubjectUpdateFields for UpdateSubjectRequest {
    fn name(&self) -> &Option<String> {
        &self.name
    }
    fn code(&self) -> &Option<String> {
        &self.code
    }
    fn public(&self) -> Option<bool> {
        self.public
    }
    fn description(&self) -> &Option<String> {
        &self.description
    }
    fn fk_country(&self) -> Option<i64> {
        self.fk_country
    }
    fn paths(&self) -> &Option<serde_json::Value> {
        &self.paths
    }
    fn lk_structure(&self) -> Option<i64> {
        self.lk_structure
    }
    fn sk_currency(&self) -> Option<i64> {
        self.sk_currency
    }
    fn ck_category(&self) -> Option<i64> {
        self.ck_category
    }
    fn sk_unit(&self) -> Option<i64> {
        self.sk_unit
    }
    fn fk_user(&self) -> Option<i64> {
        self.fk_user
    }
}

// ─── 共享 Repository ────────────────────────────────────────────

pub struct SubjectRepository<E: AliothDbEntity, C, U> {
    generic: GenericRepository<E>,
    _phantom_c: std::marker::PhantomData<C>,
    _phantom_u: std::marker::PhantomData<U>,
}

impl<E: AliothDbEntity, C, U> Clone for SubjectRepository<E, C, U> {
    fn clone(&self) -> Self {
        Self {
            generic: GenericRepository::new(self.generic.pool().clone()),
            _phantom_c: std::marker::PhantomData,
            _phantom_u: std::marker::PhantomData,
        }
    }
}

impl<E: AliothDbEntity, C, U> SubjectRepository<E, C, U> {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
            _phantom_c: std::marker::PhantomData,
            _phantom_u: std::marker::PhantomData,
        }
    }
}

impl<E: AliothDbEntity, C, U> From<PgPool> for SubjectRepository<E, C, U> {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl<E, C, U, Err> AliothRepository<E, C, U, Err> for SubjectRepository<E, C, U>
where
    E: AliothDbEntity + 'static,
    C: SubjectCreateFields,
    U: SubjectUpdateFields,
    Err: std::error::Error + From<sqlx::Error> + From<AliothError> + Send + Sync + 'static,
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<E>, Err> {
        self.generic.list(query).await.map_err(|e| e.into())
    }

    async fn get(&self, id: i64) -> Result<Option<E>, Err> {
        self.generic.get(id).await.map_err(|e| e.into())
    }

    async fn create(&self, req: C, user_id: i64) -> Result<E, Err> {
        let mut tx = self
            .generic
            .pool()
            .begin()
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;

        let sql = format!(
            r#"INSERT INTO {}
               (notice, code, public, comments,
                fk_country, paths, lk_structure, sk_currency, ck_category, sk_unit,
                dk_scene, dk_factor, dk_function,
                fk_user, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
               RETURNING {}"#,
            E::table_name(),
            E::SELECT_FIELDS
        );
        let item = sqlx::query_as::<_, E>(AssertSqlSafe(sql.as_str()))
            .bind(req.name())
            .bind(req.code())
            .bind(req.public())
            .bind(req.description())
            .bind(req.fk_country())
            .bind(req.paths())
            .bind(req.lk_structure())
            .bind(req.sk_currency())
            .bind(req.ck_category())
            .bind(req.sk_unit())
            .bind(req.dk_scene())
            .bind(req.dk_factor())
            .bind(req.dk_function())
            .bind(req.fk_user())
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;

        let result_map =
            trigger::to_record(&item).map_err(|e| AliothError::Database(e.to_string()))?;
        let _ = trigger::execute_after_insert(
            self.generic.pool(),
            TABLE_NAME,
            &result_map,
            Some(user_id),
        )
        .await;

        Ok(item)
    }

    async fn update(&self, id: i64, req: U, user_id: i64) -> Result<Option<E>, Err> {
        let mut tx = self
            .generic
            .pool()
            .begin()
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;

        let sql = format!(
            r#"
            UPDATE {}
            SET notice = COALESCE($1, notice),
                code = COALESCE($2, code),
                public = COALESCE($3, public),
                comments = COALESCE($4, comments),
                fk_country = COALESCE($5, fk_country),
                paths = COALESCE($6, paths),
                lk_structure = COALESCE($7, lk_structure),
                sk_currency = COALESCE($8, sk_currency),
                ck_category = COALESCE($9, ck_category),
                sk_unit = COALESCE($10, sk_unit),
                fk_user = COALESCE($11, fk_user),
                updated_at = NOW(),
                updated_by_id = $12
            WHERE id = $13 AND deleted_at IS NULL
            RETURNING {}"#,
            E::table_name(),
            E::SELECT_FIELDS
        );
        let item = sqlx::query_as::<_, E>(AssertSqlSafe(sql.as_str()))
            .bind(req.name())
            .bind(req.code())
            .bind(req.public())
            .bind(req.description())
            .bind(req.fk_country())
            .bind(req.paths())
            .bind(req.lk_structure())
            .bind(req.sk_currency())
            .bind(req.ck_category())
            .bind(req.sk_unit())
            .bind(req.fk_user())
            .bind(user_id)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?;

        if let Some(ref updated) = item {
            tx.commit()
                .await
                .map_err(|e| AliothError::Database(e.to_string()))?;

            let result_map =
                trigger::to_record(updated).map_err(|e| AliothError::Database(e.to_string()))?;
            let _ = trigger::execute_after_update(
                self.generic.pool(),
                TABLE_NAME,
                None,
                &result_map,
                Some(user_id),
            )
            .await;
        } else {
            let _ = tx.rollback().await;
        }

        Ok(item)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), Err> {
        self.generic.delete(id, user_id).await.map_err(|e| e.into())
    }
}
