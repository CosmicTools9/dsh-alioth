// TransitRouteRepository（split 自 repository.rs 单体，④ 候选）
//! 身份实体 Repository — 标准 CRUD 实现
//!
//! Identity 使用自定义 Repository，其余实体组合 GenericRepository，
//! 仅自定义 create/update 的 INSERT/UPDATE SQL。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::repository::AliothRepository;
use crud::GenericRepository;
use sqlx::{AssertSqlSafe, PgPool};

use crate::models::{CreateTransitRouteRequest, TransitRoute, UpdateTransitRouteRequest};

use super::resolve_coord_sys_id;

// ═══════════════════════════════════════════════
// TransitRoute Repository
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct TransitRouteRepository {
    generic: GenericRepository<TransitRoute>,
    pool: PgPool,
}

impl TransitRouteRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for TransitRouteRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<TransitRoute, CreateTransitRouteRequest, UpdateTransitRouteRequest, ApiError>
    for TransitRouteRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<TransitRoute>, ApiError> {
        self.generic.list_refs(query).await
    }
    async fn get(&self, id: i64) -> Result<Option<TransitRoute>, ApiError> {
        self.generic.get_refs(id, None).await
    }
    async fn create(
        &self,
        req: CreateTransitRouteRequest,
        user_id: i64,
    ) -> Result<TransitRoute, ApiError> {
        // 方案 B：sk_unit 语义 = 图商坐标系（zc_id_unit-geo）
        let coord_sys_id =
            resolve_coord_sys_id(&self.pool, req.coord_sys.as_deref(), req.sk_unit).await?;
        // Create coordinate records for each waypoint in ak_nodes
        let node_ids: Vec<i64> = if let Some(ref nodes) = req.ak_nodes {
            if let Some(arr) = nodes.as_array() {
                let mut ids = Vec::with_capacity(arr.len());
                for node in arr {
                    if let Some(n) = node.as_i64() {
                        ids.push(n);
                    } else if let (Some(lat), Some(lng)) = (
                        node.get("lat").and_then(|v| v.as_f64()),
                        node.get("lng").and_then(|v| v.as_f64()),
                    ) {
                        let name = node
                            .get("notice")
                            .and_then(|v| v.as_str())
                            .unwrap_or("waypoint");
                        let cid: i64 = sqlx::query_scalar(
                            r#"INSERT INTO "isahl"."zc_id_geog-point" (notice, point, sk_unit, created_by_id)
                               VALUES ($1, ST_SetSRID(ST_MakePoint($2, $3), 4326), $4, $5) RETURNING id"#,
                        )
                        .bind(name).bind(lng).bind(lat).bind(coord_sys_id).bind(user_id)
                        .fetch_one(&self.pool)
                        .await
                        .map_err(ApiError::from)?;
                        ids.push(cid);
                    }
                }
                ids
            } else {
                return Err(ApiError::Validation {
                    field: "ak_nodes".into(),
                    message: "must be an array of {lat,lng} objects or coordinate IDs".into(),
                });
            }
        } else {
            Vec::new()
        };
        let created = sqlx::query_as::<_, TransitRoute>(
            r#"INSERT INTO "isahl"."zc_id_geog-path" (notice, code, comments, ak_nodes, sk_unit, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id, notice, code, comments, ak_nodes, sk_unit, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&node_ids)
        .bind(coord_sys_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;
        // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, ("TX", "FJA", "↑_GG"))
                .await
                .map_err(ApiError::from)?;
        if let Err(e) = sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_stor-traffic_line" (id, notice, code, comments, qk_path, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES (isahl.gen_next_zuid(), $1,
                       'TL-' || COALESCE(NULLIF($2, ''), 'R' || (($3) % 1000000)),
                       $4, $3, $5, $6, $7, $8)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(&created.notice)
        .bind(&created.code)
        .bind(created.id)
        .bind(&created.comments)
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .execute(&self.pool)
        .await
        {
            eprintln!("transit-route → traffic_line 同步失败: {}", e);
        }
        Ok(created)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateTransitRouteRequest,
        user_id: i64,
    ) -> Result<Option<TransitRoute>, ApiError> {
        // 方案 B：sk_unit 语义 = 图商坐标系（zc_id_unit-geo）
        let coord_sys_id =
            resolve_coord_sys_id(&self.pool, req.coord_sys.as_deref(), req.sk_unit).await?;
        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if req.notice.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
        if req.code.is_some() {
            idx += 1;
            sets.push(format!("code = ${}", idx));
        }
        if req.comments.is_some() {
            idx += 1;
            sets.push(format!("comments = ${}", idx));
        }
        if req.ak_nodes.is_some() {
            idx += 1;
            sets.push(format!("ak_nodes = ${}", idx));
        }
        if req.sk_unit.is_some() || req.coord_sys.is_some() {
            idx += 1;
            sets.push(format!("sk_unit = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_geom-path" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, notice, code, comments, ak_nodes, sk_unit, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, TransitRoute>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(ref v) = req.ak_nodes {
            q = q.bind(v);
        }
        if req.sk_unit.is_some() || req.coord_sys.is_some() {
            q = q.bind(coord_sys_id);
        }

        q = q.bind(user_id);
        q = q.bind(id);

        let updated = q.fetch_optional(&self.pool).await.map_err(ApiError::from)?;

        // 同步约束（批注：路线改名委托页不同步）：
        // 名称变更时同步委托页线路（zc_id_stor-traffic_line，qk_path 匹配本路线）
        if let Some(route) = &updated {
            if let Some(new_notice) = &route.notice {
                let _ = sqlx::query(
                    r#"UPDATE "isahl"."zc_id_stor-traffic_line"
                       SET notice = $1, updated_at = NOW()
                       WHERE qk_path = $2 AND deleted_at IS NULL"#,
                )
                .bind(new_notice)
                .bind(id)
                .execute(&self.pool)
                .await;
            }
        }
        Ok(updated)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
