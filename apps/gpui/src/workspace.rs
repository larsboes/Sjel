//! The main window's life cycle and routing: it opens the window, reads and follows the shell,
//! decides which page is current, and owns the command palette, the selection and the toasts.
//!
//! What the window is made of lives beside it: `sidebar` draws the column and `header` the title
//! row, each reading this view and calling back into it.

use std::collections::BTreeSet;
use std::time::Duration;

use futures::StreamExt;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Theme, WindowExt};
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, Action, App, Context, Entity, FocusHandle, KeyBinding,
    SharedString, Subscription, Window, WindowAppearance,
};

use crate::ask::Ask;
use crate::backup::{self, BackupPage};
use crate::calendar_view::CalendarPage;
use crate::context_view::ContextPage;
use crate::data::{self, Busy, Remote, Repo};
use crate::documents::Documents;
use crate::feed::Feed;
use crate::finance_view::FinancePage;
use crate::finder::{Finder, FinderEvent, Hit, Open};
use crate::inbox::{Inbox, InboxEvent};
use crate::inventory_view::InventoryPage;
use crate::people_view::PeoplePage;
use crate::projects::Projects;
use crate::route::{self, Page, Target};
use crate::selection::{Item, Selection};
use crate::shell::{
    self, AgentView, AvailableFeature, Backups, GlobalTask, InstalledFeature, NodeHealth,
    ShellError, Snapshot, SoundscapeState,
};
use crate::theme::{AppearanceSettings, GlassStyle, Palette};
use crate::ytalbum::{YtAlbum, YtAlbumEvent};
use crate::{capabilities, header, home, sidebar};
use sjel_gpui_core::area::{self, Area, Tabs};
use sjel_gpui_core::terminal::Host;
use sjel_gpui_ui::sidebar::{sidebar_frame, Seat, Sidebar};
use sjel_gpui_ui::terminal::TerminalView;

actions!(
    sjel,
    [
        /// Search pages and dashboard destinations.
        OpenPalette,
        /// Focus the active page's search field or open the search palette.
        FocusSearch,
        /// Step to the next tab of the open area.
        NextPage,
        /// Step to the previous tab of the open area.
        PrevPage,
        /// Step to the next area in the sidebar.
        NextArea,
        /// Step to the previous area in the sidebar.
        PrevArea,
        Refresh,
        /// Open the current page in the web dashboard, logged in.
        OpenInDashboard,
        SelectNext,
        SelectPrevious,
        /// Allow, start, or open the selected row.
        RunPrimary,
        /// Deny or stop the selected row.
        RunSecondary,
        ClearSelection,
        /// Turn to the previous page of the open document.
        DocPrevPage,
        /// Turn to the next page of the open document.
        DocNextPage,
        /// Toggle 1x / 2x zoom on the open document reader.
        DocToggleZoom,
        /// Open Settings at the section it showed last.
        OpenSettings,
        /// Open Settings at Appearance.
        OpenAppearance,
        /// Slide the soundscape panel in, or away again.
        OpenSoundscape,
        /// Show the soundscape panel if it is hidden, hide it if it is showing.
        ToggleSoundscapePanel,
        /// Toggle playback of the soundscape ambient synthesizer.
        ToggleSoundscape,
        /// Advance to the next ambient soundscape preset.
        CycleSoundscapePreset,
        /// Check git repository for newer commits.
        CheckForUpdates,
        /// Trigger background rebuild and install of Sjel Desktop.
        RebuildApp,
    ]
);

/// Open a page. One action serves every page, so a page added to an area in `core::area` gets
/// its palette row and tab from `nav_pages` without another edit.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = sjel, no_json)]
pub struct GoTo(pub Page);

/// The columns and lines the right edge's terminal asks the pty for. The edge is 390 points wide,
/// a monospace cell is about 7.2, and the grid is padded inside the seat's frame.
const TERMINAL_COLS: usize = 48;
const TERMINAL_ROWS: usize = 44;

/// Open an area at the tab it showed last. ⌘1–⌘8, the sidebar rows and the Go menu use it.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = sjel, no_json)]
pub struct OpenArea(pub Area);

/// Every page an area holds, in sidebar then tab order. Settings has ⌘, instead, and the old
/// Today page is part of Home.
/// Everything the command palette offered, as finder results: every page with its breadcrumb,
/// the app's actions, and the dashboard pages that have no native page yet.
fn finder_commands() -> Vec<Hit> {
    let pages = nav_pages().chain([Page::Settings]).map(|page| Hit {
        kind: "page",
        title: page_title(page),
        detail: page.label().to_owned(),
        open: Open::Page(page),
    });
    let action = |title: &str, detail: &str, open: fn() -> Box<dyn Action>| Hit {
        kind: "action",
        title: title.to_owned(),
        detail: detail.to_owned(),
        open: Open::Action(open),
    };
    let actions = [
        action("Refresh", "reload this page", || Refresh.boxed_clone()),
        action("Check for App Updates", "update", || {
            CheckForUpdates.boxed_clone()
        }),
        action("Rebuild Sjel Desktop", "build install", || {
            RebuildApp.boxed_clone()
        }),
        action(
            "Appearance",
            "theme dark light accent glass transparency",
            || OpenAppearance.boxed_clone(),
        ),
        action("Open this page in the dashboard", "web browser", || {
            OpenInDashboard.boxed_clone()
        }),
        action(
            "Soundscape",
            "music audio focus ambient lofi player ⌘0",
            || OpenSoundscape.boxed_clone(),
        ),
    ];
    // A terminal is a deep link as well as a result, so the same string opens it from either.
    let terminal = std::iter::once(Hit {
        kind: "action",
        title: "Terminal".to_owned(),
        detail: "shell claude codex pi on this Mac".to_owned(),
        open: Open::Link("sjel://terminal".to_owned()),
    });
    let dashboard = route::DESTINATIONS
        .iter()
        .filter(|destination| destination.page().is_none())
        .map(|destination| Hit {
            kind: "dashboard",
            title: destination.label.to_owned(),
            detail: destination.href.to_owned(),
            open: Open::Dashboard(destination.href),
        });
    pages
        .chain(actions)
        .chain(terminal)
        .chain(dashboard)
        .collect()
}

pub fn nav_pages() -> impl Iterator<Item = Page> {
    Area::ALL
        .into_iter()
        .flat_map(|area| area.pages().iter().copied())
}

/// How the palette and the Go menu name a page: `Sjel › Packs` when the area has tabs.
pub fn page_title(page: Page) -> String {
    match Area::of(page) {
        Some(area) if area.has_tabs() => format!("{} › {}", area.label(), page.label()),
        Some(area) => area.label().to_owned(),
        None => page.label().to_owned(),
    }
}

/// Workspace key bindings. Navigation keys bind to the "Workspace" context, which only the page
/// frame carries, so they stay quiet while the palette's query field has focus.
pub fn bind_keys(cx: &mut App) {
    // What the panel itself carries: space, the next preset, and the way out.
    cx.bind_keys(crate::soundscape::Soundscape::keys());
    cx.bind_keys(Finder::keys());
    cx.bind_keys([Finder::mode_key()]);
    cx.bind_keys([
        KeyBinding::new("cmd-k", OpenPalette, None),
        KeyBinding::new("cmd-f", FocusSearch, None),
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("cmd-d", OpenInDashboard, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-[", ClearSelection, None),
        KeyBinding::new("escape", ClearSelection, None),
        // Tabs inside the open area, the way Safari and Xcode step tabs
        KeyBinding::new("cmd-alt-right", NextPage, None),
        KeyBinding::new("shift-cmd-]", NextPage, None),
        KeyBinding::new("ctrl-tab", NextPage, None),
        KeyBinding::new("cmd-alt-left", PrevPage, None),
        KeyBinding::new("shift-cmd-[", PrevPage, None),
        KeyBinding::new("ctrl-shift-tab", PrevPage, None),
        // Areas, in sidebar order
        KeyBinding::new("cmd-alt-down", NextArea, None),
        KeyBinding::new("cmd-alt-up", PrevArea, None),
        // Soundscape ambient audio controls
        KeyBinding::new("cmd-alt-space", ToggleSoundscape, None),
        KeyBinding::new("shift-cmd-u", ToggleSoundscape, None),
        KeyBinding::new("cmd-alt-p", CycleSoundscapePreset, None),
        // The player slides in over this window, so its keystroke belongs to the workspace and not
        // to the app: there is no window to open, only a panel to move.
        KeyBinding::new("shift-cmd-m", ToggleSoundscapePanel, None),
        // The slot after the nine areas: the player is the one thing every area shares.
        KeyBinding::new("cmd-0", ToggleSoundscapePanel, None),
        // Workspace scoped navigation & selection
        KeyBinding::new("/", FocusSearch, Some("Workspace")),
        KeyBinding::new("j", SelectNext, Some("Workspace")),
        KeyBinding::new("down", SelectNext, Some("Workspace")),
        KeyBinding::new("k", SelectPrevious, Some("Workspace")),
        KeyBinding::new("up", SelectPrevious, Some("Workspace")),
        KeyBinding::new("enter", RunPrimary, Some("Workspace")),
        KeyBinding::new("space", RunPrimary, Some("Workspace")),
        KeyBinding::new("cmd-backspace", RunSecondary, Some("Workspace")),
        KeyBinding::new("[", DocPrevPage, Some("Workspace")),
        KeyBinding::new("left", DocPrevPage, Some("Workspace")),
        KeyBinding::new("pageup", DocPrevPage, Some("Workspace")),
        KeyBinding::new("]", DocNextPage, Some("Workspace")),
        KeyBinding::new("right", DocNextPage, Some("Workspace")),
        KeyBinding::new("pagedown", DocNextPage, Some("Workspace")),
        KeyBinding::new("z", DocToggleZoom, Some("Workspace")),
        KeyBinding::new("cmd-=", DocToggleZoom, Some("Workspace")),
        KeyBinding::new("cmd-+", DocToggleZoom, Some("Workspace")),
        // Vim chords for fast page jumps
        KeyBinding::new("g h", GoTo(Page::Home), Some("Workspace")),
        KeyBinding::new("g o", GoTo(Page::Context), Some("Workspace")),
        KeyBinding::new("g t", GoTo(Page::Context), Some("Workspace")),
        KeyBinding::new("g m", GoTo(Page::Capabilities), Some("Workspace")),
        KeyBinding::new("g p", GoTo(Page::Projects), Some("Workspace")),
        KeyBinding::new("g a", OpenPalette, Some("Workspace")),
        KeyBinding::new("g v", GoTo(Page::Travel), Some("Workspace")),
        KeyBinding::new("g e", GoTo(Page::Feed), Some("Workspace")),
        KeyBinding::new("g n", GoTo(Page::Inbox), Some("Workspace")),
        KeyBinding::new("g c", GoTo(Page::Calendar), Some("Workspace")),
        KeyBinding::new("g d", GoTo(Page::Documents), Some("Workspace")),
        KeyBinding::new("g b", GoTo(Page::Backup), Some("Workspace")),
        KeyBinding::new("g f", GoTo(Page::Finance), Some("Workspace")),
        KeyBinding::new("g r", GoTo(Page::People), Some("Workspace")),
        KeyBinding::new("g i", GoTo(Page::Inventory), Some("Workspace")),
        KeyBinding::new("g s", GoTo(Page::Software), Some("Workspace")),
        KeyBinding::new("g u", GoTo(Page::Import), Some("Workspace")),
        KeyBinding::new("g x", GoTo(Page::Systems), Some("Workspace")),
        KeyBinding::new("g k", GoTo(Page::Packs), Some("Workspace")),
        KeyBinding::new("g l", GoTo(Page::SelfModel), Some("Workspace")),
        KeyBinding::new("g y", GoTo(Page::Cv), Some("Workspace")),
        KeyBinding::new("g w", GoTo(Page::Media), Some("Workspace")),
        KeyBinding::new("g z", GoTo(Page::Research), Some("Workspace")),
        KeyBinding::new("g 0", GoTo(Page::Setup), Some("Workspace")),
        KeyBinding::new("g ,", GoTo(Page::Settings), Some("Workspace")),
    ]);
    cx.bind_keys(
        Area::ALL
            .into_iter()
            .zip(1..=9)
            .map(|(area, n)| KeyBinding::new(&format!("cmd-{n}"), OpenArea(area), None)),
    );
    cx.bind_keys([
        KeyBinding::new("shift-cmd-d", GoTo(Page::Documents), None),
        KeyBinding::new("shift-cmd-b", GoTo(Page::Backup), None),
        KeyBinding::new("shift-cmd-f", GoTo(Page::Finance), None),
        KeyBinding::new("shift-cmd-p", GoTo(Page::People), None),
        KeyBinding::new("shift-cmd-i", GoTo(Page::Inventory), None),
        KeyBinding::new("shift-cmd-s", GoTo(Page::Software), None),
        KeyBinding::new("shift-cmd-c", GoTo(Page::Calendar), None),
        KeyBinding::new("shift-cmd-t", GoTo(Page::Travel), None),
        KeyBinding::new("shift-cmd-k", GoTo(Page::Projects), None),
        KeyBinding::new("shift-cmd-a", OpenPalette, None),
        KeyBinding::new("shift-cmd-e", GoTo(Page::Feed), None),
        KeyBinding::new("shift-cmd-n", GoTo(Page::Inbox), None),
        KeyBinding::new("shift-cmd-h", GoTo(Page::Home), None),
        KeyBinding::new("shift-cmd-o", GoTo(Page::Context), None),
    ]);
}

/// How often the open window reads the shell again.
const POLL: Duration = Duration::from_secs(10);

pub struct Workspace {
    pub page: Page,
    /// The tab each area shows, so ⌘1–⌘8 return to where you left an area.
    pub tabs: Tabs,
    repo: Repo,
    pub health: Remote<NodeHealth>,
    pub agent: Remote<AgentView>,
    pub installed_features: Remote<Vec<InstalledFeature>>,
    pub available_features: Remote<Vec<AvailableFeature>>,
    pub settings_feature_detail: Option<String>,
    pub feature_uninstall_confirmation: Option<String>,
    pub feature_install_confirmation: Option<String>,
    pub feature_manager_message: Option<String>,
    pub dismissed_feature_recommendations: BTreeSet<String>,
    pub focus_tasks: Remote<Vec<GlobalTask>>,
    pub soundscape: Remote<SoundscapeState>,
    pub backups: Remote<Backups>,
    pub projects: Entity<Projects>,
    pub ask: Entity<Ask>,
    pub accounts: Entity<crate::accounts::Accounts>,
    pub travel: Entity<sjel_gpui_ui::travel::Travel>,
    pub documents: Entity<Documents>,
    pub feed: Entity<Feed>,
    pub inbox: Entity<Inbox>,
    ytalbum_page: Option<(Entity<YtAlbum>, Subscription)>,
    _inbox_events: Subscription,
    pub calendar_page: Entity<CalendarPage>,
    pub context_page: Entity<ContextPage>,
    pub finance_page: Entity<FinancePage>,
    pub people_page: Entity<PeoplePage>,
    pub backup_page: Entity<BackupPage>,
    pub inventory_page: Entity<InventoryPage>,
    pub software: Entity<crate::software::Software>,
    pub apps_page: Entity<crate::apps::AppsPage>,
    pub docs_page: Entity<crate::docs::DocsPage>,
    pub record_page: Entity<crate::record::RecordPage>,
    pub import: Entity<crate::import::ImportPage>,
    pub systems: Entity<crate::systems::SystemsPage>,
    pub packs: Entity<crate::packs_view::PacksPage>,
    pub self_page: Entity<crate::self_view::SelfPage>,
    pub cv_page: Entity<crate::cv_view::CvPage>,
    pub media_page: Entity<crate::media_view::MediaPage>,
    pub research_page: Entity<crate::research_view::ResearchPage>,
    pub setup_page: Entity<crate::setup_view::SetupPage>,
    /// Approval ids and capability names with a request in flight.
    pub busy: Busy,
    pub selection: Selection,
    /// True once the shell pushes its answers, so the timer stops reading them.
    streaming: bool,
    pub appearance_settings: AppearanceSettings,
    /// The section Settings shows.
    pub settings_section: crate::settings::Section,
    pub settings_feature_view: crate::settings::FeatureView,
    /// Search within Settings and show or hide its contents rail.
    pub settings_query: String,
    /// A shortcut group selected from the Settings contents rail.
    pub settings_shortcut_group: Option<&'static str>,
    pub settings_toc_visible: bool,
    pub settings_search: Option<(Entity<gpui_kit::component::input::InputState>, Subscription)>,
    pub update_info: Option<crate::update::UpdateInfo>,
    pub rebuild_state: crate::update::RebuildState,
    /// Whether the failed rebuild shows cargo's whole output instead of the first error.
    pub show_rebuild_log: bool,
    focus: FocusHandle,
    /// Set when the page changes. The old page's field may have had focus and is gone from the
    /// tree, which would leave every shortcut with no path to this view.
    refocus: bool,
    _appearance: Option<Subscription>,
    /// The soundscape panel, built the first time it is asked for and kept afterwards: the audio
    /// lives in an app global, so the view may as well too. `None` until something opens it, which is
    /// what keeps a fresh window from paying for a panel nobody is looking at.
    pub soundscape_panel: Option<Entity<crate::soundscape::Soundscape>>,
    /// The docked right edge. Its state is shared with the phone and stored in `sidebar.json`.
    pub right_panel: Sidebar,
    /// The terminal seat, built the first time it is asked for and kept afterwards: closing the
    /// drawer does not end the shell, so reopening it shows the screen as it is now.
    pub terminal: Option<Entity<TerminalView>>,
    /// Why no terminal is showing, in the words the empty seat carries.
    pub terminal_note: Option<SharedString>,
    /// One input for a search or a prompt (⌘K), drawn over the whole window.
    pub finder: Entity<Finder>,
    _finder: Subscription,
    frames: Option<crate::frames::FrameProbe>,
    #[cfg(target_os = "macos")]
    native_glass: Option<crate::macos_glass::NativeGlassRegion>,
    #[cfg(target_os = "macos")]
    native_glass_attempted: bool,
}

impl Workspace {
    pub fn new(endpoint: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance = cx.observe_window_appearance(window, |view, window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
            view.sync_kit_accent(cx);
            cx.notify();
        });
        Theme::sync_system_appearance(Some(window), cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let repo = Repo::new(shell::Shell::new(endpoint).map(std::sync::Arc::new));
        let projects = cx.new(|_| Projects::new(repo.clone()));
        let ask = cx.new(|_| Ask::new(repo.clone()));
        let accounts = cx.new(|_| crate::accounts::Accounts::new(repo.clone()));
        let travel = cx.new(|_| {
            sjel_gpui_ui::travel::Travel::new(
                repo.shell().ok().map(sjel_gpui_core::travel::Source::Shell),
                sjel_gpui_ui::Density::Desktop,
            )
        });
        let documents = cx.new(|_| Documents::new(repo.clone()));
        let feed = cx.new(|cx| Feed::new(repo.clone(), cx));
        let inbox = cx.new(|cx| Inbox::new(repo.clone(), cx));
        let inbox_events = cx.subscribe_in(&inbox, window, Self::on_inbox);
        let calendar_page = cx.new(|cx| CalendarPage::new(repo.clone(), cx));
        let context_page = cx.new(|cx| ContextPage::new(repo.clone(), cx));
        let finance_page = cx.new(|cx| FinancePage::new(repo.clone(), cx));
        let people_page = cx.new(|cx| PeoplePage::new(repo.clone(), cx));
        let backup_page = cx.new(|cx| BackupPage::new(repo.clone(), cx));
        let inventory_page = cx.new(|cx| InventoryPage::new(repo.clone(), cx));
        let software = cx.new(|_| crate::software::Software::new(repo.clone()));
        let apps_page = cx.new(|_| crate::apps::AppsPage::new(repo.clone()));
        let docs_page = cx.new(|_| crate::docs::DocsPage::new());
        let record_page = cx.new(|_| crate::record::RecordPage::new(repo.clone()));
        let import = cx.new(|_| crate::import::ImportPage::new(repo.clone()));
        let systems = cx.new(|_| crate::systems::SystemsPage::new(repo.clone()));
        let packs = cx.new(|_| crate::packs_view::PacksPage::new(repo.clone()));
        let self_page = cx.new(|_| crate::self_view::SelfPage::new(repo.clone()));
        let cv_page = cx.new(|_| crate::cv_view::CvPage::new(repo.clone()));
        let media_page = cx.new(|_| crate::media_view::MediaPage::new(repo.clone()));
        let research_page = cx.new(|_| crate::research_view::ResearchPage::new(repo.clone()));
        let setup_page = cx.new(|_| crate::setup_view::SetupPage::new(repo.clone()));
        let appearance_settings = AppearanceSettings::load();
        let finder = {
            let (repo, ask) = (repo.clone(), ask.clone());
            cx.new(|cx| Finder::new(repo, ask, finder_commands(), cx))
        };
        let finder_events = cx.subscribe_in(&finder, window, Self::on_finder);
        let settings_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search settings, accounts, features, shortcuts")
        });
        let settings_search_events = cx.subscribe_in(
            &settings_search,
            window,
            |view, input, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    view.settings_query = input.read(cx).value().to_string();
                    view.settings_shortcut_group = None;
                    view.settings_feature_detail = None;
                    view.feature_install_confirmation = None;
                    view.feature_uninstall_confirmation = None;
                    let catalogue_has_match =
                        view.available_features
                            .value
                            .as_ref()
                            .is_some_and(|packages| {
                                crate::settings::catalogue_matches(packages, &view.settings_query)
                            });
                    if !view.settings_query.trim().is_empty() {
                        view.settings_section = crate::settings::search_section(
                            &view.settings_query,
                            catalogue_has_match,
                        )
                        .unwrap_or(crate::settings::Section::Shortcuts);
                        if view.settings_section == crate::settings::Section::Accounts {
                            let search_query = view.settings_query.clone();
                            view.accounts.update(cx, move |accounts, cx| {
                                if let Some(provider) =
                                    crate::settings::provider_for_search(&search_query)
                                {
                                    accounts.select_key_provider(provider, window, cx);
                                }
                                accounts.refresh_if_missing(cx);
                            });
                        }
                        if view.settings_section == crate::settings::Section::Features {
                            if let Some(feature_view) =
                                crate::settings::feature_view_for_search(&view.settings_query)
                            {
                                view.settings_feature_view = feature_view;
                            } else if catalogue_has_match {
                                view.settings_feature_view = crate::settings::FeatureView::Explore;
                            }
                        }
                    }
                    cx.notify();
                }
            },
        );
        let mut workspace = Self {
            page: Page::Home,
            tabs: Tabs::default(),
            repo,
            health: Remote::default(),
            agent: Remote::default(),
            installed_features: Remote::default(),
            available_features: Remote::default(),
            settings_feature_detail: None,
            feature_uninstall_confirmation: None,
            feature_install_confirmation: None,
            feature_manager_message: None,
            dismissed_feature_recommendations: crate::settings::load_recommendation_dismissals(),
            focus_tasks: Remote::default(),
            soundscape: Remote::default(),
            soundscape_panel: None,
            right_panel: Sidebar::load(),
            terminal: None,
            terminal_note: None,
            finder,
            _finder: finder_events,
            backups: Remote::default(),
            projects,
            ask,
            accounts,
            travel,
            documents,
            feed,
            inbox,
            ytalbum_page: None,
            _inbox_events: inbox_events,
            calendar_page,
            context_page,
            finance_page,
            people_page,
            backup_page,
            inventory_page,
            software,
            apps_page,
            docs_page,
            record_page,
            import,
            systems,
            packs,
            self_page,
            cv_page,
            media_page,
            research_page,
            setup_page,
            busy: Busy::default(),
            selection: Selection::default(),
            streaming: false,
            focus,
            refocus: false,
            appearance_settings,
            settings_section: crate::settings::Section::default(),
            settings_feature_view: crate::settings::FeatureView::default(),
            settings_query: String::new(),
            settings_shortcut_group: None,
            settings_toc_visible: true,
            settings_search: Some((settings_search, settings_search_events)),
            update_info: None,
            rebuild_state: crate::update::RebuildState::Idle,
            show_rebuild_log: false,
            _appearance: Some(appearance),
            frames: crate::frames::FrameProbe::from_env(),
            #[cfg(target_os = "macos")]
            native_glass: None,
            #[cfg(target_os = "macos")]
            native_glass_attempted: false,
        };
        workspace.sync_kit_accent(cx);
        workspace.refresh(cx);
        workspace.check_updates(cx);
        workspace.follow(cx);
        // The window opens on Today, which draws the day's timeline, so that read starts now.
        workspace
            .context_page
            .update(cx, |context, cx| context.activate(cx));
        if let (Some(frames), true) = (
            workspace.frames.clone(),
            std::env::var_os("SJEL_GPUI_BENCH_SCROLL").is_some(),
        ) {
            crate::frames::drive_scroll_bench(frames, workspace.selection.scroll.clone(), cx);
        }
        if let (Some(frames), Ok(path)) = (
            workspace.frames.clone(),
            std::env::var("SJEL_GPUI_BENCH_NOTE"),
        ) {
            workspace.go(Page::Projects, cx);
            crate::frames::drive_type_bench(frames, path, workspace.projects.clone(), window, cx);
        }
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(POLL).await;
            if this.update(cx, |view, cx| view.refresh(cx)).is_err() {
                break;
            }
        })
        .detach();
        workspace
    }

    pub fn endpoint(&self) -> SharedString {
        self.repo.endpoint().into()
    }

    /// True when a local database answers, which the sidebar shows as its read path.
    pub fn has_direct_db(&self) -> bool {
        self.repo.has_direct_db()
    }

    pub fn go(&mut self, page: Page, cx: &mut Context<Self>) {
        // Today was its own page and is part of Home now; its old shortcuts and links land here.
        let page = area::canonical(page);
        if page != Page::Settings {
            self.feature_install_confirmation = None;
            self.feature_uninstall_confirmation = None;
        }
        self.tabs.visit(page);
        // Going somewhere is what the finder was for, whichever key did it.
        if self.finder.read(cx).is_open() {
            self.finder.update(cx, |finder, cx| finder.close(cx));
        }
        if self.page != page {
            let previous = self.page;
            if previous == Page::Travel {
                self.travel.update(cx, |travel, _| travel.deactivate());
            }
            // Leaving Systems stops its clocks: three beats that ask the machine a question are
            // worth nothing while the page is not drawn.
            if previous == Page::Systems {
                self.systems
                    .update(cx, |systems, cx| systems.deactivate(cx));
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
                self.backup_page.update(cx, |b, cx| b.activate(cx));
            } else if page == Page::Software {
                self.software.update(cx, |software, cx| software.load(cx));
            } else if page == Page::Apps {
                self.apps_page.update(cx, |apps, cx| apps.load(cx));
            } else if page == Page::Import {
                self.import.update(cx, |import, cx| import.load(cx));
            } else if page == Page::Systems {
                self.systems.update(cx, |systems, cx| systems.activate(cx));
            } else if page == Page::Packs {
                self.packs.update(cx, |packs, cx| packs.load(cx));
            } else if page == Page::SelfModel {
                self.self_page.update(cx, |s, cx| s.load(cx));
            } else if page == Page::Cv {
                self.cv_page.update(cx, |cv, cx| cv.load(cx));
            } else if page == Page::Media {
                self.media_page.update(cx, |m, cx| m.load(cx));
            } else if page == Page::Research {
                self.research_page.update(cx, |r, cx| r.load(cx));
            } else if page == Page::Setup {
                self.setup_page.update(cx, |s, cx| s.load(cx));
            } else if page == Page::Documents {
                self.documents
                    .update(cx, |documents, cx| documents.load(cx));
            } else if page == Page::Feed {
                self.feed.update(cx, |feed, cx| feed.activate(cx));
            } else if page == Page::Inbox {
                self.inbox.update(cx, |inbox, cx| inbox.activate(cx));
            } else if page == Page::Calendar {
                self.calendar_page
                    .update(cx, |calendar, cx| calendar.activate(cx));
            } else if page == Page::Home {
                self.context_page
                    .update(cx, |context, cx| context.activate(cx));
            } else if page == Page::Finance {
                self.finance_page
                    .update(cx, |finance, cx| finance.activate(cx));
            } else if page == Page::People {
                self.people_page
                    .update(cx, |people, cx| people.activate(cx));
            } else if page == Page::Inventory {
                self.inventory_page.update(cx, |p, cx| p.activate(cx));
            }
        }
        cx.notify();
    }

    /// Follows the shell's change stream. Until a frame arrives the timer keeps reading health
    /// and agent, and it keeps reading them when this shell has no stream to send.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let Ok(mut events) = shell.subscribe() else {
            return;
        };
        cx.spawn(async move |weak, cx| {
            while let Some(snapshot) = events.next().await {
                let applied = weak.update(cx, |this, cx| {
                    this.streaming = true;
                    this.apply(snapshot);
                    cx.notify();
                });
                if applied.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    fn apply(&mut self, snapshot: Snapshot) {
        if let Some(health) = snapshot.health {
            self.health.settle(Ok(health));
        }
        if let Some(agent) = snapshot.agent {
            self.agent.settle(Ok(agent));
        }
        if let Some(error) = snapshot.error {
            let error = ShellError::Refused(502, Some(error));
            self.health.settle(Err(error));
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        if !self.streaming {
            data::read(
                cx,
                self,
                |w| &mut w.health,
                &repo,
                |s| async move { s.health().await },
            );
            data::read(
                cx,
                self,
                |w| &mut w.agent,
                &repo,
                |s| async move { s.agent().await },
            );
        }
        // Only while the page is open: the answer reads a receipt per capability.
        if self.page == Page::Backup {
            data::read(
                cx,
                self,
                |w| &mut w.backups,
                &repo,
                |s| async move { s.backups().await },
            );
        }
        let repo_tasks = self.repo.clone();
        data::read_repo(
            cx,
            self,
            |w| &mut w.focus_tasks,
            &repo_tasks,
            move |r| async move { r.fetch_tasks(Some("open")).await },
        );
        let repo_sound = self.repo.clone();
        data::read_repo(
            cx,
            self,
            |w| &mut w.soundscape,
            &repo_sound,
            move |r| async move { r.soundscape_state().await },
        );
        self.check_updates(cx);
    }

    pub fn check_updates(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let info = cx
                .background_executor()
                .spawn(async move {
                    crate::update::find_repo_root()
                        .and_then(|repo| crate::update::check_git_updates(&repo))
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.update_info = info;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn trigger_rebuild(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo_root) = crate::update::find_repo_root() else {
            window.push_notification(
                Notification::error("Could not locate Sjel repository root."),
                cx,
            );
            return;
        };
        self.rebuild_state = crate::update::RebuildState::Building;
        self.show_rebuild_log = false;
        cx.notify();
        window.push_notification(
            Notification::info("Rebuilding Sjel Desktop in background..."),
            cx,
        );

        let root = repo_root.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { crate::update::execute_rebuild(&root) })
                .await;
            let _ = this.update(cx, |view, cx| {
                match res {
                    Ok(msg) => {
                        view.rebuild_state = crate::update::RebuildState::Success(msg);
                    }
                    Err(err) => {
                        view.rebuild_state = crate::update::RebuildState::Failed(err);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The panel, created on first use. Separate from [`Self::open_soundscape`] because the sidebar
    /// and the palette both want it without asking for it to appear.
    fn soundscape_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<crate::soundscape::Soundscape> {
        if let Some(panel) = &self.soundscape_panel {
            return panel.clone();
        }
        let repo = self.repo.clone();
        let panel = cx.new(|cx| crate::soundscape::Soundscape::new(repo, cx));
        // It is a panel over this window, so it paints in this window's frame and follows its
        // appearance; nothing here has to move it.
        let _ = window;
        cx.notify();
        self.soundscape_panel = Some(panel.clone());
        panel
    }

    /// Slide the player in. The panel owns whether it is open, so this is idempotent: asking for a
    /// panel that is already showing does not restart the slide.
    pub fn open_soundscape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.soundscape_panel(window, cx);
        panel.update(cx, |panel, cx| panel.set_open(true, cx));
    }

    /// The transport, written through the panel: it is the one writer of play and preset, so its
    /// guard against a stale poll covers the sidebar and the global keys as well.
    pub fn toggle_soundscape(
        &mut self,
        playing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = self.soundscape.value.as_mut() {
            state.playing = playing;
        }
        let panel = self.soundscape_panel(window, cx);
        panel.update(cx, |panel, cx| panel.set_playing(playing, window, cx));
        cx.notify();
    }

    pub fn cycle_soundscape_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use sjel_gpui_core::soundscape::{music::PRESETS, Preset};
        let current = self
            .soundscape
            .value
            .as_ref()
            .and_then(|state| Preset::from_key(&state.preset))
            .unwrap_or(Preset::Edm);
        let next =
            Preset::from_key(PRESETS[(current.index() + 1) % PRESETS.len()]).unwrap_or(Preset::Edm);
        if let Some(state) = self.soundscape.value.as_mut() {
            state.preset = next.key().to_owned();
        }
        let panel = self.soundscape_panel(window, cx);
        panel.update(cx, |panel, cx| panel.choose_preset(next, window, cx));
        cx.notify();
    }

    pub fn toggle_task(
        &mut self,
        path: String,
        done: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        let id = path.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet(if done {
                "Task completed."
            } else {
                "Task reopened."
            }),
            move |s| async move { s.set_task_done(&id, done).await },
            |this, updated, cx| {
                if let Some(task) = updated {
                    if let Some(tasks) = &mut this.focus_tasks.value {
                        if let Some(pos) = tasks.iter().position(|t| t.id == task.id) {
                            if task.done {
                                tasks.remove(pos);
                            } else {
                                tasks[pos] = task;
                            }
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    pub fn cycle_task_priority(
        &mut self,
        path: String,
        current: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = (current.clamp(1, 3) % 3) + 1;
        let p_str = next.to_string();
        let target_path = path.clone();
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet("Priority updated."),
            move |s| async move { s.set_property(&target_path, "priority", &p_str).await },
            |this, updated, cx| {
                if let Some(task) = updated {
                    if let Some(tasks) = &mut this.focus_tasks.value {
                        if let Some(pos) = tasks.iter().position(|t| t.id == task.id) {
                            tasks[pos] = task;
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    pub fn go_to_project(&mut self, name: String, cx: &mut Context<Self>) {
        // Through `go`, so the frame takes back the focus the page that sent us here was holding.
        self.go(Page::Projects, cx);
        self.projects.update(cx, |projects, cx| {
            projects.open_project(name, cx);
            projects.tab = crate::projects::ProjectsTab::Hubs;
            cx.notify();
        });
        cx.notify();
    }

    pub fn decide(&mut self, id: String, allow: bool, window: &mut Window, cx: &mut Context<Self>) {
        let done = if allow {
            "Allowed the agent write."
        } else {
            "Denied the agent write."
        };
        let request_id = id.clone();
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|w| &mut w.busy, id, done),
            move |s| async move { s.decide(&request_id, allow).await },
            |this, _, cx| this.refresh(cx),
        );
    }

    pub fn set_running(
        &mut self,
        name: String,
        running: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let done = if running { "Started." } else { "Stopped." };
        let request_name = name.clone();
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|w| &mut w.busy, name, done),
            move |s| async move { s.set_running(&request_name, running).await },
            |this, _, cx| this.refresh(cx),
        );
    }

    /// Restarts the local shell through the installed checkout's supervisor, even when the shell
    /// itself refuses the current credential.
    pub fn restart_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        const BUSY_KEY: &str = "sjel-status-recovery";
        if !self.busy.start(BUSY_KEY) {
            return;
        }
        let Some(root) = crate::update::find_repo_root() else {
            self.busy.finish(BUSY_KEY);
            window.push_notification(
                Notification::error("Could not locate the Sjel checkout to restart its shell."),
                cx,
            );
            cx.notify();
            return;
        };
        cx.notify();
        let work = cx
            .background_executor()
            .spawn(async move { crate::update::restart_local_shell(&root) });
        cx.spawn_in(window, async move |weak, cx| {
            let result = work.await;
            let _ = weak.update_in(cx, |view, window, cx| {
                view.busy.finish(BUSY_KEY);
                match result {
                    Ok(()) => {
                        window.push_notification(
                            Notification::success("Sjel shell restarted. Reconnecting…"),
                            cx,
                        );
                        view.refresh(cx);
                    }
                    Err(error) => window.push_notification(Notification::error(error), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Starts a backup of every capability with a contract. Each run is accepted and returns;
    /// the poll shows them finish. Runs of different capabilities may overlap by design.
    pub fn back_up_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names: Vec<String> = self
            .backups
            .value
            .iter()
            .flat_map(|b| b.backups.iter().map(|status| status.capability.clone()))
            .collect();
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|w| &mut w.busy, backup::BACK_UP_ALL, "Backup started."),
            move |s| async move {
                for name in names {
                    s.backup(&name).await?;
                }
                Ok(())
            },
            |this, _, cx| this.refresh(cx),
        );
    }

    /// Opens `path` in the browser, logged in with a single-use ticket.
    pub fn open_dashboard(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                window.push_notification(Notification::error(error.to_string()), cx);
                return;
            }
        };
        let work = shell::run(async move { shell.dashboard_url(&path).await });
        cx.spawn_in(window, async move |_, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = cx.update(|window, cx| match result {
                Ok(url) => cx.open_url(url.as_str()),
                Err(error) => window.push_notification(Notification::error(error.to_string()), cx),
            });
        })
        .detach();
    }

    pub fn open_target(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            Target::Native(Page::Travel) => {
                self.go(Page::Travel, cx);
                self.travel.update(cx, |travel, cx| {
                    travel.open_route(sjel_gpui_core::travel::TravelRoute::Trips, cx)
                });
            }
            Target::Native(page) => self.go(page, cx),
            Target::Travel(route) => {
                self.go(Page::Travel, cx);
                self.travel
                    .update(cx, |travel, cx| travel.open_route(route, cx));
            }
            Target::Feed(tab) => {
                self.go(Page::Feed, cx);
                self.feed.update(cx, |feed, cx| feed.set_tab(tab, cx));
            }
            Target::FeedEntry(id) => {
                self.go(Page::Feed, cx);
                self.feed.update(cx, |feed, cx| feed.open_entry(id, cx));
            }
            Target::MailEntry(id) => {
                self.go(Page::Feed, cx);
                self.feed
                    .update(cx, |feed, cx| feed.open_mail_entry(id, cx));
            }
            Target::CalendarEntry(id) => {
                self.go(Page::Feed, cx);
                self.feed
                    .update(cx, |feed, cx| feed.open_calendar_entry(id, cx));
            }
            Target::Docs(slug) => {
                self.go(Page::Docs, cx);
                self.docs_page.update(cx, |docs, cx| docs.open(slug, cx));
            }
            Target::Record(id) => {
                self.go(Page::Record, cx);
                self.record_page.update(cx, |page, cx| page.open(id, cx));
            }
            Target::Ask(question) => self
                .finder
                .update(cx, |finder, cx| finder.ask(question, cx)),
            Target::Terminal => self.open_terminal(window, cx),
            Target::Dashboard(path) => self.open_dashboard(path, window, cx),
        }
    }

    /// Open a terminal in the right edge: the newest one still running, or a new login shell.
    ///
    /// The seat is opened first, so a Mac that is slow to answer shows the frame it will fill
    /// rather than nothing. The view is built once and kept: closing the drawer leaves the stream
    /// attached, and reopening it draws the live screen instead of replaying a scrollback the Mac
    /// may since have dropped.
    fn open_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.right_panel.open_on(Seat::Terminal);
        cx.notify();
        if let Some(terminal) = &self.terminal {
            if terminal.read(cx).peer().alive {
                terminal.update(cx, |view, cx| view.resize(TERMINAL_COLS, TERMINAL_ROWS, cx));
                return;
            }
        }
        let palette = Palette::for_appearance(window.appearance());
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let host = Host::Shell(shell);
        let opener = host.clone();
        let work = shell::run(async move {
            opener
                .attach_or_open(TERMINAL_COLS as u16, TERMINAL_ROWS as u16)
                .await
        });
        cx.spawn(async move |weak, cx| {
            let peer = match work.await {
                Some(Ok(peer)) => peer,
                failed => {
                    let why = failed.and_then(Result::err).unwrap_or_default();
                    let _ = weak.update(cx, |workspace, cx| {
                        workspace.terminal_note = Some(
                            format!(
                                "The terminal did not open. Check that the agent capability is running. {why}"
                            )
                            .trim_end()
                            .to_owned()
                            .into(),
                        );
                        cx.notify();
                    });
                    return;
                }
            };
            let _ = weak.update(cx, |workspace, cx| {
                let entity = cx.new(|cx| {
                    TerminalView::new(peer, host, 0, TERMINAL_COLS, TERMINAL_ROWS, palette, cx)
                });
                entity.update(cx, |view, cx| view.attach(cx));
                workspace.terminal = Some(entity);
                cx.notify();
            });
        })
        .detach();
    }

    fn open_palette(&mut self, _: &OpenPalette, _: &mut Window, cx: &mut Context<Self>) {
        self.finder.update(cx, |finder, cx| finder.toggle(cx));
    }

    /// What a finder result asks for. The finder only names a place; going there is the
    /// workspace's, the same as for a sidebar row or a deep link.
    fn on_finder(
        &mut self,
        _: &Entity<Finder>,
        event: &FinderEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            FinderEvent::AgentStarted => {
                self.right_panel.open_on(Seat::Agent);
                cx.notify();
            }
            FinderEvent::Closed => {
                self.refocus = true;
                cx.notify();
            }
            FinderEvent::Open(open) => match open.clone() {
                Open::Page(Page::Settings) => self.go(Page::Settings, cx),
                Open::Page(page) => self.go(page, cx),
                Open::Action(action) => window.dispatch_action(action(), cx),
                Open::Dashboard(href) => self.open_dashboard(href.to_owned(), window, cx),
                Open::Project(name) => self.go_to_project(name, cx),
                Open::Link(link) => {
                    if let Some(target) = route::parse_deep_link(&link) {
                        self.open_target(target, window, cx);
                    }
                }
                Open::Record { project, path } => {
                    self.go(Page::Projects, cx);
                    self.projects.update(cx, |projects, cx| {
                        projects.open_record(project, path, window, cx)
                    });
                }
            },
        }
    }

    fn on_inbox(
        &mut self,
        _: &Entity<Inbox>,
        event: &InboxEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InboxEvent::OpenYtAlbum(url) => {
                let repo = self.repo.clone();
                let page = cx.new(|cx| YtAlbum::new(repo, url, window, cx));
                let events = cx.subscribe_in(&page, window, Self::on_ytalbum);
                page.update(cx, |view, cx| view.probe(window, cx));
                self.ytalbum_page = Some((page, events));
                cx.notify();
            }
        }
    }

    fn on_ytalbum(
        &mut self,
        _: &Entity<YtAlbum>,
        event: &YtAlbumEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            YtAlbumEvent::Close => {
                self.ytalbum_page = None;
                cx.notify();
            }
        }
    }

    fn on_go_to(&mut self, GoTo(page): &GoTo, _: &mut Window, cx: &mut Context<Self>) {
        self.go(*page, cx);
    }

    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh_current_page(cx);
    }

    pub fn refresh_current_page(&mut self, cx: &mut Context<Self>) {
        self.refresh(cx);
        if self.page == Page::Projects {
            self.projects
                .update(cx, |projects, cx| projects.load_projects(cx));
        } else if self.page == Page::Travel {
            self.travel.update(cx, |travel, cx| travel.refresh(cx));
        } else if self.page == Page::Documents {
            self.documents
                .update(cx, |documents, cx| documents.load(cx));
        } else if self.page == Page::Feed {
            self.feed.update(cx, |feed, cx| feed.refresh(cx));
        } else if self.page == Page::Inbox {
            self.inbox.update(cx, |inbox, cx| inbox.load(cx));
        } else if self.page == Page::Calendar {
            self.calendar_page
                .update(cx, |calendar, cx| calendar.refresh(cx));
        } else if matches!(self.page, Page::Home | Page::Context) {
            self.context_page
                .update(cx, |context, cx| context.refresh(cx));
        } else if self.page == Page::Finance {
            self.finance_page
                .update(cx, |finance, cx| finance.refresh(cx));
        } else if self.page == Page::People {
            self.people_page.update(cx, |people, cx| people.refresh(cx));
        } else if self.page == Page::Backup {
            self.backup_page.update(cx, |b, cx| b.refresh(cx));
        } else if self.page == Page::Inventory {
            self.inventory_page.update(cx, |p, cx| p.refresh(cx));
        } else if self.page == Page::Systems {
            self.systems.update(cx, |p, cx| p.refresh(cx));
        } else if self.page == Page::Packs {
            self.packs.update(cx, |p, cx| p.refresh(cx));
        } else if self.page == Page::SelfModel {
            self.self_page.update(cx, |p, cx| p.refresh(cx));
        } else if self.page == Page::Cv {
            self.cv_page.update(cx, |p, cx| p.refresh(cx));
        } else if self.page == Page::Media {
            self.media_page.update(cx, |p, cx| p.refresh(cx));
        } else if self.page == Page::Research {
            self.research_page.update(cx, |p, cx| p.refresh(cx));
        } else if self.page == Page::Setup {
            self.setup_page.update(cx, |p, cx| p.refresh(cx));
        }
    }

    fn on_open_in_dashboard(
        &mut self,
        _: &OpenInDashboard,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_dashboard(self.page.href().to_owned(), window, cx);
    }

    fn on_open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(None, cx);
    }

    fn on_open_appearance(&mut self, _: &OpenAppearance, _: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(Some(crate::settings::Section::Appearance), cx);
    }

    /// Open the panel, or leave it where it is when it is already showing.
    fn on_open_soundscape(
        &mut self,
        _: &OpenSoundscape,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_soundscape(window, cx);
    }

    fn on_toggle_soundscape_panel(
        &mut self,
        _: &ToggleSoundscapePanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Nothing to toggle when the panel has never been built: build it, open.
        match &self.soundscape_panel {
            Some(panel) => {
                panel.update(cx, |panel, cx| panel.toggle_open(cx));
            }
            None => self.open_soundscape(window, cx),
        }
    }

    /// Opens Settings, at `section` when one is given and at the last one shown otherwise.
    pub fn open_settings(
        &mut self,
        section: Option<crate::settings::Section>,
        cx: &mut Context<Self>,
    ) {
        if let Some(section) = section {
            self.settings_section = section;
            if section == crate::settings::Section::Shortcuts {
                self.settings_shortcut_group = None;
            }
        }
        if section == Some(crate::settings::Section::Updates) && self.update_info.is_none() {
            self.check_updates(cx);
        }
        if section == Some(crate::settings::Section::Accounts) {
            self.accounts
                .update(cx, |accounts, cx| accounts.refresh(cx));
        }
        if self.available_features.first_load() || self.available_features.age_of_value() > 600 {
            self.refresh_feature_catalogue(cx);
        }
        if section.unwrap_or(self.settings_section) == crate::settings::Section::Features {
            self.refresh_installed_features(cx);
        }
        self.go(Page::Settings, cx);
    }

    pub fn refresh_installed_features(&mut self, cx: &mut Context<Self>) {
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.installed_features.settle(Err(error));
                cx.notify();
                return;
            }
        };
        if !self.installed_features.begin() {
            return;
        }
        cx.notify();
        let work = shell::run(async move { shell.installed_features().await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.installed_features.settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn refresh_feature_catalogue(&mut self, cx: &mut Context<Self>) {
        if !self.available_features.begin() {
            return;
        }
        cx.notify();
        let work = shell::run(async { sjel_gpui_core::shell::feature_catalogue().await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.available_features.settle(result);
                let catalogue_has_match =
                    view.available_features
                        .value
                        .as_ref()
                        .is_some_and(|packages| {
                            crate::settings::catalogue_matches(packages, &view.settings_query)
                        });
                if let Some(section) =
                    crate::settings::search_section(&view.settings_query, catalogue_has_match)
                {
                    view.settings_section = section;
                    if section == crate::settings::Section::Features {
                        if let Some(feature_view) =
                            crate::settings::feature_view_for_search(&view.settings_query)
                        {
                            view.settings_feature_view = feature_view;
                        } else if catalogue_has_match {
                            view.settings_feature_view = crate::settings::FeatureView::Explore;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn request_feature_uninstall(&mut self, name: String, cx: &mut Context<Self>) {
        self.feature_uninstall_confirmation = Some(name);
        self.feature_manager_message = None;
        cx.notify();
    }

    pub fn dismiss_feature_recommendation(
        &mut self,
        feature: String,
        topic: String,
        cx: &mut Context<Self>,
    ) {
        let key = format!("{feature}::{topic}");
        self.dismissed_feature_recommendations.insert(key);
        self.feature_manager_message = None;
        if let Err(error) =
            crate::settings::save_recommendation_dismissals(&self.dismissed_feature_recommendations)
        {
            self.feature_manager_message = Some(format!("Could not save this preference: {error}"));
        }
        cx.notify();
    }

    pub fn request_feature_install(&mut self, name: String, cx: &mut Context<Self>) {
        self.feature_install_confirmation = Some(name);
        self.feature_manager_message = None;
        cx.notify();
    }

    pub fn cancel_feature_install(&mut self, cx: &mut Context<Self>) {
        self.feature_install_confirmation = None;
        cx.notify();
    }

    pub fn install_feature(
        &mut self,
        name: String,
        version: String,
        sha256: String,
        cx: &mut Context<Self>,
    ) {
        let key = format!("package-install:{name}");
        if !self.busy.start(&key) {
            return;
        }
        self.feature_install_confirmation = None;
        self.feature_manager_message = None;
        cx.notify();
        let work = shell::run(async move {
            sjel_gpui_core::shell::install_feature(&name, &version, &sha256).await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.busy.finish(&key);
                match result {
                    Ok(()) => {
                        view.feature_manager_message = Some(
                            "Feature installed. It is ready to enable in Capabilities.".into(),
                        );
                        view.installed_features = Remote::default();
                        view.refresh_installed_features(cx);
                        view.available_features = Remote::default();
                        view.refresh_feature_catalogue(cx);
                    }
                    Err(error) => view.feature_manager_message = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn cancel_feature_uninstall(&mut self, cx: &mut Context<Self>) {
        self.feature_uninstall_confirmation = None;
        cx.notify();
    }

    pub fn uninstall_feature(&mut self, name: String, cx: &mut Context<Self>) {
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.feature_manager_message = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let key = format!("package-uninstall:{name}");
        if !self.busy.start(&key) {
            return;
        }
        self.feature_uninstall_confirmation = None;
        self.feature_manager_message = None;
        cx.notify();
        let work = shell::run(async move { shell.uninstall_feature(&name).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.busy.finish(&key);
                match result {
                    Ok(()) => {
                        view.feature_manager_message =
                            Some("Feature removed. Its capability data was retained.".into());
                        view.installed_features = Remote::default();
                        view.refresh_installed_features(cx);
                    }
                    Err(error) => view.feature_manager_message = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Saves `next` and applies it. A theme mode is forced on the window, so the native glass and
    /// gpui-component's widgets turn with the palette; the window's appearance observer then
    /// syncs the rest.
    pub fn set_appearance(&mut self, next: AppearanceSettings, cx: &mut Context<Self>) {
        if next.theme_mode != self.appearance_settings.theme_mode {
            cx.set_window_appearance(next.theme_mode.forced_appearance());
        }
        self.appearance_settings = next;
        if let Err(error) = next.save() {
            eprintln!("sjel-gpui: appearance not saved: {error}");
        }
        self.sync_kit_accent(cx);
        cx.notify();
    }

    /// gpui-component paints primary buttons, switches and focus rings in its own blue. This
    /// gives them the accent the palette uses.
    fn sync_kit_accent(&self, cx: &mut App) {
        let dark = Theme::global(cx).is_dark();
        let accent: gpui_kit::Hsla = rgb(self.appearance_settings.accent.color_rgb(dark)).into();
        Theme::update(cx, |theme| {
            theme.colors.primary = accent;
            theme.colors.primary_hover = accent.opacity(0.9);
            theme.colors.primary_active = accent.opacity(0.8);
            theme.colors.ring = accent;
            theme.colors.button_primary = accent;
            theme.colors.button_primary_hover = accent.opacity(0.9);
            theme.colors.button_primary_active = accent.opacity(0.8);
            theme.colors.button_primary_foreground = gpui_kit::white();
        });
    }

    fn on_check_for_updates(
        &mut self,
        _: &CheckForUpdates,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.push_notification(Notification::info("Checking for Sjel updates..."), cx);
        cx.spawn(async move |this, cx| {
            let info = cx
                .background_executor()
                .spawn(async move {
                    crate::update::find_repo_root()
                        .and_then(|r| crate::update::check_git_updates(&r))
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.update_info = info.clone();
                view.open_settings(Some(crate::settings::Section::Updates), cx);
            });
        })
        .detach();
    }

    fn on_rebuild_app(&mut self, _: &RebuildApp, window: &mut Window, cx: &mut Context<Self>) {
        self.trigger_rebuild(window, cx);
    }

    fn on_select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Documents {
            self.documents
                .update(cx, |docs, cx| docs.select_relative(1, cx));
        } else {
            self.selection.step(1, self.items().len());
        }
        cx.notify();
    }

    fn on_select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Documents {
            self.documents
                .update(cx, |docs, cx| docs.select_relative(-1, cx));
        } else {
            self.selection.step(-1, self.items().len());
        }
        cx.notify();
    }

    fn on_doc_prev_page(&mut self, _: &DocPrevPage, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Documents {
            self.documents
                .update(cx, |docs, cx| docs.go_to_page(-1, cx));
        }
    }

    fn on_doc_next_page(&mut self, _: &DocNextPage, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Documents {
            self.documents.update(cx, |docs, cx| docs.go_to_page(1, cx));
        }
    }

    fn on_doc_toggle_zoom(&mut self, _: &DocToggleZoom, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Documents {
            self.documents.update(cx, |docs, cx| docs.toggle_zoom(cx));
        }
    }

    fn on_next_page(&mut self, _: &NextPage, _: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(1, cx);
    }

    fn on_prev_page(&mut self, _: &PrevPage, _: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(-1, cx);
    }

    /// Steps through the open area's tabs. Settings has no tabs, so it stays.
    fn step_tab(&mut self, step: isize, cx: &mut Context<Self>) {
        if let Some(area) = Area::of(self.page) {
            self.go(area.step_tab(self.page, step), cx);
        }
    }

    fn on_next_area(&mut self, _: &NextArea, _: &mut Window, cx: &mut Context<Self>) {
        self.step_area(1, cx);
    }

    fn on_prev_area(&mut self, _: &PrevArea, _: &mut Window, cx: &mut Context<Self>) {
        self.step_area(-1, cx);
    }

    /// Steps through the sidebar. From Settings, down opens the first area and up the last.
    fn step_area(&mut self, step: isize, cx: &mut Context<Self>) {
        let next = match Area::of(self.page) {
            Some(area) => area.step(step),
            None if step > 0 => Area::ALL[0],
            None => Area::ALL[Area::ALL.len() - 1],
        };
        self.open_area(next, cx);
    }

    /// Opens `area` at the tab it showed last.
    pub fn open_area(&mut self, area: Area, cx: &mut Context<Self>) {
        self.go(self.tabs.current(area), cx);
    }

    fn on_open_area(&mut self, OpenArea(area): &OpenArea, _: &mut Window, cx: &mut Context<Self>) {
        self.open_area(*area, cx);
    }

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        match self.page {
            Page::Documents => {
                self.documents
                    .update(cx, |docs, cx| docs.focus_search(window, cx));
            }
            Page::Docs => {
                self.docs_page
                    .update(cx, |docs, cx| docs.focus_search(window, cx));
            }
            Page::Finance => {
                self.finance_page
                    .update(cx, |fin, cx| fin.focus_search(window, cx));
            }
            Page::People => {
                self.people_page
                    .update(cx, |people, cx| people.focus_search(window, cx));
            }
            Page::Inventory => {
                self.inventory_page
                    .update(cx, |inv, cx| inv.focus_search(window, cx));
            }
            _ => {
                self.open_palette(&OpenPalette, window, cx);
            }
        }
    }

    fn on_toggle_soundscape(
        &mut self,
        _: &ToggleSoundscape,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let playing = self
            .soundscape
            .value
            .as_ref()
            .map(|s| s.playing)
            .unwrap_or(false);
        self.toggle_soundscape(!playing, window, cx);
    }

    fn on_cycle_soundscape_preset(
        &mut self,
        _: &CycleSoundscapePreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_soundscape_preset(window, cx);
    }

    fn on_clear_selection(&mut self, _: &ClearSelection, _: &mut Window, cx: &mut Context<Self>) {
        // Escape is bound here for the whole window, and a binding is matched before any key
        // listener runs, so this is where escape reaches an open finder.
        if self.finder.read(cx).is_open() {
            self.finder.update(cx, |finder, cx| finder.close(cx));
            return;
        }
        self.selection.clear();
        if self.page == Page::Documents {
            self.documents.update(cx, |docs, cx| docs.close(cx));
        }
        cx.notify();
    }

    fn on_run_primary(&mut self, _: &RunPrimary, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected() {
            Some(Item::Approval(id)) => self.decide(id, true, window, cx),
            Some(Item::Down(name)) => self.set_running(name, true, window, cx),
            Some(Item::FocusTask(id)) => {
                let current_done = self
                    .focus_tasks
                    .value
                    .as_ref()
                    .and_then(|tasks| tasks.iter().find(|t| t.id == id))
                    .map(|t| t.done)
                    .unwrap_or(false);
                self.toggle_task(id, !current_done, window, cx);
            }
            Some(Item::Capability { name, up: false }) => self.set_running(name, true, window, cx),
            Some(Item::Capability { up: true, .. }) | None => {}
        }
    }

    fn on_run_secondary(&mut self, _: &RunSecondary, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected() {
            Some(Item::Approval(id)) => self.decide(id, false, window, cx),
            Some(Item::Capability { name, up: true }) => self.set_running(name, false, window, cx),
            _ => {}
        }
    }

    /// Liquid Glass under the sidebar, when this macOS has it and `SJEL_GPUI_NO_GLASS` is unset.
    /// AppKit then
    /// paints the tint under the sidebar, and this returns true so GPUI tints only the page.
    #[cfg(target_os = "macos")]
    fn attach_glass(&mut self, window: &mut Window, tint: u32, enabled: bool) -> bool {
        if !enabled {
            if let Some(glass) = &mut self.native_glass {
                glass.set_tint(tint);
            }
            return false;
        }
        if !self.native_glass_attempted && std::env::var_os("SJEL_GPUI_NO_GLASS").is_none() {
            self.native_glass_attempted = true;
            self.native_glass = crate::macos_glass::NativeGlassRegion::attach(
                window,
                sidebar::WIDTH.into(),
                sidebar::GLASS_INSET.into(),
                16.0,
            )
            .ok();
            if self.native_glass.is_some() {
                // Inside the panel's rounded top-left corner, not on its edge.
                window.set_traffic_light_position(gpui_kit::point(px(20.), px(20.)));
            }
        }
        match &mut self.native_glass {
            Some(glass) => {
                glass.set_tint(tint);
                true
            }
            None => false,
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn attach_glass(&mut self, _: &mut Window, _: u32, _: bool) -> bool {
        false
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(frames) = &self.frames {
            frames.render_started();
        }
        let is_dark = self.appearance_settings.theme_mode.is_dark(matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ));
        let base_palette = if is_dark {
            Palette::dark()
        } else {
            Palette::light()
        };
        let palette = self
            .appearance_settings
            .apply_to_palette(base_palette, is_dark);
        let tint = palette.window_tint;
        let sidebar_tint = palette.sidebar_surface;
        let glass_requested = self.appearance_settings.glass == GlassStyle::LiquidGlass;
        let glass = self.attach_glass(window, sidebar_tint, glass_requested);
        let right_panel = if self.right_panel.is_open() {
            let seat = self.right_panel.seat();
            let body = match seat {
                Seat::Agent => self
                    .finder
                    .update(cx, |finder, cx| finder.agent_panel_body(palette, cx)),
                Seat::Soundscape => div()
                    .p_4()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("Soundscape controls remain available with ⌘0")
                    .into_any_element(),
                Seat::Activity => div()
                    .p_4()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("Phone activity is shown on the paired iPhone.")
                    .into_any_element(),
                Seat::Terminal => match &self.terminal {
                    Some(terminal) => terminal.clone().into_any_element(),
                    None => div()
                        .p_4()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child(self.terminal_note.clone().unwrap_or_else(|| {
                            SharedString::from(
                                "A terminal opens here from ⌘K › Terminal or sjel://terminal.",
                            )
                        }))
                        .into_any_element(),
                },
            };
            let entity = cx.entity().clone();
            let on_seat = {
                let entity = entity.clone();
                move |seat: Seat, _: &mut Window, app: &mut App| {
                    entity.update(app, |view, cx| {
                        view.right_panel.choose(seat);
                        cx.notify();
                    });
                }
            };
            let on_close = move |_: &mut Window, app: &mut App| {
                entity.update(app, |view, cx| {
                    view.right_panel.close();
                    cx.notify();
                });
            };
            Some(
                div()
                    .w(px(390.))
                    .flex_none()
                    .h_full()
                    .border_l_1()
                    .border_color(rgba(palette.border))
                    .child(sidebar_frame(palette, seat, body, on_seat, on_close)),
            )
        } else {
            None
        };
        // An open finder holds the keys: a page change behind it must not take them back, or the
        // finder opened right after a navigation could not be closed with escape.
        if std::mem::take(&mut self.refocus) && !self.finder.read(cx).is_open() {
            window.focus(&self.focus, cx);
        }
        let page = match self.page {
            Page::Home => home::render(self, palette, cx).into_any_element(),
            Page::Capabilities => capabilities::render(self, palette, cx).into_any_element(),
            Page::Backup => {
                self.backup_page
                    .update(cx, |backup, cx| backup.set_palette(palette, cx));
                self.backup_page.clone().into_any_element()
            }
            Page::Software => self.software.clone().into_any_element(),
            Page::Apps => self.apps_page.clone().into_any_element(),
            Page::Docs => self.docs_page.clone().into_any_element(),
            Page::Record => self.record_page.clone().into_any_element(),
            Page::Import => self.import.clone().into_any_element(),
            Page::Systems => self.systems.clone().into_any_element(),
            Page::Packs => self.packs.clone().into_any_element(),
            Page::SelfModel => self.self_page.clone().into_any_element(),
            Page::Cv => self.cv_page.clone().into_any_element(),
            Page::Media => self.media_page.clone().into_any_element(),
            Page::Research => self.research_page.clone().into_any_element(),
            Page::Setup => self.setup_page.clone().into_any_element(),
            Page::Documents => {
                self.documents
                    .update(cx, |documents, cx| documents.set_palette(palette, cx));
                self.documents.clone().into_any_element()
            }
            Page::Feed => {
                self.feed
                    .update(cx, |feed, cx| feed.set_palette(palette, cx));
                self.feed.clone().into_any_element()
            }
            Page::Inbox => {
                if let Some((ytalbum, _)) = &self.ytalbum_page {
                    ytalbum.update(cx, |view, cx| view.set_palette(palette, cx));
                    ytalbum.clone().into_any_element()
                } else {
                    self.inbox
                        .update(cx, |inbox, cx| inbox.set_palette(palette, cx));
                    self.inbox.clone().into_any_element()
                }
            }
            Page::Calendar => {
                self.calendar_page
                    .update(cx, |calendar, cx| calendar.set_palette(palette, cx));
                self.calendar_page.clone().into_any_element()
            }
            Page::Context => unreachable!("`/context` opens Home (`core::area::canonical`)"),
            Page::Finance => {
                self.finance_page
                    .update(cx, |finance, cx| finance.set_palette(palette, cx));
                self.finance_page.clone().into_any_element()
            }
            Page::People => {
                self.people_page
                    .update(cx, |people, cx| people.set_palette(palette, cx));
                self.people_page.clone().into_any_element()
            }
            Page::Inventory => {
                self.inventory_page
                    .update(cx, |p, cx| p.set_palette(palette, cx));
                self.inventory_page.clone().into_any_element()
            }
            Page::Projects => self.projects.clone().into_any_element(),
            Page::Travel => {
                self.travel.update(cx, |travel, cx| {
                    travel.set_palette(palette, cx);
                    travel.set_compact_map(false);
                });
                self.travel.clone().into_any_element()
            }
            Page::Settings => crate::settings::render(self, palette, window, cx).into_any_element(),
        };

        if let Some(frames) = &self.frames {
            frames.render_finished();
        }
        let scroll = if self.page == Page::Travel {
            self.travel.read(cx).scroll_handle()
        } else {
            self.selection.scroll.clone()
        };
        // Actions sit on the outer element, so the palette (a child of it) can dispatch them.
        // Focus and the "Workspace" key context sit on the inner frame only, so J and K never
        // fire while a palette query has focus.
        div()
            .id("workspace")
            .when_some(self.frames.clone(), |root, frames| {
                root.on_scroll_wheel(move |_, _, _| frames.input())
            })
            .on_action(cx.listener(Self::open_palette))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_next_page))
            .on_action(cx.listener(Self::on_prev_page))
            .on_action(cx.listener(Self::on_go_to))
            .on_action(cx.listener(Self::on_open_area))
            .on_action(cx.listener(Self::on_next_area))
            .on_action(cx.listener(Self::on_prev_area))
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(Self::on_open_in_dashboard))
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_select_previous))
            .on_action(cx.listener(Self::on_clear_selection))
            .on_action(cx.listener(Self::on_doc_prev_page))
            .on_action(cx.listener(Self::on_doc_next_page))
            .on_action(cx.listener(Self::on_doc_toggle_zoom))
            .on_action(cx.listener(Self::on_toggle_soundscape))
            .on_action(cx.listener(Self::on_cycle_soundscape_preset))
            .on_action(cx.listener(Self::on_run_primary))
            .on_action(cx.listener(Self::on_run_secondary))
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(Self::on_open_appearance))
            .on_action(cx.listener(Self::on_open_soundscape))
            .on_action(cx.listener(Self::on_toggle_soundscape_panel))
            .on_action(cx.listener(Self::on_check_for_updates))
            .on_action(cx.listener(Self::on_rebuild_app))
            .size_full()
            .when(!glass, |root| root.bg(rgba(tint)))
            .text_color(rgb(palette.text))
            .child(
                div()
                    .id("frame")
                    .track_focus(&self.focus)
                    // These pages type into their fields, so J, K and Enter must reach them.
                    .when(
                        !matches!(
                            self.page,
                            Page::Projects
                                | Page::Travel
                                | Page::Feed
                                | Page::Inbox
                                | Page::Calendar
                                | Page::Software
                                | Page::Settings
                        ),
                        |frame| frame.key_context("Workspace"),
                    )
                    .size_full()
                    .flex()
                    .child(
                        div()
                            .w(px(sidebar::WIDTH))
                            .flex_none()
                            .h_full()
                            .when(glass, |column| column.p(px(sidebar::GLASS_INSET)))
                            .child(sidebar::render(self, palette, glass, window, cx)),
                    )
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            // The page column may not be wider than the window: Documents draws a
                            // table whose min-content width would otherwise push the column out.
                            .min_w_0()
                            // Glass on purpose; the low label contrast on it is accepted.
                            .h_full()
                            .when(glass, |page| page.bg(rgba(tint)))
                            .flex()
                            .flex_col()
                            .child(header::render(self, palette, cx))
                            .child(
                                if matches!(
                                    self.page,
                                    Page::Projects
                                        | Page::Settings
                                        | Page::Documents
                                        | Page::Calendar
                                        | Page::Finance
                                        | Page::People
                                        | Page::Backup
                                        | Page::Inventory
                                        | Page::Packs
                                ) {
                                    // Projects, Settings, Documents, Calendar, Finance, People,
                                    // Pages that scroll themselves get the full width.
                                    div().id("page").flex_1().min_h_0().px_8().pb_8().child(
                                        div()
                                            .size_full()
                                            .when(
                                                !matches!(
                                                    self.page,
                                                    Page::Documents
                                                        | Page::Calendar
                                                        | Page::Finance
                                                        | Page::People
                                                        | Page::Backup
                                                        | Page::Inventory
                                                        | Page::Packs
                                                ),
                                                |el| el.max_w(px(880.)),
                                            )
                                            .child(page),
                                    )
                                } else {
                                    div()
                                        .id("page")
                                        .flex_1()
                                        .min_h_0()
                                        .overflow_y_scroll()
                                        .track_scroll(&scroll)
                                        .vertical_scrollbar(&scroll)
                                        .child(div().px_8().pb_8().max_w(px(880.)).child(page))
                                },
                            )
                            // The soundscape panel slides over the page and not the navigation: a
                            // player that covers the way out of the page is a window again.
                            .children(self.soundscape_panel.clone()),
                    )
                    .children(right_panel),
            )
            // Over the whole window, navigation included: it is where you go next from.
            .child(self.finder.clone())
            .children(self.frames.as_ref().map(|frames| frames.paint_marker()))
    }
}

/// `SJEL_GPUI_OPAQUE=1` draws an opaque window instead of the blurred one, for comparison.
pub fn opaque_window() -> bool {
    std::env::var_os("SJEL_GPUI_OPAQUE").is_some()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{Entity, TestAppContext, VisualTestContext};
    use gpui_kit::component::Root;

    use super::*;

    /// The real window shape: `Root` around the workspace.
    ///
    /// Building a `Workspace` starts the pages' own reads against `http://example.invalid`. They
    /// fail at once, but the failure arrives on the HTTP runtime's thread, which the deterministic
    /// test scheduler reports as foreign activity. `allow_parking` is the scheduler's own switch for
    /// a test that mixes GPUI's executor with real I/O.
    fn open(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_kit::init(cx);
            bind_keys(cx);
        });
        let slot = Rc::new(RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| Workspace::new("http://example.invalid", window, cx));
            found.replace(Some(view.clone()));
            Root::new(view, window, cx)
        });
        let view = slot.take().expect("workspace built");
        view.update(cx, |view, cx| {
            // Through `go`, because that is the only way a page is opened: it hands the frame the
            // focus the page it replaced was holding.
            view.go(Page::Capabilities, cx);
            view.health.value = Some(
                serde_json::from_str(
                    r#"{"version":"t","uptime_seconds":1,"capabilities":{
                        "a":{"up":true},"b":{"up":false,"autostart":false},"c":{"up":true}}}"#,
                )
                .unwrap(),
            );
            cx.notify();
        });
        (view, cx)
    }

    #[gpui::test]
    fn j_and_k_move_the_selection_and_escape_clears_it(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        cx.simulate_keystrokes("j j j j k");
        view.read_with(cx, |view, _| assert_eq!(view.selection.cursor, Some(1)));
        cx.simulate_keystrokes("escape");
        view.read_with(cx, |view, _| assert_eq!(view.selection.cursor, None));
    }

    #[gpui::test]
    fn enter_on_a_stopped_capability_starts_it(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        cx.simulate_keystrokes("j j");
        view.read_with(cx, |view, _| {
            assert_eq!(
                view.selected(),
                Some(Item::Capability {
                    name: "b".into(),
                    up: false
                })
            );
        });
        // The test endpoint is not on loopback, so the start fails at once and says why.
        cx.simulate_keystrokes("enter");
        assert_eq!(cx.update(|window, cx| window.notifications(cx).len()), 1);
    }

    /// The player is a panel over this window now, so the keys and the sidebar row both have to
    /// reach it through the workspace. This is the wiring test: opening builds it once, the
    /// keystroke toggles the same view, and a second open does not build a second player on the same
    /// session.
    #[gpui::test]
    fn the_soundscape_panel_opens_once_and_toggles_from_the_keyboard(cx: &mut TestAppContext) {
        // Opening the panel starts the audio global, and a test must not take the machine's output
        // device. This is the same switch a headless run uses.
        std::env::set_var("SJEL_GPUI_NO_AUDIO", "1");
        let (view, cx) = open(cx);
        assert!(
            view.read_with(cx, |view, _| view.soundscape_panel.is_none()),
            "a fresh window pays for no panel"
        );
        cx.simulate_keystrokes("shift-cmd-m");
        let panel = view.read_with(cx, |view, _| {
            view.soundscape_panel.clone().expect("the panel was built")
        });
        let opened = cx.update(|_, cx| panel.read(cx).is_open());
        assert!(opened, "the keystroke did not open the panel");

        // The same keystroke closes the same view, rather than building a second one.
        cx.simulate_keystrokes("shift-cmd-m");
        let again = view.read_with(cx, |view, _| {
            view.soundscape_panel.clone().expect("the panel stays")
        });
        assert_eq!(
            panel.entity_id(),
            again.entity_id(),
            "a second panel was built"
        );
        let closed = cx.update(|_, cx| again.read(cx).is_open());
        assert!(!closed, "the second press did not close it");

        // ⌘0 is the same toggle, one key from the nine areas.
        cx.simulate_keystrokes("cmd-0");
        assert!(
            cx.update(|_, cx| again.read(cx).is_open()),
            "⌘0 did not open it"
        );
    }

    /// `sjel://terminal` opens the edge on its own seat. The test endpoint is not loopback, so no
    /// shell can be opened here — which is the second half of the assertion: a Mac that cannot
    /// answer leaves the seat saying so rather than a grid that never fills.
    #[gpui::test]
    fn a_terminal_link_opens_the_edge_on_the_terminal_seat(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        view.update_in(cx, |view, window, cx| {
            view.open_target(Target::Terminal, window, cx);
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(view.right_panel.is_open(), "the edge stayed closed");
            assert_eq!(view.right_panel.seat(), Seat::Terminal);
            assert!(
                view.terminal.is_none(),
                "a terminal was built without a capability to hold it"
            );
        });
    }

    #[gpui::test]
    fn typing_in_the_palette_does_not_move_the_selection(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        cx.simulate_keystrokes("cmd-k j k j");
        view.read_with(cx, |view, _| assert_eq!(view.selection.cursor, None));
    }

    /// The edit field sits inside the capture list, so the letters the list binds must type into
    /// it instead of moving the cursor or marking the capture done.
    #[gpui::test]
    fn typing_in_a_capture_edit_does_not_triage(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        cx.update(|_, cx| crate::inbox::bind_keys(cx));
        let inbox = view.update(cx, |view, cx| {
            view.go(Page::Inbox, cx);
            view.inbox.clone()
        });
        cx.run_until_parked();
        inbox.update(cx, |inbox, cx| {
            let capture = |id: &str| sjel_gpui_core::capture::Capture {
                id: id.into(),
                text: Some("shirt".into()),
                url: None,
                tag: None,
                status: "new".into(),
                routed_to: None,
                client: None,
                created_at: "2026-10-24".into(),
            };
            inbox.rows.value = Some(vec![capture("a"), capture("b")]);
            cx.notify();
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("j");
        inbox.read_with(cx, |inbox, _| {
            assert_eq!(inbox.cursor.as_deref(), Some("a"))
        });
        cx.simulate_keystrokes("enter");
        cx.simulate_input(" jed");
        inbox.read_with(cx, |inbox, cx| {
            assert_eq!(inbox.cursor.as_deref(), Some("a"));
            let (id, field, _) = inbox.editing.as_ref().expect("the text is open");
            assert_eq!(id, "a");
            assert_eq!(field.read(cx).value(), "shirt jed");
        });
    }

    /// A question in Auto goes to the one Ask session, and the finder stays open to show it; ⌘↩
    /// sends a search-looking text as a question too.
    #[gpui::test]
    fn the_finder_asks_when_the_words_are_a_question(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        cx.simulate_keystrokes("cmd-k");
        let (finder, ask) = view.read_with(cx, |view, _| (view.finder.clone(), view.ask.clone()));
        cx.simulate_input("what is on my calendar today?");
        cx.simulate_keystrokes("enter");
        let prompt = cx.update(|_, cx| ask.read(cx).latest().map(|exchange| exchange.prompt));
        assert_eq!(prompt.as_deref(), Some("what is on my calendar today?"));
        assert!(
            cx.update(|_, cx| finder.read(cx).is_open()),
            "the answer shows in place"
        );

        cx.simulate_input("packs");
        cx.simulate_keystrokes("cmd-enter");
        let prompt = cx.update(|_, cx| ask.read(cx).latest().map(|exchange| exchange.prompt));
        assert_eq!(
            prompt.as_deref(),
            Some("packs"),
            "⌘↩ asked instead of opening Packs"
        );
    }

    /// ⌘K opens the finder; typing narrows it; the arrows move through the results and Enter
    /// goes there; escape closes it and the page keys work again.
    #[gpui::test]
    fn the_finder_searches_opens_and_closes(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        cx.simulate_keystrokes("cmd-k");
        let finder = view.read_with(cx, |view, _| view.finder.clone());
        assert!(cx.update(|_, cx| finder.read(cx).is_open()));
        cx.simulate_keystrokes("escape");
        assert!(
            !cx.update(|_, cx| finder.read(cx).is_open()),
            "escape closes it"
        );
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input("packs");
        let first = cx.update(|_, cx| finder.read(cx).hits().first().map(|hit| hit.title.clone()));
        assert_eq!(first.as_deref(), Some(page_title(Page::Packs).as_str()));
        cx.simulate_keystrokes("enter");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Packs));
        assert!(
            !cx.update(|_, cx| finder.read(cx).is_open()),
            "Enter closes it"
        );

        cx.simulate_keystrokes("cmd-k");
        cx.simulate_input("set");
        // The first arrow picks the top row, the second moves to the next one.
        let second = cx.update(|_, cx| match finder.read(cx).hits()[1].open {
            Open::Page(page) => page,
            _ => panic!("\"set\" ranks pages first"),
        });
        cx.simulate_keystrokes("down down enter");
        view.read_with(cx, |view, _| {
            assert_eq!(view.page, second, "the arrows chose it")
        });

        // The input takes the focus on the frame that draws it, so the keys come one frame apart,
        // as a hand would send them.
        cx.simulate_keystrokes("cmd-k");
        cx.simulate_keystrokes("escape");
        assert!(
            !cx.update(|_, cx| finder.read(cx).is_open()),
            "escape closes it"
        );
    }

    #[gpui::test]
    fn page_shortcuts_switch_and_reset_the_selection(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        cx.simulate_keystrokes("j cmd-1");
        view.read_with(cx, |view, _| {
            assert_eq!(view.page, Page::Home);
            assert_eq!(view.selection.cursor, None);
        });
    }

    #[gpui::test]
    fn the_number_keys_open_the_nine_areas(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        for (area, n) in Area::ALL.into_iter().zip(1..=9) {
            cx.simulate_keystrokes(&format!("cmd-{n}"));
            view.read_with(cx, |view, _| assert_eq!(Area::of(view.page), Some(area)));
        }
    }

    #[gpui::test]
    fn an_area_returns_to_the_tab_it_showed_last(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        cx.simulate_keystrokes("cmd-8");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Capabilities));
        view.update(cx, |view, cx| view.go(Page::Packs, cx));
        cx.simulate_keystrokes("cmd-1");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Home));
        cx.simulate_keystrokes("cmd-8");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Packs));
    }

    #[gpui::test]
    fn home_focus_tasks_keyboard_navigation(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        view.update(cx, |view, cx| {
            view.page = Page::Home;
            view.health.value = None;
            view.focus_tasks.value = Some(vec![
                GlobalTask {
                    id: "Atlas/Projects/Test/Tasks/Task1.md".into(),
                    kind: "task".into(),
                    title: "Focus on LibSQL".into(),
                    priority: 1,
                    due: None,
                    done: false,
                    summary: None,
                    project: "Test".into(),
                },
                GlobalTask {
                    id: "Atlas/Projects/Test/Tasks/Task2.md".into(),
                    kind: "task".into(),
                    title: "Fix bug".into(),
                    priority: 2,
                    due: Some("2020-01-01".into()),
                    done: false,
                    summary: None,
                    project: "Test".into(),
                },
            ]);
            cx.notify();
        });
        cx.simulate_keystrokes("j");
        view.read_with(cx, |view, _| {
            assert_eq!(
                view.selected(),
                Some(Item::FocusTask("Atlas/Projects/Test/Tasks/Task1.md".into()))
            );
        });
        cx.simulate_keystrokes("j");
        view.read_with(cx, |view, _| {
            assert_eq!(
                view.selected(),
                Some(Item::FocusTask("Atlas/Projects/Test/Tasks/Task2.md".into()))
            );
        });
        cx.simulate_keystrokes("k");
        view.read_with(cx, |view, _| {
            assert_eq!(
                view.selected(),
                Some(Item::FocusTask("Atlas/Projects/Test/Tasks/Task1.md".into()))
            );
        });
        // Space runs primary on the selected focus task
        cx.simulate_keystrokes("space");
    }

    #[gpui::test]
    fn every_sidebar_page_has_a_shortcut(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        cx.simulate_keystrokes("shift-cmd-d");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Documents));
        cx.simulate_keystrokes("shift-cmd-b");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Backup));
        cx.simulate_keystrokes("shift-cmd-f");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Finance));
        cx.simulate_keystrokes("shift-cmd-p");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::People));
        cx.simulate_keystrokes("shift-cmd-i");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Inventory));
        cx.simulate_keystrokes("shift-cmd-s");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Software));
        // Import has no ⌘⇧ letter: shift-cmd-u toggles the soundscape. It has `g u`, which
        // needs a page without its own text input.
        cx.simulate_keystrokes("shift-cmd-h");
        cx.simulate_keystrokes("g u");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Import));
        // Systems is the same shape: no ⌘ letter of its own, so `g x`.
        cx.simulate_keystrokes("shift-cmd-h");
        cx.simulate_keystrokes("g x");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Systems));
    }

    #[gpui::test]
    fn tab_keys_step_inside_the_open_area(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        view.update(cx, |view, cx| view.go(Page::Inbox, cx));
        cx.simulate_keystrokes("cmd-alt-right");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Feed));
        cx.simulate_keystrokes("shift-cmd-]");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Inbox));
        cx.simulate_keystrokes("cmd-alt-left");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Feed));
        cx.simulate_keystrokes("ctrl-tab");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Inbox));
        // An area with one tab keeps it: stepping never leaves the area.
        view.update(cx, |view, cx| view.go(Page::Calendar, cx));
        cx.simulate_keystrokes("shift-cmd-]");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Calendar));
        // Life opens on People, and stepping back from it wraps to its last tab.
        cx.simulate_keystrokes("cmd-7");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::People));
        cx.simulate_keystrokes("cmd-alt-left");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Cv));
    }

    #[gpui::test]
    fn the_area_keys_step_down_the_sidebar(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        view.update(cx, |view, cx| view.go(Page::Home, cx));
        cx.simulate_keystrokes("cmd-alt-down");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Inbox));
        cx.simulate_keystrokes("cmd-alt-up");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Home));
        // Up from the first area wraps to the last one.
        cx.simulate_keystrokes("cmd-alt-up");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Capabilities));
    }

    #[gpui::test]
    fn vim_g_chords_switch_pages(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        cx.simulate_keystrokes("g h");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Home));
        cx.simulate_keystrokes("g d");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Documents));
        cx.simulate_keystrokes("g f");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Finance));
        cx.simulate_keystrokes("g b");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Backup));
        cx.simulate_keystrokes("g x");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Systems));
        cx.simulate_keystrokes("g k");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Packs));
        cx.simulate_keystrokes("g l");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::SelfModel));
        cx.simulate_keystrokes("g y");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Cv));
        cx.simulate_keystrokes("g w");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Media));
        cx.simulate_keystrokes("g z");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Research));
        cx.simulate_keystrokes("g 0");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Setup));
    }

    #[gpui::test]
    fn focus_search_opens_on_pages(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        // On Documents
        view.update(cx, |view, cx| view.go(Page::Documents, cx));
        cx.simulate_keystrokes("cmd-f");
        // On Finance
        view.update(cx, |view, cx| view.go(Page::Finance, cx));
        cx.simulate_keystrokes("/");
        // On People
        view.update(cx, |view, cx| view.go(Page::People, cx));
        cx.simulate_keystrokes("/");
        // On Inventory
        view.update(cx, |view, cx| view.go(Page::Inventory, cx));
        cx.simulate_keystrokes("/");
    }
}
