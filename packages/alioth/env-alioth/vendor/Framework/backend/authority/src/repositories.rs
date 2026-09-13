use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::error::AliothError;
use crud::query_builder::QueryBuilder;
use crud::repository::AliothRepository;
use sqlx::PgPool;

use super::models::{
    ApprovalRole, Approver, CreateApprovalRoleRequest, CreateApproverRequest,
    CreateEmployeeRequest, CreateSkillTagRequest, Employee, SkillTag, UpdateApprovalRoleRequest,
    UpdateApproverRequest, UpdateEmployeeRequest, UpdateSkillTagRequest,
};

// ── EmployeeRepository ────────────────────────────────────────

#[derive(Clone)]
pub struct EmployeeRepository {
    pool: PgPool,
}

impl EmployeeRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// List with optional RLS filtering.
    pub async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
    ) -> Result<PaginatedResponse<Employee>, AliothError> {
        match visible_ids {
            None => self.list(query).await,
            Some([]) => Ok(PaginatedResponse {
                items: vec![],
                total: 0,
                page: query.page,
                page_size: query.page_size,
            }),
            Some(ids) => {
                let page_size = query.page_size.clamp(1, 500);
                let offset = (query.page.max(1) - 1) * page_size;
                let rows = sqlx::query_as::<_, Employee>(
                    "SELECT id, notice AS name, code, fk_user, sk_currency, ck_category, \
                     sk_unit, created_at, updated_at, deleted_at \
                     FROM isahl.\"zc_id_subj-employee\" \
                     WHERE deleted_at IS NULL AND id = ANY($1::BIGINT[]) \
                     ORDER BY id DESC LIMIT $2 OFFSET $3",
                )
                .bind(ids.to_vec())
                .bind(page_size)
                .bind(offset)
                .fetch_all(&self.pool)
                .await
                .map_err(AliothError::from)?;
                Ok(PaginatedResponse {
                    items: rows,
                    total: ids.len() as i64,
                    page: query.page,
                    page_size,
                })
            }
        }
    }
}

#[async_trait]
impl AliothRepository<Employee, CreateEmployeeRequest, UpdateEmployeeRequest, AliothError>
    for EmployeeRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Employee>, AliothError> {
        QueryBuilder::<Employee>::from_list_query(&self.pool, query)
            .fetch(query.page, query.page_size)
            .await
    }

    async fn get(&self, id: i64) -> Result<Option<Employee>, AliothError> {
        sqlx::query_as::<_, Employee>(
            "SELECT id, notice AS name, code, fk_user, sk_currency, ck_category, \
             sk_unit, created_at, updated_at, deleted_at \
             FROM isahl.\"zc_id_subj-employee\" WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn create(
        &self,
        req: CreateEmployeeRequest,
        user_id: i64,
    ) -> Result<Employee, AliothError> {
        sqlx::query_as::<_, Employee>(
            r#"INSERT INTO isahl."zc_id_empl-natural"
               (notice, code, fk_user, sk_currency, ck_category, sk_unit, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING id, notice AS name, code, fk_user, sk_currency, ck_category, sk_unit,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(req.fk_user)
        .bind(req.sk_currency)
        .bind(req.role)
        .bind(req.team)
        .bind(user_id)
        .bind(514i64)
        .bind(529i64)
        .bind(527i64)
        .fetch_one(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateEmployeeRequest,
        user_id: i64,
    ) -> Result<Option<Employee>, AliothError> {
        let current = self.get(id).await?;
        if current.is_none() {
            return Ok(None);
        }
        let current = current.unwrap();
        let name = req.name.unwrap_or(current.name);
        let code = req.code.or(current.code);
        let fk_user = req.fk_user.or(current.fk_user);
        let sk_currency = req.sk_currency.or(current.sk_currency);
        let ck_category = req.role.or(current.ck_category);
        let sk_unit = req.team.or(current.sk_unit);
        sqlx::query_as::<_, Employee>(
            r#"UPDATE isahl."zc_id_subj-employee"
               SET notice = $1, code = $2, fk_user = $3, sk_currency = $4,
                   ck_category = $5, sk_unit = $6, updated_by_id = $7
               WHERE id = $8 AND deleted_at IS NULL
               RETURNING id, notice AS name, code, fk_user, sk_currency, ck_category, sk_unit,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&name)
        .bind(&code)
        .bind(fk_user)
        .bind(sk_currency)
        .bind(ck_category)
        .bind(sk_unit)
        .bind(user_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        sqlx::query(
            "UPDATE isahl.\"zc_id_subj-employee\" SET deleted_at = NOW(), deleted_by_id = $1 \
             WHERE id = $2 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(AliothError::from)?;
        Ok(())
    }
}

// ── SkillTagRepository ────────────────────────────────────────

#[derive(Clone)]
pub struct SkillTagRepository {
    pool: PgPool,
}

impl SkillTagRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// List with optional RLS filtering.
    pub async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
    ) -> Result<PaginatedResponse<SkillTag>, AliothError> {
        match visible_ids {
            None => self.list(query).await,
            Some([]) => Ok(PaginatedResponse {
                items: vec![],
                total: 0,
                page: query.page,
                page_size: query.page_size,
            }),
            Some(ids) => {
                let page_size = query.page_size.clamp(1, 500);
                let offset = (query.page.max(1) - 1) * page_size;
                let rows = sqlx::query_as::<_, SkillTag>(
                    "SELECT id, notice AS name, code, v_group, \
                     created_at, updated_at, deleted_at \
                     FROM isahl.\"zc_id_tags-skill\" \
                     WHERE deleted_at IS NULL AND id = ANY($1::BIGINT[]) \
                     ORDER BY id DESC LIMIT $2 OFFSET $3",
                )
                .bind(ids.to_vec())
                .bind(page_size)
                .bind(offset)
                .fetch_all(&self.pool)
                .await
                .map_err(AliothError::from)?;
                Ok(PaginatedResponse {
                    items: rows,
                    total: ids.len() as i64,
                    page: query.page,
                    page_size,
                })
            }
        }
    }
}

#[async_trait]
impl AliothRepository<SkillTag, CreateSkillTagRequest, UpdateSkillTagRequest, AliothError>
    for SkillTagRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<SkillTag>, AliothError> {
        QueryBuilder::<SkillTag>::from_list_query(&self.pool, query)
            .fetch(query.page, query.page_size)
            .await
    }

    async fn get(&self, id: i64) -> Result<Option<SkillTag>, AliothError> {
        sqlx::query_as::<_, SkillTag>(
            "SELECT id, notice AS name, code, v_group, \
             created_at, updated_at, deleted_at \
             FROM isahl.\"zc_id_tags-skill\" WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn create(
        &self,
        req: CreateSkillTagRequest,
        user_id: i64,
    ) -> Result<SkillTag, AliothError> {
        sqlx::query_as::<_, SkillTag>(
            r#"INSERT INTO isahl."zc_id_tags-skill"
               (notice, code, v_group, created_by_id)
               VALUES ($1, $2, $3, $4)
               RETURNING id, notice AS name, code, v_group,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(&req.category)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateSkillTagRequest,
        user_id: i64,
    ) -> Result<Option<SkillTag>, AliothError> {
        let current = self.get(id).await?;
        if current.is_none() {
            return Ok(None);
        }
        let current = current.unwrap();
        let name = req.name.unwrap_or(current.name);
        let code = req.code.or(current.code);
        let v_group = req.category.or(current.v_group);
        sqlx::query_as::<_, SkillTag>(
            r#"UPDATE isahl."zc_id_tags-skill"
               SET notice = $1, code = $2, v_group = $3, updated_by_id = $4
               WHERE id = $5 AND deleted_at IS NULL
               RETURNING id, notice AS name, code, v_group,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&name)
        .bind(&code)
        .bind(&v_group)
        .bind(user_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        sqlx::query(
            "UPDATE isahl.\"zc_id_tags-skill\" SET deleted_at = NOW(), deleted_by_id = $1 WHERE id = $2 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(AliothError::from)?;
        Ok(())
    }
}

impl From<PgPool> for SkillTagRepository {
    fn from(pool: PgPool) -> Self {
        Self { pool }
    }
}

// ── ApprovalRoleRepository ────────────────────────────────────

#[derive(Clone)]
pub struct ApprovalRoleRepository {
    pool: PgPool,
}

impl ApprovalRoleRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// List with optional RLS filtering.
    pub async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
    ) -> Result<PaginatedResponse<ApprovalRole>, AliothError> {
        match visible_ids {
            None => self.list(query).await,
            Some([]) => Ok(PaginatedResponse {
                items: vec![],
                total: 0,
                page: query.page,
                page_size: query.page_size,
            }),
            Some(ids) => {
                let page_size = query.page_size.clamp(1, 500);
                let offset = (query.page.max(1) - 1) * page_size;
                let rows = sqlx::query_as::<_, ApprovalRole>(
                    "SELECT id, notice AS name, \
                     created_at, updated_at, deleted_at \
                     FROM isahl.\"zc_id_cate-approve_role\" \
                     WHERE deleted_at IS NULL AND id = ANY($1::BIGINT[]) \
                     ORDER BY id DESC LIMIT $2 OFFSET $3",
                )
                .bind(ids.to_vec())
                .bind(page_size)
                .bind(offset)
                .fetch_all(&self.pool)
                .await
                .map_err(AliothError::from)?;
                Ok(PaginatedResponse {
                    items: rows,
                    total: ids.len() as i64,
                    page: query.page,
                    page_size,
                })
            }
        }
    }
}

#[async_trait]
impl
    AliothRepository<
        ApprovalRole,
        CreateApprovalRoleRequest,
        UpdateApprovalRoleRequest,
        AliothError,
    > for ApprovalRoleRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<ApprovalRole>, AliothError> {
        QueryBuilder::<ApprovalRole>::from_list_query(&self.pool, query)
            .fetch(query.page, query.page_size)
            .await
    }

    async fn get(&self, id: i64) -> Result<Option<ApprovalRole>, AliothError> {
        sqlx::query_as::<_, ApprovalRole>(
            "SELECT id, notice AS name, \
             created_at, updated_at, deleted_at \
             FROM isahl.\"zc_id_cate-approve_role\" WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn create(
        &self,
        req: CreateApprovalRoleRequest,
        user_id: i64,
    ) -> Result<ApprovalRole, AliothError> {
        sqlx::query_as::<_, ApprovalRole>(
            r#"INSERT INTO isahl."zc_id_cate-approve_role"
               (notice, created_by_id)
               VALUES ($1, $2)
               RETURNING id, notice AS name,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateApprovalRoleRequest,
        user_id: i64,
    ) -> Result<Option<ApprovalRole>, AliothError> {
        let current = self.get(id).await?;
        if current.is_none() {
            return Ok(None);
        }
        let current = current.unwrap();
        let name = req.name.unwrap_or(current.name);
        sqlx::query_as::<_, ApprovalRole>(
            r#"UPDATE isahl."zc_id_cate-approve_role"
               SET notice = $1, updated_by_id = $2
               WHERE id = $3 AND deleted_at IS NULL
               RETURNING id, notice AS name,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&name)
        .bind(user_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        sqlx::query(
            "UPDATE isahl.\"zc_id_cate-approve_role\" SET deleted_at = NOW(), deleted_by_id = $1 \
             WHERE id = $2 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(AliothError::from)?;
        Ok(())
    }
}

impl From<PgPool> for ApprovalRoleRepository {
    fn from(pool: PgPool) -> Self {
        Self { pool }
    }
}

// ── ApproverRepository ───────────────────────────────────────

#[derive(Clone)]
pub struct ApproverRepository {
    pool: PgPool,
}

impl ApproverRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// List with optional RLS filtering.
    pub async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
    ) -> Result<PaginatedResponse<Approver>, AliothError> {
        match visible_ids {
            None => self.list(query).await,
            Some([]) => Ok(PaginatedResponse {
                items: vec![],
                total: 0,
                page: query.page,
                page_size: query.page_size,
            }),
            Some(ids) => {
                let page_size = query.page_size.clamp(1, 500);
                let offset = (query.page.max(1) - 1) * page_size;
                let rows = sqlx::query_as::<_, Approver>(
                    "SELECT id, notice AS name, fk_user, ck_category, comments AS description, \
                     created_at, updated_at, deleted_at \
                     FROM isahl.\"zc_id_subj-position\" \
                     WHERE deleted_at IS NULL AND _f_ IS NULL AND id = ANY($1::BIGINT[]) \
                     ORDER BY id DESC LIMIT $2 OFFSET $3",
                )
                .bind(ids.to_vec())
                .bind(page_size)
                .bind(offset)
                .fetch_all(&self.pool)
                .await
                .map_err(AliothError::from)?;
                Ok(PaginatedResponse {
                    items: rows,
                    total: ids.len() as i64,
                    page: query.page,
                    page_size,
                })
            }
        }
    }
}

#[async_trait]
impl AliothRepository<Approver, CreateApproverRequest, UpdateApproverRequest, AliothError>
    for ApproverRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Approver>, AliothError> {
        // D-2a：_f_ IS NULL 排除编制范例行（真实岗位视图，同 identity-org 岗位读径）
        QueryBuilder::<Approver>::from_list_query(&self.pool, query)
            .raw_filter("_f_ IS NULL".into())
            .fetch(query.page, query.page_size)
            .await
    }

    async fn get(&self, id: i64) -> Result<Option<Approver>, AliothError> {
        sqlx::query_as::<_, Approver>(
            "SELECT id, notice AS name, fk_user, ck_category, comments AS description, \
             created_at, updated_at, deleted_at \
             FROM isahl.\"zc_id_subj-position\" WHERE id = $1 AND deleted_at IS NULL AND _f_ IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn create(
        &self,
        req: CreateApproverRequest,
        user_id: i64,
    ) -> Result<Approver, AliothError> {
        // ADR A-2：岗位行写收束至 identity-org org_write（语义等价：类别=role id 直绑
        // ck_category、dk_scene/dk_factor/dk_function 常量、不写桥、无 ON CONFLICT/复活）；
        // heal 内建于 service 入口——原 M222 repo 层 heal 移除（时机/幂等语义不变）
        let id = identity_org::service::org_write::create_approver_position(
            &self.pool,
            &req.name,
            req.role,
            req.description.as_deref(),
            user_id,
        )
        .await?;
        // service 仅回 id（跨 crate 免行类型依赖）；行映射复用本 repo SELECT
        Ok(self.get(id).await?.ok_or_else(|| {
            AliothError::Internal(format!("approver row created but unreadable: {id}"))
        })?)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateApproverRequest,
        user_id: i64,
    ) -> Result<Option<Approver>, AliothError> {
        // Some 门语义：未提供字段保持现值——先读 current 合并终值（现 repo 同款）
        let current = self.get(id).await?;
        let Some(current) = current else {
            return Ok(None);
        };
        let name = req.name.unwrap_or(current.name);
        let ck_category = req.role.or(current.ck_category);
        let description = req.description.or(current.description);
        // ADR A-2：写收束 org_write（已合并终值传入；heal 内建于 service）
        let updated = identity_org::service::org_write::update_approver_position(
            &self.pool,
            id,
            &name,
            ck_category,
            description.as_deref(),
            user_id,
        )
        .await?;
        if updated.is_none() {
            return Ok(None);
        }
        // 行映射复用本 repo SELECT；并发软删竞态（heal 窗口内删除）→ Ok(None)/404，
        // 与旧 RETURNING 路径不等价（旧返回删除前快照）——注释级限缩，approvers 低频端点可接受
        self.get(id).await
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        // ADR A-2：软删收束 org_write（仅软删岗位行、不级联桥、未命中静默成功——
        // 与 delete_position 的四桥级联/404 语义不同，故不可复用主入口；heal 内建于 service）
        identity_org::service::org_write::delete_approver_position(&self.pool, id, user_id).await
    }
}

impl From<PgPool> for ApproverRepository {
    fn from(pool: PgPool) -> Self {
        Self { pool }
    }
}
