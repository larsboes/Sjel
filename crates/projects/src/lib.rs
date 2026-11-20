#![deny(unsafe_code)]

pub mod ops;
pub mod record;
pub mod sources;
pub mod store;
pub mod workspace;

pub use ops::Task;
pub use record::{Kind, Record};
pub use store::Projects;

/// Open the overlay's record files and their index.
///
/// # Errors
/// No overlay is configured, so there are no files, or the store cannot open.
pub async fn open_default() -> Result<Projects, Box<dyn std::error::Error + Send + Sync>> {
    let root = Projects::default_root()
        .ok_or("SJEL_PERSONAL_ROOT is not set: no overlay holds the project files")?;
    #[cfg(feature = "turso-wal")]
    let database = {
        let path = sjel_config::env_var("SJEL_PROJECTS_DB_PATH")?;
        if path.trim().is_empty() {
            return Err("SJEL_PROJECTS_DB_PATH must name the separate Projects file".into());
        }
        let path = sjel_config::expand_tilde(&path).canonicalize()?;
        if !path.is_file()
            || sjel_config::database_path()
                .canonicalize()
                .is_ok_and(|shared| shared == path)
        {
            return Err("Projects requires an existing, separate owner file".into());
        }
        path
    };
    #[cfg(not(feature = "turso-wal"))]
    let database = sjel_config::database_path();
    let projects = Projects::open(&database, root).await?;
    Ok(projects.with_sources(sjel_config::overlay_config("projects-sources.json")))
}
