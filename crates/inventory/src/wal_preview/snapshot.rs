use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Extension};
use tokio::sync::Mutex;

use super::{boom, Error, HttpError, WalPreview};

const SNAPSHOT_FILE: &str = "inventory.db";

pub(super) struct SnapshotDirectory {
    directory: PathBuf,
    current: Mutex<()>,
}

impl SnapshotDirectory {
    pub(super) fn open(directory: &Path, protected_files: &[&Path]) -> Result<Self, Error> {
        let directory = directory.canonicalize()?;
        if !directory.is_dir() {
            return Err("provide an existing Inventory snapshot directory".into());
        }
        if let Ok(target) = directory.join(SNAPSHOT_FILE).canonicalize() {
            for path in protected_files {
                if target == path.canonicalize()? {
                    return Err("the snapshot destination overlaps an active database".into());
                }
            }
        }
        Ok(Self {
            directory,
            current: Mutex::new(()),
        })
    }

    pub(super) async fn take(&self, store: &WalPreview) -> Result<(), Error> {
        let _guard = self.current.lock().await;
        let staging = tempfile::Builder::new()
            .prefix(".inventory-snapshot-")
            .tempdir_in(&self.directory)?;
        let candidate = staging.path().join(SNAPSHOT_FILE);
        let path = candidate.to_str().ok_or("snapshot path is not UTF-8")?;
        // VACUUM INTO reads its path literally, including doubled quotes.
        let delimiter = if !path.contains('\'') {
            '\''
        } else if !path.contains('"') {
            '"'
        } else {
            return Err("Inventory snapshot path contains both quote characters".into());
        };
        let connection = store.database.connect()?;
        connection
            .execute(format!("VACUUM INTO {delimiter}{path}{delimiter}"), ())
            .await?;
        drop(connection);
        let journal = candidate.with_extension("db-wal");
        if tokio::fs::try_exists(&journal).await? && tokio::fs::metadata(&journal).await?.len() > 0
        {
            return Err("Inventory snapshot still depends on its WAL sidecar".into());
        }

        let verified = WalPreview::open(&candidate, false).await?;
        let connection = verified.database.connect()?;
        let mut rows = connection.query("PRAGMA integrity_check", ()).await?;
        let result = rows
            .next()
            .await?
            .ok_or("snapshot integrity check returned no result")?
            .get::<String>(0)?;
        if result != "ok" || rows.next().await?.is_some() {
            return Err("Inventory snapshot failed its integrity check".into());
        }
        drop(rows);
        drop(connection);
        drop(verified);
        tokio::fs::rename(&candidate, self.directory.join(SNAPSHOT_FILE)).await?;
        tokio::fs::File::open(&self.directory)
            .await?
            .sync_all()
            .await?;
        Ok(())
    }
}

pub(super) async fn export(
    State(store): State<Arc<WalPreview>>,
    Extension(directory): Extension<Arc<SnapshotDirectory>>,
) -> Result<StatusCode, HttpError> {
    directory.take(&store).await.map_err(boom)?;
    Ok(StatusCode::NO_CONTENT)
}
