//! Trigger Initialization
//!
//! Registers all triggers with the global trigger registry.

use crate::{
    business::BusinessRegistryLoader, config_driven, inheritance::SmartTriggerRegistry,
    registry::TriggerRegistry, template::TriggerHandle,
};
use sqlx::PgPool;
use std::sync::{Arc, OnceLock};

// ============================================
// Basic Trigger Registry (TriggerRegistry)
// ============================================

/// Initialize the basic trigger registry with all defined triggers
pub fn init_trigger_registry() -> TriggerRegistry {
    let mut loader = BusinessRegistryLoader::new();
    loader.register_builtin_templates();
    let mut registry = loader.into_registry();

    // Object-level triggers
    registry.register(TriggerHandle::from_template(Arc::new(
        crate::object::ObjectONumberTemplate,
    )));
    registry.register(TriggerHandle::from_template(Arc::new(
        crate::object::ObjectI18nTemplate,
    )));

    // Dynamic triggers
    registry.register(TriggerHandle::from_template(Arc::new(
        crate::dynamic::ForeignKeyRelationMapperTemplate,
    )));
    registry.register(TriggerHandle::from_template(Arc::new(
        crate::dynamic::RelationCycleDetectTemplate,
    )));

    // Auxiliary triggers
    registry.register(TriggerHandle::from_template(Arc::new(
        crate::auxiliary::ProductionAfterTemplate,
    )));
    registry.register(TriggerHandle::from_template(Arc::new(
        crate::auxiliary::ProductionDeleteTemplate,
    )));

    // Category-level triggers
    registry.register(TriggerHandle::from_template(Arc::new(
        crate::category::CategoryCSortTemplate,
    )));

    // Consensus-level v_sort trigger
    registry.register(TriggerHandle::from_template(Arc::new(
        crate::sort::ConsensusVSortTemplate,
    )));

    // Dimension-level v_sort trigger
    registry.register(TriggerHandle::from_template(Arc::new(
        crate::sort::DimensionVSortTemplate,
    )));

    // Consensus code auto-generation trigger
    registry.register(TriggerHandle::from_template(Arc::new(
        crate::sort::ConsensusCodeTemplate,
    )));

    // Cycle detection triggers
    registry.register(TriggerHandle::from_template(Arc::new(
        crate::cycle_detect::PlaceCycleDetectTemplate,
    )));
    registry.register(TriggerHandle::from_template(Arc::new(
        crate::cycle_detect::StanClauseCycleDetectTemplate,
    )));

    registry
}

// ============================================
// Smart Trigger Registry (SmartTriggerRegistry)
// ============================================

/// Register all triggers on a SmartTriggerRegistry
///
/// Triggers are registered on their parent tables and automatically
/// inherited by all leaf tables (complete child tables).
///
/// `container` controls which triggers are registered:
/// - `ObjectI18nTemplate` is only registered in `Meta` mode because it modifies `isahl_meta.meta_fields`
pub fn register_all_triggers(registry: &mut SmartTriggerRegistry, container: crate::AppContainer) {
    // ============================================
    // zc_id_object Level Triggers
    // ============================================
    registry.register_on_parent(
        "zc_id_object",
        TriggerHandle::from_template(Arc::new(crate::object::ObjectONumberTemplate)),
    );
    if container == crate::AppContainer::Meta {
        registry.register_on_parent(
            "zc_id_object",
            TriggerHandle::from_template(Arc::new(crate::object::ObjectI18nTemplate)),
        );
    }

    // ============================================
    // zc_id_lifecycle Level Triggers
    // ============================================
    registry.register_on_parent(
        "zc_id_lifecycle",
        TriggerHandle::from_template(Arc::new(crate::lifecycle::LifecycleBizSetTemplate)),
    );
    registry.register_on_parent(
        "zc_id_lifecycle",
        TriggerHandle::from_template(Arc::new(crate::lifecycle::LifecycleNumberTemplate)),
    );
    registry.register_on_parent(
        "zc_id_lifecycle",
        TriggerHandle::from_template(Arc::new(crate::lifecycle::NoticeDedupTemplate)),
    );
    registry.register_on_parent(
        "zc_id_lifecycle",
        TriggerHandle::from_template(Arc::new(crate::lifecycle::LifecycleDeleteTemplate)),
    );
    registry.register_on_parent(
        "zc_id_lifecycle",
        TriggerHandle::from_template(Arc::new(crate::lifecycle::LifecycleNonSelfDeleteTemplate)),
    );
    // Lifecycle _f_ / _t_ auto-derivation
    registry.register_on_parent(
        "zc_id_lifecycle",
        TriggerHandle::from_template(Arc::new(crate::lifecycle::LifecycleBizTemplate)),
    );
    // Lifecycle NGAC object-attribute sync
    registry.register_on_parent(
        "zc_id_lifecycle",
        TriggerHandle::from_template(Arc::new(crate::lifecycle::LifecycleNgacSyncTemplate)),
    );
    // Relation update triggers (registered on relation tables, not zc_id_lifecycle)
    registry.register_on_parent(
        "zc_id_lifecycle_r_evaluation",
        TriggerHandle::from_template(Arc::new(crate::lifecycle::LifecycleRelationUpdateTemplate)),
    );
    registry.register_on_parent(
        "zc_id_lifecycle_r_category",
        TriggerHandle::from_template(Arc::new(crate::lifecycle::LifecycleRelationUpdateTemplate)),
    );
    registry.register_on_parent(
        "zc_id_lifecycle_r_tags",
        TriggerHandle::from_template(Arc::new(crate::lifecycle::LifecycleRelationUpdateTemplate)),
    );

    // ============================================
    // zc_ad_dimension Level Triggers
    // ============================================
    // 注册点修正（B1）：LifecycleInjectiveTemplate 的 applies_to = ZC_ID_LIFECYCLE_TABLES，
    // 故须注册在 zc_id_lifecycle 而非 zc_ad_dimension（lifecycle 表并非 zc_ad_dimension 后代，
    // 原注册点导致其从不触发）。
    registry.register_on_parent(
        "zc_id_lifecycle",
        TriggerHandle::from_template(Arc::new(crate::dimension::LifecycleInjectiveTemplate)),
    );

    // ============================================
    // Business-Specific Templates (TriggerTemplate → TriggerHandle)
    // ============================================

    // 库存物化（ADR D-018）：物化逻辑为 Framework 库函数
    // （stock_materialization::apply_voucher/apply_nest/validate_nest），
    // 由业务 Service 写路径显式调用——不注册为触发器（避免双倍物化）。
    // 盘点校准 = 生成校准凭证（stat-sto-voucher）走 apply_voucher 自动物化。

    // Auto-code triggers (registered on dimension parent tables)
    registry.register_on_parent(
        "zc_id_scene",
        TriggerHandle::from_template(Arc::new(crate::business::DimensionAutoCodeTemplate::scene())),
    );
    registry.register_on_parent(
        "zc_id_factor",
        TriggerHandle::from_template(Arc::new(
            crate::business::DimensionAutoCodeTemplate::factor(),
        )),
    );
    registry.register_on_parent(
        "zc_id_function",
        TriggerHandle::from_template(Arc::new(
            crate::business::DimensionAutoCodeTemplate::function(),
        )),
    );

    // Consensus/category code auto-generation trigger
    registry.register_on_parent(
        "zc_id_consensus",
        TriggerHandle::from_template(Arc::new(crate::sort::ConsensusCodeTemplate)),
    );
    registry.register_on_parent(
        "zc_id_category",
        TriggerHandle::from_template(Arc::new(crate::sort::ConsensusCodeTemplate)),
    );

    // Entity triggers
    registry.register_on_parent(
        "zc_id_entity",
        TriggerHandle::from_template(Arc::new(crate::business::EntityUserTemplate)),
    );
    registry.register_on_parent(
        "zc_id_entity",
        TriggerHandle::from_template(Arc::new(crate::entity::EntityDefaultTemplate)),
    );

    // Version triggers
    registry.register_on_parent(
        "zc_id_version",
        TriggerHandle::from_template(Arc::new(crate::version::VersionHeadFlagTemplate)),
    );

    // Product/BOM triggers
    registry.register_on_parent(
        "zc_id_prod-sales",
        TriggerHandle::from_template(Arc::new(crate::product::ProdPNumberTemplate::sales())),
    );
    registry.register_on_parent(
        "zc_id_prod-purchase",
        TriggerHandle::from_template(Arc::new(crate::product::ProdPNumberTemplate::purchase())),
    );
    registry.register_on_parent(
        "zc_id_prod-request",
        TriggerHandle::from_template(Arc::new(crate::product::ProdPNumberTemplate::request())),
    );
    registry.register_on_parent(
        "zc_id_prod-made",
        TriggerHandle::from_template(Arc::new(crate::product::ProdPNumberTemplate::made())),
    );
    registry.register_on_parent(
        "zc_id_bom",
        TriggerHandle::from_template(Arc::new(crate::bom::BomBNumberTemplate)),
    );

    // Operation/Process triggers
    registry.register_on_parent(
        "zc_id_operation",
        TriggerHandle::from_template(Arc::new(crate::operation::OperationOpNumberTemplate)),
    );
    registry.register_on_parent(
        "zc_id_process",
        TriggerHandle::from_template(Arc::new(crate::operation::ProcessPNumberTemplate)),
    );

    // Project/Task triggers
    registry.register_on_parent(
        "zc_id_project",
        TriggerHandle::from_template(Arc::new(crate::business::ProjectParticipantsTemplate)),
    );
    registry.register_on_parent(
        "zc_id_task",
        TriggerHandle::from_template(Arc::new(crate::business::TaskInitiatorTemplate)),
    );

    // ============================================
    // Category c_sort_ Triggers
    // ============================================
    registry.register_on_parent(
        "zc_id_category",
        TriggerHandle::from_template(Arc::new(crate::category::CategoryCSortTemplate)),
    );

    // ============================================
    // Consensus v_sort Triggers
    // ============================================
    registry.register_on_parent(
        "zc_id_consensus",
        TriggerHandle::from_template(Arc::new(crate::sort::ConsensusVSortTemplate)),
    );

    // ============================================
    // Dimension v_sort Triggers
    // ============================================
    registry.register_on_parent(
        "zc_ad_dimension",
        TriggerHandle::from_template(Arc::new(crate::sort::DimensionVSortTemplate)),
    );

    // ============================================
    // Dynamic triggers
    // ============================================
    registry.register_on_parent(
        "zc_id_lifecycle",
        TriggerHandle::from_template(Arc::new(crate::dynamic::ForeignKeyRelationMapperTemplate)),
    );
    registry.register_on_parent(
        "zc_ad_relation",
        TriggerHandle::from_template(Arc::new(crate::dynamic::RelationCycleDetectTemplate)),
    );
    // rr 关系表 → evaluation 引用计数（原 DB 触发器 trg_ref_count_junction 迁移；
    // 注册在 rr 树根，通配覆盖全部 zc_id_master_rr_slave 子树）
    registry.register_on_parent(
        "zc_id_master_rr_slave",
        TriggerHandle::from_template(Arc::new(crate::dynamic::RrRefCountTemplate)),
    );

    // ============================================
    // Hierarchy Cycle Detection Triggers
    // ============================================
    registry.register_on_parent(
        "zc_id_place",
        TriggerHandle::from_template(Arc::new(crate::cycle_detect::PlaceCycleDetectTemplate)),
    );
    registry.register_on_parent(
        "zc_id_stan-clause",
        TriggerHandle::from_template(Arc::new(crate::cycle_detect::StanClauseCycleDetectTemplate)),
    );

    // ============================================
    // Auxiliary Triggers
    // ============================================
    registry.register_on_parent(
        "zc_id_production",
        TriggerHandle::from_template(Arc::new(crate::auxiliary::ProductionAfterTemplate)),
    );
    registry.register_on_parent(
        "zc_id_production",
        TriggerHandle::from_template(Arc::new(crate::auxiliary::ProductionDeleteTemplate)),
    );
}

// ============================================
// DB-Driven Initialization
// ============================================

/// Initialize a SmartTriggerRegistry from database `meta_collections.config`.
///
/// This is the runtime entry-point for Strategy A.  It loads the inheritance
/// graph from `config.inherits` and registers all Rust trigger implementations.
///
/// # Example
/// ```rust,ignore
/// let registry = init_smart_registry_from_db(&pool).await?;
/// ```
pub async fn init_smart_registry_from_db(
    pool: &PgPool,
) -> Result<SmartTriggerRegistry, sqlx::Error> {
    config_driven::init_smart_registry_from_db(pool).await
}

// ============================================
// Singleton / Global Registry
// ============================================

/// Get a pre-configured trigger registry (singleton pattern)
static TRIGGER_REGISTRY: OnceLock<TriggerRegistry> = OnceLock::new();

pub fn get_trigger_registry() -> &'static TriggerRegistry {
    TRIGGER_REGISTRY.get_or_init(init_trigger_registry)
}

/// Global SmartTriggerRegistry cached as Arc<RwLock<SmartTriggerRegistry>>
static SMART_REGISTRY: OnceLock<Arc<tokio::sync::RwLock<SmartTriggerRegistry>>> = OnceLock::new();

/// Initialize the global SmartTriggerRegistry from database configuration.
///
/// Should be called once at application startup (e.g. in `main.rs`).
/// On failure, falls back to the hard-coded default hierarchy so that
/// the server can still start.
///
/// # Container-aware loading
/// - `AppContainer::Meta`: attempts to load inheritance graph from `isahl_meta.meta_collections`
pub async fn init_smart_registry_global(
    _pool: &PgPool,
    container: crate::AppContainer,
) -> Result<(), String> {
    let mut registry = SmartTriggerRegistry::new();

    // Inheritance graph loaded at compile time via build.rs.
    // For runtime hot-reload after schema changes, use refresh_smart_registry_from_db().
    registry.inheritance.load_default_alioth_hierarchy();

    // Register all static Rust trigger implementations (container-aware)
    register_all_triggers(&mut registry, container);

    SMART_REGISTRY
        .set(Arc::new(tokio::sync::RwLock::new(registry)))
        .map_err(|_| "SmartTriggerRegistry already initialized".to_string())?;

    Ok(())
}

/// Get the global SmartTriggerRegistry.
///
/// Returns `None` if `init_smart_registry_global` has not been called yet.
pub fn get_smart_registry() -> Option<Arc<tokio::sync::RwLock<SmartTriggerRegistry>>> {
    SMART_REGISTRY.get().cloned()
}

/// Refresh the global SmartTriggerRegistry inheritance graph from the database.
///
/// Call this after `meta_collections.config` inheritance relationships have
/// been modified (e.g. via Meta Admin).
pub async fn refresh_smart_registry_from_db(pool: &PgPool) -> Result<(), String> {
    let Some(registry_arc) = get_smart_registry() else {
        return Err("SmartTriggerRegistry not initialized".to_string());
    };

    let graph = config_driven::load_inheritance_graph_from_db(pool)
        .await
        .map_err(|e| format!("Failed to load inheritance graph: {}", e))?;

    let mut registry = registry_arc.write().await;
    registry.set_inheritance(graph);

    Ok(())
}

// ============================================
// Tests
// ============================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TriggerOperation, TriggerTiming};

    #[test]
    fn test_trigger_registry_initialization() {
        let registry = init_trigger_registry();

        // Check that zc_id_scene has triggers registered
        assert!(registry.has_triggers(
            "zc_id_scene",
            TriggerTiming::Before,
            TriggerOperation::Insert
        ));
        assert!(registry.has_triggers(
            "zc_id_scene",
            TriggerTiming::After,
            TriggerOperation::Insert
        ));

        // Check that zc_id_bill has lifecycle triggers
        assert!(registry.has_triggers(
            "zc_id_bill",
            TriggerTiming::Before,
            TriggerOperation::Insert
        ));
    }

    #[test]
    fn test_smart_registry_registration() {
        let mut registry = SmartTriggerRegistry::new();
        registry.inheritance.load_default_alioth_hierarchy();
        register_all_triggers(&mut registry, crate::AppContainer::Meta);

        // zc_id_scene is a leaf table that has triggers registered directly on it
        let triggers = registry.get_triggers_for_leaf(
            "zc_id_scene",
            TriggerTiming::Before,
            TriggerOperation::Insert,
        );
        assert!(!triggers.is_empty());
    }

    #[test]
    fn test_singleton_registry() {
        let registry1 = get_trigger_registry();
        let registry2 = get_trigger_registry();

        // Both should point to the same instance
        assert!(std::ptr::eq(registry1, registry2));
    }
}
