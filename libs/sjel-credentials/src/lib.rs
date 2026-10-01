//! Provider-neutral credential operations.
//!
//! Credential descriptors contain only identifiers, labels, provider selection, and provider
//! references. Secret bytes are a separate type and are never serializable or printable. This
//! boundary is shared by the operator CLI and native Tauri commands; callers must not return a
//! [`SecretValue`] to a webview or include one in diagnostics.

mod keychain;
mod manager;

pub use keychain::{CommandOutput, CommandRunner, KeychainProvider, SystemCommandRunner};
pub use manager::CredentialManager;

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use thiserror::Error;
use zeroize::Zeroizing;

/// Stable, non-secret name for one managed credential.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialId(String);

impl CredentialId {
    /// Accept a short slug so the ID can be used safely in provider references and file names.
    pub fn parse(value: impl Into<String>) -> Result<Self, CredentialError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        if !valid {
            return Err(CredentialError::InvalidId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CredentialId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The store that is authoritative for a credential. Provider choice belongs to each entry,
/// not to the deployment as a whole.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Keychain,
    Bitwarden,
}

/// Non-secret location information for a provider item.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum ProviderReference {
    Keychain { account: String },
    Bitwarden { item_id: String },
}

impl ProviderReference {
    pub fn kind(&self) -> ProviderKind {
        match self {
            Self::Keychain { .. } => ProviderKind::Keychain,
            Self::Bitwarden { .. } => ProviderKind::Bitwarden,
        }
    }
}

/// Metadata shown by CLI and UI. It deliberately has no field for a secret value.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialDescriptor {
    pub id: CredentialId,
    pub label: String,
    pub reference: ProviderReference,
    #[serde(default)]
    pub consumers: Vec<String>,
}

/// Non-secret inventory of credential references. Values remain in their selected providers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialCatalog {
    #[serde(default = "catalog_version")]
    pub version: u32,
    #[serde(default)]
    pub credentials: Vec<CredentialDescriptor>,
}

fn catalog_version() -> u32 {
    1
}

impl Default for CredentialCatalog {
    fn default() -> Self {
        Self {
            version: catalog_version(),
            credentials: Vec::new(),
        }
    }
}

impl CredentialCatalog {
    /// Load metadata from TOML. A missing file means an empty catalog; other I/O or parse
    /// failures are reported without including file contents.
    pub fn load(path: &Path) -> Result<Self, CredentialError> {
        let body = match std::fs::read_to_string(path) {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(_) => return Err(CredentialError::CatalogOperation { operation: "read" }),
        };
        let catalog: Self = toml::from_str(&body)
            .map_err(|_| CredentialError::CatalogOperation { operation: "parse" })?;
        catalog.validate()?;
        Ok(catalog)
    }

    /// Atomically replace the metadata file with owner-only permissions on Unix systems.
    pub fn save(&self, path: &Path) -> Result<(), CredentialError> {
        self.validate()?;
        let body = toml::to_string_pretty(self).map_err(|_| CredentialError::CatalogOperation {
            operation: "serialize",
        })?;
        let parent = path
            .parent()
            .ok_or(CredentialError::CatalogOperation { operation: "path" })?;
        std::fs::create_dir_all(parent).map_err(|_| CredentialError::CatalogOperation {
            operation: "create directory",
        })?;
        let file_name = path
            .file_name()
            .ok_or(CredentialError::CatalogOperation { operation: "path" })?
            .to_string_lossy();
        let temp = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp)
            .map_err(|_| CredentialError::CatalogOperation {
                operation: "create temp",
            })?;
        use std::io::Write;
        if file.write_all(body.as_bytes()).is_err() {
            let _ = std::fs::remove_file(&temp);
            return Err(CredentialError::CatalogOperation { operation: "write" });
        }
        file.sync_all()
            .map_err(|_| CredentialError::CatalogOperation { operation: "sync" })?;
        drop(file);
        if std::fs::rename(&temp, path).is_err() {
            let _ = std::fs::remove_file(&temp);
            return Err(CredentialError::CatalogOperation {
                operation: "replace",
            });
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), CredentialError> {
        if self.version != catalog_version() {
            return Err(CredentialError::CatalogOperation {
                operation: "version",
            });
        }
        let mut ids = std::collections::HashSet::new();
        for entry in &self.credentials {
            CredentialId::parse(entry.id.as_str().to_string())?;
            if entry.label.trim().is_empty() || !ids.insert(entry.id.as_str()) {
                return Err(CredentialError::InvalidCatalog);
            }
            if let ProviderReference::Bitwarden { item_id } = &entry.reference {
                if item_id.trim().is_empty() || item_id.len() > 128 {
                    return Err(CredentialError::InvalidReference);
                }
            }
        }
        Ok(())
    }
}

/// Secret bytes held only while a provider operation is in flight.
///
/// This type does not implement `Debug`, `Display`, `Serialize`, or `Deserialize`; callers must
/// opt in to borrowing the bytes. Its owned buffer is zeroized when dropped.
pub struct SecretValue(Zeroizing<Vec<u8>>);

impl SecretValue {
    pub fn new(value: impl Into<Vec<u8>>) -> Result<Self, CredentialError> {
        let value = value.into();
        if value.is_empty() {
            return Err(CredentialError::EmptySecret);
        }
        Ok(Self(Zeroizing::new(value)))
    }

    pub(crate) fn from_zeroizing(value: Zeroizing<Vec<u8>>) -> Result<Self, CredentialError> {
        if value.is_empty() {
            return Err(CredentialError::EmptySecret);
        }
        Ok(Self(value))
    }

    pub fn expose_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Presence {
    Present,
    Missing,
}

/// Provider implementations expose lifecycle operations without deciding how metadata is
/// stored or how callers present a secret-entry prompt.
pub trait CredentialProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn status(
        &self,
        id: &CredentialId,
        reference: &ProviderReference,
    ) -> Result<Presence, CredentialError>;
    fn create(
        &self,
        id: &CredentialId,
        label: &str,
        value: &SecretValue,
    ) -> Result<ProviderReference, CredentialError>;
    fn update(
        &self,
        id: &CredentialId,
        reference: &ProviderReference,
        value: &SecretValue,
    ) -> Result<(), CredentialError>;
    fn get(
        &self,
        id: &CredentialId,
        reference: &ProviderReference,
    ) -> Result<SecretValue, CredentialError>;
    fn delete(
        &self,
        id: &CredentialId,
        reference: &ProviderReference,
    ) -> Result<(), CredentialError>;
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum CredentialError {
    #[error("credential ID must be 1–64 ASCII letters, digits, '-' or '_'")]
    InvalidId,
    #[error("credential secret must not be empty")]
    EmptySecret,
    #[error("provider reference does not match the selected provider")]
    WrongProviderReference,
    #[error("provider reference is invalid")]
    InvalidReference,
    #[error("credential was not found in the selected provider")]
    NotFound,
    #[error("provider operation failed: {operation}")]
    ProviderOperation { operation: &'static str },
    #[error("provider is not available on this platform")]
    UnsupportedPlatform,
    #[error("provider returned an invalid secret representation")]
    InvalidSecretRepresentation,
    #[error("credential catalog is invalid")]
    InvalidCatalog,
    #[error("credential already exists")]
    AlreadyExists,
    #[error("provider is not registered")]
    ProviderUnavailable,
    #[error("a provider was registered more than once")]
    DuplicateProvider,
    #[error("credential materialization failed")]
    MaterializationFailed,
    #[error("credential catalog operation failed: {operation}")]
    CatalogOperation { operation: &'static str },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_safe_slugs() {
        assert_eq!(
            CredentialId::parse("inbound-auth").unwrap().as_str(),
            "inbound-auth"
        );
        for invalid in ["", "with space", "../file", "dot.id", "a/b", "a\nb"] {
            assert_eq!(
                CredentialId::parse(invalid),
                Err(CredentialError::InvalidId)
            );
        }
    }

    #[test]
    fn descriptors_serialize_references_but_cannot_carry_secret_values() {
        let descriptor = CredentialDescriptor {
            id: CredentialId::parse("inbound-auth").unwrap(),
            label: "Deployment inbound auth".into(),
            reference: ProviderReference::Keychain {
                account: "sjel-inbound-auth".into(),
            },
            consumers: vec!["sparpreis-watch".into()],
        };
        let json = serde_json::to_string(&descriptor).unwrap();
        assert!(json.contains("inbound-auth"));
        assert!(json.contains("keychain"));
        assert!(!json.contains("secret"));
    }

    #[test]
    fn catalog_round_trip_contains_references_but_not_values() {
        let root =
            std::env::temp_dir().join(format!("sjel-credentials-test-{}", std::process::id()));
        let path = root.join("credentials.toml");
        let catalog = CredentialCatalog {
            version: 1,
            credentials: vec![CredentialDescriptor {
                id: CredentialId::parse("inbound-auth").unwrap(),
                label: "Deployment inbound auth".into(),
                reference: ProviderReference::Keychain {
                    account: "sparpreis-watch".into(),
                },
                consumers: vec!["sparpreis-watch".into()],
            }],
        };

        catalog.save(&path).unwrap();
        let restored = CredentialCatalog::load(&path).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(restored, catalog);
        assert!(body.contains("keychain"));
        assert!(!body.contains("password"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn catalog_rejects_unknown_secret_fields() {
        let parsed = toml::from_str::<CredentialCatalog>(
            "version = 1\n\n[[credentials]]\nid = 'token'\nlabel = 'Token'\nsecret = 'must-not-load'\n[credentials.reference]\nprovider = 'keychain'\naccount = 'token'\n",
        );
        assert!(parsed.is_err());
    }

    #[test]
    fn empty_secrets_are_rejected() {
        assert!(matches!(
            SecretValue::new(Vec::new()),
            Err(CredentialError::EmptySecret)
        ));
    }
}
