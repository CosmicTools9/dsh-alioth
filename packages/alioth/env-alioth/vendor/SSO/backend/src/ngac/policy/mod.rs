pub mod access_rights;
pub mod object_attribute;
pub mod user_attribute;

pub use access_rights::*;
pub use object_attribute::*;
pub use user_attribute::*;

// Re-export for backward compatibility
pub use crate::ngac::pdp::{
    NgacAccessRight as AccessRight, NgacAssociation as Association, NgacProhibition as Prohibition,
};
