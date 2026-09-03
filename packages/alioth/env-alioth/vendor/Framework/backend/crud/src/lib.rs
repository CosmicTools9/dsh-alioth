//! Alioth CRUD Framework — v2
//!
//! 提供标准化的 CRUD 基础设施，采用**深度模块**设计：
//!
//! - **`AliothDbEntity`**：实体元数据 trait（表名、字段列表、软删除/审计约定）
//! - **`QueryBuilder<E>`**：SQL 组合模块，从元数据生成标准查询
//! - **`AliothRepository<E, C, U>`**：模块级 CRUD 接口（seam）
//! - **`SubtableRouter`**：子表路由 seam
//! - **`crud_routes`** / **`crud_*`**：泛型 actix-web handler 工厂
//!
//! # 快速开始
//!
//! ```rust,ignore
//! use crud::{AliothDbEntity, AliothRepository, crud_routes};
//!
//! // 1. 为实体声明元数据
//! impl AliothDbEntity for Product {
//!     const TABLE_NAME: &'static str = "isahl.zc_id_production";
//!     const SELECT_FIELDS: &'static str = "id, code, notice, ...";
//!     const SOFT_DELETE: bool = true;
//!     const HAS_AUDIT: bool = true;
//! }
//!
//! // 2. 实现 AliothRepository
//! #[async_trait]
//! impl AliothRepository<Product, CreateRequest, UpdateRequest> for ProductRepository {
//!     async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Product>, AliothError> {
//!         QueryBuilder::<Product>::from_list_query(&self.pool, query).fetch(1, 20).await
//!     }
//!     // ... create/update 由模块自行实现
//! }
//!
//! // 3. 注册路由
//! pub fn config(cfg: &mut web::ServiceConfig) {
//!     cfg.configure(crud_routes::<Product, CreateRequest, UpdateRequest, ProductRepository>("/products"));
//! }
//! ```

pub mod audit_outbox;
pub mod batch;
pub mod bind_json;
pub mod cascade;
pub mod column_types;
pub mod document_ingester;
pub mod entity;
pub mod error;
pub mod filter;
pub mod fk_index;
pub mod generic_repository;
pub mod handler;
pub mod ontology_handler;
pub mod pagination;
pub mod query_builder;
pub mod reference;
pub mod repository;
pub mod schema_handler;
pub mod schema_repository;
pub mod search;
pub mod sort;
pub mod subject;
pub mod transaction;
pub mod trigger;

// 重新导出常用类型
pub use audit_outbox::{
    enqueue as audit_enqueue, enqueue_tx as audit_enqueue_tx, replay as audit_replay, AuditAction,
    AuditScope, OutboxEvent, OutboxWorker, ReplayFilter,
};
pub use batch::{BatchCreateRequest, BatchDeleteRequest, BatchResponse};
pub use cascade::{
    bare_table_name, cascade_soft_delete, derive_cascade_targets, CascadeConfig, CascadeKind,
    CascadeTarget,
};
pub use entity::{AliothDbEntity, Identifiable};

pub use common::PaginatedResponse;
pub use document_ingester::{ingest_document, DocumentIngester};
pub use error::CrudError;
pub use generic_repository::GenericRepository;
pub use handler::{
    crud_batch_delete, crud_create, crud_create_with_extensions, crud_delete,
    crud_delete_with_extensions, crud_get, crud_get_refs, crud_list, crud_list_refs,
    crud_ref_routes, crud_routes, crud_routes_with_extensions, crud_update,
    crud_update_with_extensions, extract_user_id, parse_authorized_columns, parse_visible_ids,
    register_created_resource_ngac, resolve_dk_ctx,
};
pub use ontology_handler::{
    ontology_routes, ontology_routes_with_reference, reference_routes, LeafListResponse,
};
pub use pagination::{ListQuery, ListQueryExt};
pub use query_builder::QueryBuilder;
pub use reference::{
    build_refs_select_suffix, Card, HasReferenceJoins, JoinKind, JunctionField, ReferenceJoin,
};
pub use repository::{AliothRepository, SubtableRouter};
pub use schema_handler::schema_routes;
pub use schema_repository::{AliothLeaf, Binding, SchemaRepository};
pub use search::KeywordSearchable;
pub use sort::Sort;
