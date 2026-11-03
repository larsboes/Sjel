#![deny(unsafe_code)]

pub mod classification;
mod classification_model;
pub mod duplicates;
pub mod inbox;
mod library;
pub mod review;
pub mod server;
pub mod split;
pub mod workflow;

pub use library::{Document, Library, LibraryError, Review, Stored};

/// Open the shared store and this deployment's canonical document directory.
///
/// # Errors
/// No overlay is configured, or the store or document root cannot be opened.
pub fn open_default() -> Result<Library, LibraryError> {
    let root = sjel_config::overlay_data_dir("documents")
        .ok_or_else(|| LibraryError::Invalid("no active Sjel overlay".into()))?;
    let classification =
        classification::ClassificationConfig::load().map_err(LibraryError::Invalid)?;
    Library::open_with_config(&sjel_config::database_path(), root, classification)
}
