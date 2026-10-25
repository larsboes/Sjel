//! The areas the app is organised into: eight top-level places, each holding one or more pages
//! as tabs. The sidebar lists areas (⌘1–⌘8), the header lists the open area's tabs.
//!
//! An area answers "where am I", a page answers "what am I looking at". Pages that answer the
//! same question share an area instead of competing for a sidebar row: Home and Today are one
//! page, Inbox and Feed are both what came in, and everything about Sjel itself sits under
//! Sjel. Both clients read this table, so a phone tab bar and the Mac sidebar cannot disagree.

use crate::route::Page;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Area {
    Today,
    Inbox,
    Calendar,
    Projects,
    Documents,
    Travel,
    Life,
    Sjel,
}

impl Area {
    /// Sidebar order, which is also the ⌘ number order.
    pub const ALL: [Area; 8] = [
        Area::Today,
        Area::Inbox,
        Area::Calendar,
        Area::Projects,
        Area::Documents,
        Area::Travel,
        Area::Life,
        Area::Sjel,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Inbox => "Inbox",
            Self::Calendar => "Calendar",
            Self::Projects => "Projects",
            Self::Documents => "Documents",
            Self::Travel => "Location",
            Self::Life => "Life",
            Self::Sjel => "Sjel",
        }
    }

    /// The tabs, in the order the header draws them. The first one is where the area opens
    /// until another tab has been visited.
    pub fn pages(self) -> &'static [Page] {
        match self {
            Self::Today => &[Page::Home],
            Self::Inbox => &[Page::Inbox, Page::Feed],
            Self::Calendar => &[Page::Calendar],
            Self::Projects => &[Page::Projects],
            Self::Documents => &[Page::Documents, Page::Media, Page::Import, Page::Docs],
            Self::Travel => &[Page::Travel],
            Self::Life => &[Page::People, Page::Finance, Page::Inventory, Page::Cv],
            Self::Sjel => &[
                Page::Capabilities,
                Page::Apps,
                Page::Systems,
                Page::Packs,
                Page::SelfModel,
                Page::Research,
                Page::Backup,
                Page::Setup,
                Page::Software,
            ],
        }
    }

    /// The area a page lives in. Settings lives in none: it is reached with ⌘, from anywhere.
    pub fn of(page: Page) -> Option<Area> {
        let page = canonical(page);
        Self::ALL
            .into_iter()
            .find(|area| area.pages().contains(&page))
    }

    /// The tab the area opens on, until another one in it has been visited.
    pub fn default_page(self) -> Page {
        self.pages()[0]
    }

    /// Whether the header draws a tab strip.
    pub fn has_tabs(self) -> bool {
        self.pages().len() > 1
    }

    /// The page `step` tabs away from `page`, wrapping. A page outside the area opens its first tab.
    pub fn step_tab(self, page: Page, step: isize) -> Page {
        let pages = self.pages();
        let Some(at) = pages.iter().position(|p| *p == canonical(page)) else {
            return pages[0];
        };
        pages[(at as isize + step).rem_euclid(pages.len() as isize) as usize]
    }

    /// The area `step` places away in the sidebar, wrapping.
    pub fn step(self, step: isize) -> Area {
        let at = Self::ALL.iter().position(|a| *a == self).unwrap_or(0);
        Self::ALL[(at as isize + step).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

/// The page a link or shortcut actually opens. Today (`/context`) and Home (`/`) were two pages
/// and are one now, so the old Today address lands on Home.
pub fn canonical(page: Page) -> Page {
    match page {
        Page::Context => Page::Home,
        page => page,
    }
}

/// Which tab each area shows. An area remembers the tab last opened in it, so ⌘8 returns to
/// Packs if Packs was the last Sjel page, and the header never forgets where you were.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tabs([Page; Area::ALL.len()]);

impl Default for Tabs {
    fn default() -> Self {
        Self(Area::ALL.map(Area::default_page))
    }
}

impl Tabs {
    pub fn current(&self, area: Area) -> Page {
        self.0[Self::index(area)]
    }

    /// Records `page` as its area's current tab. Settings has no area and is not recorded.
    pub fn visit(&mut self, page: Page) {
        if let Some(area) = Area::of(page) {
            self.0[Self::index(area)] = canonical(page);
        }
    }

    fn index(area: Area) -> usize {
        Area::ALL.iter().position(|a| *a == area).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_but_settings_has_exactly_one_area() {
        for page in Page::ALL {
            let homes = Area::ALL
                .into_iter()
                .filter(|area| area.pages().contains(&canonical(page)))
                .count();
            let expected = usize::from(page != Page::Settings);
            assert_eq!(homes, expected, "{page:?} sits in {homes} areas");
        }
    }

    #[test]
    fn the_areas_fit_the_number_keys() {
        assert!(Area::ALL.len() <= 9);
    }

    #[test]
    fn the_old_today_address_opens_the_merged_page() {
        assert_eq!(Area::of(Page::Context), Some(Area::Today));
        assert_eq!(canonical(Page::Context), Page::Home);
    }

    #[test]
    fn tabs_wrap_both_ways_and_a_single_page_area_stays_put() {
        assert_eq!(Area::Sjel.step_tab(Page::Capabilities, -1), Page::Software);
        assert_eq!(Area::Sjel.step_tab(Page::Software, 1), Page::Capabilities);
        assert_eq!(Area::Inbox.step_tab(Page::Inbox, 1), Page::Feed);
        assert_eq!(Area::Calendar.step_tab(Page::Calendar, 1), Page::Calendar);
        assert_eq!(Area::Life.step_tab(Page::Inbox, 1), Page::People);
    }

    #[test]
    fn areas_wrap_in_sidebar_order() {
        assert_eq!(Area::Today.step(-1), Area::Sjel);
        assert_eq!(Area::Sjel.step(1), Area::Today);
        assert_eq!(Area::Today.step(1), Area::Inbox);
    }

    #[test]
    fn an_area_remembers_its_last_tab() {
        let mut tabs = Tabs::default();
        assert_eq!(tabs.current(Area::Sjel), Page::Capabilities);
        tabs.visit(Page::Packs);
        tabs.visit(Page::Settings);
        tabs.visit(Page::Context);
        assert_eq!(tabs.current(Area::Sjel), Page::Packs);
        assert_eq!(tabs.current(Area::Today), Page::Home);
        assert_eq!(tabs.current(Area::Life), Page::People);
    }
}
