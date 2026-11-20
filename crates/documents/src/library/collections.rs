//! Named collections of documents, such as every paper for one flat, contract or employer.
//!
//! A collection holds references only: membership never moves, files or archives a record.
//! The tables are the index; one sidecar per collection under `records/collections/`
//! lets `restore-index` rebuild them.

use std::fs;

use super::{
    check_id, engine, params, write_atomic, Connection, Fallible, Library, LibraryError,
    OptionalExtension, Sha256,
};
use documents_api::DocumentCollection as Collection;
use sha2::Digest;

const MAX_NAME_CHARS: usize = 120;

pub(super) fn migrate(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS documents_collections (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            category TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS documents_collection_members (
            collection_id TEXT NOT NULL REFERENCES documents_collections(id) ON DELETE CASCADE,
            document_id TEXT NOT NULL REFERENCES documents_items(id),
            PRIMARY KEY (collection_id, document_id)
        );",
    )?;
    Ok(())
}

fn clean_name(name: &str) -> Fallible<String> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(LibraryError::Invalid(format!(
            "a collection name needs 1..={MAX_NAME_CHARS} printable characters"
        )));
    }
    Ok(name.to_string())
}

impl Library {
    /// Create an empty collection.
    ///
    /// # Errors
    /// The name is invalid or already used, or persistence fails.
    pub fn create_collection(&self, name: &str, category: Option<&str>) -> Fallible<Collection> {
        let name = clean_name(name)?;
        let category = category.map(clean_name).transpose()?;
        let created_at = chrono::Utc::now().to_rfc3339();
        let id = super::hex(&Sha256::digest(format!("{name}\n{created_at}").as_bytes()))[..32]
            .to_string();
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        if self.collection(&name)?.is_some() {
            return Err(LibraryError::Invalid(
                "a collection with that name exists".into(),
            ));
        }
        conn.execute(
            "INSERT INTO documents_collections (id, name, category, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id.as_str(), name.as_str(), category.as_deref(), created_at.as_str()],
        )?;
        let collection = Collection {
            id,
            name,
            category,
            created_at,
            member_ids: Vec::new(),
        };
        self.write_collection(&collection)?;
        Ok(collection)
    }

    /// Every collection with its members, by name.
    ///
    /// # Errors
    /// The tables cannot be read.
    pub fn collections(&self) -> Fallible<Vec<Collection>> {
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        let mut statement = conn.prepare(
            "SELECT id, name, category, created_at FROM documents_collections ORDER BY name",
        )?;
        let rows = statement.query_map(params![], |row| {
            Ok(Collection {
                id: row.get(0)?,
                name: row.get(1)?,
                category: row.get(2)?,
                created_at: row.get(3)?,
                member_ids: Vec::new(),
            })
        })?;
        let mut collections = rows.collect::<Result<Vec<_>, _>>()?;
        let mut members = conn.prepare(
            "SELECT document_id FROM documents_collection_members WHERE collection_id = ?1 ORDER BY document_id",
        )?;
        for collection in &mut collections {
            collection.member_ids = members
                .query_map(params![collection.id.as_str()], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
        }
        Ok(collections)
    }

    /// One collection by id or exact name.
    ///
    /// # Errors
    /// The tables cannot be read.
    pub fn collection(&self, id_or_name: &str) -> Fallible<Option<Collection>> {
        Ok(self
            .collections()?
            .into_iter()
            .find(|c| c.id == id_or_name || c.name == id_or_name.trim()))
    }

    /// Add and remove members in one transaction. Unknown documents are refused.
    ///
    /// # Errors
    /// The collection or a document is missing, or persistence fails.
    pub fn update_collection(
        &self,
        id_or_name: &str,
        add: &[String],
        remove: &[String],
    ) -> Fallible<Collection> {
        let collection = self
            .collection(id_or_name)?
            .ok_or_else(|| LibraryError::Invalid("no collection with that id or name".into()))?;
        let mut conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        let transaction = engine::write_transaction(&mut conn)?;
        for id in add {
            check_id(id)?;
            let exists = transaction
                .query_row(
                    "SELECT 1 FROM documents_items WHERE id = ?1",
                    params![id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .is_some();
            if !exists {
                return Err(LibraryError::Invalid(format!("no document {id}")));
            }
            transaction.execute(
                "INSERT OR IGNORE INTO documents_collection_members (collection_id, document_id) VALUES (?1, ?2)",
                params![collection.id.as_str(), id.as_str()],
            )?;
        }
        for id in remove {
            check_id(id)?;
            transaction.execute(
                "DELETE FROM documents_collection_members WHERE collection_id = ?1 AND document_id = ?2",
                params![collection.id.as_str(), id.as_str()],
            )?;
        }
        transaction.commit()?;
        let updated = self
            .collection(&collection.id)?
            .ok_or_else(|| LibraryError::Invalid("collection disappeared".into()))?;
        self.write_collection(&updated)?;
        Ok(updated)
    }

    fn write_collection(&self, collection: &Collection) -> Fallible<()> {
        let directory = self.root.join("records").join("collections");
        fs::create_dir_all(&directory)?;
        let bytes =
            serde_json::to_vec(collection).map_err(|e| LibraryError::Invalid(e.to_string()))?;
        write_atomic(&directory.join(format!("{}.json", collection.id)), &bytes)
    }

    /// Rebuild missing collections and memberships from their sidecars.
    pub(super) fn restore_collection_sidecars(&self) -> Fallible<()> {
        let directory = self.root.join("records").join("collections");
        let Ok(entries) = fs::read_dir(&directory) else {
            return Ok(());
        };
        let conn = self
            .pool
            .get()
            .map_err(|e| LibraryError::Store(e.to_string()))?;
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let collection: Collection = serde_json::from_slice(&fs::read(&path)?)
                .map_err(|e| LibraryError::Invalid(format!("invalid collection sidecar: {e}")))?;
            if path.file_stem().and_then(|s| s.to_str()) != Some(collection.id.as_str()) {
                return Err(LibraryError::Invalid(
                    "collection sidecar filename does not match its id".into(),
                ));
            }
            conn.execute(
                "INSERT OR IGNORE INTO documents_collections (id, name, category, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![collection.id.as_str(), collection.name.as_str(), collection.category.as_deref(), collection.created_at.as_str()],
            )?;
            for id in &collection.member_ids {
                if self.get(id)?.is_some() {
                    conn.execute(
                        "INSERT OR IGNORE INTO documents_collection_members (collection_id, document_id) VALUES (?1, ?2)",
                        params![collection.id.as_str(), id.as_str()],
                    )?;
                }
            }
        }
        Ok(())
    }
}
