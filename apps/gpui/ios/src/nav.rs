//! Which screen the phone shows: its tabs, and the Mac's destinations mapped onto them.

use super::*;

/// Where a destination lands on the phone.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Screen {
    Tab(Tab),
    Travel(sjel_gpui_core::travel::TravelRoute),
    Terminal,
    /// A page only the Mac draws, named for the note the phone shows instead.
    Mac(&'static str),
}

/// The phone's answer to the Mac's destinations (`route::Target`). A page the phone does not draw
/// says so rather than opening something else without a word.
pub(crate) fn phone_screen(target: &Target) -> Screen {
    match target {
        Target::Native(Page::Home | Page::Context) => Screen::Tab(Tab::Today),
        Target::Native(Page::Projects) => Screen::Tab(Tab::Projects),
        Target::Native(Page::Calendar) | Target::CalendarEntry(_) => Screen::Tab(Tab::Calendar),
        Target::Native(Page::People) => Screen::Tab(Tab::People),
        Target::Native(Page::Inbox) => Screen::Tab(Tab::Inbox),
        Target::Native(Page::Travel) => Screen::Tab(Tab::Travel),
        Target::Native(Page::Documents) => Screen::Tab(Tab::Documents),
        Target::Native(Page::Finance) => Screen::Tab(Tab::Finance),
        Target::Native(Page::Inventory) => Screen::Tab(Tab::Inventory),
        Target::Native(Page::Settings | Page::Setup) => Screen::Tab(Tab::Settings),
        Target::Native(Page::Feed) | Target::Feed(FeedTab::Mail) | Target::MailEntry(_) => {
            Screen::Tab(Tab::Mail)
        }
        Target::Travel(route) => Screen::Travel(route.clone()),
        Target::Terminal => Screen::Terminal,
        Target::Native(page) => Screen::Mac(page.label()),
        Target::Feed(_) | Target::FeedEntry(_) => Screen::Mac("Feed"),
        Target::Docs(_) => Screen::Mac("Docs"),
        Target::Record(_) => Screen::Mac("This record"),
        Target::Ask(_) => Screen::Mac("Ask"),
        Target::Scratchpad => Screen::Mac("Scratchpad"),
        Target::Unknown(path) if path.split('?').next() == Some("/interior") => {
            Screen::Tab(Tab::Interior)
        }
        Target::Unknown(path) => match route::first_page(path) {
            Some(page) => phone_screen(&Target::Native(page)),
            None => Screen::Mac("This link"),
        },
    }
}

impl AppState {
    pub(crate) fn select_tab(&mut self, tab: Tab) {
        let page = match tab {
            Tab::Today => {
                phone_today::load();
                PhonePage::Today
            }
            Tab::Projects => PhonePage::Projects,
            Tab::Calendar => {
                phone_calendar::load();
                PhonePage::Calendar
            }
            Tab::People => {
                sjel_gpui_ui::phone_people::load();
                PhonePage::People
            }
            Tab::Interior => {
                sjel_gpui_ui::phone_interior::load();
                load_roomplan_archive();
                PhonePage::Interior
            }
            Tab::Inbox => {
                sjel_gpui_ui::phone_capture::load();
                PhonePage::Capture
            }
            Tab::Travel => {
                sjel_gpui_ui::phone_travel::open(sjel_gpui_core::travel::TravelRoute::Trips);
                PhonePage::Travel
            }
            Tab::Settings => {
                notify::report();
                PhonePage::Settings
            }
            Tab::Mail => PhonePage::Mail,
            Tab::Documents => PhonePage::Documents,
            Tab::Finance => {
                sjel_gpui_ui::phone_finance::load();
                PhonePage::Finance
            }
            Tab::Inventory => {
                sjel_gpui_ui::phone_inventory::load();
                PhonePage::Inventory
            }
        };
        open_page(page);
    }
    pub(crate) fn open_link(&mut self, link: &str) {
        if let Some(day) = reminders::link_day(link) {
            self.select_tab(Tab::Calendar);
            phone_calendar::open_day(day);
        } else if let Some(screen) = phone_link(link) {
            match screen {
                Screen::Tab(tab) => self.select_tab(tab),
                Screen::Travel(route) => sjel_gpui_ui::phone_travel::open(route),
                Screen::Terminal => phone_edge::open(Seat::Terminal),
                Screen::Mac(page) => {
                    self.select_tab(Tab::Today);
                    phone_today::set_note(Some(format!("{page} opens on the Mac.")));
                }
            }
        }
    }
    pub(crate) fn on_today(&mut self, event: TodayEvent) {
        match event {
            TodayEvent::OpenDay(day) => {
                self.select_tab(Tab::Calendar);
                phone_calendar::open_day(day);
            }
            TodayEvent::OpenProject(name) => {
                #[cfg(target_os = "ios")]
                crate::projects::open_project(name);
                #[cfg(not(target_os = "ios"))]
                let _ = name;
                self.select_tab(Tab::Projects);
            }
            TodayEvent::Settings => self.select_tab(Tab::Settings),
            TodayEvent::Soundscape => phone_edge::open(Seat::Soundscape),
            TodayEvent::Activity => phone_edge::open(Seat::Activity),
            TodayEvent::Refresh => {
                if self.board.is_some() {
                    self.maybe_sync();
                } else {
                    self.refresh_copy();
                }
            }
            TodayEvent::Wrote => self.maybe_sync(),
            TodayEvent::Capture => self.select_tab(Tab::Inbox),
            TodayEvent::People => self.select_tab(Tab::People),
            TodayEvent::Documents => self.select_tab(Tab::Documents),
            TodayEvent::Projects => self.select_tab(Tab::Projects),
            TodayEvent::Interior => self.select_tab(Tab::Interior),
            TodayEvent::Finance => self.select_tab(Tab::Finance),
            TodayEvent::Inventory => self.select_tab(Tab::Inventory),
        }
    }
}

fn phone_link(link: &str) -> Option<Screen> {
    if link.trim_end_matches('/') == "sjel://interior" {
        Some(Screen::Tab(Tab::Interior))
    } else {
        route::parse_deep_link(link).map(|target| phone_screen(&target))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mac_link_lands_on_the_phone_screen_that_has_it() {
        let open = |link: &str| phone_link(link).unwrap();
        assert_eq!(open("sjel://"), Screen::Tab(Tab::Today));
        assert_eq!(open("sjel://calendar"), Screen::Tab(Tab::Calendar));
        assert_eq!(open("sjel://feed?view=mail"), Screen::Tab(Tab::Mail));
        assert_eq!(
            open("sjel://calendar/2026-10-15"),
            Screen::Tab(Tab::Calendar)
        );
        assert_eq!(open("sjel://terminal"), Screen::Terminal);
        assert_eq!(open("sjel://interior"), Screen::Tab(Tab::Interior));
        assert!(matches!(open("sjel://travel/map"), Screen::Travel(_)));
        assert_eq!(open("sjel://finance"), Screen::Tab(Tab::Finance));
        assert_eq!(open("sjel://inventory"), Screen::Tab(Tab::Inventory));
        assert_eq!(open("sjel://backup"), Screen::Mac("Backup"));
        assert!(matches!(open("sjel://no-such-page"), Screen::Mac(_)));
    }
}
