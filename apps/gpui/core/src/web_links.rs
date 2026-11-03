//! The native app's view of the `web-links` launcher and reviewed browser-history suggestions.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct WebLink {
    pub id: String,
    pub url: String,
    pub title: String,
    pub host: String,
    #[serde(default)]
    pub tenant: Option<String>,
    pub group: String,
    #[serde(default)]
    pub group_override: Option<String>,
    pub pinned: bool,
    #[serde(default)]
    pub opens: u64,
    #[serde(default)]
    pub last_opened_at: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct WebLinks {
    pub links: Vec<WebLink>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct WebLinkCandidate {
    pub id: String,
    pub url: String,
    pub title: String,
    pub host: String,
    #[serde(default)]
    pub tenant: Option<String>,
    pub browser: String,
    pub visits: u64,
    #[serde(default)]
    pub last_visit: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct WebLinkHistoryReply {
    pub staged: usize,
    pub candidates: Vec<WebLinkCandidate>,
}
