//! The main window's life cycle and routing: it opens the window, reads and follows the shell,
//! decides which page is current, and owns the command palette, the selection and the toasts.
//!
//! What the window is made of lives beside it: `sidebar` draws the column and `header` the title
//! row, each reading this view and calling back into it.

use std::time::Duration;

use futures::StreamExt;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{IconName, IndexPath, Theme, WindowExt};
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, Action, App, Context, Entity, FocusHandle, Focusable,
    KeyBinding, ScrollHandle, SharedString, Subscription, Window, WindowAppearance,
};

use crate::ask::Ask;
use crate::backup::{self, BackupPage};
use crate::calendar_view::CalendarPage;
use crate::context_view::ContextPage;
use crate::data::{self, Busy, Remote, Repo};
use crate::documents::Documents;
use crate::feed::Feed;
use crate::finance_view::FinancePage;
use crate::inbox::Inbox;
use crate::inventory_view::InventoryPage;
use crate::people_view::PeoplePage;
use crate::projects::Projects;
use crate::route::{self, Destination, Page, Target};
use crate::selection::{Item, Selection};
use crate::shell::{
    self, AgentView, Backups, CalendarEntry, GlobalTask, NodeHealth, ShellError, Snapshot,
    SoundscapeState,
};
use crate::theme::{AppearanceSettings, GlassStyle, Palette};
use crate::{capabilities, header, home, sidebar};

actions!(
    sjel,
    [
        /// Search pages and dashboard destinations.
        OpenPalette,
        /// Focus the active page's search field or open the search palette.
        FocusSearch,
        /// Step to the next page in the sidebar.
        NextPage,
        /// Step to the previous page in the sidebar.
        PrevPage,
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
        /// Open the soundscape player window.
        OpenSoundscape,
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

/// Open a page. One action serves every page, so a page added to `Page::ALL` gets its ⌘ number,
/// sidebar hint, palette row and Go menu item from `nav_pages` without another edit.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = sjel, no_json)]
pub struct GoTo(pub Page);

/// The pages the sidebar, ⌘1–⌘9 and the Go menu list, in that order. Settings has ⌘, instead.
pub fn nav_pages() -> impl Iterator<Item = Page> {
    Page::ALL.into_iter().filter(|page| *page != Page::Settings)
}

fn page_icon(page: Page) -> IconName {
    match page {
        Page::Home | Page::Software => IconName::LayoutDashboard,
        Page::Context => IconName::Sun,
        Page::Capabilities => IconName::Cpu,
        Page::Systems => IconName::MemoryStick,
        Page::Projects | Page::Documents | Page::Import => IconName::Folder,
        Page::Ask => IconName::Bot,
        Page::Travel => IconName::Globe,
        Page::Feed => IconName::BookOpen,
        Page::Inbox => IconName::Inbox,
        Page::Calendar => IconName::Calendar,
        Page::Backup => IconName::HardDrive,
        Page::Finance => IconName::ChartPie,
        Page::People => IconName::CircleUser,
        Page::Inventory => IconName::Folder,
        Page::Settings => IconName::Settings,
    }
}

/// Workspace key bindings. Navigation keys bind to the "Workspace" context, which only the page
/// frame carries, so they stay quiet while the palette's query field has focus.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-k", OpenPalette, None),
        KeyBinding::new("cmd-f", FocusSearch, None),
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("cmd-d", OpenInDashboard, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-[", ClearSelection, None),
        KeyBinding::new("escape", ClearSelection, None),
        // Sequential page cycling
        KeyBinding::new("cmd-alt-right", NextPage, None),
        KeyBinding::new("shift-cmd-]", NextPage, None),
        KeyBinding::new("ctrl-tab", NextPage, None),
        KeyBinding::new("cmd-alt-left", PrevPage, None),
        KeyBinding::new("shift-cmd-[", PrevPage, None),
        KeyBinding::new("ctrl-shift-tab", PrevPage, None),
        // Soundscape ambient audio controls
        KeyBinding::new("cmd-alt-space", ToggleSoundscape, None),
        KeyBinding::new("shift-cmd-u", ToggleSoundscape, None),
        KeyBinding::new("cmd-alt-p", CycleSoundscapePreset, None),
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
        KeyBinding::new("g a", GoTo(Page::Ask), Some("Workspace")),
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
        KeyBinding::new("g ,", GoTo(Page::Settings), Some("Workspace")),
    ]);
    cx.bind_keys(
        nav_pages()
            .zip(1..=9)
            .map(|(page, n)| KeyBinding::new(&format!("cmd-{n}"), GoTo(page), None)),
    );
    cx.bind_keys([
        KeyBinding::new("cmd-0", GoTo(Page::Documents), None),
        KeyBinding::new("shift-cmd-d", GoTo(Page::Documents), None),
        KeyBinding::new("shift-cmd-b", GoTo(Page::Backup), None),
        KeyBinding::new("shift-cmd-f", GoTo(Page::Finance), None),
        KeyBinding::new("shift-cmd-p", GoTo(Page::People), None),
        KeyBinding::new("shift-cmd-i", GoTo(Page::Inventory), None),
        KeyBinding::new("shift-cmd-s", GoTo(Page::Software), None),
        KeyBinding::new("shift-cmd-c", GoTo(Page::Calendar), None),
        KeyBinding::new("shift-cmd-t", GoTo(Page::Travel), None),
        KeyBinding::new("shift-cmd-k", GoTo(Page::Projects), None),
        KeyBinding::new("shift-cmd-a", GoTo(Page::Ask), None),
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
    repo: Repo,
    pub health: Remote<NodeHealth>,
    pub agent: Remote<AgentView>,
    pub calendar: Remote<Vec<CalendarEntry>>,
    pub focus_tasks: Remote<Vec<GlobalTask>>,
    pub soundscape: Remote<SoundscapeState>,
    pub backups: Remote<Backups>,
    pub projects: Entity<Projects>,
    pub ask: Entity<Ask>,
    pub travel: Entity<sjel_gpui_ui::travel::Travel>,
    pub documents: Entity<Documents>,
    pub feed: Entity<Feed>,
    pub inbox: Entity<Inbox>,
    pub calendar_page: Entity<CalendarPage>,
    pub context_page: Entity<ContextPage>,
    pub finance_page: Entity<FinancePage>,
    pub people_page: Entity<PeoplePage>,
    pub backup_page: Entity<BackupPage>,
    pub inventory_page: Entity<InventoryPage>,
    pub software: Entity<crate::software::Software>,
    pub import: Entity<crate::import::ImportPage>,
    pub systems: Entity<crate::systems::SystemsPage>,
    /// Approval ids and capability names with a request in flight.
    pub busy: Busy,
    pub selection: Selection,
    /// The page list in the sidebar. At eighteen pages it is taller than a small window, so the
    /// column scrolls and the selected row is brought into view when the page changes.
    pub nav_scroll: ScrollHandle,
    /// True once the shell pushes its answers, so the timer stops reading them.
    streaming: bool,
    pub appearance_settings: AppearanceSettings,
    /// The section Settings shows.
    pub settings_section: crate::settings::Section,
    pub update_info: Option<crate::update::UpdateInfo>,
    pub rebuild_state: crate::update::RebuildState,
    focus: FocusHandle,
    /// Set when the page changes. The old page's field may have had focus and is gone from the
    /// tree, which would leave every shortcut with no path to this view.
    refocus: bool,
    _appearance: Option<Subscription>,
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
        let travel = cx.new(|_| {
            sjel_gpui_ui::travel::Travel::new(
                repo.shell().ok().map(sjel_gpui_core::travel::Source::Shell),
                sjel_gpui_ui::Density::Desktop,
            )
        });
        let documents = cx.new(|_| Documents::new(repo.clone()));
        let feed = cx.new(|cx| Feed::new(repo.clone(), cx));
        let inbox = cx.new(|cx| Inbox::new(repo.clone(), cx));
        let calendar_page = cx.new(|cx| CalendarPage::new(repo.clone(), cx));
        let context_page = cx.new(|cx| ContextPage::new(repo.clone(), cx));
        let finance_page = cx.new(|cx| FinancePage::new(repo.clone(), cx));
        let people_page = cx.new(|cx| PeoplePage::new(repo.clone(), cx));
        let backup_page = cx.new(|cx| BackupPage::new(repo.clone(), cx));
        let inventory_page = cx.new(|cx| InventoryPage::new(repo.clone(), cx));
        let software = cx.new(|_| crate::software::Software::new(repo.clone()));
        let import = cx.new(|_| crate::import::ImportPage::new(repo.clone()));
        let systems = cx.new(|_| crate::systems::SystemsPage::new(repo.clone()));
        let appearance_settings = AppearanceSettings::load();
        let mut workspace = Self {
            page: Page::Home,
            repo,
            health: Remote::default(),
            agent: Remote::default(),
            calendar: Remote::default(),
            focus_tasks: Remote::default(),
            soundscape: Remote::default(),
            backups: Remote::default(),
            projects,
            ask,
            travel,
            documents,
            feed,
            inbox,
            calendar_page,
            context_page,
            finance_page,
            people_page,
            backup_page,
            inventory_page,
            software,
            import,
            systems,
            busy: Busy::default(),
            selection: Selection::default(),
            nav_scroll: ScrollHandle::new(),
            streaming: false,
            focus,
            refocus: false,
            appearance_settings,
            settings_section: crate::settings::Section::default(),
            update_info: None,
            rebuild_state: crate::update::RebuildState::Idle,
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
            self.reveal_page(page, cx);
            if page == Page::Projects {
                self.projects
                    .update(cx, |projects, cx| projects.load_projects(cx));
            } else if page == Page::Ask {
                self.ask.update(cx, |ask, cx| ask.focus_input(cx));
            } else if page == Page::Travel {
                self.travel.update(cx, |travel, cx| travel.activate(cx));
            } else if page == Page::Backup {
                self.backup_page.update(cx, |b, cx| b.activate(cx));
            } else if page == Page::Software {
                self.software.update(cx, |software, cx| software.load(cx));
            } else if page == Page::Import {
                self.import.update(cx, |import, cx| import.load(cx));
            } else if page == Page::Systems {
                self.systems.update(cx, |systems, cx| systems.activate(cx));
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
            } else if page == Page::Context {
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

    /// Brings the open page's row to the top of the sidebar list.
    ///
    /// One request is not enough. A scroll target is resolved against the rows the viewport
    /// has painted, so the first request can only reach as far as the list had already been
    /// scrolled, and each further frame reaches a little further. Four frames covers the
    /// eighteen rows, and a list that fits its window is four no-ops.
    fn reveal_page(&mut self, page: Page, cx: &mut Context<Self>) {
        let Some(index) = nav_pages().position(|listed| listed == page) else {
            return;
        };
        cx.spawn(async move |weak, cx| {
            for _ in 0..4 {
                let requested = weak.update(cx, |view, cx| {
                    view.nav_scroll.scroll_to_top_of_item(index);
                    cx.notify();
                });
                if requested.is_err() {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;
            }
        })
        .detach();
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
        let (from, to) = crate::calendar::range(chrono::Local::now().naive_local());
        let repo = self.repo.clone();
        data::read_repo(
            cx,
            self,
            |w| &mut w.calendar,
            &repo,
            move |r| async move { r.fetch_calendar(&from, &to).await },
        );
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

    pub fn toggle_soundscape(
        &mut self,
        playing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet(if playing {
                "Soundscape started."
            } else {
                "Soundscape paused."
            }),
            move |s| async move { s.set_soundscape_playing(playing).await },
            |this, updated, cx| {
                if let Some(state) = updated {
                    this.soundscape.value = Some(state);
                    cx.notify();
                }
            },
        );
    }

    pub fn cycle_soundscape_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self
            .soundscape
            .value
            .as_ref()
            .map(|s| s.preset.as_str())
            .unwrap_or("edm");
        const PRESETS: [&str; 6] = ["edm", "ambient", "lofi", "focus", "relax", "sleep"];
        let next_idx = match PRESETS.iter().position(|&p| p == current) {
            Some(i) => (i + 1) % PRESETS.len(),
            None => 0,
        };
        let next = PRESETS[next_idx].to_string();
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet("Preset updated."),
            move |s| {
                let n = next.clone();
                async move { s.set_soundscape_preset(&n).await }
            },
            |this, updated, cx| {
                if let Some(state) = updated {
                    this.soundscape.value = Some(state);
                    cx.notify();
                }
            },
        );
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
        self.page = Page::Projects;
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
            Target::Ask(question) => {
                self.go(Page::Ask, cx);
                self.ask.update(cx, |ask, cx| ask.submit(question, cx));
            }
            Target::Dashboard(path) => self.open_dashboard(path, window, cx),
        }
    }

    fn open_palette(&mut self, _: &OpenPalette, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let state = cx.new(|cx| CommandState::new(window, cx));
        let this = cx.entity();
        let native: Vec<Page> = Page::ALL.to_vec();
        let web: Vec<Destination> = route::DESTINATIONS
            .iter()
            .copied()
            .filter(|d| d.page().is_none())
            .collect();
        let palette_state = state.clone();
        window.open_dialog(cx, move |dialog: Dialog, _, _| {
            let this = this.clone();
            let (native, web) = (native.clone(), web.clone());
            let pages = CommandGroup::new()
                .label("Go to")
                .items(native.iter().map(|&page| {
                    let item = CommandItem::new().label(page.label()).icon(page_icon(page));
                    match page {
                        Page::Settings => item.action(OpenSettings.boxed_clone()),
                        _ => item.action(GoTo(page).boxed_clone()),
                    }
                }));
            let actions = CommandGroup::new().label("Actions").items([
                CommandItem::new()
                    .label("Refresh")
                    .icon(IconName::RotateCw)
                    .action(Refresh.boxed_clone()),
                CommandItem::new()
                    .label("Check for App Updates")
                    .icon(IconName::RotateCw)
                    .action(CheckForUpdates.boxed_clone()),
                CommandItem::new()
                    .label("Rebuild Sjel Desktop")
                    .icon(IconName::RotateCw)
                    .action(RebuildApp.boxed_clone()),
                CommandItem::new()
                    .label("Appearance")
                    .icon(IconName::Palette)
                    .keywords(["theme", "dark", "light", "accent", "glass", "transparency"])
                    .action(OpenAppearance.boxed_clone()),
                CommandItem::new()
                    .label("Open this page in the dashboard")
                    .icon(IconName::ExternalLink)
                    .action(OpenInDashboard.boxed_clone()),
                CommandItem::new()
                    .label("Soundscape")
                    .icon(IconName::Play)
                    .keywords(["music", "audio", "focus", "ambient", "lofi", "player"])
                    .action(OpenSoundscape.boxed_clone()),
            ]);
            let dashboard = CommandGroup::new()
                .label("Open in dashboard")
                .items(web.iter().map(|d| {
                    CommandItem::new()
                        .label(d.label)
                        .icon(IconName::Globe)
                        .keywords([d.href])
                }));
            let on_confirm = {
                let web = web.clone();
                move |path: IndexPath, window: &mut Window, cx: &mut App| {
                    window.close_dialog(cx);
                    if path.section == 2 {
                        if let Some(d) = web.get(path.row) {
                            let href = d.href.to_owned();
                            this.update(cx, |view, cx| view.open_dashboard(href, window, cx));
                        }
                    }
                }
            };
            dialog.w(px(560.)).close_button(false).p_0().child(
                Command::new(&palette_state)
                    .bordered(false)
                    .placeholder("Go to a page or open the dashboard…")
                    .max_h(px(380.))
                    .group(pages)
                    .group(actions)
                    .group(dashboard)
                    .on_confirm(on_confirm)
                    .on_cancel(|window, cx| window.close_dialog(cx)),
            )
        });
        let query = state.read(cx).focus_handle(cx);
        window.focus(&query, cx);
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
        } else if self.page == Page::Context {
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

    /// The player is its own window, so this hands the gesture to `soundscape::open`.
    fn on_open_soundscape(&mut self, _: &OpenSoundscape, _: &mut Window, cx: &mut Context<Self>) {
        crate::soundscape::open(cx, crate::soundscape::Mode::Full);
    }

    /// Opens Settings, at `section` when one is given and at the last one shown otherwise.
    pub fn open_settings(
        &mut self,
        section: Option<crate::settings::Section>,
        cx: &mut Context<Self>,
    ) {
        if let Some(section) = section {
            self.settings_section = section;
        }
        if section == Some(crate::settings::Section::Updates) && self.update_info.is_none() {
            self.check_updates(cx);
        }
        self.go(Page::Settings, cx);
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
        let pages: Vec<Page> = nav_pages().collect();
        if let Some(pos) = pages.iter().position(|&p| p == self.page) {
            let next = pages[(pos + 1) % pages.len()];
            self.go(next, cx);
        } else if let Some(&first) = pages.first() {
            self.go(first, cx);
        }
    }

    fn on_prev_page(&mut self, _: &PrevPage, _: &mut Window, cx: &mut Context<Self>) {
        let pages: Vec<Page> = nav_pages().collect();
        if let Some(pos) = pages.iter().position(|&p| p == self.page) {
            let prev = pages[(pos + pages.len() - 1) % pages.len()];
            self.go(prev, cx);
        } else if let Some(&first) = pages.first() {
            self.go(first, cx);
        }
    }

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        match self.page {
            Page::Documents => {
                self.documents
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
            Some(Item::Entry(_)) => self.open_dashboard("/calendar".into(), window, cx),
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
        let glass_requested = self.appearance_settings.glass == GlassStyle::LiquidGlass;
        let glass = self.attach_glass(window, tint, glass_requested);
        if std::mem::take(&mut self.refocus) {
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
            Page::Import => self.import.clone().into_any_element(),
            Page::Systems => self.systems.clone().into_any_element(),
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
                self.inbox
                    .update(cx, |inbox, cx| inbox.set_palette(palette, cx));
                self.inbox.clone().into_any_element()
            }
            Page::Calendar => {
                self.calendar_page
                    .update(cx, |calendar, cx| calendar.set_palette(palette, cx));
                self.calendar_page.clone().into_any_element()
            }
            Page::Context => {
                self.context_page
                    .update(cx, |context, cx| context.set_palette(palette, cx));
                self.context_page.clone().into_any_element()
            }
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
            Page::Ask => self.ask.clone().into_any_element(),
            Page::Travel => {
                self.travel
                    .update(cx, |travel, cx| travel.set_palette(palette, cx));
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
            .on_action(cx.listener(Self::on_check_for_updates))
            .on_action(cx.listener(Self::on_rebuild_app))
            .size_full()
            .when(!glass, |root| root.bg(rgba(tint)))
            .text_color(rgb(palette.text))
            .child(
                div()
                    .id("frame")
                    .track_focus(&self.focus)
                    // Projects and Ask type into their fields, so J, K and Enter must reach them.
                    .when(
                        !matches!(
                            self.page,
                            Page::Projects
                                | Page::Ask
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
                                        | Page::Context
                                        | Page::Finance
                                        | Page::People
                                        | Page::Backup
                                        | Page::Inventory
                                ) {
                                    // Projects, Settings, Documents, Calendar, Context and Finance manage their own layout
                                    // and scroll boundaries. Documents, Calendar, Context and Finance need full width.
                                    div().id("page").flex_1().min_h_0().px_8().pb_8().child(
                                        div()
                                            .size_full()
                                            .when(
                                                !matches!(
                                                    self.page,
                                                    Page::Documents
                                                        | Page::Calendar
                                                        | Page::Context
                                                        | Page::Finance
                                                        | Page::People
                                                        | Page::Backup
                                                        | Page::Inventory
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
                            ),
                    ),
            )
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

    /// The real window shape: `Root` around the workspace. A non-loopback endpoint makes every
    /// request fail at once, so the test never touches the network.
    fn open(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
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
            view.page = Page::Capabilities;
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

    #[gpui::test]
    fn typing_in_the_palette_does_not_move_the_selection(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        cx.simulate_keystrokes("cmd-k j k j");
        view.read_with(cx, |view, _| assert_eq!(view.selection.cursor, None));
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
    fn the_first_nine_sidebar_pages_have_number_keys(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        for (page, n) in nav_pages().zip(1..=9) {
            cx.simulate_keystrokes(&format!("cmd-{n}"));
            view.read_with(cx, |view, _| assert_eq!(view.page, page));
        }
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
        cx.simulate_keystrokes("cmd-0");
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
    fn sequential_page_cycling(cx: &mut TestAppContext) {
        let (view, cx) = open(cx);
        view.update(cx, |view, cx| view.go(Page::Home, cx));
        cx.simulate_keystrokes("cmd-alt-right");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Context));
        cx.simulate_keystrokes("cmd-alt-left");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Home));
        // Backwards from the first page wraps to the last one the sidebar lists, and Systems
        // sits fourth in that history rather than at the end.
        cx.simulate_keystrokes("shift-cmd-[");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Import));
        cx.simulate_keystrokes("shift-cmd-[");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Software));
        view.update(cx, |view, cx| view.go(Page::Capabilities, cx));
        cx.simulate_keystrokes("shift-cmd-]");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Systems));
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
