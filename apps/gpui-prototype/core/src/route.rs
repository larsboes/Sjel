//! Destinations shared with the web dashboard, and `sjel://` deep links.
//!
//! `DESTINATIONS` mirrors `PRIMARY_NAV` and `UTILITY_NAV` in `dashboard/src/lib/nav.ts`; the
//! `matches_dashboard_nav` test fails when the two drift. A destination with a native page opens
//! here; every other one opens the dashboard at the same path.

/// A page this app draws itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    Capabilities,
}

impl Page {
    pub const ALL: [Page; 2] = [Page::Home, Page::Capabilities];

    pub fn href(self) -> &'static str {
        match self {
            Self::Home => "/",
            Self::Capabilities => "/capabilities",
        }
    }

    pub fn label(self) -> &'static str {
        destination(self.href()).map_or("", |d| d.label)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Destination {
    pub href: &'static str,
    pub label: &'static str,
}

impl Destination {
    pub fn page(self) -> Option<Page> {
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
    d("/projects", "Projects"),
    d("/research", "Research"),
    d("/systems", "Systems"),
    d("/capabilities", "Capabilities"),
    d("/backup", "Backup"),
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
    /// A dashboard path, query included.
    Dashboard(String),
}

/// `sjel://calendar/2026-10-15` → the dashboard path `/calendar/2026-10-15`. `sjel://` and
/// `sjel://home` are Home. Only an exact native path opens natively; a sub-path or a query
/// goes to the dashboard, which owns the detail.
pub fn parse_deep_link(url: &str) -> Option<Target> {
    let rest = url.strip_prefix("sjel://")?.trim_end_matches('/');
    let path = match rest {
        "" | "home" => "/".to_owned(),
        rest => format!("/{rest}"),
    };
    if path.contains("..") || path.chars().any(char::is_whitespace) {
        return None;
    }
    Some(match destination(&path).and_then(Destination::page) {
        Some(page) => Target::Native(page),
        None => Target::Dashboard(path),
    })
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
