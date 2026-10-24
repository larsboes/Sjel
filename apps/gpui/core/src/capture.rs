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

/// The tag the rules give an untagged capture: a link to a place the feed reads is `look`.
///
/// One function for both a capture the Mac answered with and one this device typed, so the preview
/// on the phone and the triage on the Mac cannot disagree about a URL.
pub fn suggested_tag(tag: Option<&str>, url: Option<&str>) -> Option<&'static str> {
    if let Some(tag) = tag {
        return TAGS.iter().find(|known| **known == tag).copied();
    }
    let host = host(url)?;
    FEED_HOSTS
        .iter()
        .any(|f| host == *f || host.ends_with(&format!(".{f}")))
        .then_some("look")
}

/// Where a capture with this tag and url can go. A text-only note without a tag stays in the inbox.
pub fn route_for(tag: Option<&str>, url: Option<&str>) -> Option<Route> {
    match suggested_tag(tag, url)? {
        "buy" => Some(Route::Wishlist),
        "look" | "learn" if url.is_some() => Some(Route::Feed),
        "go" => Some(Route::Places),
        _ => None,
    }
}

impl Capture {
    /// The tag the rules give an untagged capture. A set tag always wins.
    pub fn suggested_tag(&self) -> Option<&str> {
        suggested_tag(self.tag.as_deref(), self.url.as_deref())
    }

    /// Where the capture can go. A text-only note without a tag stays in the inbox.
    pub fn route(&self) -> Option<Route> {
        route_for(self.tag.as_deref(), self.url.as_deref())
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

/// What one capture field typed, before it is anything on a Mac.
///
/// The phone has one line and no triage screen, so the parsing that the capability does on arrival
/// is done here instead: `#buy` in the text sets the tag, a lone URL becomes the URL rather than the
/// text, and the preview says where the Mac will put it. The Mac re-reads all of it — nothing here
/// is trusted over the capability's own rules (`capabilities/comms/src/store/captures.rs`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub text: Option<String>,
    pub url: Option<String>,
    pub tag: Option<String>,
}

impl Draft {
    /// Reads one typed line. `tag` is the chip the person pressed, and wins over a `#tag` in the
    /// text, because the last thing they touched is the one they meant.
    pub fn parse(input: &str, tag: Option<&str>) -> Result<Self, String> {
        let mut text = Vec::new();
        let mut hashtag = None;
        for word in input.split_whitespace() {
            match word.strip_prefix('#') {
                Some(name)
                    if hashtag.is_none() && TAGS.contains(&name.to_ascii_lowercase().as_str()) =>
                {
                    hashtag = Some(name.to_ascii_lowercase())
                }
                _ => text.push(word),
            }
        }
        let text = text.join(" ");
        if text.is_empty() {
            return Err("Type what you want to keep, or paste a link.".to_owned());
        }
        // A line that is nothing but a link is a link: the capability stores both fields, and a
        // capture whose text is its own URL reads twice in every list that shows both.
        let (text, url) = match Url::parse(&text) {
            Ok(url) if url.has_host() => (None, Some(text)),
            _ => (Some(text), None),
        };
        let tag = tag
            .map(str::to_owned)
            .or(hashtag)
            .filter(|tag| TAGS.contains(&tag.as_str()));
        Ok(Self { text, url, tag })
    }

    /// What the row says.
    pub fn label(&self) -> String {
        self.text
            .clone()
            .filter(|text| !text.is_empty())
            .or_else(|| host(self.url.as_deref()))
            .or_else(|| self.url.clone())
            .unwrap_or_default()
    }

    pub fn suggested_tag(&self) -> Option<&'static str> {
        suggested_tag(self.tag.as_deref(), self.url.as_deref())
    }

    pub fn route(&self) -> Option<Route> {
        route_for(self.tag.as_deref(), self.url.as_deref())
    }

    /// One sentence for the field's preview: where this will land, or that it waits in the inbox.
    pub fn preview(&self) -> String {
        match self.route() {
            Some(route) => route.label().to_owned(),
            None => "Stays in the inbox".to_owned(),
        }
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
    fn a_typed_line_becomes_a_draft() {
        let bought = Draft::parse("Linen shirt 49,90 € #buy", None).expect("a draft");
        assert_eq!(bought.text.as_deref(), Some("Linen shirt 49,90 €"));
        assert_eq!(bought.tag.as_deref(), Some("buy"));
        assert_eq!(bought.preview(), "To wishlist");
        assert_eq!(bought.route(), Some(Route::Wishlist));

        // A lone link is the url, not the text, and the host decides the route.
        let link = Draft::parse("https://www.youtube.com/watch?v=x", None).expect("a draft");
        assert_eq!(link.text, None);
        assert_eq!(
            link.url.as_deref(),
            Some("https://www.youtube.com/watch?v=x")
        );
        assert_eq!(link.suggested_tag(), Some("look"));
        assert_eq!(link.route(), Some(Route::Feed));

        // The chip wins over a hashtag typed in the same line.
        let chipped = Draft::parse("#look Lisbon", Some("go")).expect("a draft");
        assert_eq!(chipped.tag.as_deref(), Some("go"));
        assert_eq!(chipped.text.as_deref(), Some("Lisbon"));
        assert_eq!(chipped.preview(), "To places");

        // An unknown hashtag stays in the text, the way the capability keeps it.
        let unknown = Draft::parse("#later call the plumber", None).expect("a draft");
        assert_eq!(unknown.tag, None);
        assert_eq!(unknown.text.as_deref(), Some("#later call the plumber"));
        assert_eq!(unknown.preview(), "Stays in the inbox");
    }

    #[test]
    fn an_empty_line_is_not_a_capture() {
        assert!(Draft::parse("   ", None).is_err());
        // A tag on its own is nothing to keep, which is also what the capability answers.
        assert!(Draft::parse("#buy", None).is_err());
        let kept = Draft::parse("#buy", None).unwrap_err();
        assert!(kept.contains("Type what you want to keep"), "{kept}");
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
