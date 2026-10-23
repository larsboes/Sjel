//! Destinations shared with the web dashboard, and `sjel://` deep links.
//!
//! `DESTINATIONS` mirrors `PRIMARY_NAV` and `UTILITY_NAV` in `dashboard/src/lib/nav.ts`; the
//! `matches_dashboard_nav` test fails when the two drift. A destination with a native page opens
//! here; every other one opens the dashboard at the same path.

/// A page this app draws itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    Context,
    Capabilities,
    Projects,
    /// Multi-turn capability session (`sjel://ask?q=…`).
    Ask,
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
}

impl Page {
    pub const ALL: [Page; 23] = [
        Page::Home,
        Page::Context,
        Page::Capabilities,
        Page::Systems,
        Page::Projects,
        Page::Ask,
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
        Page::Settings,
    ];

    pub fn href(self) -> &'static str {
        match self {
            Self::Home => "/",
            Self::Context => "/context",
            Self::Capabilities => "/capabilities",
            Self::Projects => "/projects",
            Self::Ask => "/ask",
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
            Self::Settings => "/settings",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ask => "Ask",
            Self::Settings => "Settings",
            Self::Software => "Software",
            Self::Import => "Import",
            Self::Systems => "Systems",
            Self::Packs => "Packs",
            Self::SelfModel => "Self-model",
            Self::Cv => "CV",
            Self::Media => "Media",
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

/// Where a link lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Native(Page),
    Travel(crate::travel::TravelRoute),
    /// Open the native Ask session and submit this question in-session.
    Ask(String),
    /// A dashboard path, query included.
    Dashboard(String),
}

/// `sjel://calendar/2026-10-15` → the dashboard path `/calendar/2026-10-15`. `sjel://` and
/// `sjel://home` are Home. `sjel://ask?q=…` (or `sjel://?ask=…`) opens the native Ask session
/// with the prompt. Travel also recognizes its trip subpages. Other subpaths and queries go to
/// the dashboard, which owns those details.
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
    if path != "/travel" {
        if let Some(route) = crate::travel::TravelRoute::parse(&path) {
            return Some(Target::Travel(route));
        }
    }
    if matches!(path_only, "/ask" | "/") && path.contains('?') {
        if let Ok(parsed) = reqwest::Url::parse(&format!("http://127.0.0.1{path}")) {
            if let Some((_, q)) = parsed
                .query_pairs()
                .find(|(k, _)| matches!(k.as_ref(), "q" | "ask"))
            {
                let question = q.trim();
                if !question.is_empty() {
                    return Some(Target::Ask(question.to_owned()));
                }
                return Some(Target::Native(Page::Ask));
            }
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
