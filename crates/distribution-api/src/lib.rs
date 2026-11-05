//! Pure wire models and contracts for Sjel package distribution and runtime features.
//!
//! Shared across `sjel-cli` (package installer, verification, and runtime placement)
//! and `sjel-gpui-core` (client desktop feature catalogue and package manager).
//! Zero server dependencies.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Component, Path};

pub const MAX_MANIFEST: u64 = 1_048_576;
pub const MAX_PACKAGE: u64 = 1_073_741_824;
pub const OFFICIAL_RELEASE_URL: &str = "https://api.github.com/repos/larsboes/Sjel/releases/latest";
pub const CATALOGUE_LIMIT: usize = 4 * 1024 * 1024;

/// Target host platform triple string formatted as `<arch>-<os-suffix>`.
pub fn host_platform() -> String {
    let suffix = match std::env::consts::OS {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-gnu",
        other => other,
    };
    format!("{}-{suffix}", std::env::consts::ARCH)
}

/// Checks whether an identifier is a valid package or feature name.
pub fn valid_package_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !value.starts_with('-')
}

/// Validates a release version tag (must be `vMAJOR.MINOR.PATCH`).
pub fn valid_release_version(value: &str) -> bool {
    let Some(version) = value.strip_prefix('v') else {
        return false;
    };
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Validates a SHA-256 hexadecimal string (64 lowercase hex digits).
pub fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// Validates a relative file path inside a package archive.
pub fn relative_file(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// Validates feature metadata according to package specification.
pub fn valid_feature_metadata(feature: &FeatureMetadata) -> bool {
    feature.schema == 1
        && !feature.title.trim().is_empty()
        && feature.title.len() <= 80
        && !feature.summary.trim().is_empty()
        && feature.summary.len() <= 240
        && !feature.category.trim().is_empty()
        && feature.category.len() <= 40
        && !feature.recommend_topics.is_empty()
        && feature.recommend_topics.len() <= 24
        && feature.recommend_topics.iter().all(|topic| {
            !topic.is_empty()
                && topic.len() <= 48
                && topic
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
        && feature
            .recommend_topics
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            == feature.recommend_topics.len()
        && feature.data_access.len() <= 32
        && feature
            .data_access
            .iter()
            .all(|item| !item.trim().is_empty() && item.len() <= 120)
        && feature.data_access.iter().collect::<BTreeSet<_>>().len() == feature.data_access.len()
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeatureMetadata {
    pub schema: u32,
    pub title: String,
    pub summary: String,
    pub category: String,
    #[serde(default)]
    pub recommend_topics: Vec<String>,
    #[serde(default)]
    pub data_access: Vec<String>,
}

pub type InstalledFeatureMetadata = FeatureMetadata;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub executable: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub schema: u32,
    pub name: String,
    pub version: String,
    pub platform: String,
    #[serde(default)]
    pub requires: Vec<String>,
    pub feature: FeatureMetadata,
    #[serde(default)]
    pub files: Vec<PackageFile>,
}

impl Package {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || !valid_package_identifier(&self.name)
            || self.platform != host_platform()
        {
            return Err("unsupported package schema, name or host platform".into());
        }
        if !valid_release_version(&self.version) {
            return Err("package version must be vMAJOR.MINOR.PATCH".into());
        }
        if self
            .requires
            .iter()
            .any(|r| !valid_package_identifier(r) || r == &self.name)
        {
            return Err("invalid package prerequisites".into());
        }
        if self.requires.iter().collect::<BTreeSet<_>>().len() != self.requires.len() {
            return Err("repeated package prerequisite".into());
        }
        if !valid_feature_metadata(&self.feature) {
            return Err("invalid package feature metadata".into());
        }
        if self.files.is_empty() || self.files.len() > 4096 {
            return Err("package must contain 1–4096 files".into());
        }
        let mut paths = BTreeSet::new();
        let mut total = 0_u64;
        for file in &self.files {
            if !relative_file(&file.path)
                || file.path == "sjel-package.json"
                || !valid_sha256(&file.sha256)
                || !paths.insert(&file.path)
            {
                return Err("invalid or repeated package file".into());
            }
            total = total
                .checked_add(file.bytes)
                .ok_or("package size overflow")?;
            if total > MAX_PACKAGE {
                return Err("package exceeds the 1 GiB unpacked limit".into());
            }
        }
        if !paths.contains(&"service.toml".to_owned()) {
            return Err("package is missing service.toml".into());
        }
        for path in &paths {
            if Path::new(path).ancestors().skip(1).any(|parent| {
                parent
                    .to_str()
                    .map(|p| paths.contains(&p.to_owned()))
                    .unwrap_or(false)
            }) {
                return Err("a package file is also used as a directory".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageReceipt {
    pub schema: u32,
    pub package_sha256: String,
    pub package: Package,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    pub platform: String,
    pub feature: FeatureMetadata,
    #[serde(default)]
    pub requires: Vec<String>,
    pub package_sha256: String,
    pub integrity: String,
}

pub type InstalledFeature = InstalledPackage;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct AvailableFeature {
    pub name: String,
    pub version: String,
    pub platform: String,
    #[serde(default)]
    pub requires: Vec<String>,
    pub feature: FeatureMetadata,
    pub asset: String,
    pub bytes: u64,
    pub sha256: String,
    pub download_url: String,
    pub installable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct ReleaseSource {
    pub repository: String,
    pub commit: String,
    pub dirty: bool,
    pub rebuilt: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct ReleaseAsset {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct ReleaseCapability {
    pub name: String,
    pub version: String,
    pub platform: String,
    #[serde(default)]
    pub requires: Vec<String>,
    pub feature: FeatureMetadata,
    pub asset: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct ReleaseCatalogue {
    pub schema: u32,
    pub version: String,
    pub source: ReleaseSource,
    pub friend_ready: bool,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
    #[serde(default)]
    pub capabilities: Vec<ReleaseCapability>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct GithubReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct GithubRelease {
    pub tag_name: String,
    #[serde(default)]
    pub assets: Vec<GithubReleaseAsset>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct Runtime {
    #[serde(default)]
    pub schema: u32,
    pub version: String,
    pub platform: String,
    #[serde(default)]
    pub core: Vec<String>,
}

pub type RuntimeIdentity = Runtime;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_identifiers() {
        assert!(valid_package_identifier("soundscape"));
        assert!(valid_package_identifier("sjel-status-1"));
        assert!(!valid_package_identifier("-invalid"));
        assert!(!valid_package_identifier("Invalid"));
        assert!(!valid_package_identifier(""));
    }

    #[test]
    fn validates_release_version() {
        assert!(valid_release_version("v0.1.0"));
        assert!(valid_release_version("v1.23.45"));
        assert!(!valid_release_version("0.1.0"));
        assert!(!valid_release_version("v0.1"));
        assert!(!valid_release_version("v0.1.0-alpha"));
    }

    #[test]
    fn validates_feature_metadata() {
        let meta = FeatureMetadata {
            schema: 1,
            title: "Soundscape".into(),
            summary: "Generative audio synthesis".into(),
            category: "Media".into(),
            recommend_topics: vec!["audio".into(), "ambient".into()],
            data_access: vec!["mic".into()],
        };
        assert!(valid_feature_metadata(&meta));
    }
}
