//! The capture inbox (comms `/captures`) and where each item goes from it.
//!
//! The rules are the dashboard's (`dashboard/src/lib/capture.ts`, `inventory/money.ts`); change
//! both together until the dashboard page retires.

use reqwest::Url;
use serde::Deserialize;

pub const TAGS: [&str; 4] = ["buy", "look", "go", "learn"];

/// One row of `GET /comms/captures` (`capabilities/comms/src/store/captures.rs`).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Capture {
    pub id: String,
    pub text: Option<String>,
    pub url: Option<String>,
    pub tag: Option<String>,
    pub status: String,
    /// Where it went: `feed:<id>`, `places:<id>` or `inventory:<id>`.
    pub routed_to: Option<String>,
    pub client: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Wishlist,
    Feed,
    Places,
}

impl Route {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wishlist => "To wishlist",
            Self::Feed => "To feed",
            Self::Places => "To places",
        }
    }
}

/// Domains the comms feed has a reader for (`capabilities/comms/src/media`).
const FEED_HOSTS: [&str; 7] = [
    "github.com",
    "reddit.com",
    "arxiv.org",
    "huggingface.co",
    "youtube.com",
    "youtu.be",
    "instagram.com",
];

pub fn host(url: Option<&str>) -> Option<String> {
    let host = Url::parse(url?).ok()?.host_str()?.to_owned();
    Some(host.strip_prefix("www.").map(str::to_owned).unwrap_or(host))
}

impl Capture {
    /// The tag the rules give an untagged capture. A set tag always wins.
    pub fn suggested_tag(&self) -> Option<&str> {
        if let Some(tag) = self.tag.as_deref() {
            return Some(tag);
        }
        let host = host(self.url.as_deref())?;
        FEED_HOSTS
            .iter()
            .any(|f| host == *f || host.ends_with(&format!(".{f}")))
            .then_some("look")
    }

    /// Where the capture can go. A text-only note without a tag stays in the inbox.
    pub fn route(&self) -> Option<Route> {
        match self.suggested_tag()? {
            "buy" => Some(Route::Wishlist),
            "look" | "learn" if self.url.is_some() => Some(Route::Feed),
            "go" => Some(Route::Places),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        self.text
            .clone()
            .filter(|t| !t.is_empty())
            .or_else(|| host(self.url.as_deref()))
            .or_else(|| self.url.clone())
            .unwrap_or_default()
    }
}

/// `"49,90 €"` → 4990. `None` for text that is not a price.
pub fn cents_from(text: &str) -> Option<i64> {
    let lower = text.to_lowercase();
    let mut t = lower.replace('€', "").replace("eur", "").trim().to_owned();
    if t.is_empty() {
        return None;
    }
    if t.contains('.') && t.contains(',') {
        t = t.replace('.', "");
    }
    let t = t.replacen(',', ".", 1);
    let (whole, frac) = t.split_once('.').unwrap_or((&t, ""));
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || !(frac.is_empty() || (digits(frac) && frac.len() <= 2)) {
        return None;
    }
    let frac = format!("{frac:0<2}");
    Some(whole.parse::<i64>().ok()? * 100 + frac.parse::<i64>().ok()?)
}

/// `"Linen Shirt 49.90 €"` → `("Linen Shirt", Some(4990))`: a price typed at the end of the text.
pub fn extract_price(text: Option<&str>) -> (String, Option<i64>) {
    let trimmed = text.unwrap_or_default().trim();
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.len() >= 2 {
        for take in [2, 1] {
            if let Some(cents) = cents_from(&words[words.len() - take..].join(" ")) {
                return (words[..words.len() - take].join(" "), Some(cents));
            }
        }
    }
    (trimmed.to_owned(), None)
}

/// `Proper shoes (On?)` → `proper-shoes-on`, the shape inventory's own ids have.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.to_lowercase().chars() {
        let c = match c {
            'ä' => 'a',
            'ö' => 'o',
            'ü' => 'u',
            'ß' => 's',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "wunsch".into()
    } else {
        out.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(text: Option<&str>, url: Option<&str>, tag: Option<&str>) -> Capture {
        Capture {
            id: "0123456789abcdef".into(),
            text: text.map(Into::into),
            url: url.map(Into::into),
            tag: tag.map(Into::into),
            status: "new".into(),
            routed_to: None,
            client: None,
            created_at: "2026-10-22T00:00:00Z".into(),
        }
    }

    #[test]
    fn routes_follow_the_tag_and_the_host() {
        assert_eq!(
            capture(Some("shoes"), None, Some("buy")).route(),
            Some(Route::Wishlist)
        );
        assert_eq!(
            capture(None, Some("https://www.youtube.com/watch?v=x"), None).route(),
            Some(Route::Feed)
        );
        assert_eq!(
            capture(None, Some("https://m.reddit.com/r/x"), None).route(),
            Some(Route::Feed)
        );
        assert_eq!(
            capture(Some("read this"), None, Some("learn")).route(),
            None
        );
        assert_eq!(
            capture(Some("Lisbon"), None, Some("go")).route(),
            Some(Route::Places)
        );
        assert_eq!(capture(Some("a thought"), None, None).route(), None);
        assert_eq!(
            capture(None, Some("https://example.org"), None).route(),
            None
        );
    }

    #[test]
    fn prices_read_the_way_the_dashboard_reads_them() {
        assert_eq!(cents_from("49,90 €"), Some(4990));
        assert_eq!(cents_from("1.299,00"), Some(129900));
        assert_eq!(cents_from("39.9"), Some(3990));
        assert_eq!(cents_from("12 EUR"), Some(1200));
        assert_eq!(cents_from("shirt"), None);
        assert_eq!(cents_from("1.234"), None);
        assert_eq!(
            extract_price(Some("Linen Shirt 49.90 €")),
            ("Linen Shirt".into(), Some(4990))
        );
        assert_eq!(
            extract_price(Some("Linen Shirt 49")),
            ("Linen Shirt".into(), Some(4900))
        );
        assert_eq!(
            extract_price(Some("Linen Shirt")),
            ("Linen Shirt".into(), None)
        );
    }

    #[test]
    fn slugs_match_inventory_ids() {
        assert_eq!(slug("Proper shoes (On?)"), "proper-shoes-on");
        assert_eq!(slug("Größe Jäcke"), "grose-jacke");
        assert_eq!(slug("???"), "wunsch");
    }
}
