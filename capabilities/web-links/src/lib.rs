#![deny(unsafe_code)]

pub mod history;
pub mod store;

use serde::{Deserialize, Serialize};
use url::Url;

pub use history::{collect as collect_history, Browser, HistoryCandidate};
pub use store::{Candidate, CandidateDecision, Link, LinksStore};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    pub url: String,
    pub host: String,
    pub group: String,
    pub tenant: Option<String>,
}

/// Validate and normalize an HTTP(S) destination without dropping meaningful path or query data.
pub fn destination(raw: &str) -> Result<Destination, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 2_048 {
        return Err("Enter a URL up to 2,048 bytes.".into());
    }
    let mut parsed =
        Url::parse(raw).map_err(|_| "Enter a complete http:// or https:// URL.".to_owned())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("Only HTTP and HTTPS destinations are supported.".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URLs containing credentials cannot be saved.".into());
    }
    parsed.set_fragment(None);
    let host = parsed
        .host_str()
        .ok_or_else(|| "The URL has no host.".to_owned())?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let group = site_name(&host);
    let tenant = tenant(&host, parsed.path());
    Ok(Destination {
        url: parsed.into(),
        host,
        group,
        tenant,
    })
}

fn site_name(host: &str) -> String {
    if host == "x.com"
        || host.ends_with(".x.com")
        || host == "twitter.com"
        || host.ends_with(".twitter.com")
    {
        return "X".into();
    }
    if host == "reddit.com" || host.ends_with(".reddit.com") {
        return "Reddit".into();
    }
    if host == "github.com" || host.ends_with(".github.com") {
        return "GitHub".into();
    }
    if host == "youtube.com" || host.ends_with(".youtube.com") || host == "youtu.be" {
        return "YouTube".into();
    }
    if host == "google.com" || host.ends_with(".google.com") {
        return "Google".into();
    }
    if host == "linkedin.com" || host.ends_with(".linkedin.com") {
        return "LinkedIn".into();
    }
    host.strip_prefix("www.").unwrap_or(host).to_owned()
}

fn tenant(host: &str, path: &str) -> Option<String> {
    let parts: Vec<_> = path.split('/').filter(|part| !part.is_empty()).collect();
    if (host == "reddit.com" || host.ends_with(".reddit.com")) && parts.len() >= 2 {
        return matches!(parts[0], "r" | "u" | "user")
            .then(|| format!("{}/{}", parts[0], parts[1]));
    }
    if (host == "x.com"
        || host.ends_with(".x.com")
        || host == "twitter.com"
        || host.ends_with(".twitter.com"))
        && parts.first().is_some_and(|part| {
            !matches!(
                *part,
                "home" | "explore" | "notifications" | "messages" | "i"
            )
        })
    {
        return parts.first().map(|part| format!("@{part}"));
    }
    if (host == "github.com" || host.ends_with(".github.com")) && !parts.is_empty() {
        if parts[0] == "notifications" {
            return Some("Notifications".into());
        }
        return Some(parts.iter().take(2).copied().collect::<Vec<_>>().join("/"));
    }
    None
}

/// A user-editable write request for a destination.
#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct WriteLink {
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub pinned: bool,
}

/// User changes that do not replace the destination URL.
#[derive(Clone, Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
pub struct EditLink {
    pub title: Option<String>,
    pub group: Option<String>,
    pub pinned: Option<bool>,
}

/// A decision on a staged browser-history suggestion.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HistoryDecision {
    Keep,
    Dismiss,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_hosts_but_preserves_destination_paths_and_queries() {
        let parsed = destination(" HTTPS://WWW.Reddit.com/r/rust/?sort=top#rules ").unwrap();
        assert_eq!(parsed.host, "www.reddit.com");
        assert_eq!(parsed.group, "Reddit");
        assert_eq!(parsed.tenant.as_deref(), Some("r/rust"));
        assert_eq!(parsed.url, "https://www.reddit.com/r/rust/?sort=top");
    }

    #[test]
    fn refuses_non_web_and_credential_urls() {
        for input in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://user:pass@example.com",
        ] {
            assert!(destination(input).is_err(), "accepted {input}");
        }
    }

    #[test]
    fn recognizes_site_tenants() {
        assert_eq!(
            destination("https://x.com/someone")
                .unwrap()
                .tenant
                .as_deref(),
            Some("@someone")
        );
        assert_eq!(
            destination("https://github.com/zed-industries/zed")
                .unwrap()
                .tenant
                .as_deref(),
            Some("zed-industries/zed")
        );
        assert_eq!(
            destination("https://github.com/notifications")
                .unwrap()
                .tenant
                .as_deref(),
            Some("Notifications")
        );
    }
}
