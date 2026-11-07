//! The capture inbox (comms `/captures`) and where each item goes from it.
//!
//! The rules are the dashboard's (`dashboard/src/lib/capture.ts`, `inventory/money.ts`); change
//! both together until the dashboard page retires.

use reqwest::Url;

pub use feed_api::{
    cents_from, extract_price, host, route_for, slug, suggested_tag, Capture,
    CaptureRoute as Route, FEED_HOSTS, TAGS,
};

/// What one capture field typed, before it is anything on a Mac.
///
/// The phone has one line and no triage screen, so the parsing that the capability does on arrival
/// is done here instead: `#buy` in the text sets the tag, a lone URL becomes the URL rather than the
/// text, and the preview says where the Mac will put it. The Mac re-reads all of it — nothing here
/// is trusted over the capability's own rules (`crates/comms/src/store/captures.rs`).
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
