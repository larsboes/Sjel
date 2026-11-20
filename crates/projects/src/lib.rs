#![deny(unsafe_code)]

pub mod commands;
pub mod jobs;
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
        let path = owner_file().ok_or("no Projects owner file; set SJEL_PROJECTS_DB_PATH or stage data/projects-index/projects.db")?;
        let path = path.canonicalize()?;
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

/// The Projects owner file: `SJEL_PROJECTS_DB_PATH`, or `data/projects-index/projects.db` in the
/// overlay when that file exists. It stays outside `data/projects`, the Markdown record tree.
#[must_use]
pub fn owner_file() -> Option<std::path::PathBuf> {
    if let Some(path) = sjel_config::env_var("SJEL_PROJECTS_DB_PATH")
        .ok()
        .filter(|path| !path.trim().is_empty())
    {
        return Some(sjel_config::expand_tilde(&path));
    }
    sjel_config::overlay_data_dir("projects-index")
        .map(|dir| dir.join("projects.db"))
        .filter(|path| path.is_file())
}
