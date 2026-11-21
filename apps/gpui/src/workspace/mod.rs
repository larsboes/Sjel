//! The main window's life cycle and routing: it opens the window, reads and follows the shell,
//! decides which page is current, and owns the command palette, the selection and the toasts.
//!
//! What the window is made of lives beside it: `sidebar` draws the column and `header` the title
//! row, each reading this view and calling back into it.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Theme;
use gpui_kit::{
    actions, div, point, prelude::*, px, rgb, rgba, Action, App, Context, Entity, FocusHandle,
    KeyBinding, MouseButton, MouseMoveEvent, SharedString, Subscription, Window,
};
use sjel_gpui_ui::ely::feedback::Toast;
use sjel_gpui_ui::ely::forms::{InputEvent, TextInput};
use sjel_gpui_ui::ely::primitives::Severity;
use sjel_gpui_ui::feedback::WindowToastExt;

use crate::data::{self, Busy, Remote, Repo};
use crate::finder::{Finder, FinderEvent, Hit, Open};
use crate::route::{self, Page, Target};
use crate::selection::{Item, Selection};
use crate::shell::{
    self, AgentView, AvailableFeature, Backups, GlobalTask, InstalledFeature, NodeHealth,
    ShellError, Snapshot, SoundscapeState,
};
use crate::{header, sidebar};
use sjel_gpui_core::area::{Area, Tabs};
use sjel_gpui_ui::accounts::Accounts;
use sjel_gpui_ui::ask::Ask;
use sjel_gpui_ui::backup::{self, BackupPage};
use sjel_gpui_ui::calendar::page::CalendarPage;
use sjel_gpui_ui::clipper::ClipperSettings;
use sjel_gpui_ui::context::{ContextEvent, ContextPage};
use sjel_gpui_ui::cv::CvPage;
use sjel_gpui_ui::docs::DocsPage;
use sjel_gpui_ui::documents::Documents;
use sjel_gpui_ui::feed::Feed;
use sjel_gpui_ui::finance::FinancePage;
use sjel_gpui_ui::import::ImportPage;
use sjel_gpui_ui::inbox::triage::{Inbox, InboxEvent};
use sjel_gpui_ui::inventory::InventoryPage;
use sjel_gpui_ui::media::MediaPage;
use sjel_gpui_ui::music::{Music, MusicEvent};
use sjel_gpui_ui::packs::PacksPage;
use sjel_gpui_ui::people::page::PeoplePage;
use sjel_gpui_ui::posts::PostsPage;
use sjel_gpui_ui::projects::Projects;
use sjel_gpui_ui::record::RecordPage;
use sjel_gpui_ui::research::ResearchPage;
use sjel_gpui_ui::scratchpad::{ScratchpadEvent, ScratchpadSeat};
use sjel_gpui_ui::self_view::SelfPage;
use sjel_gpui_ui::setup::SetupPage;
use sjel_gpui_ui::sidebar::{Seat, Sidebar};
use sjel_gpui_ui::software::Software;
use sjel_gpui_ui::systems::SystemsPage;
use sjel_gpui_ui::terminal::TerminalDeck;
use sjel_gpui_ui::theme::{
    step_font_size, step_terminal_font_size, AppearanceSettings, GlassStyle, ThemeContext,
    DEFAULT_UI_FONT_SIZE, UI_FONT_SIZES,
};

mod features;
mod panels;
mod routing;

/// Pinch travel per app zoom step (0.1 is a 10% pinch).
const UI_PINCH_STEP: f32 = 0.15;
/// Trackpad scroll travel per app zoom step.
const UI_SCROLL_PIXELS_PER_STEP: f32 = 40.;

actions!(
    sjel,
    [
        /// Search pages and actions.
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
        SelectNext,
        SelectPrevious,
        /// Allow, start, or open the selected row.
        RunPrimary,
        /// Deny or stop the selected row.
        RunSecondary,
        ClearSelection,
        DismissOverlay,
        /// Turn to the previous page of the open document.
        DocPrevPage,
        /// Turn to the next page of the open document.
        DocNextPage,
        /// Toggle 1x / 2x zoom on the open document reader.
        DocToggleZoom,
        /// Zoom in on the active map, document, or terminal.
        ZoomIn,
        /// Zoom out on the active map, document, or terminal.
        ZoomOut,
        /// Make the whole app larger.
        UiZoomIn,
        /// Make the whole app smaller.
        UiZoomOut,
        /// Return the app to its default size.
        UiZoomReset,
        /// Open Settings at the section it showed last.
        OpenSettings,
        /// Toggle or focus the 24-hour quick Scratchpad in the right edge.
        OpenScratchpad,
        /// Trigger immediate sorting of the 24-hour Scratchpad into Sjel.
        SortScratchpadNow,
        /// Open the terminal in the right edge.
        OpenTerminal,
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
        /// Step to the next sub-tab within the current page header.
        NextSubTab,
        /// Step to the previous sub-tab within the current page header.
        PrevSubTab,
    ]
);

/// Open a page. One action serves every page, so a page added to an area in `core::area` gets
/// its palette row and tab from `nav_pages` without another edit.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = sjel, no_json)]
pub struct GoTo(pub Page);

/// The narrowest the right edge can be dragged, and what it always leaves for the page beside it.
const EDGE_MIN_WIDTH: f32 = 320.;
const EDGE_PAGE_MIN_WIDTH: f32 = 480.;

/// The size a new terminal starts at, before the grid measures itself and resizes the pty.
const TERMINAL_COLS: usize = 48;
const TERMINAL_ROWS: usize = 44;

/// Open an area at the tab it showed last. ⌘1–⌘8, the sidebar rows and the Go menu use it.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = sjel, no_json)]
pub struct OpenArea(pub Area);

/// Every page an area holds, in sidebar then tab order. Settings has ⌘, instead, and the old
/// Today page is part of Home.
/// Everything the command palette offered, as finder results: every page with its breadcrumb,
/// and the app's actions.
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
        action(
            "Soundscape",
            "music audio focus ambient lofi player ⌘0",
            || OpenSoundscape.boxed_clone(),
        ),
        action(
            "Scratchpad",
            "quick daily notepad links tasks notes 24h auto-sort ⌘\\",
            || OpenScratchpad.boxed_clone(),
        ),
        action(
            "Sort Scratchpad into Sjel",
            "sort daily scratchpad notes links tasks wishlist places",
            || SortScratchpadNow.boxed_clone(),
        ),
    ];
    // A terminal is a deep link as well as a result, so the same string opens it from either.
    let terminal = std::iter::once(Hit {
        kind: "action",
        title: "Terminal".to_owned(),
        detail: "shell claude codex pi on this Mac".to_owned(),
        open: Open::Link("sjel://terminal".to_owned()),
    });
    pages.chain(actions).chain(terminal).collect()
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

/// Where the page's single-key navigation applies: the page frame, but not a terminal in the right
/// edge, which needs Enter, J, K and every other key for the program it runs.
const PAGE_KEYS: &str = "Workspace && !Terminal";

/// Workspace key bindings. Navigation keys bind to the "Workspace" context, which only the page
/// frame carries, so they stay quiet while the palette's query field has focus.
pub fn bind_keys(cx: &mut App) {
    // What the panel itself carries: space, the next preset, and the way out.
    cx.bind_keys(crate::soundscape::Soundscape::keys());
    cx.bind_keys(Finder::keys());
    cx.bind_keys(sjel_gpui_ui::terminal::keys());
    cx.bind_keys([Finder::mode_key()]);
    cx.bind_keys([
        KeyBinding::new("cmd-k", OpenPalette, None),
        KeyBinding::new("cmd-f", FocusSearch, None),
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-\\", OpenScratchpad, None),
        KeyBinding::new("shift-cmd-j", OpenScratchpad, None),
        // Inside the terminal the same key starts a new one (`ui/src/terminal.rs`).
        KeyBinding::new("cmd-t", OpenTerminal, None),
        KeyBinding::new("cmd-[", ClearSelection, None),
        // A field on the Projects page takes escape itself, to put its old text back.
        KeyBinding::new("escape", DismissOverlay, Some("!Terminal && !(Projects > Input)")),
        // Tabs inside the open area, the way Safari and Xcode step tabs
        KeyBinding::new("cmd-alt-right", NextPage, None),
        KeyBinding::new("shift-cmd-]", NextPage, None),
        KeyBinding::new("ctrl-tab", NextPage, None),
        KeyBinding::new("cmd-alt-left", PrevPage, None),
        KeyBinding::new("shift-cmd-[", PrevPage, None),
        KeyBinding::new("ctrl-shift-tab", PrevPage, None),
        KeyBinding::new("alt-]", NextSubTab, None),
        KeyBinding::new("alt-[", PrevSubTab, None),
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
        KeyBinding::new("/", FocusSearch, Some(PAGE_KEYS)),
        KeyBinding::new("j", SelectNext, Some(PAGE_KEYS)),
        KeyBinding::new("down", SelectNext, Some(PAGE_KEYS)),
        KeyBinding::new("k", SelectPrevious, Some(PAGE_KEYS)),
        KeyBinding::new("up", SelectPrevious, Some(PAGE_KEYS)),
        KeyBinding::new("enter", RunPrimary, Some(PAGE_KEYS)),
        KeyBinding::new("space", RunPrimary, Some(PAGE_KEYS)),
        KeyBinding::new("cmd-backspace", RunSecondary, Some(PAGE_KEYS)),
        KeyBinding::new("[", DocPrevPage, Some(PAGE_KEYS)),
        KeyBinding::new("left", DocPrevPage, Some(PAGE_KEYS)),
        KeyBinding::new("pageup", DocPrevPage, Some(PAGE_KEYS)),
        KeyBinding::new("]", DocNextPage, Some(PAGE_KEYS)),
        KeyBinding::new("right", DocNextPage, Some(PAGE_KEYS)),
        KeyBinding::new("pagedown", DocNextPage, Some(PAGE_KEYS)),
        KeyBinding::new("z", DocToggleZoom, Some(PAGE_KEYS)),
        KeyBinding::new("+", ZoomIn, Some(PAGE_KEYS)),
        KeyBinding::new("=", ZoomIn, Some(PAGE_KEYS)),
        KeyBinding::new("shift-=", ZoomIn, Some(PAGE_KEYS)),
        KeyBinding::new("shift-+", ZoomIn, Some(PAGE_KEYS)),
        KeyBinding::new("-", ZoomOut, Some(PAGE_KEYS)),
        KeyBinding::new("_", ZoomOut, Some(PAGE_KEYS)),
        KeyBinding::new("shift--", ZoomOut, Some(PAGE_KEYS)),
        KeyBinding::new("cmd-=", UiZoomIn, None),
        KeyBinding::new("cmd-+", UiZoomIn, None),
        KeyBinding::new("shift-cmd-=", UiZoomIn, None),
        KeyBinding::new("cmd--", UiZoomOut, None),
        KeyBinding::new("shift-cmd--", UiZoomOut, None),
        KeyBinding::new("alt-cmd-0", UiZoomReset, None),
        // Vim chords for fast page jumps
        KeyBinding::new("g h", GoTo(Page::Home), Some(PAGE_KEYS)),
        KeyBinding::new("g o", GoTo(Page::Context), Some(PAGE_KEYS)),
        KeyBinding::new("g t", GoTo(Page::Context), Some(PAGE_KEYS)),
        KeyBinding::new("g m", GoTo(Page::Capabilities), Some(PAGE_KEYS)),
        KeyBinding::new("g p", GoTo(Page::Projects), Some(PAGE_KEYS)),
        KeyBinding::new("g a", OpenPalette, Some(PAGE_KEYS)),
        KeyBinding::new("g v", GoTo(Page::Travel), Some(PAGE_KEYS)),
        KeyBinding::new("g e", GoTo(Page::Feed), Some(PAGE_KEYS)),
        KeyBinding::new("g n", GoTo(Page::Inbox), Some(PAGE_KEYS)),
        KeyBinding::new("g c", GoTo(Page::Calendar), Some(PAGE_KEYS)),
        KeyBinding::new("g d", GoTo(Page::Documents), Some(PAGE_KEYS)),
        KeyBinding::new("g b", GoTo(Page::Backup), Some(PAGE_KEYS)),
        KeyBinding::new("g f", GoTo(Page::Finance), Some(PAGE_KEYS)),
        KeyBinding::new("g r", GoTo(Page::People), Some(PAGE_KEYS)),
        KeyBinding::new("g i", GoTo(Page::Inventory), Some(PAGE_KEYS)),
        KeyBinding::new("g s", GoTo(Page::Software), Some(PAGE_KEYS)),
        KeyBinding::new("g u", GoTo(Page::Import), Some(PAGE_KEYS)),
        KeyBinding::new("g x", GoTo(Page::Systems), Some(PAGE_KEYS)),
        KeyBinding::new("g k", GoTo(Page::Packs), Some(PAGE_KEYS)),
        KeyBinding::new("g l", GoTo(Page::SelfModel), Some(PAGE_KEYS)),
        KeyBinding::new("g y", GoTo(Page::Cv), Some(PAGE_KEYS)),
        KeyBinding::new("g j", GoTo(Page::Posts), Some(PAGE_KEYS)),
        KeyBinding::new("g w", GoTo(Page::Media), Some(PAGE_KEYS)),
        KeyBinding::new("g z", GoTo(Page::Research), Some(PAGE_KEYS)),
        KeyBinding::new("g 0", GoTo(Page::Setup), Some(PAGE_KEYS)),
        KeyBinding::new("g ,", GoTo(Page::Settings), Some(PAGE_KEYS)),
    ]);
    cx.bind_keys(
        Area::ALL
            .into_iter()
            .zip([
                "cmd-1", "cmd-2", "cmd-3", "cmd-4", "cmd-5", "cmd-6", "cmd-7", "cmd-8", "cmd-9",
            ])
            .map(|(area, keys)| KeyBinding::new(keys, OpenArea(area), None)),
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

/// How often the poll asks git whether the checkout moved. A checkout changes on the scale of a
/// working session, so a fork of `git rev-parse` on every poll buys nothing; ⌘⇧U and Settings
/// still ask at once.
const UPDATE_CHECK: Duration = Duration::from_secs(300);

fn restore_task_done(tasks: &mut [GlobalTask], path: &str, done: bool) {
    if let Some(task) = tasks.iter_mut().find(|task| task.id == path) {
        task.done = done;
    }
}

fn calendar_trip_route(plan_id: String) -> sjel_gpui_core::travel::TravelRoute {
    sjel_gpui_core::travel::TravelRoute::Trip {
        id: plan_id,
        section: sjel_gpui_core::travel::TripSection::Overview,
    }
}

pub struct Workspace {
    pub page: Page,
    /// The tab each area shows, so ⌘1–⌘8 return to where you left an area.
    pub tabs: Tabs,
    repo: Repo,
    pub health: Remote<NodeHealth>,
    pub agent: Remote<AgentView>,
    /// Prompt drafts waiting in comms' outbox, for the Today page.
    pub prompt_drafts: Vec<sjel_gpui_core::prompts::Draft>,
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
    pub accounts: Entity<Accounts>,
    pub clipper_settings: Entity<ClipperSettings>,
    pub travel: Entity<sjel_gpui_ui::travel::Travel>,
    pub documents: Option<Entity<Documents>>,
    pub feed: Option<Entity<Feed>>,
    pub inbox: Entity<Inbox>,
    music_page: Option<(Entity<Music>, Subscription)>,
    _inbox_events: Subscription,
    pub calendar_page: Option<Entity<CalendarPage>>,
    _calendar_events: Subscription,
    pub context_page: Entity<ContextPage>,
    _context_events: Subscription,
    _travel_events: Subscription,
    pub finance_page: Option<Entity<FinancePage>>,
    pub people_page: Option<Entity<PeoplePage>>,
    pub backup_page: Option<Entity<BackupPage>>,
    pub inventory_page: Option<Entity<InventoryPage>>,
    pub software: Option<Entity<Software>>,
    pub docs_page: Option<Entity<DocsPage>>,
    pub record_page: Option<Entity<RecordPage>>,
    pub import: Option<Entity<ImportPage>>,
    pub systems: Option<Entity<SystemsPage>>,
    pub packs: Option<Entity<PacksPage>>,
    pub self_page: Option<Entity<SelfPage>>,
    pub cv_page: Option<Entity<CvPage>>,
    pub posts_page: Option<Entity<PostsPage>>,
    pub media_page: Option<Entity<MediaPage>>,
    pub research_page: Option<Entity<ResearchPage>>,
    pub setup_page: Option<Entity<SetupPage>>,
    /// Approval ids and capability names with a request in flight.
    pub busy: Busy,
    pub selection: Selection,
    /// True once the shell pushes its answers, so the timer stops reading them.
    streaming: bool,
    pub appearance_settings: AppearanceSettings,
    pub home_prefs: crate::home::HomePreferences,
    pub home_customize_open: bool,
    /// Pinch or Cmd-scroll travel not yet turned into a zoom step.
    ui_zoom_gesture: f32,
    /// The section Settings shows.
    pub settings_section: crate::settings::Section,
    pub(crate) fda_helper: crate::permissions::HelperStatus,
    pub settings_feature_view: crate::settings::FeatureView,
    /// Search within Settings and show or hide its contents rail.
    pub settings_query: String,
    /// A shortcut group selected from the Settings contents rail.
    pub settings_shortcut_group: Option<&'static str>,
    pub settings_search: Option<(Entity<TextInput>, Subscription)>,
    pub update_info: Option<crate::update::UpdateInfo>,
    /// When the last `git` check started, so the poll can skip the ones it does not need.
    update_checked_at: Option<Instant>,
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
    /// True while the edge's left border is being dragged.
    resizing_edge: bool,
    /// The terminal seat, built the first time it is asked for and kept afterwards: closing the
    /// drawer does not end the shell, so reopening it shows the screen as it is now.
    pub terminal: Option<Entity<TerminalDeck>>,
    /// Why no terminal is showing, in the words the empty seat carries.
    pub terminal_note: Option<SharedString>,
    /// The 24-hour quick scratchpad seat in the right edge.
    pub scratchpad: Entity<ScratchpadSeat>,
    _scratchpad: Subscription,
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
        let projects = cx.new(|cx| Projects::new(repo.clone(), cx));
        let ask = cx.new(|_| Ask::new(repo.clone()));
        let accounts = cx.new(|_| Accounts::new(repo.clone()));
        let clipper_settings = cx.new(|_| ClipperSettings::new());
        let travel = cx.new(|cx| {
            sjel_gpui_ui::travel::Travel::new(
                repo.shell().ok().map(sjel_gpui_core::travel::Source::Shell),
                sjel_gpui_ui::Density::Desktop,
                cx,
            )
        });
        #[cfg(target_os = "macos")]
        cx.set_global(sjel_gpui_ui::travel::InlineAppleMapHost(
            std::sync::Arc::new(crate::apple_maps::handle_inline_command),
        ));
        let travel_events = cx.subscribe_in(
            &travel,
            window,
            |_, _, event: &sjel_gpui_ui::travel::TravelEvent, _, cx| match event {
                sjel_gpui_ui::travel::TravelEvent::OpenAppleMaps {
                    title,
                    center,
                    zoom,
                    pins,
                } => {
                    #[cfg(target_os = "macos")]
                    {
                        if let Err(err) =
                            crate::apple_maps::open_map_window(title, *center, *zoom, pins)
                        {
                            eprintln!("apple maps window unavailable: {err}");
                            cx.open_url(&format!(
                                "https://maps.apple.com/?ll={:.6},{:.6}",
                                center.latitude, center.longitude
                            ));
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        let _ = (title, zoom, pins);
                        cx.open_url(&format!(
                            "https://maps.apple.com/?ll={:.6},{:.6}",
                            center.latitude, center.longitude
                        ));
                    }
                }
            },
        );
        let inbox = cx.new(|cx| Inbox::new(repo.clone(), cx));
        let inbox_events = cx.subscribe_in(&inbox, window, Self::on_inbox);
        let context_page = cx.new(|cx| ContextPage::new(repo.clone(), cx));
        let context_events = cx.subscribe_in(&context_page, window, Self::on_context);
        let calendar_page = cx.new(|cx| CalendarPage::new(repo.clone(), cx));
        let calendar_events = cx.subscribe_in(
            &calendar_page,
            window,
            |workspace, _, intent: &sjel_gpui_ui::calendar::CalendarIntent, window, cx| match intent
            {
                sjel_gpui_ui::calendar::CalendarIntent::PlanTravel { day } => {
                    workspace.go(Page::Travel, cx);
                    workspace
                        .travel
                        .update(cx, |travel, cx| travel.plan_calendar_day(*day, window, cx));
                }
                sjel_gpui_ui::calendar::CalendarIntent::OpenTrip { plan_id } => {
                    workspace.go(Page::Travel, cx);
                    workspace.travel.update(cx, |travel, cx| {
                        travel.open_route(calendar_trip_route(plan_id.clone()), cx);
                    });
                }
                sjel_gpui_ui::calendar::CalendarIntent::OpenMail { item_id } => {
                    if let Some(id) = item_id {
                        workspace.open_target(
                            sjel_gpui_core::route::Target::MailEntry(id.clone()),
                            window,
                            cx,
                        );
                    } else {
                        workspace.open_target(
                            sjel_gpui_core::route::Target::Feed(
                                sjel_gpui_core::route::FeedTab::Mail,
                            ),
                            window,
                            cx,
                        );
                    }
                }
                sjel_gpui_ui::calendar::CalendarIntent::OpenInbox => {
                    workspace.go(Page::Inbox, cx);
                }
            },
        );
        let appearance_settings = AppearanceSettings::load();
        let scratchpad = {
            let repo = repo.clone();
            cx.new(|cx| ScratchpadSeat::new(repo, cx))
        };
        let scratchpad_events = cx.subscribe_in(&scratchpad, window, Self::on_scratchpad);
        let finder = {
            let (repo, ask) = (repo.clone(), ask.clone());
            cx.new(|cx| Finder::new(repo, ask, finder_commands(), cx))
        };
        let finder_events = cx.subscribe_in(&finder, window, Self::on_finder);
        let settings_search = cx.new(|cx| {
            TextInput::new(window, cx).placeholder("Search settings, accounts, features, shortcuts")
        });
        let settings_search_events = cx.subscribe_in(
            &settings_search,
            window,
            |view, input, event, window, cx| {
                if matches!(event, InputEvent::Changed) {
                    view.settings_query = input.read(cx).text().to_owned();
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
                        let previous_section = view.settings_section;
                        view.settings_section = crate::settings::search_section(
                            &view.settings_query,
                            catalogue_has_match,
                        )
                        .unwrap_or(crate::settings::Section::Shortcuts);
                        if view.settings_section == crate::settings::Section::Permissions
                            && previous_section != view.settings_section
                        {
                            view.check_fda_helper(cx);
                        }
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
            prompt_drafts: Vec::new(),
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
            resizing_edge: false,
            terminal: None,
            terminal_note: None,
            scratchpad,
            _scratchpad: scratchpad_events,
            finder,
            _finder: finder_events,
            backups: Remote::default(),
            projects,
            ask,
            accounts,
            clipper_settings,
            travel,
            documents: None,
            feed: None,
            inbox,
            music_page: None,
            _inbox_events: inbox_events,
            calendar_page: Some(calendar_page),
            _calendar_events: calendar_events,
            context_page,
            _context_events: context_events,
            _travel_events: travel_events,
            finance_page: None,
            people_page: None,
            backup_page: None,
            inventory_page: None,
            software: None,
            docs_page: None,
            record_page: None,
            import: None,
            systems: None,
            packs: None,
            self_page: None,
            cv_page: None,
            posts_page: None,
            media_page: None,
            research_page: None,
            setup_page: None,
            busy: Busy::default(),
            selection: Selection::default(),
            streaming: false,
            focus,
            refocus: false,
            appearance_settings,
            home_prefs: crate::home::HomePreferences::load(),
            home_customize_open: false,
            ui_zoom_gesture: 0.,
            settings_section: crate::settings::Section::default(),
            fda_helper: crate::permissions::HelperStatus::default(),
            settings_feature_view: crate::settings::FeatureView::default(),
            settings_query: String::new(),
            settings_shortcut_group: None,
            settings_search: Some((settings_search, settings_search_events)),
            update_info: None,
            update_checked_at: None,
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
        // The window opens on Today, which draws the day's timeline, inbox summary, and project momentum.
        workspace
            .context_page
            .update(cx, |context, cx| context.activate(cx));
        workspace.load_home_feeds(cx);
        if let (Some(frames), true) = (
            workspace.frames.clone(),
            std::env::var_os("SJEL_GPUI_BENCH_SCROLL").is_some(),
        ) {
            crate::frames::drive_scroll_bench(frames, cx);
        }
        if let (Some(frames), Ok(path)) = (
            workspace.frames.clone(),
            std::env::var("SJEL_GPUI_BENCH_NOTE"),
        ) {
            workspace.go(Page::Projects, cx);
            crate::frames::drive_type_bench(frames, path, workspace.projects.clone(), window, cx);
        }
        if let (Some(frames), Ok(path)) = (
            workspace.frames.clone(),
            std::env::var("SJEL_GPUI_BENCH_CLICK"),
        ) {
            workspace.go(Page::Projects, cx);
            crate::frames::drive_click_bench(frames, path, workspace.projects.clone(), window, cx);
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
                    if this.apply(snapshot) {
                        cx.notify();
                    }
                });
                if applied.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    fn apply(&mut self, snapshot: Snapshot) -> bool {
        let mut changed = false;
        if let Some(health) = snapshot.health {
            changed |= self.health.settle_changed(Ok(health));
        }
        if let Some(agent) = snapshot.agent {
            changed |= self.agent.settle_changed(Ok(agent));
        }
        if let Some(error) = snapshot.error {
            let error = ShellError::Refused(502, Some(error));
            changed |= self.health.settle_changed(Err(error));
        }
        changed
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        if !self.streaming {
            data::read_quiet(
                cx,
                self,
                |w| &mut w.health,
                &repo,
                |s| async move { s.health().await },
            );
            data::read_quiet(
                cx,
                self,
                |w| &mut w.agent,
                &repo,
                |s| async move { s.agent().await },
            );
        }
        // Only while the page is open: the answer reads a receipt per capability.
        if self.page == Page::Backup {
            data::read_quiet(
                cx,
                self,
                |w| &mut w.backups,
                &repo,
                |s| async move { s.backups().await },
            );
        }
        self.read_prompt_drafts(cx);
        let repo_tasks = self.repo.clone();
        data::read_repo_quiet(
            cx,
            self,
            |w| &mut w.focus_tasks,
            &repo_tasks,
            move |r| async move { r.fetch_tasks(Some("open")).await },
        );
        let repo_sound = self.repo.clone();
        data::read_repo_quiet(
            cx,
            self,
            |w| &mut w.soundscape,
            &repo_sound,
            move |r| async move { r.soundscape_state().await },
        );
        if self
            .update_checked_at
            .is_none_or(|at| at.elapsed() >= UPDATE_CHECK)
        {
            self.check_updates(cx);
        }
        self.scratchpad
            .update(cx, |seat, cx| seat.check_auto_sort(None, cx));
    }

    /// A failed read keeps the last list: comms restarting must not blank the Today page.
    fn read_prompt_drafts(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { sjel_gpui_core::prompts::drafts() })
                .await;
            if let Ok(drafts) = result {
                let _ = this.update(cx, |view, cx| {
                    if view.prompt_drafts != drafts {
                        view.prompt_drafts = drafts;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    /// Copy a draft to the clipboard, then take it out of the outbox. The card goes at once;
    /// if comms refuses the take, the next refresh brings it back.
    pub fn copy_prompt_draft(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(draft) = self.prompt_drafts.iter().find(|d| d.id == id) else {
            return;
        };
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(draft.body.clone()));
        self.prompt_drafts.retain(|d| d.id != id);
        cx.notify();
        cx.background_executor()
            .spawn(async move { sjel_gpui_core::prompts::take(id) })
            .detach();
    }

    pub fn check_updates(&mut self, cx: &mut Context<Self>) {
        self.update_checked_at = Some(Instant::now());
        cx.spawn(async move |this, cx| {
            let info = cx
                .background_executor()
                .spawn(async move {
                    crate::update::find_build_root()
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
        let Some(repo_root) = crate::update::find_build_root() else {
            window.push_toast(
                Toast::new("Could not locate Sjel repository root.").severity(Severity::Danger),
                cx,
            );
            return;
        };
        self.rebuild_state = crate::update::RebuildState::Building;
        self.show_rebuild_log = false;
        cx.notify();
        window.push_toast(
            Toast::new("Rebuilding Sjel Desktop in background...").severity(Severity::Info),
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

    pub fn toggle_task(
        &mut self,
        path: String,
        done: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Optimistic: the row reacts on the click, not on the shell's answer. Without a shell
        // `data::write` refuses before the write, so there is nothing to undo later.
        let previous_done = self
            .repo
            .shell()
            .ok()
            .and(self.focus_tasks.value.as_mut())
            .and_then(|tasks| tasks.iter_mut().find(|t| t.id == path))
            .map(|task| {
                let previous = task.done;
                task.done = done;
                previous
            });
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
            move |this, updated, cx| {
                let Some(task) = updated else {
                    // Restore immediately; a failed recovery read must not leave optimistic state.
                    if let (Some(previous), Some(tasks)) =
                        (previous_done, this.focus_tasks.value.as_mut())
                    {
                        restore_task_done(tasks, &path, previous);
                    }
                    this.refresh(cx);
                    cx.notify();
                    return;
                };
                if let Some(tasks) = &mut this.focus_tasks.value {
                    if let Some(pos) = tasks.iter().position(|t| t.id == task.id) {
                        if task.done {
                            tasks.remove(pos);
                        } else {
                            tasks[pos] = task;
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
            window.push_toast(
                Toast::new("Could not locate the Sjel checkout to restart its shell.")
                    .severity(Severity::Danger),
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
                        window.push_toast(
                            Toast::new("Sjel shell restarted. Reconnecting…")
                                .severity(Severity::Success),
                            cx,
                        );
                        view.refresh(cx);
                    }
                    Err(error) => {
                        window.push_toast(Toast::new(error).severity(Severity::Danger), cx)
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Relaunches on the other node: the live app from the dev one, or `sjel demo app` from
    /// the live one. This process quits once the other is started.
    pub fn switch_node(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        const BUSY_KEY: &str = "switch-node";
        if !self.busy.start(BUSY_KEY) {
            return;
        }
        cx.notify();
        let work = cx
            .background_executor()
            .spawn(async move { crate::update::launch_other_node() });
        cx.spawn_in(window, async move |weak, cx| {
            let result = work.await;
            let _ = weak.update_in(cx, |view, window, cx| {
                view.busy.finish(BUSY_KEY);
                match result {
                    Ok(()) => cx.quit(),
                    Err(error) => {
                        window.push_toast(Toast::new(error).severity(Severity::Danger), cx)
                    }
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
                self.ensure_feed(cx)
                    .update(cx, |feed, cx| feed.set_tab(tab, cx));
            }
            Target::FeedEntry(id) => {
                self.go(Page::Feed, cx);
                self.ensure_feed(cx)
                    .update(cx, |feed, cx| feed.open_entry(id, cx));
            }
            Target::MailEntry(id) => {
                self.go(Page::Feed, cx);
                self.ensure_feed(cx)
                    .update(cx, |feed, cx| feed.open_mail_entry(id, cx));
            }
            Target::CalendarEntry(id) => {
                self.go(Page::Feed, cx);
                self.ensure_feed(cx)
                    .update(cx, |feed, cx| feed.open_calendar_entry(id, cx));
            }
            Target::Docs(slug) => {
                self.go(Page::Docs, cx);
                self.ensure_docs_page(cx)
                    .update(cx, |docs, cx| docs.open(slug, cx));
            }
            Target::Record(id) => {
                self.go(Page::Record, cx);
                self.ensure_record_page(cx)
                    .update(cx, |page, cx| page.open(id, cx));
            }
            Target::Ask(question) => self
                .finder
                .update(cx, |finder, cx| finder.ask(question, cx)),
            Target::Scratchpad => self.open_scratchpad(window, cx),
            Target::Terminal => self.open_terminal(window, cx),
            Target::Unknown(path) => match route::first_page(&path) {
                Some(page) => self.go(page, cx),
                None => window.push_toast(
                    Toast::new(format!("No page answers {path}.")).severity(Severity::Danger),
                    cx,
                ),
            },
        }
    }

    pub(crate) fn open_palette(&mut self, _: &OpenPalette, _: &mut Window, cx: &mut Context<Self>) {
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
                Open::Project(name) => self.go_to_project(name, cx),
                Open::Link(link) => {
                    if let Some(target) = route::parse_deep_link(&link) {
                        self.open_target(target, window, cx);
                    }
                }
                Open::Url(url) => cx.open_url(&url),
                Open::Record { project, path } => {
                    self.go(Page::Projects, cx);
                    self.projects.update(cx, |projects, cx| {
                        projects.open_record(project, path, window, cx)
                    });
                }
            },
        }
    }

    fn on_scratchpad(
        &mut self,
        _: &Entity<ScratchpadSeat>,
        event: &ScratchpadEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ScratchpadEvent::Navigate(target) => {
                self.open_target(target.clone(), window, cx);
            }
            ScratchpadEvent::Sorted(_) => {
                self.inbox.update(cx, |inbox, cx| inbox.load(cx));
                self.projects
                    .update(cx, |projects, cx| projects.load_projects(cx));
                if let Some(feed) = &self.feed {
                    feed.update(cx, |feed, cx| feed.refresh(cx));
                }
                if let Some(inv) = &self.inventory_page {
                    inv.update(cx, |inv, cx| inv.refresh(cx));
                }
                self.refresh(cx);
                cx.notify();
            }
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
            InboxEvent::OpenMusic(url) => {
                let repo = self.repo.clone();
                let page = cx.new(|cx| Music::new(repo, url, window, cx));
                let events = cx.subscribe_in(&page, window, Self::on_music);
                page.update(cx, |view, cx| view.probe(window, cx));
                self.music_page = Some((page, events));
                cx.notify();
            }
        }
    }

    fn on_music(
        &mut self,
        _: &Entity<Music>,
        event: &MusicEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            MusicEvent::Close => {
                self.music_page = None;
                cx.notify();
            }
        }
    }

    fn on_context(
        &mut self,
        _: &Entity<ContextPage>,
        event: &ContextEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ContextEvent::OpenCalendar { day, entry_id } => {
                let day = *day;
                let entry_id = entry_id.clone();
                self.go(Page::Calendar, cx);
                self.ensure_calendar_page(cx)
                    .update(cx, |cal, cx| cal.open_day(day, entry_id, cx));
            }
            ContextEvent::OpenProject(project) => {
                if project.is_empty() {
                    self.go(Page::Projects, cx);
                } else {
                    self.go_to_project(project.clone(), cx);
                }
            }
            ContextEvent::OpenFinance { day } => {
                let date = day.format("%Y-%m-%d").to_string();
                self.go(Page::Finance, cx);
                self.ensure_finance_page(cx)
                    .update(cx, |fin, cx| fin.open_date(date, window, cx));
            }
            ContextEvent::OpenInbox => {
                self.go(Page::Inbox, cx);
            }
        }
    }

    pub(crate) fn load_home_feeds(&mut self, cx: &mut Context<Self>) {
        if self.inbox.read(cx).rows.value.is_none() && !self.inbox.read(cx).rows.loading {
            self.inbox.update(cx, |inbox, cx| inbox.load(cx));
        }
        if self.projects.read(cx).projects.value.is_none()
            && !self.projects.read(cx).projects.loading
        {
            self.projects
                .update(cx, |projects, cx| projects.load_projects(cx));
        }
    }

    fn on_go_to(&mut self, GoTo(page): &GoTo, _: &mut Window, cx: &mut Context<Self>) {
        self.go(*page, cx);
    }

    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh_current_page(cx);
    }

    /// `SJEL_GPUI_BENCH_SCROLL`: step the open page's own scroller to `fraction` of its height.
    ///
    /// A page that draws a virtual list owns the scroll, so the bench asks the page rather than
    /// reading the workspace's handle.
    pub fn bench_scroll(&mut self, fraction: f32, cx: &mut Context<Self>) {
        match self.page {
            Page::Feed => {
                self.ensure_feed(cx)
                    .update(cx, |feed, cx| feed.bench_scroll(fraction, cx));
                return;
            }
            Page::Documents => {
                self.ensure_documents(cx)
                    .update(cx, |docs, cx| docs.bench_scroll(fraction, cx));
                return;
            }
            Page::Inventory => {
                self.ensure_inventory_page(cx)
                    .update(cx, |inv, cx| inv.bench_scroll(fraction, cx));
                return;
            }
            Page::Finance => {
                self.ensure_finance_page(cx)
                    .update(cx, |fin, cx| fin.bench_scroll(fraction, cx));
                return;
            }
            Page::People => {
                self.ensure_people_page(cx)
                    .update(cx, |people, cx| people.bench_scroll(fraction, cx));
                return;
            }
            _ => {}
        }
        let scroll = match self.page {
            Page::Travel => self.travel.read(cx).scroll_handle(),
            Page::Projects => self.projects.read(cx).scroll_handle(),
            Page::Docs => self.ensure_docs_page(cx).read(cx).scroll_handle(),
            Page::Backup => self.ensure_backup_page(cx).read(cx).scroll_handle(),
            Page::Calendar => self.ensure_calendar_page(cx).read(cx).scroll_handle(),
            Page::Packs => self.ensure_packs(cx).read(cx).scroll_handle(),
            _ => self.selection.scroll.clone(),
        };
        let max = scroll.max_offset().y;
        scroll.set_offset(point(px(0.), -max * fraction));
        cx.notify();
    }

    fn on_open_terminal(&mut self, _: &OpenTerminal, window: &mut Window, cx: &mut Context<Self>) {
        self.open_terminal(window, cx);
    }

    fn on_open_scratchpad(
        &mut self,
        _: &OpenScratchpad,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_scratchpad(window, cx);
    }

    fn on_sort_scratchpad_now(
        &mut self,
        _: &SortScratchpadNow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scratchpad
            .update(cx, |seat, cx| seat.sort_now(Some(window), cx));
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
        if section.unwrap_or(self.settings_section) == crate::settings::Section::General
            && self.update_info.is_none()
        {
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
        if self.settings_section == crate::settings::Section::Permissions {
            self.check_fda_helper(cx);
        }
        self.go(Page::Settings, cx);
    }

    fn check_fda_helper(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |workspace, cx| {
            let status = cx
                .background_executor()
                .spawn(async { crate::permissions::HelperStatus::detect() })
                .await;
            let _ = workspace.update(cx, |view, cx| {
                view.fda_helper = status;
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
    /// gives them the accent the palette uses, and gives Ely's controls Sjel's palette.
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
            // `Root` sets the window's rem size from this on every frame, so it is the app's zoom.
            theme.font_size = px(f32::from(self.appearance_settings.ui_font_size));
        });
        sjel_gpui_ui::theme::ely::sync(self.appearance_settings, dark, cx);
    }

    fn on_check_for_updates(
        &mut self,
        _: &CheckForUpdates,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.push_toast(
            Toast::new("Checking for Sjel updates...").severity(Severity::Info),
            cx,
        );
        cx.spawn(async move |this, cx| {
            let info = cx
                .background_executor()
                .spawn(async move {
                    crate::update::find_build_root()
                        .and_then(|r| crate::update::check_git_updates(&r))
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.update_info = info.clone();
                view.open_settings(Some(crate::settings::Section::General), cx);
            });
        })
        .detach();
    }

    fn on_rebuild_app(&mut self, _: &RebuildApp, window: &mut Window, cx: &mut Context<Self>) {
        self.trigger_rebuild(window, cx);
    }

    fn on_select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Documents {
            self.ensure_documents(cx)
                .update(cx, |docs, cx| docs.select_relative(1, cx));
        } else {
            self.selection.step(1, self.items().len());
        }
        cx.notify();
    }

    fn on_select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Documents {
            self.ensure_documents(cx)
                .update(cx, |docs, cx| docs.select_relative(-1, cx));
        } else {
            self.selection.step(-1, self.items().len());
        }
        cx.notify();
    }

    fn on_doc_prev_page(&mut self, _: &DocPrevPage, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Documents {
            self.ensure_documents(cx)
                .update(cx, |docs, cx| docs.go_to_page(-1, cx));
        }
    }

    fn on_doc_next_page(&mut self, _: &DocNextPage, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Documents {
            self.ensure_documents(cx)
                .update(cx, |docs, cx| docs.go_to_page(1, cx));
        }
    }

    fn on_doc_toggle_zoom(&mut self, _: &DocToggleZoom, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Documents {
            self.ensure_documents(cx)
                .update(cx, |docs, cx| docs.toggle_zoom(cx));
        }
    }

    fn on_zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        match self.page {
            Page::Travel => {
                self.travel
                    .update(cx, |travel, cx| travel.zoom_by(1., None, cx));
            }
            Page::Documents => {
                self.ensure_documents(cx)
                    .update(cx, |docs, cx| docs.zoom_in(cx));
            }
            _ => self.step_terminal_zoom(1, cx),
        }
    }

    fn on_zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        match self.page {
            Page::Travel => {
                self.travel
                    .update(cx, |travel, cx| travel.zoom_by(-1., None, cx));
            }
            Page::Documents => {
                self.ensure_documents(cx)
                    .update(cx, |docs, cx| docs.zoom_out(cx));
            }
            _ => self.step_terminal_zoom(-1, cx),
        }
    }

    fn on_ui_zoom_in(&mut self, _: &UiZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Travel {
            self.travel
                .update(cx, |travel, cx| travel.zoom_by(1., None, cx));
            return;
        }
        if self.page == Page::Documents {
            self.ensure_documents(cx)
                .update(cx, |docs, cx| docs.zoom_in(cx));
            return;
        }
        self.step_ui_zoom(1, cx);
    }

    fn on_ui_zoom_out(&mut self, _: &UiZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Travel {
            self.travel
                .update(cx, |travel, cx| travel.zoom_by(-1., None, cx));
            return;
        }
        if self.page == Page::Documents {
            self.ensure_documents(cx)
                .update(cx, |docs, cx| docs.zoom_out(cx));
            return;
        }
        self.step_ui_zoom(-1, cx);
    }

    fn on_ui_zoom_reset(&mut self, _: &UiZoomReset, _: &mut Window, cx: &mut Context<Self>) {
        self.set_appearance(
            AppearanceSettings {
                ui_font_size: DEFAULT_UI_FONT_SIZE,
                ..self.appearance_settings
            },
            cx,
        );
    }

    fn step_ui_zoom(&mut self, steps: i32, cx: &mut Context<Self>) {
        let settings = self.appearance_settings;
        let ui_font_size = step_font_size(&UI_FONT_SIZES, settings.ui_font_size, steps);
        if ui_font_size != settings.ui_font_size {
            self.set_appearance(
                AppearanceSettings {
                    ui_font_size,
                    ..settings
                },
                cx,
            );
        }
    }

    fn step_terminal_zoom(&mut self, steps: i32, cx: &mut Context<Self>) {
        let settings = self.appearance_settings;
        let terminal_font_size = step_terminal_font_size(settings.terminal_font_size, steps);
        self.set_appearance(
            AppearanceSettings {
                terminal_font_size,
                ..settings
            },
            cx,
        );
    }

    fn on_next_page(&mut self, _: &NextPage, _: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(1, cx);
    }

    fn on_prev_page(&mut self, _: &PrevPage, _: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(-1, cx);
    }

    fn step_sub_tab(&mut self, delta: isize, cx: &mut Context<Self>) {
        let wrap = |ix: usize, len: usize| -> usize {
            ((ix as isize + delta).rem_euclid(len as isize)) as usize
        };
        match self.page {
            Page::Feed => {
                use sjel_gpui_ui::feed::Tab as FeedTab;
                const TABS: [FeedTab; 6] = [
                    FeedTab::Reading,
                    FeedTab::Mail,
                    FeedTab::Discover,
                    FeedTab::Clips,
                    FeedTab::Review,
                    FeedTab::Library,
                ];
                let feed = self.ensure_feed(cx);
                let cur = feed.read(cx).tab;
                let ix = TABS.iter().position(|&t| t == cur).unwrap_or(0);
                let next = TABS[wrap(ix, TABS.len())];
                feed.update(cx, |f, cx| f.set_tab(next, cx));
            }
            Page::Projects => {
                use sjel_gpui_ui::projects::{ProjectSection, ProjectsTab};
                if self.projects.read(cx).active_project_name().is_some() {
                    const SECS: [ProjectSection; 5] = [
                        ProjectSection::Note,
                        ProjectSection::Tasks,
                        ProjectSection::Notes,
                        ProjectSection::MailReview,
                        ProjectSection::InventoryLinks,
                    ];
                    let cur = self.projects.read(cx).project_section;
                    let ix = SECS.iter().position(|&s| s == cur).unwrap_or(0);
                    let next = SECS[wrap(ix, SECS.len())];
                    self.projects
                        .update(cx, |p, cx| p.set_project_section(next, cx));
                } else {
                    const TABS: [ProjectsTab; 3] = [
                        ProjectsTab::Hubs,
                        ProjectsTab::AllTasks,
                        ProjectsTab::AllNotes,
                    ];
                    let cur = self.projects.read(cx).tab;
                    let ix = TABS.iter().position(|&t| t == cur).unwrap_or(0);
                    let next = TABS[wrap(ix, TABS.len())];
                    self.projects.update(cx, |p, cx| {
                        p.tab = next;
                        cx.notify();
                    });
                }
            }
            Page::Finance => {
                use sjel_gpui_ui::finance::View as FinanceView;
                const VIEWS: [FinanceView; 5] = [
                    FinanceView::Overview,
                    FinanceView::Transactions,
                    FinanceView::Subscriptions,
                    FinanceView::ImportReview,
                    FinanceView::Ideas,
                ];
                let fp = self.ensure_finance_page(cx);
                let cur = fp.read(cx).view;
                let ix = VIEWS.iter().position(|&v| v == cur).unwrap_or(0);
                let next = VIEWS[wrap(ix, VIEWS.len())];
                fp.update(cx, |p, cx| p.set_view(next, cx));
            }
            Page::Inventory => {
                use sjel_gpui_ui::inventory::InventoryTab;
                const TABS: [InventoryTab; 4] = [
                    InventoryTab::Owned,
                    InventoryTab::Wishlist,
                    InventoryTab::Interior,
                    InventoryTab::MailReview,
                ];
                let ip = self.ensure_inventory_page(cx);
                let cur = ip.read(cx).tab;
                let ix = TABS.iter().position(|&t| t == cur).unwrap_or(0);
                let next = TABS[wrap(ix, TABS.len())];
                ip.update(cx, |p, cx| p.set_tab(next, cx));
            }
            Page::People => {
                let pp = self.ensure_people_page(cx);
                let cur = pp.read(cx).active_tab_index();
                let next = wrap(cur, 3);
                pp.update(cx, |p, cx| p.set_tab_index(next, cx));
            }
            Page::Systems => {
                use sjel_gpui_ui::systems::SystemsTab;
                const TABS: [SystemsTab; 3] =
                    [SystemsTab::Overview, SystemsTab::Storage, SystemsTab::Agent];
                let sp = self.ensure_systems(cx);
                let cur = sp.read(cx).tab;
                let ix = TABS.iter().position(|&t| t == cur).unwrap_or(0);
                let next = TABS[wrap(ix, TABS.len())];
                sp.update(cx, |p, cx| p.set_tab(next, cx));
            }
            Page::Backup => {
                use sjel_gpui_ui::backup::BackupTab;
                const TABS: [BackupTab; 3] =
                    [BackupTab::Contracts, BackupTab::Targets, BackupTab::History];
                let bp = self.ensure_backup_page(cx);
                let cur = bp.read(cx).tab;
                let ix = TABS.iter().position(|&t| t == cur).unwrap_or(0);
                let next = TABS[wrap(ix, TABS.len())];
                bp.update(cx, |p, cx| p.set_tab(next, cx));
            }
            Page::Calendar => {
                use sjel_gpui_ui::calendar::View as CalendarView;
                const VIEWS: [CalendarView; 3] =
                    [CalendarView::Day, CalendarView::Week, CalendarView::Month];
                let cp = self.ensure_calendar_page(cx);
                let cur = cp.read(cx).view;
                let ix = VIEWS.iter().position(|&v| v == cur).unwrap_or(2);
                let next = VIEWS[wrap(ix, VIEWS.len())];
                cp.update(cx, |p, cx| p.set_view(next, cx));
            }
            Page::Research => {
                use sjel_gpui_ui::research::ResearchTab;
                const TABS: [ResearchTab; 3] = [
                    ResearchTab::Articles,
                    ResearchTab::Projects,
                    ResearchTab::Benchmarks,
                ];
                let rp = self.ensure_research_page(cx);
                let cur = rp.read(cx).active_tab;
                let ix = TABS.iter().position(|&t| t == cur).unwrap_or(0);
                let next = TABS[wrap(ix, TABS.len())];
                rp.update(cx, |p, cx| p.switch_tab(next, cx));
            }
            Page::Travel => {
                use sjel_gpui_core::travel::{TravelRoute, TripSection};
                if self.travel.read(cx).active_plan_title().is_some() {
                    let cur = self.travel.read(cx).active_section();
                    let ix = TripSection::ALL.iter().position(|&s| s == cur).unwrap_or(0);
                    let next = TripSection::ALL[wrap(ix, TripSection::ALL.len())];
                    self.travel.update(cx, |t, cx| t.set_section(next, cx));
                } else {
                    let cur = self.travel.read(cx).route().clone();
                    let ix = match cur {
                        TravelRoute::Map => 0,
                        TravelRoute::Connections => 2,
                        _ => 1,
                    };
                    let next = match wrap(ix, 3) {
                        0 => TravelRoute::Map,
                        2 => TravelRoute::Connections,
                        _ => TravelRoute::Trips,
                    };
                    self.travel.update(cx, |t, cx| t.open_route(next, cx));
                }
            }
            _ => {}
        }
    }

    fn on_next_sub_tab(&mut self, _: &NextSubTab, _: &mut Window, cx: &mut Context<Self>) {
        self.step_sub_tab(1, cx);
    }

    fn on_prev_sub_tab(&mut self, _: &PrevSubTab, _: &mut Window, cx: &mut Context<Self>) {
        self.step_sub_tab(-1, cx);
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
                self.ensure_documents(cx)
                    .update(cx, |docs, cx| docs.focus_search(window, cx));
            }
            Page::Docs => {
                self.ensure_docs_page(cx)
                    .update(cx, |docs, cx| docs.focus_search(window, cx));
            }
            Page::Finance => {
                self.ensure_finance_page(cx)
                    .update(cx, |fin, cx| fin.focus_search(window, cx));
            }
            Page::People => {
                self.ensure_people_page(cx)
                    .update(cx, |people, cx| people.focus_search(window, cx));
            }
            Page::Inventory => {
                self.ensure_inventory_page(cx)
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

    fn on_dismiss_overlay(
        &mut self,
        _: &DismissOverlay,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.finder.read(cx).is_open() {
            if let Some(panel) = self
                .soundscape_panel
                .as_ref()
                .filter(|panel| panel.read(cx).is_open())
            {
                panel.update(cx, |panel, cx| panel.set_open(false, cx));
                return;
            }
        }
        self.on_clear_selection(&ClearSelection, window, cx);
    }

    fn on_clear_selection(&mut self, _: &ClearSelection, _: &mut Window, cx: &mut Context<Self>) {
        if self.finder.read(cx).is_open() {
            self.finder.update(cx, |finder, cx| finder.close(cx));
            return;
        }
        // The player binds escape in its own context, but only while it holds focus.
        if let Some(panel) = self
            .soundscape_panel
            .as_ref()
            .filter(|panel| panel.read(cx).is_open())
        {
            panel.update(cx, |panel, cx| panel.set_open(false, cx));
            return;
        }
        self.selection.clear();
        if self.page == Page::Documents {
            self.ensure_documents(cx)
                .update(cx, |docs, cx| docs.close(cx));
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
        #[cfg(target_os = "macos")]
        crate::apple_maps::hide_inline();
        let palette = ThemeContext::sync(self.appearance_settings, window, cx);
        let tint = palette.window_tint;
        let sidebar_tint = palette.sidebar_surface;
        let glass_requested = self.appearance_settings.glass == GlassStyle::LiquidGlass;
        let glass = self.attach_glass(window, sidebar_tint, glass_requested);
        let right_panel = self.render_right_panel(palette, cx);
        // An open finder holds the keys: a page change behind it must not take them back, or the
        // finder opened right after a navigation could not be closed with escape.
        if std::mem::take(&mut self.refocus) && !self.finder.read(cx).is_open() {
            window.focus(&self.focus, cx);
        }
        let page = self.render_current_page(palette, window, cx);

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
            .on_scroll(cx.listener(|view, event: &gpui::ScrollEvent, _, cx| {
                if let Some(frames) = &view.frames {
                    frames.input();
                }
                if event.modifiers.platform || event.modifiers.control {
                    match view.page {
                        Page::Travel => {
                            cx.stop_propagation();
                            view.travel
                                .update(cx, |travel, cx| travel.on_wheel(event, cx));
                        }
                        Page::Documents => {
                            cx.stop_propagation();
                            view.ensure_documents(cx)
                                .update(cx, |docs, cx| docs.on_wheel_zoom(event, cx));
                        }
                        _ if view.right_panel.is_open()
                            && view.right_panel.seat() == Seat::Terminal =>
                        {
                            let dy = match event.delta {
                                gpui::ScrollDelta::Pixels(d) => f32::from(d.y),
                                gpui::ScrollDelta::Lines(d) => d.y,
                            };
                            let step = dy.signum() as i32;
                            if step != 0 {
                                cx.stop_propagation();
                                view.step_terminal_zoom(step, cx);
                            }
                        }
                        _ => {
                            let lines = match event.delta {
                                gpui::ScrollDelta::Lines(delta) => delta.y,
                                gpui::ScrollDelta::Pixels(delta) => {
                                    delta.y / px(UI_SCROLL_PIXELS_PER_STEP)
                                }
                            };
                            view.ui_zoom_gesture += lines;
                            let steps = view.ui_zoom_gesture.trunc();
                            if steps != 0. {
                                view.ui_zoom_gesture -= steps;
                                view.step_ui_zoom(steps as i32, cx);
                            }
                        }
                    }
                }
            }))
            .on_action(cx.listener(Self::open_palette))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_next_page))
            .on_action(cx.listener(Self::on_prev_page))
            .on_action(cx.listener(Self::on_next_sub_tab))
            .on_action(cx.listener(Self::on_prev_sub_tab))
            .on_action(cx.listener(Self::on_go_to))
            .on_action(cx.listener(Self::on_open_area))
            .on_action(cx.listener(Self::on_next_area))
            .on_action(cx.listener(Self::on_prev_area))
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_select_previous))
            .on_action(cx.listener(Self::on_clear_selection))
            .on_action(cx.listener(Self::on_dismiss_overlay))
            .on_action(cx.listener(Self::on_doc_prev_page))
            .on_action(cx.listener(Self::on_doc_next_page))
            .on_action(cx.listener(Self::on_doc_toggle_zoom))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_ui_zoom_in))
            .on_action(cx.listener(Self::on_ui_zoom_out))
            .on_action(cx.listener(Self::on_ui_zoom_reset))
            // A pinch or a Cmd-scroll over the map or the terminal zooms that view; they stop
            // the event, so what reaches here zooms the app.
            .on_pinch(cx.listener(|view, event: &gpui::PinchEvent, _, cx| {
                view.ui_zoom_gesture += event.delta;
                let steps = (view.ui_zoom_gesture / UI_PINCH_STEP).trunc();
                if steps != 0. {
                    view.ui_zoom_gesture -= steps * UI_PINCH_STEP;
                    view.step_ui_zoom(steps as i32, cx);
                }
                if event.phase == gpui::TouchPhase::Ended {
                    view.ui_zoom_gesture = 0.;
                }
            }))
            .on_action(cx.listener(Self::on_toggle_soundscape))
            .on_action(cx.listener(Self::on_cycle_soundscape_preset))
            .on_action(cx.listener(Self::on_run_primary))
            .on_action(cx.listener(Self::on_run_secondary))
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(Self::on_open_scratchpad))
            .on_action(cx.listener(Self::on_sort_scratchpad_now))
            .on_action(cx.listener(Self::on_open_terminal))
            .on_action(cx.listener(Self::on_open_appearance))
            .on_action(cx.listener(Self::on_open_soundscape))
            .on_action(cx.listener(Self::on_toggle_soundscape_panel))
            .on_action(cx.listener(Self::on_check_for_updates))
            .on_action(cx.listener(Self::on_rebuild_app))
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, window, cx| {
                if !view.resizing_edge {
                    return;
                }
                let window_width = f32::from(window.viewport_size().width);
                let width = (window_width - f32::from(event.position.x)).clamp(
                    EDGE_MIN_WIDTH,
                    (window_width - EDGE_PAGE_MIN_WIDTH).max(EDGE_MIN_WIDTH),
                );
                view.right_panel.set_width(width, false);
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, _| {
                    if view.resizing_edge {
                        view.resizing_edge = false;
                        let width = view.right_panel.width();
                        view.right_panel.set_width(width, true);
                    }
                }),
            )
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
                                        | Page::Docs
                                        | Page::Calendar
                                        | Page::Finance
                                        | Page::People
                                        | Page::Backup
                                        | Page::Inventory
                                        | Page::Feed
                                        | Page::Packs
                                ) {
                                    // Projects, Settings, Documents, Docs, Calendar, Finance, People,
                                    // Pages that scroll themselves get the full width.
                                    div().id("page").flex_1().min_h_0().px_8().pb_8().child(
                                        div()
                                            .size_full()
                                            .when(
                                                !matches!(
                                                    self.page,
                                                    Page::Documents
                                                        | Page::Docs
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
                                        .child(
                                            div()
                                                .px_8()
                                                .pb_8()
                                                .w_full()
                                                .when(self.page == Page::Home, |el| {
                                                    el.max_w(px(1440.))
                                                })
                                                .when(self.page != Page::Home, |el| {
                                                    el.max_w(px(880.))
                                                })
                                                .child(page),
                                        )
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
mod tests;
