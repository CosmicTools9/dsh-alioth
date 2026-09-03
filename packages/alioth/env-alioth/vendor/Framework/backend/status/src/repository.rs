//! 状态 Repository（共享内核）——完整 CRUD + RLS 读覆盖
//!
//! create 依赖列默认 `gen_next_uid(12)`（zc_id_status 表 id 默认实测）。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::{make_repository, AliothRepository, GenericRepository};
use sqlx::{AssertSqlSafe, PgPool};

use crate::models::{
    CreateAccidentRequest, CreateDamageRequest, CreateEventRequest, CreateStatusRequest,
    DamageReport, EventAccident, EventTracking, Status, UpdateAccidentRequest, UpdateDamageRequest,
    UpdateEventRequest, UpdateStatusRequest,
};

#[derive(Clone)]
pub struct StatusRepository {
    generic: GenericRepository<Status>,
}

impl From<PgPool> for StatusRepository {
    fn from(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }
}

impl StatusRepository {
    pub fn new(pool: PgPool) -> Self {
        Self::from(pool)
    }

    /// 获取内部数据库连接池引用
    pub fn pool(&self) -> &PgPool {
        self.generic.pool()
    }
}

#[async_trait]
impl AliothRepository<Status, CreateStatusRequest, UpdateStatusRequest, ApiError>
    for StatusRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Status>, ApiError> {
        self.generic.list(query).await
    }
    async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<PaginatedResponse<Status>, ApiError> {
        self.generic
            .list_with_rls(query, visible_ids, authorized_columns)
            .await
    }

    async fn get(&self, id: i64) -> Result<Option<Status>, ApiError> {
        self.generic.get(id).await
    }
    async fn get_with_rls(
        &self,
        id: i64,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<Option<Status>, ApiError> {
        self.generic
            .get_with_rls(id, visible_ids, authorized_columns)
            .await
    }

    async fn create(&self, req: CreateStatusRequest, user_id: i64) -> Result<Status, ApiError> {
        let p = self.generic.pool();
        sqlx::query_as::<_, Status>(
            r#"INSERT INTO "isahl"."zc_id_status"
               (notice, code, flag, enable, comments, created_by_id)
               VALUES ($1, $2, $3::status_flag, $4, $5, $6)
               RETURNING id, notice, code, flag::text, enable, comments, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.flag)
        .bind(req.enable)
        .bind(&req.comments)
        .bind(user_id)
        .fetch_one(p)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateStatusRequest,
        user_id: i64,
    ) -> Result<Option<Status>, ApiError> {
        let p = self.generic.pool();
        let mut sets: Vec<String> = Vec::new();
        let mut idx: usize = 0;

        if req.notice.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
        if req.code.is_some() {
            idx += 1;
            sets.push(format!("code = ${}", idx));
        }
        if req.flag.is_some() {
            idx += 1;
            sets.push(format!("flag = ${}::status_flag", idx));
        }
        if req.enable.is_some() {
            idx += 1;
            sets.push(format!("enable = ${}", idx));
        }
        if req.comments.is_some() {
            idx += 1;
            sets.push(format!("comments = ${}", idx));
        }

        if sets.is_empty() {
            return self.generic.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_status"
               SET {}
               WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, notice, code, flag::text, enable, comments, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, Status>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.flag {
            q = q.bind(v);
        }
        if let Some(v) = req.enable {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(p).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}

make_repository!(
    DamageReportRepository,
    DamageReport,
    CreateDamageRequest,
    UpdateDamageRequest
);
make_repository!(
    EventTrackingRepository,
    EventTracking,
    CreateEventRequest,
    UpdateEventRequest
);
make_repository!(
    EventAccidentRepository,
    EventAccident,
    CreateAccidentRequest,
    UpdateAccidentRequest
);

#[cfg(test)]
mod not_implemented_tests {
    use actix_web::ResponseError;
    use common::AliothError;

    #[test]
    fn not_implemented_variant_maps_to_501() {
        let err = AliothError::NotImplemented("test".into());
        assert_eq!(err.status_code(), 501);
    }
}
