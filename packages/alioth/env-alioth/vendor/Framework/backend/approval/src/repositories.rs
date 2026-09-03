use async_trait::async_trait;
use chrono::Utc;
use common::data::{ListQuery, PaginatedResponse};
use common::error::AliothError;
use crud::query_builder::QueryBuilder;
use crud::repository::AliothRepository;
use sqlx::PgPool;

use super::models::{
    ApprovalAction, ApprovalFlow, ApprovalInstance, CreateApprovalActionRequest,
    CreateApprovalFlowRequest, CreateApprovalInstanceRequest, CreateDelegationRuleRequest,
    CreateFlowNodeRequest, DelegationRule, FlowNode, UpdateApprovalActionRequest,
    UpdateApprovalFlowRequest, UpdateApprovalInstanceRequest, UpdateDelegationRuleRequest,
    UpdateFlowNodeRequest,
};

// ── ApprovalFlowRepository ────────────────────────────────────
// 表: isahl."zc_id_proc-approve"（create 落子类；读经基表 zc_id_process 继承并集）
#[derive(Clone)]
pub struct ApprovalFlowRepository {
    pool: PgPool,
}

impl ApprovalFlowRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl From<PgPool> for ApprovalFlowRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl
    AliothRepository<
        ApprovalFlow,
        CreateApprovalFlowRequest,
        UpdateApprovalFlowRequest,
        AliothError,
    > for ApprovalFlowRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<ApprovalFlow>, AliothError> {
        QueryBuilder::<ApprovalFlow>::from_list_query(&self.pool, query)
            .fetch(query.page, query.page_size)
            .await
    }

    async fn get(&self, id: i64) -> Result<Option<ApprovalFlow>, AliothError> {
        sqlx::query_as::<_, ApprovalFlow>(
            "SELECT id, notice AS name, code, t_color_, comments, meta, mermaid, fk_context, \
             tableoid::regclass::text AS branch, \
             (SELECT c.notice FROM isahl.\"zc_id_proc-context\" c \
              WHERE c.id = zc_id_process.fk_context AND c.deleted_at IS NULL) AS context_concept, \
             (SELECT replace(c.tableoid::regclass::text, '\"', '') FROM isahl.\"zc_id_proc-context\" c \
              WHERE c.id = zc_id_process.fk_context AND c.deleted_at IS NULL) AS context_leaf, \
             (SELECT s.code FROM isahl.\"zc_id_lifecycle_r_primary-status\" ls \
              JOIN isahl.\"zc_id_stus-process\" s ON s.id = ls.ref_right \
              WHERE ls.ref_left = zc_id_process.id AND ls.deleted_at IS NULL) AS status, \
             created_at, updated_at, deleted_at \
             FROM isahl.zc_id_process WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn create(
        &self,
        req: CreateApprovalFlowRequest,
        user_id: i64,
    ) -> Result<ApprovalFlow, AliothError> {
        // fk_context 归属（refactor-flow-node-operation-model 阶段 3）：
        // - context_table（新契约）：域叶表 → 域父表创建流程专属上下文范例行
        //   （_t_='flow-context'），fk_context → 范例行——模板链
        //   even-approve(范例) ↔ operation(范例) ↔ process(范例).fk_context
        // - context_id（旧契约兼容）：proc-context 族在册 scope-definition 行
        let mut resolved_context: Option<i64> = req.context_id;
        if let Some(table) = &req.context_table {
            let domain = crate::context_domain::domain_of_leaf(table).ok_or_else(|| {
                AliothError::Validation {
                    field: "context_table".to_string(),
                    message: format!("context_table '{table}' 不在三域上下文叶表内"),
                }
            })?;
            let insert_sql =
                crate::context_domain::flow_context_insert_sql(domain).ok_or_else(|| {
                    AliothError::Validation {
                        field: "context_table".to_string(),
                        message: format!("域 '{domain}' 无范例落表"),
                    }
                })?;
            resolved_context = Some(
                sqlx::query_scalar::<_, i64>(insert_sql)
                    .bind(req.name.trim())
                    .bind(user_id)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(AliothError::from)?,
            );
        }
        if let Some(ctx) = resolved_context {
            let valid: bool = sqlx::query_scalar(
                r#"SELECT EXISTS(
                     SELECT 1 FROM isahl."zc_id_proc-context"
                     WHERE id = $1 AND deleted_at IS NULL
                   )"#,
            )
            .bind(ctx)
            .fetch_one(&self.pool)
            .await
            .map_err(AliothError::from)?;
            if !valid {
                return Err(AliothError::Validation {
                    field: "context_id".to_string(),
                    message: format!(
                        "非法流程输入范畴 context_id={ctx}——须为 proc-context 族在册行"
                    ),
                });
            }
        }
        // 定义落位（flow-process-continuity 规约）：流程定义按用户选定的叶表分支
        // 落子类表——PG 继承下基表 zc_id_process 查询自动并入子表行。
        // 静态 match 分发（7 个已知叶表），禁 format! 动态表名（sqlx 注入审计）；
        // 缺省 proc-approve 向后兼容既有调用方。
        // dk 三元组经 dk.rs（JC/FTA/↑_NA 坐标码）解析；失败 warn + NULL，
        // 不写悬空 ZUID（对齐 crud::handler::resolve_dk_ctx 范式）。
        const RETURNING: &str =
            "RETURNING id, notice AS name, code, t_color_, comments, meta, mermaid, fk_context, \
             tableoid::regclass::text AS branch, \
             (SELECT c.notice FROM isahl.\"zc_id_proc-context\" c \
              WHERE c.id = fk_context AND c.deleted_at IS NULL) AS context_concept, \
             (SELECT replace(c.tableoid::regclass::text, '\"', '') FROM isahl.\"zc_id_proc-context\" c \
              WHERE c.id = fk_context AND c.deleted_at IS NULL) AS context_leaf, \
             created_at, updated_at, deleted_at";
        let branch = req.branch.as_deref().unwrap_or("zc_id_proc-approve");
        let table = match branch {
            "zc_id_proc-approve" => "isahl.\"zc_id_proc-approve\"",
            "zc_id_proc-cicd" => "isahl.\"zc_id_proc-cicd\"",
            "zc_id_proc-loading" => "isahl.\"zc_id_proc-loading\"",
            "zc_id_proc-make" => "isahl.\"zc_id_proc-make\"",
            "zc_id_proc-project" => "isahl.\"zc_id_proc-project\"",
            "zc_id_proc-purchase" => "isahl.\"zc_id_proc-purchase\"",
            "zc_id_proc-service" => "isahl.\"zc_id_proc-service\"",
            other => {
                return Err(AliothError::Validation {
                    field: "branch".to_string(),
                    message: format!(
                        "非法流程分支 '{other}'——合法分支：zc_id_proc-approve / zc_id_proc-cicd / \
                         zc_id_proc-purchase / zc_id_proc-service"
                    ),
                });
            }
        };
        let insert_sql = format!(
            r#"INSERT INTO {table}
               (notice, code, comments, meta, mermaid, fk_context, created_by_id,
                dk_scene, dk_factor, dk_function, _f_, _t_)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, '设计', '实例') {RETURNING}"#
        );
        // 设计图 → mermaid 整体结构（保存时引擎自动生成，幂等重写）
        let mermaid = req.meta.as_ref().map(crate::mermaid::graph_to_mermaid);
        // context_table 新契约的范例行校验已在上方完成（resolved_context）
        let (dk_scene, dk_factor, dk_function) =
            crate::dk::resolve_ontology_coords_pool(&self.pool, crate::dk::DkEntity::DkJcFtaNa)
                .await
                .unwrap_or_else(|e| {
                    common::telemetry::warn!(
                        "approval-flow dk 解析失败（JC/FTA/↑_NA），dk_* 置 NULL: {}",
                        e
                    );
                    (None, None, None)
                });
        // 静态 match 产出的固定 SQL（表名为编译期常量），AssertSqlSafe 声明已审计
        sqlx::query_as::<_, ApprovalFlow>(sqlx::AssertSqlSafe(insert_sql.as_str()))
            .bind(&req.name)
            .bind(&req.code)
            .bind(&req.comments)
            .bind(&req.meta)
            .bind(&mermaid)
            .bind(resolved_context)
            .bind(user_id)
            .bind(dk_scene)
            .bind(dk_factor)
            .bind(dk_function)
            .fetch_one(&self.pool)
            .await
            .map_err(AliothError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateApprovalFlowRequest,
        user_id: i64,
    ) -> Result<Option<ApprovalFlow>, AliothError> {
        let current = self.get(id).await?;
        if current.is_none() {
            return Ok(None);
        }
        let current = current.unwrap();
        let name = req.name.unwrap_or(current.name);
        // code 为引擎发布位/业务码（publish/unpublish 独占），update 不写
        let comments = req.comments.or(current.comments);
        // 设计图 JSON 信封（meta jsonb）+ mermaid 整体结构（保存时引擎自动生成）
        let meta = req.meta.or(current.meta);
        let mermaid = meta.as_ref().map(crate::mermaid::graph_to_mermaid);
        // fk_context 重绑（refactor-flow-node-operation-model 阶段 3）：
        // context_table（新契约）→ 域父表建新范例行；context_id（旧契约）兼容
        let fk_context =
            if let Some(table) = &req.context_table {
                let domain = crate::context_domain::domain_of_leaf(table).ok_or_else(|| {
                    AliothError::Validation {
                        field: "context_table".to_string(),
                        message: format!("context_table '{table}' 不在三域上下文叶表内"),
                    }
                })?;
                let insert_sql = crate::context_domain::flow_context_insert_sql(domain)
                    .ok_or_else(|| AliothError::Validation {
                        field: "context_table".to_string(),
                        message: format!("域 '{domain}' 无范例落表"),
                    })?;
                Some(
                    sqlx::query_scalar::<_, i64>(insert_sql)
                        .bind(name.trim())
                        .bind(user_id)
                        .fetch_one(&self.pool)
                        .await
                        .map_err(AliothError::from)?,
                )
            } else {
                match req.context_id {
                    Some(ctx) => {
                        let valid: bool = sqlx::query_scalar(
                            r#"SELECT EXISTS(
                             SELECT 1 FROM isahl."zc_id_proc-context"
                             WHERE id = $1 AND _t_ = 'scope-definition' AND deleted_at IS NULL
                           )"#,
                        )
                        .bind(ctx)
                        .fetch_one(&self.pool)
                        .await?;
                        if !valid {
                            return Err(AliothError::Validation {
                                field: "context_id".to_string(),
                                message: format!(
                                    "非法流程输入范畴 context_id={ctx}——须为 proc-context 族 \
                                 _t_='scope-definition' 范畴定义行"
                                ),
                            });
                        }
                        Some(ctx)
                    }
                    None => current.fk_context,
                }
            };
        sqlx::query_as::<_, ApprovalFlow>(
            r#"UPDATE isahl.zc_id_process
               SET notice = $1, comments = $2, meta = $3, mermaid = $4,
                   fk_context = $5, updated_by_id = $6
               WHERE id = $7 AND deleted_at IS NULL
               RETURNING id, notice AS name, code, t_color_, comments, meta, mermaid, fk_context,
                         tableoid::regclass::text AS branch,
                         (SELECT c.notice FROM isahl."zc_id_proc-context" c
                          WHERE c.id = fk_context AND c.deleted_at IS NULL) AS context_concept,
                         (SELECT replace(c.tableoid::regclass::text, '"', '') FROM isahl."zc_id_proc-context" c
                          WHERE c.id = fk_context AND c.deleted_at IS NULL) AS context_leaf,
                         created_at, updated_at, deleted_at"#,
        )
        .bind(&name)
        .bind(&comments)
        .bind(&meta)
        .bind(&mermaid)
        .bind(fk_context)
        .bind(user_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    /// 删除流程 = 事务内级联软删（fix-flow-designer-chain-breaks §D2；
    /// 2026-08-29 终端节点语义修正后重写定位）：
    /// 血缘链 流程 → DAG op 行（rr_operation.ref_right，全叶覆盖 gate/approve/
    /// check/action）→ 终端语义实体（end statement 范例 + rr_statement 桥 /
    /// task 驱动 start task 范例 + rr_task 桥）→ even-approve 节点（rr_event 模板桥）
    /// → oper-approve 实例（经 rr_event 桥）→ deta-opinion 意见（fk_list）；
    /// 关系行仅当 ref 端命中删除集才软删。共享值对象不删。
    async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        let mut tx = self.pool.begin().await.map_err(AliothError::from)?;

        // 模板 op 行集（本流程在册 DAG 节点主体，全叶表经基表 UPDATE 级联）
        let op_ids: Vec<i64> = sqlx::query_scalar(
            r#"SELECT ref_right FROM isahl.zc_id_process_rr_operation
               WHERE ref_left = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await
        .map_err(AliothError::from)?;

        if !op_ids.is_empty() {
            // 终端语义实体：end statement 范例（先删行后删桥——行经在册桥定位）
            sqlx::query(
                r#"UPDATE isahl.zc_id_statement
                   SET deleted_at = NOW(), deleted_by_id = $1
                   WHERE id IN (SELECT rs.ref_right FROM isahl.zc_id_operation_rr_statement rs
                                WHERE rs.ref_left = ANY($2) AND rs.deleted_at IS NULL)
                     AND deleted_at IS NULL"#,
            )
            .bind(user_id)
            .bind(&op_ids)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
            sqlx::query(
                r#"UPDATE isahl.zc_id_operation_rr_statement
                   SET deleted_at = NOW(), deleted_by_id = $1
                   WHERE ref_left = ANY($2) AND deleted_at IS NULL"#,
            )
            .bind(user_id)
            .bind(&op_ids)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
            // task 驱动 start 的 task 范例（同序）
            sqlx::query(
                r#"UPDATE isahl.zc_id_task
                   SET deleted_at = NOW(), deleted_by_id = $1
                   WHERE id IN (SELECT rt.ref_right FROM isahl.zc_id_operation_rr_task rt
                                WHERE rt.ref_left = ANY($2) AND rt.deleted_at IS NULL)
                     AND deleted_at IS NULL"#,
            )
            .bind(user_id)
            .bind(&op_ids)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
            sqlx::query(
                r#"UPDATE isahl.zc_id_operation_rr_task
                   SET deleted_at = NOW(), deleted_by_id = $1
                   WHERE ref_left = ANY($2) AND deleted_at IS NULL"#,
            )
            .bind(user_id)
            .bind(&op_ids)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;

            // 意见行（实例 fk_list 锚定）：实例集 = rr_event 桥 ref_left 命中 op 行
            // 或（审批实例挂 even-approve 模板）——模板桥+实例桥统一按 op 集收口。
            let inst_ids: Vec<i64> = sqlx::query_scalar(
                r#"UPDATE isahl."zc_id_oper-approve" oa
                   SET deleted_at = NOW(), deleted_by_id = $1
                   WHERE oa.id IN (
                       SELECT oe.ref_left FROM isahl.zc_id_operation_rr_event oe
                       WHERE oe.ref_left = ANY($2) AND oe.deleted_at IS NULL
                   )
                     AND oa.deleted_at IS NULL
                   RETURNING oa.id"#,
            )
            .bind(user_id)
            .bind(&op_ids)
            .fetch_all(&mut *tx)
            .await
            .map_err(AliothError::from)?;
            if !inst_ids.is_empty() {
                sqlx::query(
                    r#"UPDATE isahl."zc_id_deta-opinion"
                       SET deleted_at = NOW(), deleted_by_id = $1
                       WHERE fk_list = ANY($2) AND deleted_at IS NULL"#,
                )
                .bind(user_id)
                .bind(&inst_ids)
                .execute(&mut *tx)
                .await
                .map_err(AliothError::from)?;
            }

            // 操作 ↔ 实体桥（event 模板桥/岗位桥）+ 全叶 op 行（基表 UPDATE 级联）
            sqlx::query(
                r#"UPDATE isahl.zc_id_operation_rr_event
                   SET deleted_at = NOW(), deleted_by_id = $1
                   WHERE ref_left = ANY($2) AND deleted_at IS NULL"#,
            )
            .bind(user_id)
            .bind(&op_ids)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
            sqlx::query(
                r#"UPDATE isahl.zc_id_operation_rr_approve
                   SET deleted_at = NOW(), deleted_by_id = $1
                   WHERE ref_left = ANY($2) AND deleted_at IS NULL"#,
            )
            .bind(user_id)
            .bind(&op_ids)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
            sqlx::query(
                r#"UPDATE isahl.zc_id_operation_rr_review
                   SET deleted_at = NOW(), deleted_by_id = $1
                   WHERE ref_left = ANY($2) AND deleted_at IS NULL"#,
            )
            .bind(user_id)
            .bind(&op_ids)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
            sqlx::query(
                r#"UPDATE isahl.zc_id_operation_rr_post
                   SET deleted_at = NOW(), deleted_by_id = $1
                   WHERE ref_left = ANY($2) AND deleted_at IS NULL"#,
            )
            .bind(user_id)
            .bind(&op_ids)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
            sqlx::query(
                r#"UPDATE isahl.zc_id_operation
                   SET deleted_at = NOW(), deleted_by_id = $1
                   WHERE id = ANY($2) AND deleted_at IS NULL"#,
            )
            .bind(user_id)
            .bind(&op_ids)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
        }

        // even-approve 节点语义行（桥链定位：process_rr_operation 在册 oper 节点
        // 经 rr_event 模板桥指向的 even 行，含 timeline 快照载体）+
        // 实例侧 rr_event 桥（ref_right=模板行）。无 oper 锚定的 even 行
        // （AVIC 审计记录等业务事件）不属本流程节点，不级联。
        // 注：子查询不过滤桥 deleted_at——上方操作行块已先软删 ref_left 侧桥行，
        // 桥行仍唯一标识本流程的事件模板（oper 为本流程专属），过滤反而漏删。
        let node_ids: Vec<i64> = if op_ids.is_empty() {
            Vec::new()
        } else {
            sqlx::query_scalar(
                r#"UPDATE isahl."zc_id_even-approve"
                   SET deleted_at = NOW(), deleted_by_id = $1
                   WHERE id IN (SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
                                WHERE oe.ref_left = ANY($2))
                     AND deleted_at IS NULL
                   RETURNING id"#,
            )
            .bind(user_id)
            .bind(&op_ids)
            .fetch_all(&mut *tx)
            .await
            .map_err(AliothError::from)?
        };
        if !node_ids.is_empty() {
            sqlx::query(
                r#"UPDATE isahl.zc_id_operation_rr_event
                   SET deleted_at = NOW(), deleted_by_id = $1
                   WHERE ref_right = ANY($2) AND deleted_at IS NULL"#,
            )
            .bind(user_id)
            .bind(&node_ids)
            .execute(&mut *tx)
            .await
            .map_err(AliothError::from)?;
        }

        // 流程 ↔ 节点关系行（含 next-ops DAG 边）+ 流程行自身
        sqlx::query(
            r#"UPDATE isahl.zc_id_process_rr_operation
               SET deleted_at = NOW(), deleted_by_id = $1
               WHERE ref_left = $2 AND deleted_at IS NULL"#,
        )
        .bind(user_id)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(AliothError::from)?;

        sqlx::query(
            "UPDATE isahl.zc_id_process SET deleted_at = NOW(), deleted_by_id = $1 WHERE id = $2 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(AliothError::from)?;

        tx.commit().await.map_err(AliothError::from)?;
        Ok(())
    }
}

// ── FlowNodeRepository ────────────────────────────────────────
// 保持不变（同表 zc_id_even-approve，非本次校对范围）

#[derive(Clone)]
pub struct FlowNodeRepository {
    pool: PgPool,
}

impl FlowNodeRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl From<PgPool> for FlowNodeRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl AliothRepository<FlowNode, CreateFlowNodeRequest, UpdateFlowNodeRequest, AliothError>
    for FlowNodeRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<FlowNode>, AliothError> {
        QueryBuilder::<FlowNode>::from_list_query(&self.pool, query)
            .fetch(query.page, query.page_size)
            .await
    }

    async fn get(&self, id: i64) -> Result<Option<FlowNode>, AliothError> {
        sqlx::query_as::<_, FlowNode>(
            "SELECT id, notice AS label, code, t_color_, comments, created_at, updated_at, deleted_at \
             FROM isahl.\"zc_id_even-approve\" WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn create(
        &self,
        req: CreateFlowNodeRequest,
        user_id: i64,
    ) -> Result<FlowNode, AliothError> {
        sqlx::query_as::<_, FlowNode>(
            r#"INSERT INTO isahl."zc_id_even-approve" (notice, code, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id, notice AS label, code, t_color_, comments, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.label)
        .bind(&req.code)
        .bind(user_id)
        .bind(515i64)
        .bind(522i64)
        .bind(526i64)
        .fetch_one(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateFlowNodeRequest,
        user_id: i64,
    ) -> Result<Option<FlowNode>, AliothError> {
        let current = self.get(id).await?;
        if current.is_none() {
            return Ok(None);
        }
        let current = current.unwrap();
        let label = req.label.unwrap_or(current.label);
        let code = req.code.or(current.code);
        sqlx::query_as::<_, FlowNode>(
            r#"UPDATE isahl."zc_id_even-approve"
               SET notice = $1, code = $2, updated_by_id = $3
               WHERE id = $4 AND deleted_at IS NULL
               RETURNING id, notice AS label, code, t_color_, comments, created_at, updated_at, deleted_at"#,
        )
        .bind(&label)
        .bind(&code)
        .bind(user_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        sqlx::query(
            "UPDATE isahl.\"zc_id_even-approve\" SET deleted_at = NOW(), deleted_by_id = $1 WHERE id = $2 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(AliothError::from)?;
        Ok(())
    }
}

// ── ApprovalInstanceRepository ────────────────────────────────
// 表: isahl.zc_id_oper-approve

#[derive(Clone)]
pub struct ApprovalInstanceRepository {
    pool: PgPool,
}

impl ApprovalInstanceRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl From<PgPool> for ApprovalInstanceRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl
    AliothRepository<
        ApprovalInstance,
        CreateApprovalInstanceRequest,
        UpdateApprovalInstanceRequest,
        AliothError,
    > for ApprovalInstanceRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<ApprovalInstance>, AliothError> {
        QueryBuilder::<ApprovalInstance>::from_list_query(&self.pool, query)
            .fetch(query.page, query.page_size)
            .await
    }

    async fn get(&self, id: i64) -> Result<Option<ApprovalInstance>, AliothError> {
        sqlx::query_as::<_, ApprovalInstance>(
            "SELECT id, notice AS node_name, code, \
             (SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe \
              WHERE oe.ref_left = \"zc_id_oper-approve\".id AND oe.deleted_at IS NULL \
                AND EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_event oe2 \
                            JOIN isahl.zc_id_process_rr_operation rro2 \
                              ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL \
                            WHERE oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL) \
              ORDER BY oe.created_at LIMIT 1) AS fk_approve, \
             fk_subject, comments, created_at, updated_at, deleted_at \
             FROM isahl.\"zc_id_oper-approve\" WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn create(
        &self,
        req: CreateApprovalInstanceRequest,
        user_id: i64,
    ) -> Result<ApprovalInstance, AliothError> {
        // Auto-resolve fk_approve: if the provided value doesn't exist in zc_id_even-approve,
        // assume it's a process ID and find the latest published approval event.
        let effective_fk: Option<i64> = match req.fk_approve {
            Some(fk) => {
                let exists: bool = sqlx::query_scalar(
                    r#"SELECT EXISTS(SELECT 1 FROM isahl."zc_id_even-approve" WHERE id = $1 AND deleted_at IS NULL)"#,
                )
                .bind(fk)
                .fetch_one(&self.pool)
                .await
                .unwrap_or(false);
                if exists {
                    Some(fk)
                } else {
                    // FK not found — try to resolve from the latest published event for this process
                    let resolved: Option<i64> = sqlx::query_scalar(
                        r#"SELECT oe.ref_right FROM isahl.zc_id_process_rr_operation rro
                           JOIN isahl.zc_id_operation_rr_event oe
                             ON oe.ref_left = rro.ref_right AND oe.deleted_at IS NULL
                           WHERE rro.ref_left = $1 AND rro.deleted_at IS NULL
                           ORDER BY oe.created_at DESC LIMIT 1"#,
                    )
                    .bind(fk)
                    .fetch_one(&self.pool)
                    .await
                    .unwrap_or(None);
                    Some(resolved.unwrap_or(fk))
                }
            }
            None => None,
        };

        let instance: ApprovalInstance = sqlx::query_as::<_, ApprovalInstance>(
            r#"INSERT INTO isahl."zc_id_oper-approve" (notice, code, comments, fk_subject, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               RETURNING id, notice AS node_name, code, \
                 (SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe \
                  WHERE oe.ref_left = \"zc_id_oper-approve\".id AND oe.deleted_at IS NULL \
                    AND EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_event oe2 \
                                JOIN isahl.zc_id_process_rr_operation rro2 \
                                  ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL \
                                WHERE oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL) \
                  ORDER BY oe.created_at LIMIT 1) AS fk_approve, \
                 fk_subject, comments, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.node_name)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(user_id)
        .bind(user_id)
        .bind(515i64)
        .bind(522i64)
        .bind(524i64)
        .fetch_one(&self.pool)
        .await
        .map_err(AliothError::from)?;
        // fk_approve 列已移除：实例↔审批事件经 operation_rr_event 桥（fk_approve
        // 入参语义=节点事件模板，保留为实例桥）
        if let Some(tpl) = effective_fk {
            sqlx::query(
                r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3)"#,
            )
            .bind(instance.id)
            .bind(tpl)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(AliothError::from)?;
        }
        Ok(instance)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateApprovalInstanceRequest,
        user_id: i64,
    ) -> Result<Option<ApprovalInstance>, AliothError> {
        let current = self.get(id).await?;
        if current.is_none() {
            return Ok(None);
        }
        let current = current.unwrap();
        let node_name = req.node_name.unwrap_or(current.node_name);
        let code = req.code.or(current.code);
        let comments = req.comments.or(current.comments);
        sqlx::query_as::<_, ApprovalInstance>(
            r#"UPDATE isahl."zc_id_oper-approve"
               SET notice = $1, code = $2, comments = $3, updated_by_id = $4
               WHERE id = $5 AND deleted_at IS NULL
               RETURNING id, notice AS node_name, code, \
                 (SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe \
                  WHERE oe.ref_left = \"zc_id_oper-approve\".id AND oe.deleted_at IS NULL \
                    AND EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_event oe2 \
                                JOIN isahl.zc_id_process_rr_operation rro2 \
                                  ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL \
                                WHERE oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL) \
                  ORDER BY oe.created_at LIMIT 1) AS fk_approve, \
                 fk_subject, comments, created_at, updated_at, deleted_at"#,
        )
        .bind(&node_name)
        .bind(&code)
        .bind(&comments)
        .bind(user_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        sqlx::query(
            "UPDATE isahl.\"zc_id_oper-approve\" SET deleted_at = NOW(), deleted_by_id = $1 WHERE id = $2 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(AliothError::from)?;
        Ok(())
    }
}

// ── ApprovalActionRepository ──────────────────────────────────
// 表: isahl.zc_id_deta-opinion

#[derive(Clone)]
pub struct ApprovalActionRepository {
    pool: PgPool,
}

impl ApprovalActionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl From<PgPool> for ApprovalActionRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl
    AliothRepository<
        ApprovalAction,
        CreateApprovalActionRequest,
        UpdateApprovalActionRequest,
        AliothError,
    > for ApprovalActionRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<ApprovalAction>, AliothError> {
        QueryBuilder::<ApprovalAction>::from_list_query(&self.pool, query)
            .fetch(query.page, query.page_size)
            .await
    }

    async fn get(&self, id: i64) -> Result<Option<ApprovalAction>, AliothError> {
        sqlx::query_as::<_, ApprovalAction>(
            "SELECT id, notice AS summary, opinion AS opinion, code, fk_list, fk_subject AS fk_biller, qk_date, created_at, updated_at, deleted_at \
             FROM isahl.\"zc_id_deta-opinion\" WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn create(
        &self,
        req: CreateApprovalActionRequest,
        user_id: i64,
    ) -> Result<ApprovalAction, AliothError> {
        // 时间锚 + dk 静态绑定（flow-process-continuity）：意见属审批内容域（同 ApprovalFlow 坐标）；
        // dk 解析失败 warn + NULL，不写悬空 ZUID（对齐 crud::handler::resolve_dk_ctx 范式）
        let date_anchor = crate::handlers::approve_reject::today_date_anchor(&self.pool).await?;
        let (dk_scene, dk_factor, dk_function) =
            crate::dk::resolve_ontology_coords_pool(&self.pool, crate::dk::DkEntity::DkJcFtaNa)
                .await
                .unwrap_or_else(|e| {
                    common::telemetry::warn!(
                        "approval-action dk 解析失败（JC/FTA/↑_NA），dk_* 置 NULL: {}",
                        e
                    );
                    (None, None, None)
                });
        sqlx::query_as::<_, ApprovalAction>(
            r#"INSERT INTO isahl."zc_id_deta-opinion" (notice, code, fk_list, qk_date, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               RETURNING id, notice AS summary, opinion AS opinion, code, fk_list, fk_subject AS fk_biller, qk_date, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.summary)
        .bind(&req.code)
        .bind(req.fk_list)
        .bind(date_anchor)
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateApprovalActionRequest,
        user_id: i64,
    ) -> Result<Option<ApprovalAction>, AliothError> {
        let current = self.get(id).await?;
        if current.is_none() {
            return Ok(None);
        }
        let current = current.unwrap();
        let summary = req.summary.unwrap_or(current.summary);
        let code = req.code.or(current.code);
        sqlx::query_as::<_, ApprovalAction>(
            r#"UPDATE isahl."zc_id_deta-opinion"
               SET notice = $1, code = $2, updated_by_id = $3
               WHERE id = $4 AND deleted_at IS NULL
               RETURNING id, notice AS summary, opinion AS opinion, code, fk_list, fk_subject AS fk_biller, qk_date, created_at, updated_at, deleted_at"#,
        )
        .bind(&summary)
        .bind(&code)
        .bind(user_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        sqlx::query(
            "UPDATE isahl.\"zc_id_deta-opinion\" SET deleted_at = NOW(), deleted_by_id = $1 WHERE id = $2 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(AliothError::from)?;
        Ok(())
    }
}

// ── DelegationRuleRepository ─────────────────────────────────
// 表: isahl.zc_id_operation（公式驱动委托规则，当前保留占位 CRUD）

#[derive(Clone)]
pub struct DelegationRuleRepository {
    pool: PgPool,
}

impl DelegationRuleRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl From<PgPool> for DelegationRuleRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

impl DelegationRuleRepository {
    /// 起止时间标量解析（_refs 契约）：date_st/date_ed → zc_id_segm-date 行（幂等查/建）→ qk_period。
    /// 任一为 None → 不建立 segm-date 行（qk_period=NULL，仅语义上有界委托）。
    async fn resolve_period(
        &self,
        date_st: Option<chrono::DateTime<Utc>>,
        date_ed: Option<chrono::DateTime<Utc>>,
    ) -> Result<Option<i64>, AliothError> {
        let (Some(st), Some(ed)) = (date_st, date_ed) else {
            return Ok(None);
        };
        // 幂等：同 (date_st, date_ed) 复用既有行
        if let Some(id) = sqlx::query_scalar::<_, i64>(
            r#"SELECT id FROM isahl."zc_id_segm-date"
               WHERE date_st = $1 AND date_ed = $2 AND deleted_at IS NULL LIMIT 1"#,
        )
        .bind(st)
        .bind(ed)
        .fetch_optional(&self.pool)
        .await?
        {
            return Ok(Some(id));
        }
        let id = sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO isahl."zc_id_segm-date" (id, date_st, date_ed, notice, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3, 1)
               RETURNING id"#,
        )
        .bind(st)
        .bind(ed)
        .bind(format!(
            "委托期 {} ~ {}",
            st.format("%Y-%m-%d"),
            ed.format("%Y-%m-%d")
        ))
        .fetch_one(&self.pool)
        .await?;
        Ok(Some(id))
    }
}

#[async_trait]
impl
    AliothRepository<
        DelegationRule,
        CreateDelegationRuleRequest,
        UpdateDelegationRuleRequest,
        AliothError,
    > for DelegationRuleRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<DelegationRule>, AliothError> {
        QueryBuilder::<DelegationRule>::from_list_query(&self.pool, query)
            .fetch_refs(query.page, query.page_size)
            .await
    }

    async fn get(&self, id: i64) -> Result<Option<DelegationRule>, AliothError> {
        QueryBuilder::<DelegationRule>::get_refs(&self.pool, id, None).await
    }

    async fn create(
        &self,
        req: CreateDelegationRuleRequest,
        user_id: i64,
    ) -> Result<DelegationRule, AliothError> {
        // D8（fix-approval-engine-gap-closure）：委托人归因——前端仅发受托人姓名时
        // fk_subject 缺省为创建者（委托人）；fk_operator 缺失则按 req.name 解析活跃
        // 用户（username 或 name 精确匹配，LIMIT 1）；解析不到 → Validation 400
        // fail-closed，不再产出永远无法转派的死规则。
        let fk_subject = req.fk_subject.unwrap_or(user_id);
        let fk_operator = match req.fk_operator {
            Some(uid) => Some(uid),
            None => sqlx::query_scalar::<_, i64>(
                r#"SELECT id FROM isahl_auth.auth_users
                   WHERE (username = $1 OR name = $1) AND is_active = TRUE
                   ORDER BY id LIMIT 1"#,
            )
            .bind(&req.name)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| AliothError::Validation {
                field: "name".into(),
                message: format!(
                    "受托人不可解析：'{}' 不是活跃用户（username/name 均不匹配）",
                    req.name
                ),
            })
            .map(Some)?,
        };
        // _t_='delegation-rule' 判别值：advance::apply_delegation 按此检索有效委托
        // dk 静态绑定：委托属审批内容域（JC/FTA/↑_NA）；解析失败 warn + NULL，不写悬空 ZUID
        let qk_period = self.resolve_period(req.date_st, req.date_ed).await?;
        let (dk_scene, dk_factor, dk_function) =
            crate::dk::resolve_ontology_coords_pool(&self.pool, crate::dk::DkEntity::DkJcFtaNa)
                .await
                .unwrap_or_else(|e| {
                    common::telemetry::warn!(
                        "delegation-rule dk 解析失败（JC/FTA/↑_NA），dk_* 置 NULL: {}",
                        e
                    );
                    (None, None, None)
                });
        sqlx::query_as::<_, DelegationRule>(
            r#"INSERT INTO isahl.zc_id_operation
               (notice, code, fk_subject, fk_operator, comments, qk_period, _t_, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, 'delegation-rule', $7, $8, $9, $10)
               RETURNING id, notice AS name, code, fk_subject, fk_operator, comments, qk_period, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(&req.code)
        .bind(fk_subject)
        .bind(fk_operator)
        .bind(&req.comments)
        .bind(qk_period)
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateDelegationRuleRequest,
        user_id: i64,
    ) -> Result<Option<DelegationRule>, AliothError> {
        let current = self.get(id).await?;
        if current.is_none() {
            return Ok(None);
        }
        let current = current.unwrap();
        let name = req.name.unwrap_or(current.name);
        let code = req.code.or(current.code);
        let fk_subject = req.fk_subject.or(current.fk_subject);
        let fk_operator = req.fk_operator.or(current.fk_operator);
        let comments = req.comments.or(current.comments);
        // 更新起止时间：新 date_st/date_ed 输入 → 重解析 segm-date；缺省沿用当前 qk_period
        let qk_period = if req.date_st.is_some() || req.date_ed.is_some() {
            self.resolve_period(
                req.date_st.or_else(|| {
                    current
                        ._refs
                        .as_ref()
                        .and_then(|r| r.get("qk_period"))
                        .and_then(|v| v.get("date_st"))
                        .and_then(|v| v.as_str())
                        .and_then(|s| {
                            chrono::DateTime::parse_from_rfc3339(s)
                                .ok()
                                .map(|d| d.with_timezone(&chrono::Utc))
                        })
                }),
                req.date_ed.or_else(|| {
                    current
                        ._refs
                        .as_ref()
                        .and_then(|r| r.get("qk_period"))
                        .and_then(|v| v.get("date_ed"))
                        .and_then(|v| v.as_str())
                        .and_then(|s| {
                            chrono::DateTime::parse_from_rfc3339(s)
                                .ok()
                                .map(|d| d.with_timezone(&chrono::Utc))
                        })
                }),
            )
            .await?
        } else {
            current.qk_period
        };
        sqlx::query_as::<_, DelegationRule>(
            r#"UPDATE isahl.zc_id_operation
               SET notice = $1, code = $2, fk_subject = $3, fk_operator = $4, comments = $5, qk_period = $6, updated_by_id = $7
               WHERE id = $8 AND deleted_at IS NULL
               RETURNING id, notice AS name, code, fk_subject, fk_operator, comments, qk_period, created_at, updated_at, deleted_at"#,
        )
        .bind(&name)
        .bind(&code)
        .bind(fk_subject)
        .bind(fk_operator)
        .bind(&comments)
        .bind(qk_period)
        .bind(user_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AliothError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        sqlx::query(
            "UPDATE isahl.zc_id_operation SET deleted_at = NOW(), deleted_by_id = $1 WHERE id = $2 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(AliothError::from)?;
        Ok(())
    }
}
