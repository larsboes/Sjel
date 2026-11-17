//! Registry projections over owner-classified Projects replica reads.

use projects_api::reads::RecordReadResponse;
use serde_json::json;

use super::{project, request, Binding, View};
use crate::{
    shell::{ShellError, ShellResult},
    store::TursoStore,
};

/// Open the bundled Projects declaration against the local copy without network access.
///
/// # Errors
/// Refuses missing classification metadata or queued project edits.
pub async fn load(store: &TursoStore) -> ShellResult<View> {
    read(
        store,
        Binding {
            object: "projects.project".into(),
            read: "list".into(),
            parameters: Default::default(),
            equals: Default::default(),
        },
    )
    .await
}

/// Resolve a declared Projects read; writes and arbitrary routes are unavailable.
///
/// # Errors
/// Refuses invalid bindings, incomplete metadata or unavailable replica records.
pub async fn read(store: &TursoStore, binding: Binding) -> ShellResult<View> {
    let registry = projects_api::registry::declaration();
    let (path, member, many) = request(&registry, &binding)?;
    let url = reqwest::Url::parse(&format!("http://localhost{path}"))
        .map_err(|_| ShellError::Unreadable)?;
    let response = match url.path() {
        "/projects/api/projects" => json!(store.registry_projects().await?),
        "/projects/api/project" => {
            json!(store.registry_records(&binding.parameters["name"]).await?)
        }
        "/projects/api/record" => {
            let path = &binding.parameters["path"];
            let record = store
                .fetch_record(path)
                .await?
                .ok_or_else(|| ShellError::Store("Record is not on this device".into()))?;
            let mut classified = store
                .registry_records(&record.project)
                .await?
                .records
                .into_iter()
                .find(|row| &row.path == path)
                .ok_or_else(|| ShellError::Store("Record metadata is not on this device".into()))?;
            classified.content = Some(record.content);
            json!(RecordReadResponse { record: classified })
        }
        _ => {
            return Err(ShellError::Refused(
                400,
                Some("Read is not available in the replica".into()),
            ))
        }
    };
    project(registry, binding, &response, &member, many)
}
