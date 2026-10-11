pub mod client;
pub mod resource_registry;
pub mod types;

pub use client::HttpNgacClient;
pub use resource_registry::{
    action_for_method, PrefixRule, ResolvedResource, ResourceRegistry, ResourceTypeDef,
};
pub use types::*;
