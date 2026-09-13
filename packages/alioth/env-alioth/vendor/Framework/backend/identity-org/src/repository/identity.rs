// IdentityRepository（split 自 repository.rs 单体，④ 候选）
//! 身份实体 Repository — 标准 CRUD 实现
//!
//! Identity 使用自定义 Repository，其余实体组合 GenericRepository，
//! 仅自定义 create/update 的 INSERT/UPDATE SQL。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::query_builder::QueryBuilder;
use crud::repository::AliothRepository;
use crud::GenericRepository;
use crud::SubtableRouter;
use sqlx::{AssertSqlSafe, PgPool};

use crate::models::{CreateIdentityRequest, Identity, UpdateIdentityRequest};
macro_rules! subject_leaf_repository {
    ($repo:ident, $entity:ident, $create:ident, $update:ident, $table:literal, $coord:literal) => {
        #[derive(Clone)]
        pub struct $repo {
            generic: GenericRepository<$entity>,
            pool: PgPool,
        }

        impl $repo {
            pub fn new(pool: PgPool) -> Self {
                Self {
                    generic: GenericRepository::new(pool.clone()),
                    pool,
                }
            }
        }

        impl From<PgPool> for $repo {
            fn from(pool: PgPool) -> Self {
                Self::new(pool)
            }
        }

        #[async_trait]
        impl AliothRepository<$entity, $create, $update, ApiError> for $repo {
            async fn list(
                &self,
                query: &ListQuery,
            ) -> Result<PaginatedResponse<$entity>, ApiError> {
                self.generic.list_refs(query).await
            }

            async fn get(&self, id: i64) -> Result<Option<$entity>, ApiError> {
                self.generic.get_refs(id, None).await
            }

            async fn create(&self, req: $create, user_id: i64) -> Result<$entity, ApiError> {
                let (dk_scene, dk_factor, dk_function) =
                    ontology_binding::resolve(&self.pool, $coord).await?;
                sqlx::query_as::<_, $entity>(
                    concat!(
                        "INSERT INTO ", $table,
                        " (code, notice, o_number, comments, created_by_id, dk_scene, dk_factor, dk_function)",
                        " VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                        " RETURNING id, code, notice, o_number, comments, created_at, updated_at, deleted_at"
                    ),
                )
                .bind(&req.code)
                .bind(&req.notice)
                .bind(&req.o_number)
                .bind(&req.comments)
                .bind(user_id)
                .bind(dk_scene)
                .bind(dk_factor)
                .bind(dk_function)
                .fetch_one(&self.pool)
                .await
                .map_err(ApiError::from)
            }

            async fn update(
                &self,
                id: i64,
                req: $update,
                user_id: i64,
            ) -> Result<Option<$entity>, ApiError> {
                let mut sets = Vec::new();
                let mut idx: usize = 0;
                if req.code.is_some() {
                    idx += 1;
                    sets.push(format!("code = ${}", idx));
                }
                if req.notice.is_some() {
                    idx += 1;
                    sets.push(format!("notice = ${}", idx));
                }
                if req.o_number.is_some() {
                    idx += 1;
                    sets.push(format!("o_number = ${}", idx));
                }
                if req.comments.is_some() {
                    idx += 1;
                    sets.push(format!("comments = ${}", idx));
                }
                if sets.is_empty() {
                    return self.get(id).await;
                }
                sets.push("updated_at = NOW()".into());
                idx += 1;
                sets.push(format!("updated_by_id = ${}", idx));
                let id_param = idx + 1;
                let sql = format!(
                    concat!(
                        "UPDATE ", $table, " SET {} WHERE id = ${} AND deleted_at IS NULL",
                        " RETURNING id, code, notice, o_number, comments, created_at, updated_at, deleted_at"
                    ),
                    sets.join(", "),
                    id_param
                );
                let mut q = sqlx::query_as::<_, $entity>(AssertSqlSafe(sql.as_str()));
                if let Some(ref v) = req.code {
                    q = q.bind(v);
                }
                if let Some(ref v) = req.notice {
                    q = q.bind(v);
                }
                if let Some(ref v) = req.o_number {
                    q = q.bind(v);
                }
                if let Some(ref v) = req.comments {
                    q = q.bind(v);
                }
                q = q.bind(user_id);
                q = q.bind(id);
                q.fetch_optional(&self.pool).await.map_err(ApiError::from)
            }

            async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
                self.generic.delete(id, user_id).await
            }
        }
    };
}

use crate::models::{
    CreateEmploymentAgentRequest, CreateSubjectBankRequest, CreateSubjectCountryRequest,
    CreateSubjectEmployeeRequest, CreateSubjectGroupRequest, CreateSubjectMinistryRequest,
    CreateSubjectSovereignRequest, CreateSubjectSupranationalRequest, EmploymentAgent, SubjectBank,
    SubjectCountry, SubjectEmployee, SubjectGroup, SubjectMinistry, SubjectSovereign,
    SubjectSupranational, UpdateEmploymentAgentRequest, UpdateSubjectBankRequest,
    UpdateSubjectCountryRequest, UpdateSubjectEmployeeRequest, UpdateSubjectGroupRequest,
    UpdateSubjectMinistryRequest, UpdateSubjectSovereignRequest, UpdateSubjectSupranationalRequest,
};

use super::ontology_binding;
#[derive(Clone)]
pub struct IdentityRepository {
    pool: PgPool,
}

impl IdentityRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl From<PgPool> for IdentityRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

/// MDM 主数据编码 upsert/清除（wz_fssc.subject_mdm 侧表；空串=删除记录）。
/// isahl schema 冻结 + comments 禁嵌 JSON，主体级 MDM 编码落 WZ 扩展表
/// （change: add-subject-mdm-code；先例：subject_bank_card / subject_invoice_info）。
async fn sync_subject_mdm(
    pool: &sqlx::PgPool,
    subject_id: i64,
    mdm_code: &str,
    user_id: i64,
) -> Result<(), ApiError> {
    crate::handlers::subjects::ensure_subject_mdm(pool).await?;
    let trimmed = mdm_code.trim();
    if trimmed.is_empty() {
        sqlx::query("DELETE FROM wz_fssc.subject_mdm WHERE subject_id = $1")
            .bind(subject_id)
            .execute(pool)
            .await
            .map_err(ApiError::from)?;
    } else {
        sqlx::query(
            r#"INSERT INTO wz_fssc.subject_mdm (subject_id, mdm_code, created_by_id, updated_by_id)
               VALUES ($1, $2, $3, $3)
               ON CONFLICT (subject_id)
               DO UPDATE SET mdm_code = EXCLUDED.mdm_code,
                             updated_by_id = EXCLUDED.updated_by_id,
                             updated_at = now()"#,
        )
        .bind(subject_id)
        .bind(trimmed)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(ApiError::from)?;
    }
    Ok(())
}

#[async_trait]
impl AliothRepository<Identity, CreateIdentityRequest, UpdateIdentityRequest, ApiError>
    for IdentityRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Identity>, ApiError> {
        QueryBuilder::<Identity>::from_list_query(&self.pool, query)
            .fetch_refs(query.page, query.page_size)
            .await
    }
    async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<PaginatedResponse<Identity>, ApiError> {
        let mut qb = QueryBuilder::<Identity>::from_list_query(&self.pool, query);
        if let Some(ids) = visible_ids {
            qb = qb.with_visible_ids(ids.to_vec());
        }
        if let Some(cols) = authorized_columns {
            qb = qb.with_authorized_columns(cols.to_vec());
        }
        qb.fetch_refs(query.page, query.page_size).await
    }

    async fn get(&self, id: i64) -> Result<Option<Identity>, ApiError> {
        QueryBuilder::<Identity>::get_refs(&self.pool, id, None).await
    }

    async fn create(&self, req: CreateIdentityRequest, user_id: i64) -> Result<Identity, ApiError> {
        let table = req.resolve_subtable(Some(&req.subject_type))?;
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "Identity").await?;
        let sql = format!(
            r#"INSERT INTO {} (notice, code, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id, notice AS name, code, notice, created_at, updated_at, deleted_at"#,
            table
        );
        sqlx::query_as::<_, Identity>(AssertSqlSafe(sql.as_str()))
            .bind(&req.name)
            .bind(&req.code)
            .bind(user_id)
            .bind(dk_scene)
            .bind(dk_factor)
            .bind(dk_function)
            .fetch_one(&self.pool)
            .await
            .map_err(ApiError::from)
    }
    async fn update(
        &self,
        id: i64,
        req: UpdateIdentityRequest,
        user_id: i64,
    ) -> Result<Option<Identity>, ApiError> {
        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if req.name.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
        if req.code.is_some() {
            idx += 1;
            sets.push(format!("code = ${}", idx));
        }
        if req.notice.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
        if req.comments.is_some() {
            idx += 1;
            sets.push(format!("comments = ${}", idx));
        }

        if sets.is_empty() {
            // 仅 MDM 编码更新的场景（无主体列变更）：确认主体存在后写侧表
            if let Some(ref mdm_code) = req.mdm_code {
                if self.get(id).await?.is_none() {
                    return Ok(None);
                }
                sync_subject_mdm(&self.pool, id, mdm_code, user_id).await?;
            }
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE isahl.zc_id_subjects SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, notice AS name, code, notice, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, Identity>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.name {
            q = q.bind(v);
        }
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }

        q = q.bind(user_id);
        q = q.bind(id);

        let updated = q.fetch_optional(&self.pool).await.map_err(ApiError::from)?;
        // MDM 主数据编码（wz_fssc.subject_mdm 侧表）：主体更新成功后写入
        if updated.is_some() {
            if let Some(ref mdm_code) = req.mdm_code {
                sync_subject_mdm(&self.pool, id, mdm_code, user_id).await?;
            }
        }
        Ok(updated)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        let rows = sqlx::query(
        "UPDATE isahl.zc_id_subjects SET deleted_at = NOW(), updated_by_id = $2 WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(ApiError::from)?;

        if rows.rows_affected() == 0 {
            return Err(ApiError::NotFound(format!("Identity {} not found", id)));
        }
        Ok(())
    }
}

// ═+
// 主体域叶表 Repository（strengthen-identity-org）——同构 CRUD + dk 坐标注入，

// 经宏生成。坐标见 coords_for_entity 对应臂（scene/factor/function 均经 DB 维度表核实）。
// ═══════════════════════════════════════════════════════════════════════════════

// 生成主体域叶表 Repository：list/get 走 GenericRepository（refs 解析），
// create 注入 dk 三元组，update 动态 SET，delete 软删委托 GenericRepository。

subject_leaf_repository!(
    SubjectGroupRepository,
    SubjectGroup,
    CreateSubjectGroupRequest,
    UpdateSubjectGroupRequest,
    "\"isahl\".\"zc_id_subj-group\"",
    "SubjectGroup"
);
subject_leaf_repository!(
    SubjectEmployeeRepository,
    SubjectEmployee,
    CreateSubjectEmployeeRequest,
    UpdateSubjectEmployeeRequest,
    "\"isahl\".\"zc_id_subj-employee\"",
    "SubjectEmployee"
);
subject_leaf_repository!(
    EmploymentAgentRepository,
    EmploymentAgent,
    CreateEmploymentAgentRequest,
    UpdateEmploymentAgentRequest,
    "\"isahl\".\"zc_id_empl-agent\"",
    "EmploymentAgent"
);
subject_leaf_repository!(
    SubjectCountryRepository,
    SubjectCountry,
    CreateSubjectCountryRequest,
    UpdateSubjectCountryRequest,
    "\"isahl\".\"zc_id_subj-country\"",
    "SubjectCountry"
);
subject_leaf_repository!(
    SubjectBankRepository,
    SubjectBank,
    CreateSubjectBankRequest,
    UpdateSubjectBankRequest,
    "\"isahl\".\"zc_id_subj-bank\"",
    "SubjectBank"
);
subject_leaf_repository!(
    SubjectMinistryRepository,
    SubjectMinistry,
    CreateSubjectMinistryRequest,
    UpdateSubjectMinistryRequest,
    "\"isahl\".\"zc_id_subj-ministry\"",
    "SubjectMinistry"
);
subject_leaf_repository!(
    SubjectSovereignRepository,
    SubjectSovereign,
    CreateSubjectSovereignRequest,
    UpdateSubjectSovereignRequest,
    "\"isahl\".\"zc_id_subj-sovereign\"",
    "SubjectSovereign"
);
subject_leaf_repository!(
    SubjectSupranationalRepository,
    SubjectSupranational,
    CreateSubjectSupranationalRequest,
    UpdateSubjectSupranationalRequest,
    "\"isahl\".\"zc_id_subj-supranational\"",
    "SubjectSupranational"
);

/// 封签关联运单的既有载体解析（P3 迁移；Seal 无运单列，零 DDL）。
///
/// 载体 = `projection`（文本载荷列；既有约定先例：合同附件 `projection='定稿合同'`、
/// 定价协定 `projection`=新单价）——落**运单编号**（`zc_id_orde-land.code`）；
/// `comments` 保持自由文本语义，不再承载 JSON。
///
/// DTO 入参仍为运单 id（契约不变）→ 经 id 解析为编号；解析不到即 400（不静默丢弃）。
///
/// 排除项（依据）：`o_number` 为维度派生的声明编号、DTO 禁写（`DTO_DESIGN_SPEC.md` §5.2 /
/// `MODULE_SPEC.md` §5），不可用作载荷；真结构路径为
/// `zc_id_tsp-voucher_rr_devi-seal` + `zc_id_orde-traffic_rr_tsp-voucher` 两跳桥（经装车条），
/// 其写侧归装车条组装流程（`transport-operations`），非封签管理页可造。
pub(crate) async fn resolve_seal_waybill_code(
    pool: &PgPool,
    waybill_id: i64,
) -> Result<String, ApiError> {
    let code: Option<String> = sqlx::query_scalar(
        r#"SELECT code FROM "isahl"."zc_id_orde-land" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(waybill_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from)?;
    match code.filter(|c| !c.trim().is_empty()) {
        Some(code) => Ok(code),
        None => Err(ApiError::BadRequest(format!(
            "关联运单不存在: {waybill_id}"
        ))),
    }
}
