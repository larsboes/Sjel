#![deny(unsafe_code)]

pub mod inbox;
mod library;
pub mod server;

pub use library::{Document, Library, LibraryError, Review, Stored};

/// Open the shared store and this deployment's canonical document directory.
///
/// # Errors
/// No overlay is configured, or the store or document root cannot be opened.
pub fn open_default() -> Result<Library, LibraryError> {
    let root = sjel_config::overlay_data_dir("documents")
        .ok_or_else(|| LibraryError::Invalid("no active Sjel overlay".into()))?;
    Library::open(&sjel_config::database_path(), root)
}
