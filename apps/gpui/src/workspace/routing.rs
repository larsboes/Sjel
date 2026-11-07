//! Route dispatching, lazy page instantiation, navigation state transitions, and page views.

use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Entity, Window};
use sjel_gpui_core::area;
use sjel_gpui_ui::backup::BackupPage;
use sjel_gpui_ui::calendar::page::CalendarPage;
use sjel_gpui_ui::capabilities;
use sjel_gpui_ui::cv::CvPage;
use sjel_gpui_ui::docs::DocsPage;
use sjel_gpui_ui::documents::Documents;
use sjel_gpui_ui::feed::Feed;
use sjel_gpui_ui::finance::FinancePage;
use sjel_gpui_ui::import::ImportPage;
use sjel_gpui_ui::inventory::InventoryPage;
use sjel_gpui_ui::media::MediaPage;
use sjel_gpui_ui::packs::PacksPage;
use sjel_gpui_ui::people::page::PeoplePage;
use sjel_gpui_ui::projects::ProjectsTab;
use sjel_gpui_ui::record::RecordPage;
use sjel_gpui_ui::research::ResearchPage;
use sjel_gpui_ui::self_view::SelfPage;
use sjel_gpui_ui::setup::SetupPage;
use sjel_gpui_ui::software::Software;
use sjel_gpui_ui::systems::SystemsPage;
use sjel_gpui_ui::theme::Palette;
use sjel_gpui_ui::web_links::LinksPage;

use super::Workspace;
use crate::home;
use crate::route::Page;

impl Workspace {
    pub(super) fn ensure_documents(&mut self, cx: &mut Context<Self>) -> Entity<Documents> {
        let repo = self.repo.clone();
        self.documents
            .get_or_insert_with(|| cx.new(|_| Documents::new(repo)))
            .clone()
    }

    pub(super) fn ensure_feed(&mut self, cx: &mut Context<Self>) -> Entity<Feed> {
        let repo = self.repo.clone();
        self.feed
            .get_or_insert_with(|| cx.new(|cx| Feed::new(repo, cx)))
            .clone()
    }

    pub(super) fn ensure_calendar_page(&mut self, cx: &mut Context<Self>) -> Entity<CalendarPage> {
        let repo = self.repo.clone();
        self.calendar_page
            .get_or_insert_with(|| cx.new(|cx| CalendarPage::new(repo, cx)))
            .clone()
    }

    pub(super) fn ensure_finance_page(&mut self, cx: &mut Context<Self>) -> Entity<FinancePage> {
        let repo = self.repo.clone();
        self.finance_page
            .get_or_insert_with(|| cx.new(|cx| FinancePage::new(repo, cx)))
            .clone()
    }

    pub(super) fn ensure_people_page(&mut self, cx: &mut Context<Self>) -> Entity<PeoplePage> {
        let repo = self.repo.clone();
        self.people_page
            .get_or_insert_with(|| cx.new(|cx| PeoplePage::new(repo, cx)))
            .clone()
    }

    pub(super) fn ensure_backup_page(&mut self, cx: &mut Context<Self>) -> Entity<BackupPage> {
        let repo = self.repo.clone();
        self.backup_page
            .get_or_insert_with(|| cx.new(|cx| BackupPage::new(repo, cx)))
            .clone()
    }

    pub(super) fn ensure_inventory_page(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Entity<InventoryPage> {
        let repo = self.repo.clone();
        self.inventory_page
            .get_or_insert_with(|| {
                cx.new(|cx| {
                    #[allow(unused_mut)]
                    let mut page = InventoryPage::new(repo, cx);
                    #[cfg(target_os = "macos")]
                    page.set_cache_preview_handler(crate::roomplan::cache_preview, cx);
                    page
                })
            })
            .clone()
    }

    pub(super) fn ensure_software(&mut self, cx: &mut Context<Self>) -> Entity<Software> {
        let repo = self.repo.clone();
        self.software
            .get_or_insert_with(|| cx.new(|_| Software::new(repo)))
            .clone()
    }

    pub(super) fn ensure_links_page(&mut self, cx: &mut Context<Self>) -> Entity<LinksPage> {
        let repo = self.repo.clone();
        self.links_page
            .get_or_insert_with(|| cx.new(|_| LinksPage::new(repo)))
            .clone()
    }

    pub(super) fn ensure_docs_page(&mut self, cx: &mut Context<Self>) -> Entity<DocsPage> {
        self.docs_page
            .get_or_insert_with(|| cx.new(|_| DocsPage::new()))
            .clone()
    }

    pub(super) fn ensure_record_page(&mut self, cx: &mut Context<Self>) -> Entity<RecordPage> {
        let repo = self.repo.clone();
        self.record_page
            .get_or_insert_with(|| cx.new(|_| RecordPage::new(repo)))
            .clone()
    }

    pub(super) fn ensure_import(&mut self, cx: &mut Context<Self>) -> Entity<ImportPage> {
        let repo = self.repo.clone();
        self.import
            .get_or_insert_with(|| cx.new(|_| ImportPage::new(repo)))
            .clone()
    }

    pub(super) fn ensure_systems(&mut self, cx: &mut Context<Self>) -> Entity<SystemsPage> {
        let repo = self.repo.clone();
        self.systems
            .get_or_insert_with(|| cx.new(|_| SystemsPage::new(repo)))
            .clone()
    }

    pub(super) fn ensure_packs(&mut self, cx: &mut Context<Self>) -> Entity<PacksPage> {
        let repo = self.repo.clone();
        self.packs
            .get_or_insert_with(|| cx.new(|_| PacksPage::new(repo)))
            .clone()
    }

    pub(super) fn ensure_self_page(&mut self, cx: &mut Context<Self>) -> Entity<SelfPage> {
        let repo = self.repo.clone();
        self.self_page
            .get_or_insert_with(|| cx.new(|_| SelfPage::new(repo)))
            .clone()
    }

    pub(super) fn ensure_cv_page(&mut self, cx: &mut Context<Self>) -> Entity<CvPage> {
        let repo = self.repo.clone();
        self.cv_page
            .get_or_insert_with(|| cx.new(|_| CvPage::new(repo)))
            .clone()
    }

    pub(super) fn ensure_media_page(&mut self, cx: &mut Context<Self>) -> Entity<MediaPage> {
        let repo = self.repo.clone();
        self.media_page
            .get_or_insert_with(|| cx.new(|_| MediaPage::new(repo)))
            .clone()
    }

    pub(super) fn ensure_research_page(&mut self, cx: &mut Context<Self>) -> Entity<ResearchPage> {
        let repo = self.repo.clone();
        self.research_page
            .get_or_insert_with(|| cx.new(|_| ResearchPage::new(repo)))
            .clone()
    }

    pub(super) fn ensure_setup_page(&mut self, cx: &mut Context<Self>) -> Entity<SetupPage> {
        let repo = self.repo.clone();
        self.setup_page
            .get_or_insert_with(|| cx.new(|_| SetupPage::new(repo)))
            .clone()
    }

    pub fn go(&mut self, page: Page, cx: &mut Context<Self>) {
        let page = area::canonical(page);
        if page != Page::Settings {
            self.feature_install_confirmation = None;
            self.feature_uninstall_confirmation = None;
        }
        self.tabs.visit(page);
        if self.finder.read(cx).is_open() {
            self.finder.update(cx, |finder, cx| finder.close(cx));
        }
        if self.page != page {
            let previous = self.page;
            if previous == Page::Travel {
                self.travel.update(cx, |travel, _| travel.deactivate());
            }
            if previous == Page::Systems {
                if let Some(systems) = &self.systems {
                    systems.update(cx, |systems, cx| systems.deactivate(cx));
                }
            }
            self.page = page;
            self.refocus = true;
            self.selection.reset();
            if page == Page::Projects {
                self.projects
                    .update(cx, |projects, cx| projects.load_projects(cx));
            } else if page == Page::Travel {
                self.travel.update(cx, |travel, cx| travel.activate(cx));
            } else if page == Page::Backup {
                self.ensure_backup_page(cx)
                    .update(cx, |b, cx| b.activate(cx));
            } else if page == Page::Software {
                self.ensure_software(cx)
                    .update(cx, |software, cx| software.load(cx));
            } else if page == Page::Links {
                self.ensure_links_page(cx).update(cx, |links, cx| {
                    links.load(cx);
                    links.load_candidates(cx);
                });
            } else if page == Page::Import {
                self.ensure_import(cx)
                    .update(cx, |import, cx| import.load(cx));
            } else if page == Page::Systems {
                self.ensure_systems(cx)
                    .update(cx, |systems, cx| systems.activate(cx));
            } else if page == Page::Packs {
                self.ensure_packs(cx).update(cx, |packs, cx| packs.load(cx));
            } else if page == Page::SelfModel {
                self.ensure_self_page(cx).update(cx, |s, cx| s.load(cx));
            } else if page == Page::Cv {
                self.ensure_cv_page(cx).update(cx, |cv, cx| cv.load(cx));
            } else if page == Page::Media {
                self.ensure_media_page(cx).update(cx, |m, cx| m.load(cx));
            } else if page == Page::Research {
                self.ensure_research_page(cx).update(cx, |r, cx| r.load(cx));
            } else if page == Page::Setup {
                self.ensure_setup_page(cx).update(cx, |s, cx| s.load(cx));
            } else if page == Page::Documents {
                self.ensure_documents(cx)
                    .update(cx, |documents, cx| documents.load(cx));
            } else if page == Page::Feed {
                self.ensure_feed(cx)
                    .update(cx, |feed, cx| feed.activate(cx));
            } else if page == Page::Inbox {
                self.inbox.update(cx, |inbox, cx| inbox.activate(cx));
            } else if page == Page::Calendar {
                self.ensure_calendar_page(cx)
                    .update(cx, |calendar, cx| calendar.activate(cx));
            } else if page == Page::Home {
                self.context_page
                    .update(cx, |context, cx| context.activate(cx));
            } else if page == Page::Finance {
                self.ensure_finance_page(cx)
                    .update(cx, |finance, cx| finance.activate(cx));
            } else if page == Page::People {
                self.ensure_people_page(cx)
                    .update(cx, |people, cx| people.activate(cx));
            } else if page == Page::Inventory {
                self.ensure_inventory_page(cx)
                    .update(cx, |p, cx| p.activate(cx));
            }
        }
        cx.notify();
    }

    pub fn go_to_project(&mut self, name: String, cx: &mut Context<Self>) {
        self.go(Page::Projects, cx);
        self.projects.update(cx, |projects, cx| {
            projects.open_project(name, cx);
            projects.tab = ProjectsTab::Hubs;
            cx.notify();
        });
        cx.notify();
    }

    pub fn refresh_current_page(&mut self, cx: &mut Context<Self>) {
        self.refresh(cx);
        if self.page == Page::Projects {
            self.projects
                .update(cx, |projects, cx| projects.load_projects(cx));
        } else if self.page == Page::Links {
            self.ensure_links_page(cx).update(cx, |links, cx| {
                links.load(cx);
                links.load_candidates(cx);
            });
        } else if self.page == Page::Travel {
            self.travel.update(cx, |travel, cx| travel.refresh(cx));
        } else if self.page == Page::Documents {
            self.ensure_documents(cx)
                .update(cx, |documents, cx| documents.load(cx));
        } else if self.page == Page::Feed {
            self.ensure_feed(cx).update(cx, |feed, cx| feed.refresh(cx));
        } else if self.page == Page::Inbox {
            self.inbox.update(cx, |inbox, cx| inbox.load(cx));
        } else if self.page == Page::Calendar {
            self.ensure_calendar_page(cx)
                .update(cx, |calendar, cx| calendar.refresh(cx));
        } else if matches!(self.page, Page::Home | Page::Context) {
            self.context_page
                .update(cx, |context, cx| context.refresh(cx));
        } else if self.page == Page::Finance {
            self.ensure_finance_page(cx)
                .update(cx, |finance, cx| finance.refresh(cx));
        } else if self.page == Page::People {
            self.ensure_people_page(cx)
                .update(cx, |people, cx| people.refresh(cx));
        } else if self.page == Page::Backup {
            self.ensure_backup_page(cx)
                .update(cx, |b, cx| b.refresh(cx));
        } else if self.page == Page::Inventory {
            self.ensure_inventory_page(cx)
                .update(cx, |p, cx| p.refresh(cx));
        } else if self.page == Page::Systems {
            self.ensure_systems(cx).update(cx, |p, cx| p.refresh(cx));
        } else if self.page == Page::Packs {
            self.ensure_packs(cx).update(cx, |p, cx| p.refresh(cx));
        } else if self.page == Page::SelfModel {
            self.ensure_self_page(cx).update(cx, |p, cx| p.refresh(cx));
        } else if self.page == Page::Cv {
            self.ensure_cv_page(cx).update(cx, |p, cx| p.refresh(cx));
        } else if self.page == Page::Media {
            self.ensure_media_page(cx).update(cx, |p, cx| p.refresh(cx));
        } else if self.page == Page::Research {
            self.ensure_research_page(cx)
                .update(cx, |p, cx| p.refresh(cx));
        } else if self.page == Page::Setup {
            self.ensure_setup_page(cx).update(cx, |p, cx| p.refresh(cx));
        }
    }

    /// Dispatches rendering for the active page route.
    pub(super) fn render_current_page(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.page {
            Page::Home => home::render(self, palette, window, cx).into_any_element(),
            Page::Capabilities => capabilities::render(
                capabilities::State {
                    health: &self.health,
                    agent: &self.agent,
                    busy: &self.busy,
                    selected: self.selected(),
                    selection: &self.selection,
                },
                palette,
                cx,
                |view, name, start, window, cx| view.set_running(name, start, window, cx),
                |view, name, _, cx| {
                    let page = match name.as_str() {
                        "finance" => Page::Finance,
                        "calendar" => Page::Calendar,
                        "trips" | "transit" | "places" => Page::Travel,
                        "inventory" | "interior" => Page::Inventory,
                        "projects" => Page::Projects,
                        "documents" => Page::Documents,
                        "media" => Page::Media,
                        "backup" => Page::Backup,
                        "import" => Page::Import,
                        "software" => Page::Software,
                        "web-links" | "links" => Page::Links,
                        "feed" | "comms" => Page::Feed,
                        "systems" => Page::Systems,
                        "packs" => Page::Packs,
                        "self" => Page::SelfModel,
                        "setup" => Page::Setup,
                        _ => Page::SelfModel,
                    };
                    view.go(page, cx);
                },
            )
            .into_any_element(),
            Page::Backup => {
                let page = self.ensure_backup_page(cx);
                page.update(cx, |backup, cx| backup.set_palette(palette, cx));
                page.into_any_element()
            }
            Page::Software => self.ensure_software(cx).into_any_element(),
            Page::Links => self.ensure_links_page(cx).into_any_element(),
            Page::Docs => self.ensure_docs_page(cx).into_any_element(),
            Page::Record => self.ensure_record_page(cx).into_any_element(),
            Page::Import => self.ensure_import(cx).into_any_element(),
            Page::Systems => self.ensure_systems(cx).into_any_element(),
            Page::Packs => self.ensure_packs(cx).into_any_element(),
            Page::SelfModel => self.ensure_self_page(cx).into_any_element(),
            Page::Cv => self.ensure_cv_page(cx).into_any_element(),
            Page::Media => self.ensure_media_page(cx).into_any_element(),
            Page::Research => self.ensure_research_page(cx).into_any_element(),
            Page::Setup => self.ensure_setup_page(cx).into_any_element(),
            Page::Documents => {
                let page = self.ensure_documents(cx);
                page.update(cx, |documents, cx| documents.set_palette(palette, cx));
                page.into_any_element()
            }
            Page::Feed => {
                let page = self.ensure_feed(cx);
                page.update(cx, |feed, cx| feed.set_palette(palette, cx));
                page.into_any_element()
            }
            Page::Inbox => {
                if let Some((music, _)) = &self.music_page {
                    music.update(cx, |view, cx| view.set_palette(palette, cx));
                    music.clone().into_any_element()
                } else {
                    self.inbox
                        .update(cx, |inbox, cx| inbox.set_palette(palette, cx));
                    self.inbox.clone().into_any_element()
                }
            }
            Page::Calendar => {
                let page = self.ensure_calendar_page(cx);
                page.update(cx, |calendar, cx| calendar.set_palette(palette, cx));
                page.into_any_element()
            }
            Page::Context => unreachable!("`/context` opens Home (`core::area::canonical`)"),
            Page::Finance => {
                let page = self.ensure_finance_page(cx);
                page.update(cx, |finance, cx| finance.set_palette(palette, cx));
                page.into_any_element()
            }
            Page::People => {
                let page = self.ensure_people_page(cx);
                page.update(cx, |people, cx| people.set_palette(palette, cx));
                page.into_any_element()
            }
            Page::Inventory => {
                let page = self.ensure_inventory_page(cx);
                page.update(cx, |p, cx| p.set_palette(palette, cx));
                page.into_any_element()
            }
            Page::Projects => {
                self.projects
                    .update(cx, |projects, cx| projects.set_palette(palette, cx));
                self.projects.clone().into_any_element()
            }
            Page::Travel => {
                self.travel.update(cx, |travel, cx| {
                    travel.set_palette(palette, cx);
                    travel.set_compact_map(false);
                });
                self.travel.clone().into_any_element()
            }
            Page::Settings => crate::settings::render(self, palette, window, cx).into_any_element(),
        }
    }
}
