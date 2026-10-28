//! Destinations shared with the web dashboard, and `sjel://` deep links.
//!
//! `DESTINATIONS` mirrors `PRIMARY_NAV` and `UTILITY_NAV` in `dashboard/src/lib/nav.ts`; the
//! `matches_dashboard_nav` test fails when the two drift. Native pages and recognized deep links
//! open here; other destinations open the dashboard at the same path.

/// A page this app draws itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    Context,
    Capabilities,
    Projects,
    /// This Mac's own numbers: temperature, power, disk, updates and the agent's reach
    /// (`sjel://systems`). Named after the dashboard page that retires when this covers it.
    Systems,
    Travel,
    Feed,
    Inbox,
    Calendar,
    Documents,
    Backup,
    Finance,
    People,
    Inventory,
    /// Apps on this Mac and the operator's decision about each (`sjel://software`). The
    /// dashboard has no page of that name, so it is not in `DESTINATIONS`.
    Software,
    /// Sort a folder into Sjel and see what each file became (`sjel://import`). The dashboard
    /// has no page of that name, so it is not in `DESTINATIONS`.
    Import,
    /// Appearance, connection, updates and shortcuts (`sjel://settings`). The dashboard has no
    /// page of that name, so it is not in `DESTINATIONS`.
    Settings,
    /// Agent skills across installed harnesses (`sjel://packs`).
    Packs,
    /// Architectural self-model and live topology (`sjel://self`).
    SelfModel,
    /// CV builder, profiles, and job applications ledger (`sjel://cv`).
    Cv,
    /// Media ingest gate and indexed volume integrity ledger (`sjel://media`).
    Media,
    /// Research articles, upstream projects, and model benchmarks (`sjel://research`).
    Research,
    /// Node setup, machine probes, and device pairing (`sjel://setup`).
    Setup,
    /// Capability sites exposed by the local service registry (`sjel://apps`).
    Apps,
    /// The tracked repository documentation (`sjel://docs`).
    Docs,
    /// A typed capability record opened from a related-record link.
    Record,
}

impl Page {
    pub const ALL: [Page; 25] = [
        Page::Home,
        Page::Context,
        Page::Capabilities,
        Page::Systems,
        Page::Projects,
        Page::Travel,
        Page::Feed,
        Page::Inbox,
        Page::Calendar,
        Page::Documents,
        Page::Backup,
        Page::Finance,
        Page::People,
        Page::Inventory,
        Page::Software,
        Page::Import,
        Page::Packs,
        Page::SelfModel,
        Page::Cv,
        Page::Media,
        Page::Research,
        Page::Setup,
        Page::Settings,
        Page::Apps,
        Page::Docs,
    ];

    pub fn href(self) -> &'static str {
        match self {
            Self::Home => "/",
            Self::Context => "/context",
            Self::Capabilities => "/capabilities",
            Self::Projects => "/projects",
            Self::Travel => "/travel",
            Self::Feed => "/feed",
            Self::Inbox => "/inbox",
            Self::Calendar => "/calendar",
            Self::Documents => "/documents",
            Self::Backup => "/backup",
            Self::Finance => "/finance",
            Self::People => "/people",
            Self::Inventory => "/inventory",
            Self::Software => "/software",
            Self::Import => "/import",
            Self::Systems => "/systems",
            Self::Packs => "/packs",
            Self::SelfModel => "/self",
            Self::Cv => "/cv",
            Self::Media => "/media",
            Self::Research => "/research",
            Self::Setup => "/setup",
            Self::Settings => "/settings",
            Self::Apps => "/apps",
            Self::Docs => "/docs",
            Self::Record => "/record",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Travel => "Location",
            Self::Settings => "Settings",
            Self::Software => "Software",
            Self::Import => "Import",
            Self::Systems => "Systems",
            Self::Packs => "Packs",
            Self::SelfModel => "Self-model",
            Self::Cv => "CV",
            Self::Media => "Media",
            Self::Apps => "Apps",
            Self::Docs => "Docs",
            Self::Record => "Record",
            _ => destination(self.href()).map_or("", |d| d.label),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Destination {
    pub href: &'static str,
    pub label: &'static str,
}

impl Destination {
    pub fn page(self) -> Option<Page> {
        if self.href == "/map" {
            return Some(Page::Travel);
        }
        if self.href == "/wishlist" || self.href == "/interior" {
            return Some(Page::Inventory);
        }
        Page::ALL.into_iter().find(|page| page.href() == self.href)
    }
}

const fn d(href: &'static str, label: &'static str) -> Destination {
    Destination { href, label }
}

pub const DESTINATIONS: &[Destination] = &[
    d("/", "Home"),
    d("/context", "Today"),
    d("/calendar", "Calendar"),
    d("/projects", "Projects"),
    d("/documents", "Documents"),
    d("/inbox", "Inbox"),
    d("/feed", "Feed"),
    d("/travel", "Travel"),
    d("/map", "Map"),
    d("/finance", "Finance"),
    d("/inventory", "Inventory"),
    d("/wishlist", "Wishlist"),
    d("/people", "People"),
    d("/interior", "Interior"),
    d("/media", "Media"),
    d("/apps", "Apps"),
    d("/research", "Research"),
    d("/systems", "Systems"),
    d("/capabilities", "Capabilities"),
    d("/backup", "Backup"),
    d("/setup", "Setup"),
    d("/self", "Self-model"),
    d("/cv", "CV"),
    d("/packs", "Packs"),
    d("/docs", "Docs"),
];

pub fn destination(href: &str) -> Option<Destination> {
    DESTINATIONS.iter().copied().find(|d| d.href == href)
}

/// The Feed page's tabs. Reading is the comms feed, Review is comms' computed quality queue
/// (`/feed/review`), Library is the whole archive (`/feed/library`), Mail is Gmail triage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedTab {
    Reading,
    Review,
    Library,
    Mail,
    Discover,
    /// Sjel Clipper documents, read over comms' clip socket.
    Clips,
}

/// Where a link lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Native(Page),
    Travel(crate::travel::TravelRoute),
    Feed(FeedTab),
    /// A comms entry opened from its canonical `/feed/<sha256>` link.
    FeedEntry(String),
    /// A mail item opened from `/feed/<id>?source=mail`.
    MailEntry(String),
    /// A calendar item opened from `/feed/<id>?source=calendar`.
    CalendarEntry(String),
    /// A repository document slug under `/docs`.
    Docs(String),
    /// A typed record id under `/record?id=…`.
    Record(String),
    /// Open the finder, and ask this question in the one Ask session when it is not empty.
    Ask(String),
    /// A terminal on the Mac: the newest one still running, or a new login shell.
    Terminal,
    /// A dashboard path, query included.
    Dashboard(String),
}

/// `sjel://calendar/2026-10-15` → the dashboard path `/calendar/2026-10-15`. `sjel://` and
/// `sjel://home` are Home. `sjel://ask` opens the finder; `sjel://ask?q=…` (or `sjel://?ask=…`)
/// also asks the prompt. Travel also recognizes its trip subpages. Standalone capability panel
/// links open the native Apps registry; unsupported subpaths and queries go to the dashboard.
pub fn parse_deep_link(url: &str) -> Option<Target> {
    let rest = url.strip_prefix("sjel://")?.trim_end_matches('/');
    let path = match rest {
        "" | "home" => "/".to_owned(),
        rest if rest.starts_with('?') => format!("/{rest}"),
        rest => format!("/{rest}"),
    };
    let path_only = path.split_once('?').map_or(path.as_str(), |(p, _)| p);
    if path_only.contains("..") || path.chars().any(char::is_whitespace) {
        return None;
    }
    if path_only
        .strip_prefix("/panel/")
        .is_some_and(|name| !name.is_empty() && !name.contains('/'))
    {
        return Some(Target::Native(Page::Apps));
    }
    if path_only == "/docs" {
        return Some(Target::Native(Page::Docs));
    }
    if path_only == "/terminal" {
        return Some(Target::Terminal);
    }
    if let Some(slug) = path_only.strip_prefix("/docs/") {
        let slug = match slug {
            "README" | "index" => "index",
            slug => slug,
        };
        return Some(Target::Docs(slug.to_owned()));
    }
    if path_only == "/record" {
        let id = reqwest::Url::parse(&format!("http://127.0.0.1{path}"))
            .ok()?
            .query_pairs()
            .find(|(key, _)| key == "id")
            .map(|(_, value)| value.into_owned())?;
        return Some(Target::Record(id));
    }
    if path != "/travel" {
        if let Some(route) = crate::travel::TravelRoute::parse(&path) {
            return Some(Target::Travel(route));
        }
    }
    if path_only.starts_with("/feed/") {
        let source = reqwest::Url::parse(&format!("http://127.0.0.1{path}"))
            .ok()?
            .query_pairs()
            .find(|(key, _)| key == "source")
            .map(|(_, value)| value.into_owned());
        let id = path_only.strip_prefix("/feed/")?;
        match source.as_deref() {
            Some("mail")
                if !id.is_empty()
                    && id.len() <= 128
                    && id.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
                    }) =>
            {
                return Some(Target::MailEntry(id.to_owned()));
            }
            Some("calendar")
                if !id.is_empty()
                    && id.len() <= 128
                    && id.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':')
                    }) =>
            {
                return Some(Target::CalendarEntry(id.to_owned()));
            }
            None | Some("feed")
                if id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
            {
                return Some(Target::FeedEntry(id.to_owned()));
            }
            _ => {}
        }
    }
    match path.as_str() {
        "/trips" => return Some(Target::Native(Page::Travel)),
        "/scout" => return Some(Target::Feed(FeedTab::Discover)),
        "/feed/review" => return Some(Target::Feed(FeedTab::Review)),
        "/feed/library" => return Some(Target::Feed(FeedTab::Library)),
        "/feed/clips" => return Some(Target::Feed(FeedTab::Clips)),
        _ => {}
    }
    if path_only == "/feed" {
        let view = reqwest::Url::parse(&format!("http://127.0.0.1{path}"))
            .ok()?
            .query_pairs()
            .find(|(key, _)| key == "view")
            .map(|(_, value)| value.into_owned());
        match view.as_deref() {
            Some("discover") => return Some(Target::Feed(FeedTab::Discover)),
            Some("mail") => return Some(Target::Feed(FeedTab::Mail)),
            Some("clips") => return Some(Target::Feed(FeedTab::Clips)),
            _ => {}
        }
    }
    if matches!(path_only, "/ask" | "/") {
        let question = reqwest::Url::parse(&format!("http://127.0.0.1{path}"))
            .ok()
            .and_then(|parsed| {
                parsed
                    .query_pairs()
                    .find(|(k, _)| matches!(k.as_ref(), "q" | "ask"))
                    .map(|(_, q)| q.trim().to_owned())
            });
        if path_only == "/ask" || question.is_some() {
            return Some(Target::Ask(question.unwrap_or_default()));
        }
    }
    Some(
        match Page::ALL
            .into_iter()
            .find(|page| page.href() == path)
            .or_else(|| destination(&path).and_then(Destination::page))
        {
            Some(page) => Target::Native(page),
            None => Target::Dashboard(path),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deep_link(href: &str) -> String {
        format!("sjel://{}", href.trim_start_matches('/'))
    }

    #[test]
    fn matches_dashboard_nav() {
        let nav = include_str!("../../../../dashboard/src/lib/nav.ts");
        let start = nav
            .find("export const PRIMARY_NAV")
            .expect("PRIMARY_NAV in nav.ts");
        let end = nav
            .find("export const capabilityForPath")
            .expect("capabilityForPath in nav.ts");
        let field = |line: &str, key: &str| -> Option<String> {
            let after = &line[line.find(&format!("{key}: \""))? + key.len() + 3..];
            Some(after[..after.find('"')?].to_owned())
        };
        let web: Vec<(String, String)> = nav[start..end]
            .lines()
            .filter(|line| line.trim_start().starts_with("{ href:"))
            .filter_map(|line| Some((field(line, "href")?, field(line, "label")?)))
            .collect();
        let native: Vec<(String, String)> = DESTINATIONS
            .iter()
            .map(|d| (d.href.to_owned(), d.label.to_owned()))
            .collect();
        assert_eq!(
            native, web,
            "route.rs DESTINATIONS drifted from dashboard/src/lib/nav.ts"
        );
    }

    #[test]
    fn deep_links_round_trip() {
        for section in crate::travel::TripSection::ALL {
            let trip = crate::travel::TravelRoute::Trip {
                id: "trip:plan:1234abcd".into(),
                section,
            };
            assert_eq!(
                parse_deep_link(&deep_link(&trip.href())),
                Some(Target::Travel(trip))
            );
        }
        assert_eq!(
            parse_deep_link("sjel://map"),
            Some(Target::Travel(crate::travel::TravelRoute::Map))
        );
        assert_eq!(
            parse_deep_link("sjel://feed/review"),
            Some(Target::Feed(FeedTab::Review))
        );
        assert_eq!(
            parse_deep_link("sjel://feed/library"),
            Some(Target::Feed(FeedTab::Library))
        );
        assert_eq!(
            parse_deep_link("sjel://trips"),
            Some(Target::Native(Page::Travel))
        );
        assert_eq!(
            parse_deep_link("sjel://scout"),
            Some(Target::Feed(FeedTab::Discover))
        );
        assert_eq!(
            parse_deep_link("sjel://feed?view=discover"),
            Some(Target::Feed(FeedTab::Discover))
        );
        let feed_id = "a".repeat(64);
        assert_eq!(
            parse_deep_link(&format!("sjel://feed/{feed_id}")),
            Some(Target::FeedEntry(feed_id.clone()))
        );
        assert_eq!(
            parse_deep_link(&format!("sjel://feed/{feed_id}?source=mail")),
            Some(Target::MailEntry(feed_id.clone()))
        );
        assert_eq!(
            parse_deep_link("sjel://feed/cal:entry:ab12?source=calendar"),
            Some(Target::CalendarEntry("cal:entry:ab12".into()))
        );
        assert_eq!(parse_deep_link("sjel://"), Some(Target::Native(Page::Home)));
        assert_eq!(
            parse_deep_link("sjel://home"),
            Some(Target::Native(Page::Home))
        );
        assert_eq!(
            parse_deep_link("sjel://capabilities/"),
            Some(Target::Native(Page::Capabilities))
        );
        assert_eq!(
            parse_deep_link("sjel://calendar/2026-10-15"),
            Some(Target::Dashboard("/calendar/2026-10-15".into()))
        );
        assert_eq!(
            parse_deep_link("sjel://ask?q=what%20is%20up%3F"),
            Some(Target::Ask("what is up?".into()))
        );
        assert_eq!(
            parse_deep_link("sjel://?ask=calendar%20today"),
            Some(Target::Ask("calendar today".into()))
        );
        assert_eq!(
            parse_deep_link("sjel://ask"),
            Some(Target::Ask(String::new()))
        );
        // A terminal is a place in the app: the finder's result, a link and a handoff all name
        // it the same way, and the workspace decides which shell is attached.
        assert_eq!(parse_deep_link("sjel://terminal"), Some(Target::Terminal));
        assert_eq!(parse_deep_link("sjel://terminal/"), Some(Target::Terminal));
        assert_eq!(parse_deep_link("sjel://../etc"), None);
        assert_eq!(parse_deep_link("https://calendar"), None);
        for page in Page::ALL {
            assert_eq!(
                parse_deep_link(&deep_link(page.href())),
                Some(Target::Native(page))
            );
        }
    }
}
