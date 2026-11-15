#![deny(unsafe_code)]

pub mod ops;
pub mod record;
pub mod sources;
pub mod store;
pub mod workspace;

pub use ops::Task;
pub use record::{Kind, Record};
pub use store::Projects;

/// Open the shared store and the overlay's rendered tree.
///
/// # Errors
/// No overlay is configured, so a write would have no second copy, or the store cannot open.
pub async fn open_default() -> Result<Projects, Box<dyn std::error::Error + Send + Sync>> {
    let root = Projects::default_root()
        .ok_or("SJEL_PERSONAL_ROOT is not set: no overlay to render projects into")?;
    let projects = Projects::open(&sjel_config::database_path(), root).await?;
    Ok(projects.with_sources(sjel_config::overlay_config("projects-sources.json")))
}
