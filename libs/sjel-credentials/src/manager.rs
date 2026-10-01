use crate::{
    CredentialCatalog, CredentialDescriptor, CredentialError, CredentialId, CredentialProvider,
    Presence, ProviderKind, SecretValue,
};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Lifecycle coordinator shared by the operator CLI and native desktop commands.
///
/// The catalog stores provider references only. `materialize` is the sole operation that copies
/// secret bytes out of a provider, and it writes them directly to an owner-only runtime file.
pub struct CredentialManager {
    catalog_path: PathBuf,
    providers: HashMap<ProviderKind, Arc<dyn CredentialProvider>>,
}

impl CredentialManager {
    pub fn new(
        catalog_path: impl Into<PathBuf>,
        providers: impl IntoIterator<Item = Arc<dyn CredentialProvider>>,
    ) -> Result<Self, CredentialError> {
        let mut registered = HashMap::new();
        for provider in providers {
            if registered.insert(provider.kind(), provider).is_some() {
                return Err(CredentialError::DuplicateProvider);
            }
        }
        Ok(Self {
            catalog_path: catalog_path.into(),
            providers: registered,
        })
    }

    pub fn list(&self) -> Result<Vec<CredentialDescriptor>, CredentialError> {
        Ok(CredentialCatalog::load(&self.catalog_path)?.credentials)
    }

    pub fn status(&self, id: &CredentialId) -> Result<Presence, CredentialError> {
        let catalog = CredentialCatalog::load(&self.catalog_path)?;
        let entry = catalog
            .credentials
            .iter()
            .find(|entry| &entry.id == id)
            .ok_or(CredentialError::NotFound)?;
        self.provider(entry.reference.kind())?
            .status(id, &entry.reference)
    }

    pub fn create(
        &self,
        id: CredentialId,
        label: String,
        provider_kind: ProviderKind,
        consumers: Vec<String>,
        value: &SecretValue,
    ) -> Result<CredentialDescriptor, CredentialError> {
        let mut catalog = CredentialCatalog::load(&self.catalog_path)?;
        if catalog.credentials.iter().any(|entry| entry.id == id) {
            return Err(CredentialError::AlreadyExists);
        }
        if label.trim().is_empty() {
            return Err(CredentialError::InvalidCatalog);
        }
        let provider = self.provider(provider_kind)?;
        let reference = provider.create(&id, &label, value)?;
        if reference.kind() != provider_kind {
            return Err(CredentialError::WrongProviderReference);
        }
        let descriptor = CredentialDescriptor {
            id: id.clone(),
            label,
            reference,
            consumers,
        };
        catalog.credentials.push(descriptor.clone());
        if let Err(error) = catalog.save(&self.catalog_path) {
            let _ = provider.delete(&id, &descriptor.reference);
            return Err(error);
        }
        Ok(descriptor)
    }

    pub fn update(&self, id: &CredentialId, value: &SecretValue) -> Result<(), CredentialError> {
        let catalog = CredentialCatalog::load(&self.catalog_path)?;
        let entry = catalog
            .credentials
            .iter()
            .find(|entry| &entry.id == id)
            .ok_or(CredentialError::NotFound)?;
        self.provider(entry.reference.kind())?
            .update(id, &entry.reference, value)
    }

    pub fn delete(&self, id: &CredentialId) -> Result<(), CredentialError> {
        let mut catalog = CredentialCatalog::load(&self.catalog_path)?;
        let index = catalog
            .credentials
            .iter()
            .position(|entry| &entry.id == id)
            .ok_or(CredentialError::NotFound)?;
        let entry = catalog.credentials.remove(index);
        let provider = self.provider(entry.reference.kind())?;
        match provider.delete(id, &entry.reference) {
            Ok(()) | Err(CredentialError::NotFound) => {}
            Err(error) => return Err(error),
        }
        catalog.save(&self.catalog_path)
    }

    /// Copy a value to an unattended consumer's runtime file. The file is replaced atomically
    /// with mode 0600; no secret bytes are returned to the caller.
    pub fn materialize(
        &self,
        id: &CredentialId,
        destination: &Path,
    ) -> Result<(), CredentialError> {
        let catalog = CredentialCatalog::load(&self.catalog_path)?;
        let entry = catalog
            .credentials
            .iter()
            .find(|entry| &entry.id == id)
            .ok_or(CredentialError::NotFound)?;
        let secret = self
            .provider(entry.reference.kind())?
            .get(id, &entry.reference)?;
        write_runtime_file(destination, secret.expose_bytes())
    }

    fn provider(&self, kind: ProviderKind) -> Result<&dyn CredentialProvider, CredentialError> {
        self.providers
            .get(&kind)
            .map(Arc::as_ref)
            .ok_or(CredentialError::ProviderUnavailable)
    }
}

fn write_runtime_file(path: &Path, value: &[u8]) -> Result<(), CredentialError> {
    let parent = path
        .parent()
        .ok_or(CredentialError::MaterializationFailed)?;
    fs::create_dir_all(parent).map_err(|_| CredentialError::MaterializationFailed)?;
    let file_name = path
        .file_name()
        .ok_or(CredentialError::MaterializationFailed)?
        .to_string_lossy();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        counter
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp)
        .map_err(|_| CredentialError::MaterializationFailed)?;
    if file.write_all(value).is_err() || file.sync_all().is_err() {
        drop(file);
        let _ = fs::remove_file(&temp);
        return Err(CredentialError::MaterializationFailed);
    }
    drop(file);
    if fs::rename(&temp, path).is_err() {
        let _ = fs::remove_file(&temp);
        return Err(CredentialError::MaterializationFailed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderReference;
    use std::sync::Mutex;

    struct MemoryProvider {
        values: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl MemoryProvider {
        fn new() -> Self {
            Self {
                values: Mutex::new(HashMap::new()),
            }
        }
    }

    impl CredentialProvider for MemoryProvider {
        fn kind(&self) -> ProviderKind {
            ProviderKind::Bitwarden
        }

        fn status(
            &self,
            id: &CredentialId,
            _: &ProviderReference,
        ) -> Result<Presence, CredentialError> {
            Ok(if self.values.lock().unwrap().contains_key(id.as_str()) {
                Presence::Present
            } else {
                Presence::Missing
            })
        }

        fn create(
            &self,
            id: &CredentialId,
            _: &str,
            value: &SecretValue,
        ) -> Result<ProviderReference, CredentialError> {
            self.values
                .lock()
                .unwrap()
                .insert(id.to_string(), value.expose_bytes().to_vec());
            Ok(ProviderReference::Bitwarden {
                item_id: format!("fake-{}", id.as_str()),
            })
        }

        fn update(
            &self,
            id: &CredentialId,
            _: &ProviderReference,
            value: &SecretValue,
        ) -> Result<(), CredentialError> {
            self.values
                .lock()
                .unwrap()
                .insert(id.to_string(), value.expose_bytes().to_vec());
            Ok(())
        }

        fn get(
            &self,
            id: &CredentialId,
            _: &ProviderReference,
        ) -> Result<SecretValue, CredentialError> {
            let value = self
                .values
                .lock()
                .unwrap()
                .get(id.as_str())
                .cloned()
                .ok_or(CredentialError::NotFound)?;
            SecretValue::new(value)
        }

        fn delete(&self, id: &CredentialId, _: &ProviderReference) -> Result<(), CredentialError> {
            self.values
                .lock()
                .unwrap()
                .remove(id.as_str())
                .map(|_| ())
                .ok_or(CredentialError::NotFound)
        }
    }

    #[test]
    fn lifecycle_materializes_only_to_owner_only_file() {
        let root =
            std::env::temp_dir().join(format!("sjel-credentials-manager-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let catalog_path = root.join("config/credentials.toml");
        let runtime_path = root.join("run/inbound-token");
        let provider = Arc::new(MemoryProvider::new());
        let manager = CredentialManager::new(
            &catalog_path,
            vec![provider.clone() as Arc<dyn CredentialProvider>],
        )
        .unwrap();
        let id = CredentialId::parse("inbound-auth").unwrap();
        let secret = SecretValue::new(b"test-token-only".to_vec()).unwrap();

        manager
            .create(
                id.clone(),
                "Deployment inbound auth".into(),
                ProviderKind::Bitwarden,
                vec!["sparpreis-watch".into()],
                &secret,
            )
            .unwrap();
        assert_eq!(manager.status(&id).unwrap(), Presence::Present);
        manager.materialize(&id, &runtime_path).unwrap();
        assert_eq!(fs::read(&runtime_path).unwrap(), b"test-token-only");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&runtime_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        manager
            .update(&id, &SecretValue::new(b"rotated-token".to_vec()).unwrap())
            .unwrap();
        manager.materialize(&id, &runtime_path).unwrap();
        assert_eq!(fs::read(&runtime_path).unwrap(), b"rotated-token");
        manager.delete(&id).unwrap();
        assert!(manager.list().unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_provider_kinds_are_rejected() {
        let root =
            std::env::temp_dir().join(format!("sjel-credentials-providers-{}", std::process::id()));
        let memory = Arc::new(MemoryProvider::new());
        let first: Arc<dyn CredentialProvider> = memory.clone();
        let second: Arc<dyn CredentialProvider> = memory;
        assert!(matches!(
            CredentialManager::new(&root, [first, second]),
            Err(CredentialError::DuplicateProvider)
        ));
        let _ = fs::remove_dir_all(root);
    }
}
