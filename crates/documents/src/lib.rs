#![deny(unsafe_code)]

pub mod classification;
mod classification_model;
pub mod commands;
pub mod duplicates;
pub mod inbox;
#[cfg(feature = "turso-wal")]
pub mod jobs;
mod library;
pub mod review;
#[cfg(any(feature = "turso-wal", test))]
mod search_tokens;
pub mod server;
pub mod split;
pub mod workflow;

pub use library::{Document, Library, LibraryError, Review, Stored};

/// Open the metadata store and the canonical document directory.
///
/// # Errors
/// No overlay is configured, or the store or document root cannot be opened.
pub fn open_default() -> Result<Library, LibraryError> {
    let root = sjel_config::overlay_data_dir("documents")
        .ok_or_else(|| LibraryError::Invalid("no active Sjel overlay".into()))?;
    let classification =
        classification::ClassificationConfig::load().map_err(LibraryError::Invalid)?;
    Library::open_with_config(&database_path()?, root, classification)
}

fn database_path() -> Result<std::path::PathBuf, LibraryError> {
    #[cfg(feature = "turso-wal")]
    {
        sjel_config::env_var("SJEL_DOCUMENTS_DB_PATH")
            .ok()
            .filter(|path| !path.trim().is_empty())
            .map(|path| sjel_config::expand_tilde(&path))
            .or_else(|| {
                let path = sjel_config::overlay_data_dir("documents-store")?.join("documents.db");
                path.is_file().then_some(path)
            })
            .ok_or_else(|| {
                LibraryError::Invalid(
                    "Turso requires SJEL_DOCUMENTS_DB_PATH to name the separate metadata file"
                        .into(),
                )
            })
    }
    #[cfg(not(feature = "turso-wal"))]
    {
        Ok(sjel_config::database_path())
    }
}
