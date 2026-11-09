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
        Target::Unknown(path) => match route::first_page(path) {
            Some(page) => phone_screen(&Target::Native(page)),
            None => Screen::Mac("This link"),
        },
    }
}

impl Capabilities {
    /// Where a tapped reminder goes: the day it announced, on the Calendar tab.
    /// A reminder's `calendar:<day>` link, or a `sjel://` link in the Mac's vocabulary
    /// (`core/src/route.rs`).
    pub(super) fn open_link(&mut self, link: &str, cx: &mut Context<Self>) {
        if let Some(day) = reminders::link_day(link) {
            self.select_tab(Tab::Calendar, cx);
            self.calendar
                .update(cx, |calendar, cx| calendar.open_day(day, cx));
        } else if let Some(target) = route::parse_deep_link(link) {
            self.open_target(target, cx);
        }
    }

    /// Opens a destination the Mac would open, on the screen the phone has for it.
    pub(super) fn open_target(&mut self, target: Target, cx: &mut Context<Self>) {
        match phone_screen(&target) {
            Screen::Tab(tab) => self.select_tab(tab, cx),
            Screen::Travel(route) => {
                self.select_tab(Tab::Travel, cx);
                self.travel
                    .update(cx, |travel, cx| travel.open_route(route, cx));
            }
            Screen::Terminal => self.open_edge(Seat::Terminal, cx),
            Screen::Mac(what) => {
                self.select_tab(Tab::Today, cx);
                self.status = format!("{what} opens on the Mac, not on the phone.").into();
            }
        }
    }

    /// What the NSH Today screen asked for. A day or a project is a place in another tab: the page
    /// names it and the app goes there.
    pub(super) fn on_today(&mut self, event: TodayEvent, cx: &mut Context<Self>) {
        #[cfg(target_os = "ios")]
        if !matches!(event, TodayEvent::Refresh | TodayEvent::Wrote) {
            crate::host::hide_nsh();
        }
        match event {
            TodayEvent::OpenDay(day) => {
                self.select_tab(Tab::Calendar, cx);
                self.calendar
                    .update(cx, |calendar, cx| calendar.open_day(day, cx));
            }
            TodayEvent::OpenProject(name) => {
                self.select_tab(Tab::Projects, cx);
                if let Some(projects) = &self.projects {
                    projects.update(cx, |projects, cx| projects.open_project(name, cx));
                }
            }
            TodayEvent::Settings => self.select_tab(Tab::Settings, cx),
            // The edge slides in over the page: the bar holds five items and is full.
            TodayEvent::Soundscape => self.open_edge(Seat::Soundscape, cx),
            TodayEvent::Activity => self.open_edge(Seat::Activity, cx),
            TodayEvent::Refresh => {
                if self.board.is_some() {
                    self.maybe_sync(cx);
                } else {
                    self.refresh_copy(cx);
                }
            }
            TodayEvent::Wrote => self.maybe_sync(cx),
            TodayEvent::Capture => self.select_tab(Tab::Inbox, cx),
            TodayEvent::People => self.select_tab(Tab::People, cx),
            TodayEvent::Documents => self.select_tab(Tab::Documents, cx),
        }
    }

    /// Shows `tab` and starts what it reads. Every way onto a tab comes through here: the bar,
    /// Today's shortcuts, a reminder and a link.
    pub(super) fn select_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        // Today, Documents and Mail are NSH screens over the GPUI view; the tab under them stays
        // where it was.
        #[cfg(target_os = "ios")]
        if tab == Tab::Settings {
            sjel_gpui_ui::nsh_documents::set_nsh_page(
                sjel_gpui_ui::nsh_documents::NshPage::Settings,
            );
            crate::host::show_nsh();
            notify::report();
            return;
        }
        #[cfg(target_os = "ios")]
        if tab == Tab::People {
            sjel_gpui_ui::nsh_documents::set_nsh_page(sjel_gpui_ui::nsh_documents::NshPage::People);
            crate::host::show_nsh();
            sjel_gpui_ui::nsh_people::load();
            return;
        }
        #[cfg(target_os = "ios")]
        if tab == Tab::Inbox {
            sjel_gpui_ui::nsh_documents::set_nsh_page(
                sjel_gpui_ui::nsh_documents::NshPage::Capture,
            );
            crate::host::show_nsh();
            sjel_gpui_ui::nsh_capture::load();
            return;
        }
        #[cfg(target_os = "ios")]
        if tab == Tab::Today {
            sjel_gpui_ui::nsh_documents::set_nsh_page(sjel_gpui_ui::nsh_documents::NshPage::Today);
            crate::host::show_nsh();
            sjel_gpui_ui::nsh_today::load();
            return;
        }
        #[cfg(target_os = "ios")]
        if tab == Tab::Documents {
            sjel_gpui_ui::nsh_documents::set_nsh_page(
                sjel_gpui_ui::nsh_documents::NshPage::Documents,
            );
            crate::host::show_nsh();
            return;
        }
        #[cfg(target_os = "ios")]
        if tab == Tab::Mail {
            sjel_gpui_ui::nsh_documents::set_nsh_page(sjel_gpui_ui::nsh_documents::NshPage::Mail);
            crate::host::show_nsh();
            return;
        }
        if self.active_tab == Tab::Travel && tab != Tab::Travel {
            self.travel.update(cx, |travel, _| travel.deactivate());
        }
        self.active_tab = tab;
        match tab {
            Tab::Mail => self.mail.update(cx, |mail, cx| mail.load(cx)),
            Tab::Travel => self.travel.update(cx, |travel, cx| travel.activate(cx)),
            Tab::Inbox => sjel_gpui_ui::nsh_capture::load(),
            Tab::Documents => self.documents.update(cx, |docs, cx| docs.load(cx)),
            Tab::Interior => {
                self.interior.update(cx, |history, cx| history.refresh(cx));
                load_roomplan_archive();
            }
            Tab::Settings => notify::report(),
            Tab::Today | Tab::Calendar | Tab::People => {
                if self.copy.is_none() && !self.copying {
                    self.refresh_copy(cx);
                }
            }
            Tab::Projects => {}
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mac_link_lands_on_the_phone_screen_that_has_it() {
        let open = |link: &str| phone_screen(&route::parse_deep_link(link).unwrap());
        assert_eq!(open("sjel://"), Screen::Tab(Tab::Today));
        assert_eq!(open("sjel://calendar"), Screen::Tab(Tab::Calendar));
        assert_eq!(open("sjel://feed?view=mail"), Screen::Tab(Tab::Mail));
        assert_eq!(
            open("sjel://calendar/2026-10-15"),
            Screen::Tab(Tab::Calendar)
        );
        assert_eq!(open("sjel://terminal"), Screen::Terminal);
        assert!(matches!(open("sjel://travel/map"), Screen::Travel(_)));
        assert_eq!(open("sjel://finance"), Screen::Mac("Finance"));
        assert!(matches!(open("sjel://no-such-page"), Screen::Mac(_)));
    }
}
