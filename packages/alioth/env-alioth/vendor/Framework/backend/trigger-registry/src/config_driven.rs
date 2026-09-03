//! Config-Driven Trigger Registry Initialization
//!
//! Loads the inheritance graph from `isahl_meta.meta_collections.config`
//! at runtime, enabling `SmartTriggerRegistry` to resolve triggers dynamically
//! without relying on the hard-coded `load_default_alioth_hierarchy()`.
//!
//! ## Design
//!
//! 1. `meta_collections.config.inherits` → builds `InheritanceGraph`
//! 2. Trigger implementations are registered statically via
//!    `register_all_triggers()` (Strategy A: full Rust replacement)

use crate::inheritance::{InheritanceGraph, SmartTriggerRegistry};
use sqlx::{PgPool, Row};

/// Load inheritance graph from `meta_collections.config.inherits`.
///
/// Every row's `config->inherits` array is read and fed into
/// `InheritanceGraph::add_inheritance`.  Tables with no inherits
/// become roots.
pub async fn load_inheritance_graph_from_db(
    pool: &PgPool,
) -> Result<InheritanceGraph, sqlx::Error> {
    let rows = sqlx::query("SELECT table_name, config FROM isahl_meta.meta_collections")
        .fetch_all(pool)
        .await?;

    let mut graph = InheritanceGraph::new();

    for row in &rows {
        let table_name: String = row.get("table_name");
        let config: serde_json::Value = row.get("config");

        let parents: Vec<String> = config
            .get("inherits")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        graph.add_inheritance(&table_name, parents);
    }

    Ok(graph)
}

/// Initialize a `SmartTriggerRegistry` from database configuration.
///
/// 1. Loads `InheritanceGraph` from `meta_collections.config.inherits`
/// 2. Registers all static Rust trigger implementations
///
/// # Example
/// ```rust,ignore
/// let mut registry = init_smart_registry_from_db(&pool).await?;
/// let triggers = registry.get_triggers_for_leaf(
///     "zc_id_scene", TriggerTiming::Before, TriggerOperation::Insert
/// );
/// ```
pub async fn init_smart_registry_from_db(
    pool: &PgPool,
) -> Result<SmartTriggerRegistry, sqlx::Error> {
    let mut registry = SmartTriggerRegistry::new();

    // Dynamic inheritance graph from DB config
    let graph = load_inheritance_graph_from_db(pool).await?;
    registry.set_inheritance(graph);

    // Static trigger implementations (Strategy A) — DB-driven init is Meta-only
    super::init::register_all_triggers(&mut registry, crate::AppContainer::Meta);

    Ok(registry)
}
