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
    KeyBinding, SharedString, Subscription, Window, WindowAppearance,
};

use crate::ask::Ask;
use crate::data::{self, Busy, Remote, Repo};
use crate::projects::Projects;
use crate::route::{self, Destination, Page, Target};
use crate::selection::{Item, Selection};
use crate::shell::{
    self, AgentView, CalendarEntry, GlobalTask, NodeHealth, ShellError, Snapshot, SoundscapeState,
};
use crate::theme::{AppearanceSettings, GlassStyle, Palette};
use crate::{capabilities, header, home, sidebar};

actions!(
    sjel,
    [
        /// Search pages and dashboard destinations.
        OpenPalette,
        GoHome,
        GoCapabilities,
        GoProjects,
        GoAsk,
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
        /// Toggle the appearance and transparency panel.
        ToggleAppearancePanel,
        /// Toggle the application update popover.
        ToggleUpdatePopover,
        /// Check git repository for newer commits.
        CheckForUpdates,
        /// Trigger background rebuild and install of Sjel Desktop.
        RebuildApp,
    ]
);

/// Workspace key bindings. Navigation keys bind to the "Workspace" context, which only the page
/// frame carries, so they stay quiet while the palette's query field has focus.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-k", OpenPalette, None),
        KeyBinding::new("cmd-1", GoHome, None),
        KeyBinding::new("cmd-2", GoCapabilities, None),
        KeyBinding::new("cmd-3", GoProjects, None),
        KeyBinding::new("cmd-4", GoAsk, None),
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("cmd-d", OpenInDashboard, None),
        KeyBinding::new("j", SelectNext, Some("Workspace")),
        KeyBinding::new("down", SelectNext, Some("Workspace")),
        KeyBinding::new("k", SelectPrevious, Some("Workspace")),
        KeyBinding::new("up", SelectPrevious, Some("Workspace")),
        KeyBinding::new("enter", RunPrimary, Some("Workspace")),
        KeyBinding::new("space", RunPrimary, Some("Workspace")),
        KeyBinding::new("cmd-backspace", RunSecondary, Some("Workspace")),
        KeyBinding::new("escape", ClearSelection, Some("Workspace")),
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
    pub projects: Entity<Projects>,
    pub ask: Entity<Ask>,
    /// Approval ids and capability names with a request in flight.
    pub busy: Busy,
    pub selection: Selection,
    /// True once the shell pushes its answers, so the timer stops reading them.
    streaming: bool,
    pub show_appearance_panel: bool,
    pub appearance_settings: AppearanceSettings,
    pub update_info: Option<crate::update::UpdateInfo>,
    pub show_update_popover: bool,
    pub rebuild_state: crate::update::RebuildState,
    focus: FocusHandle,
    _appearance: Option<Subscription>,
    frames: Option<crate::frames::FrameProbe>,
    #[cfg(target_os = "macos")]
    native_glass: Option<crate::macos_glass::NativeGlassRegion>,
    #[cfg(target_os = "macos")]
    native_glass_attempted: bool,
}

impl Workspace {
    pub fn new(endpoint: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance = cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
            cx.notify();
        });
        Theme::sync_system_appearance(Some(window), cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let repo = Repo::new(shell::Shell::new(endpoint).map(std::sync::Arc::new));
        let projects = cx.new(|_| Projects::new(repo.clone()));
        let ask = cx.new(|_| Ask::new(repo.clone()));
        let appearance_settings = AppearanceSettings::load();
        let mut workspace = Self {
            page: Page::Home,
            repo,
            health: Remote::default(),
            agent: Remote::default(),
            calendar: Remote::default(),
            focus_tasks: Remote::default(),
            soundscape: Remote::default(),
            projects,
            ask,
            busy: Busy::default(),
            selection: Selection::default(),
            streaming: false,
            focus,
            show_appearance_panel: false,
            appearance_settings,
            update_info: None,
            show_update_popover: false,
            rebuild_state: crate::update::RebuildState::Idle,
            _appearance: Some(appearance),
            frames: crate::frames::FrameProbe::from_env(),
            #[cfg(target_os = "macos")]
            native_glass: None,
            #[cfg(target_os = "macos")]
            native_glass_attempted: false,
        };
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
            self.page = page;
            self.selection.reset();
            if page == Page::Projects {
                self.projects
                    .update(cx, |projects, cx| projects.load_projects(cx));
            } else if page == Page::Ask {
                self.ask.update(cx, |ask, cx| ask.focus_input(cx));
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
        self.show_update_popover = true;
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
            Target::Native(page) => self.go(page, cx),
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
                .items(native.iter().map(|page| {
                    let item = CommandItem::new().label(page.label());
                    match page {
                        Page::Home => item
                            .icon(IconName::LayoutDashboard)
                            .action(GoHome.boxed_clone()),
                        Page::Capabilities => item
                            .icon(IconName::Cpu)
                            .action(GoCapabilities.boxed_clone()),
                        Page::Projects => {
                            item.icon(IconName::Folder).action(GoProjects.boxed_clone())
                        }
                        Page::Ask => item.icon(IconName::Bot).action(GoAsk.boxed_clone()),
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
                    .label("Appearance & Transparency")
                    .icon(IconName::Palette)
                    .action(ToggleAppearancePanel.boxed_clone()),
                CommandItem::new()
                    .label("Open this page in the dashboard")
                    .icon(IconName::ExternalLink)
                    .action(OpenInDashboard.boxed_clone()),
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

    fn on_go_home(&mut self, _: &GoHome, _: &mut Window, cx: &mut Context<Self>) {
        self.go(Page::Home, cx);
    }

    fn on_go_capabilities(&mut self, _: &GoCapabilities, _: &mut Window, cx: &mut Context<Self>) {
        self.go(Page::Capabilities, cx);
    }

    fn on_go_projects(&mut self, _: &GoProjects, _: &mut Window, cx: &mut Context<Self>) {
        self.go(Page::Projects, cx);
    }

    fn on_go_ask(&mut self, _: &GoAsk, _: &mut Window, cx: &mut Context<Self>) {
        self.go(Page::Ask, cx);
    }

    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
        if self.page == Page::Projects {
            self.projects
                .update(cx, |projects, cx| projects.load_projects(cx));
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

    fn on_toggle_appearance_panel(
        &mut self,
        _: &ToggleAppearancePanel,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_appearance_panel = !self.show_appearance_panel;
        cx.notify();
    }

    fn on_toggle_update_popover(
        &mut self,
        _: &ToggleUpdatePopover,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_update_popover = !self.show_update_popover;
        cx.notify();
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
                if let Some(ref update) = info {
                    if update.is_update_available() {
                        view.show_update_popover = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn on_rebuild_app(&mut self, _: &RebuildApp, window: &mut Window, cx: &mut Context<Self>) {
        self.trigger_rebuild(window, cx);
    }

    fn on_select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.selection.step(1, self.items().len());
        cx.notify();
    }

    fn on_select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.selection.step(-1, self.items().len());
        cx.notify();
    }

    fn on_clear_selection(&mut self, _: &ClearSelection, _: &mut Window, cx: &mut Context<Self>) {
        self.selection.clear();
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
        let page = match self.page {
            Page::Home => home::render(self, palette, cx).into_any_element(),
            Page::Capabilities => capabilities::render(self, palette, cx).into_any_element(),
            Page::Projects => self.projects.clone().into_any_element(),
            Page::Ask => self.ask.clone().into_any_element(),
        };

        if let Some(frames) = &self.frames {
            frames.render_finished();
        }
        let scroll = self.selection.scroll.clone();
        // Actions sit on the outer element, so the palette (a child of it) can dispatch them.
        // Focus and the "Workspace" key context sit on the inner frame only, so J and K never
        // fire while a palette query has focus.
        div()
            .id("workspace")
            .when_some(self.frames.clone(), |root, frames| {
                root.on_scroll_wheel(move |_, _, _| frames.input())
            })
            .on_action(cx.listener(Self::open_palette))
            .on_action(cx.listener(Self::on_go_home))
            .on_action(cx.listener(Self::on_go_capabilities))
            .on_action(cx.listener(Self::on_go_projects))
            .on_action(cx.listener(Self::on_go_ask))
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(Self::on_open_in_dashboard))
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_select_previous))
            .on_action(cx.listener(Self::on_clear_selection))
            .on_action(cx.listener(Self::on_run_primary))
            .on_action(cx.listener(Self::on_run_secondary))
            .on_action(cx.listener(Self::on_toggle_appearance_panel))
            .on_action(cx.listener(Self::on_toggle_update_popover))
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
                    .when(!matches!(self.page, Page::Projects | Page::Ask), |frame| {
                        frame.key_context("Workspace")
                    })
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
                            .h_full()
                            .when(glass, |page| page.bg(rgba(tint)))
                            .flex()
                            .flex_col()
                            .child(header::render(self, palette))
                            .when(self.show_appearance_panel, |col| {
                                let entity = cx.entity().clone();
                                let change_entity = entity.clone();
                                col.child(div().absolute().top(px(85.)).right(px(32.)).child(
                                    crate::ui::appearance_panel(
                                        self.appearance_settings,
                                        palette,
                                        move |new_settings, _, app| {
                                            let _ = new_settings.save();
                                            change_entity.update(app, |view, cx| {
                                                view.appearance_settings = new_settings;
                                                cx.notify();
                                            });
                                        },
                                        move |_, app| {
                                            entity.update(app, |view, cx| {
                                                view.show_appearance_panel = false;
                                                cx.notify();
                                            });
                                        },
                                    ),
                                ))
                            })
                            .when(self.show_update_popover, |col| {
                                if let Some(info) = &self.update_info {
                                    let entity = cx.entity().clone();
                                    let rebuild_entity = entity.clone();
                                    let close_entity = entity.clone();
                                    let info_clone = info.clone();
                                    let state = self.rebuild_state.clone();
                                    col.child(div().absolute().top(px(85.)).right(px(32.)).child(
                                        crate::update::update_popover(
                                            &info_clone,
                                            &state,
                                            palette,
                                            move |window, app| {
                                                rebuild_entity.update(app, |view, cx| {
                                                    view.trigger_rebuild(window, cx);
                                                });
                                            },
                                            move |_, app| {
                                                close_entity.update(app, |view, cx| {
                                                    view.show_update_popover = false;
                                                    cx.notify();
                                                });
                                            },
                                            move |_, _| {
                                                crate::update::relaunch_app();
                                            },
                                        ),
                                    ))
                                } else {
                                    col
                                }
                            })
                            .child(if self.page == Page::Projects {
                                // Projects scrolls inside itself: its editor lays out only the
                                // blocks on screen, which needs a bounded height.
                                div()
                                    .id("page")
                                    .flex_1()
                                    .min_h_0()
                                    .px_8()
                                    .pb_8()
                                    .child(div().size_full().max_w(px(880.)).child(page))
                            } else {
                                div()
                                    .id("page")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .track_scroll(&scroll)
                                    .vertical_scrollbar(&scroll)
                                    .child(div().px_8().pb_8().max_w(px(880.)).child(page))
                            }),
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
}
