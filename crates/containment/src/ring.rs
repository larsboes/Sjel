use std::collections::HashMap;
use std::path::Path;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ring {
    Trusted,
    Contained,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderEntry {
    pub highest_data_class: Option<String>,
    pub reviewed_at: Option<String>,
    pub why: Option<String>,
    pub ring: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct ProvidersFile {
    #[serde(flatten)]
    pub providers: HashMap<String, ProviderEntry>,
}

impl ProvidersFile {
    pub fn load_from_file(path: &Path) -> Result<Self, std::io::Error> {
        let content = std::fs::read_to_string(path)?;
        let parsed: ProvidersFile = toml::from_str(&content).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })?;
        Ok(parsed)
    }

    pub fn classify_ring(&self, provider: &str) -> Ring {
        if let Some(entry) = self.providers.get(provider) {
            if let Some(ref r) = entry.ring {
                if r == "trusted" {
                    return Ring::Trusted;
                }
            }
            if provider == "openai" || provider == "anthropic" {
                return Ring::Trusted;
            }
        }
        Ring::Contained
    }
}
