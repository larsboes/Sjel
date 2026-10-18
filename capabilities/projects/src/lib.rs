#![deny(unsafe_code)]

pub mod ops;
pub mod record;
pub mod store;

pub use ops::Task;
pub use record::{Kind, Record};
pub use store::Projects;

/// Open the shared store and the overlay's rendered tree.
///
/// # Errors
/// No overlay is configured, so a write would have no second copy, or the store cannot open.
pub fn open_default() -> Result<Projects, Box<dyn std::error::Error>> {
    let root = Projects::default_root()
        .ok_or("SJEL_PERSONAL_ROOT is not set: no overlay to render projects into")?;
    Projects::open(&sjel_config::database_path(), root)
}
