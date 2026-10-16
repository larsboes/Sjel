//! The main window: sidebar, page header, the current page, the command palette and toasts.

use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{IconName, IndexPath, Root, Sizable, Theme, WindowExt};
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, Action, App, Context, FocusHandle, Focusable,
    FontWeight, KeyBinding, SharedString, Subscription, Window,
};

use crate::route::{self, Destination, Page, Target};
use crate::selection::{Item, Selection};
use crate::shell::{self, AgentView, CalendarEntry, NodeHealth, Shell, ShellError, ShellResult};
use crate::theme::Palette;
use crate::{capabilities, home};

actions!(
    sjel,
    [
        /// Search pages and dashboard destinations.
        OpenPalette,
        GoHome,
        GoCapabilities,
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
    ]
);

/// Workspace key bindings. Navigation keys bind to the "Workspace" context, which only the page
/// frame carries, so they stay quiet while the palette's query field has focus.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-k", OpenPalette, None),
        KeyBinding::new("cmd-1", GoHome, None),
        KeyBinding::new("cmd-2", GoCapabilities, None),
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("cmd-d", OpenInDashboard, None),
        KeyBinding::new("j", SelectNext, Some("Workspace")),
        KeyBinding::new("down", SelectNext, Some("Workspace")),
        KeyBinding::new("k", SelectPrevious, Some("Workspace")),
        KeyBinding::new("up", SelectPrevious, Some("Workspace")),
        KeyBinding::new("enter", RunPrimary, Some("Workspace")),
        KeyBinding::new("cmd-backspace", RunSecondary, Some("Workspace")),
        KeyBinding::new("escape", ClearSelection, Some("Workspace")),
    ]);
}

const SIDEBAR_WIDTH: f32 = 208.;
/// The gap between the glass sidebar and the window edge.
const GLASS_INSET: f32 = 8.;
/// How often the open window reads the shell again.
const POLL: Duration = Duration::from_secs(10);
/// Data older than this shows its age.
pub const STALE_AFTER: Duration = Duration::from_secs(30);

/// Remote data: the last good value survives a failed refresh, so the page does not blank.
pub struct Remote<T> {
    pub value: Option<T>,
    pub error: Option<ShellError>,
    pub loaded_at: Option<Instant>,
    pub loading: bool,
}

impl<T> Default for Remote<T> {
    fn default() -> Self {
        Self {
            value: None,
            error: None,
            loaded_at: None,
            loading: false,
        }
    }
}

impl<T> Remote<T> {
    fn settle(&mut self, result: ShellResult<T>) {
        self.loading = false;
        match result {
            Ok(value) => {
                self.value = Some(value);
                self.error = None;
                self.loaded_at = Some(Instant::now());
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Never loaded yet and loading now: reserve space, show no error.
    pub fn first_load(&self) -> bool {
        self.value.is_none() && self.error.is_none()
    }
}

pub struct Workspace {
    pub page: Page,
    shell: Result<Arc<Shell>, ShellError>,
    pub health: Remote<NodeHealth>,
    pub agent: Remote<AgentView>,
    pub calendar: Remote<Vec<CalendarEntry>>,
    /// Approval ids and capability names with a request in flight.
    pub busy: HashSet<String>,
    pub selection: Selection,
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
        let mut workspace = Self {
            page: Page::Home,
            shell: Shell::new(endpoint).map(Arc::new),
            health: Remote::default(),
            agent: Remote::default(),
            calendar: Remote::default(),
            busy: HashSet::new(),
            selection: Selection::default(),
            focus,
            _appearance: Some(appearance),
            frames: crate::frames::FrameProbe::from_env(),
            #[cfg(target_os = "macos")]
            native_glass: None,
            #[cfg(target_os = "macos")]
            native_glass_attempted: false,
        };
        workspace.refresh(cx);
        if let (Some(frames), true) = (
            workspace.frames.clone(),
            std::env::var_os("SJEL_GPUI_BENCH_SCROLL").is_some(),
        ) {
            crate::frames::drive_scroll_bench(frames, workspace.selection.scroll.clone(), cx);
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
        match &self.shell {
            Ok(shell) => shell.endpoint().to_owned().into(),
            Err(error) => error.to_string().into(),
        }
    }

    pub fn go(&mut self, page: Page, cx: &mut Context<Self>) {
        if self.page != page {
            self.page = page;
            self.selection.reset();
        }
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx, |w| &mut w.health, |s| async move { s.health().await });
        self.load(cx, |w| &mut w.agent, |s| async move { s.agent().await });
        let (from, to) = crate::calendar::range(chrono::Local::now().naive_local());
        self.load(
            cx,
            |w| &mut w.calendar,
            move |s| async move { s.calendar(&from, &to).await },
        );
    }

    fn load<T, F, Fut>(
        &mut self,
        cx: &mut Context<Self>,
        slot: fn(&mut Self) -> &mut Remote<T>,
        request: F,
    ) where
        T: Send + 'static,
        F: FnOnce(Arc<Shell>) -> Fut,
        Fut: Future<Output = ShellResult<T>> + Send + 'static,
    {
        let shell = match &self.shell {
            Ok(shell) => shell.clone(),
            Err(error) => {
                let error = error.clone();
                slot(self).settle(Err(error));
                return;
            }
        };
        let remote = slot(self);
        if remote.loading {
            return;
        }
        remote.loading = true;
        let work = shell::run(request(shell));
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                slot(view).settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs one write against the shell with `key` marked busy, then toasts the outcome and
    /// refreshes.
    fn act<F, Fut>(
        &mut self,
        key: String,
        done: &'static str,
        request: F,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(Arc<Shell>) -> Fut,
        Fut: Future<Output = ShellResult<()>> + Send + 'static,
    {
        let shell = match &self.shell {
            Ok(shell) => shell.clone(),
            Err(error) => {
                window.push_notification(Notification::error(error.to_string()), cx);
                return;
            }
        };
        if !self.busy.insert(key.clone()) {
            return;
        }
        cx.notify();
        let work = shell::run(request(shell));
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update_in(cx, |view, window, cx| {
                view.busy.remove(&key);
                match result {
                    Ok(()) => window.push_notification(Notification::success(done), cx),
                    Err(error) => {
                        window.push_notification(Notification::error(error.to_string()), cx)
                    }
                }
                view.refresh(cx);
            });
        })
        .detach();
    }

    pub fn decide(&mut self, id: String, allow: bool, window: &mut Window, cx: &mut Context<Self>) {
        let done = if allow {
            "Allowed the agent write."
        } else {
            "Denied the agent write."
        };
        let request_id = id.clone();
        self.act(
            id,
            done,
            move |s| async move { s.decide(&request_id, allow).await },
            window,
            cx,
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
        self.act(
            name,
            done,
            move |s| async move { s.set_running(&request_name, running).await },
            window,
            cx,
        );
    }

    /// Opens `path` in the browser, logged in with a single-use ticket.
    pub fn open_dashboard(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let shell = match &self.shell {
            Ok(shell) => shell.clone(),
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
                    }
                }));
            let actions = CommandGroup::new().label("Actions").items([
                CommandItem::new()
                    .label("Refresh")
                    .icon(IconName::RotateCw)
                    .action(Refresh.boxed_clone()),
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

    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
    }

    fn on_open_in_dashboard(
        &mut self,
        _: &OpenInDashboard,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_dashboard(self.page.href().to_owned(), window, cx);
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

    /// `SJEL_GPUI_GLASS=1`: Liquid Glass under the sidebar, when this macOS has it. AppKit then
    /// paints the tint under the sidebar, and this returns true so GPUI tints only the page.
    #[cfg(target_os = "macos")]
    fn attach_glass(&mut self, window: &mut Window, tint: u32) -> bool {
        if !self.native_glass_attempted && std::env::var_os("SJEL_GPUI_GLASS").is_some() {
            self.native_glass_attempted = true;
            self.native_glass = crate::macos_glass::NativeGlassRegion::attach(
                window,
                SIDEBAR_WIDTH.into(),
                GLASS_INSET.into(),
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
    fn attach_glass(&mut self, _: &mut Window, _: u32) -> bool {
        false
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(frames) = &self.frames {
            frames.render_started();
        }
        let palette = Palette::for_appearance(window.appearance());
        let tint = if opaque_window() {
            palette.window_tint | 0xff
        } else {
            palette.window_tint
        };
        let glass = self.attach_glass(window, tint);
        let page = match self.page {
            Page::Home => home::render(self, palette, cx).into_any_element(),
            Page::Capabilities => capabilities::render(self, palette, cx).into_any_element(),
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
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(Self::on_open_in_dashboard))
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_select_previous))
            .on_action(cx.listener(Self::on_clear_selection))
            .on_action(cx.listener(Self::on_run_primary))
            .on_action(cx.listener(Self::on_run_secondary))
            .size_full()
            .when(!glass, |root| root.bg(rgba(tint)))
            .text_color(rgb(palette.text))
            .child(
                div()
                    .id("frame")
                    .track_focus(&self.focus)
                    .key_context("Workspace")
                    .size_full()
                    .flex()
                    .child(
                        div()
                            .w(px(SIDEBAR_WIDTH))
                            .flex_none()
                            .h_full()
                            .when(glass, |column| column.p(px(GLASS_INSET)))
                            .child(sidebar(self, palette, glass, window, cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .when(glass, |page| page.bg(rgba(tint)))
                            .flex()
                            .flex_col()
                            .child(header(self, palette))
                            .child(
                                div()
                                    .id("page")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .track_scroll(&scroll)
                                    .vertical_scrollbar(&scroll)
                                    .child(div().px_8().pb_8().max_w(px(880.)).child(page)),
                            ),
                    ),
            )
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
            .children(self.frames.as_ref().map(|frames| frames.paint_marker()))
    }
}

fn sidebar(
    view: &Workspace,
    palette: Palette,
    glass: bool,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let status = match (&view.health.value, &view.health.error) {
        (_, Some(error)) => (palette.red, SharedString::from(error.to_string())),
        (Some(health), None) => (
            if health.down().is_empty() {
                palette.green
            } else {
                palette.amber
            },
            format!("{} · up {}", health.version, uptime(health.uptime_seconds)).into(),
        ),
        (None, None) => (palette.muted, "Connecting…".into()),
    };
    let nav = Page::ALL.into_iter().map(|page| {
        let selected = page == view.page;
        let shortcut = match page {
            Page::Home => Kbd::binding_for_action(&GoHome, None, window),
            Page::Capabilities => Kbd::binding_for_action(&GoCapabilities, None, window),
        };
        div()
            .id(page.href())
            .flex()
            .items_center()
            .justify_between()
            .px_3()
            .py(px(6.))
            .rounded(px(7.))
            .when(selected, |row| row.bg(rgba(palette.raised)))
            .text_color(rgb(if selected {
                palette.text
            } else {
                palette.muted
            }))
            .cursor_pointer()
            .hover(move |style| style.bg(rgba(palette.raised)).text_color(rgb(palette.text)))
            .on_click(cx.listener(move |view, _, _, cx| view.go(page, cx)))
            .child(page.label())
            .children(shortcut.map(|kbd| kbd.appearance(false)))
    });

    div()
        .size_full()
        .flex()
        .flex_col()
        .gap_1()
        // Clears the transparent titlebar's traffic lights; the glass inset already moves it down.
        .pt(px(if glass { 52. - GLASS_INSET } else { 52. }))
        .px_3()
        .pb_4()
        .when(!glass, |sidebar| {
            sidebar
                .bg(rgba(palette.raised))
                .border_r_1()
                .border_color(rgba(palette.border))
        })
        .children(nav)
        .child(div().flex_1())
        .child(
            div()
                .id("palette-hint")
                .flex()
                .items_center()
                .justify_between()
                .px_3()
                .py(px(6.))
                .rounded(px(7.))
                .text_sm()
                .text_color(rgb(palette.muted))
                .cursor_pointer()
                .hover(move |style| style.bg(rgba(palette.raised)))
                .on_click(|_, window, cx| window.dispatch_action(OpenPalette.boxed_clone(), cx))
                .child("Search")
                .children(
                    Kbd::binding_for_action(&OpenPalette, None, window)
                        .map(|k| k.appearance(false)),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .pt_2()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(
                    div()
                        .flex_none()
                        .size(px(7.))
                        .rounded_full()
                        .bg(rgb(status.0)),
                )
                .child(div().truncate().child(status.1)),
        )
}

fn header(view: &Workspace, palette: Palette) -> impl IntoElement {
    let subtitle: SharedString = match view.page {
        Page::Home => chrono::Local::now().format("%A, %B %-d").to_string().into(),
        Page::Capabilities => view.endpoint(),
    };
    let checked = view
        .health
        .loaded_at
        .filter(|at| at.elapsed() > STALE_AFTER)
        .map(|at| format!("checked {} ago", ago(at.elapsed())));
    div()
        .flex()
        .items_end()
        .justify_between()
        .px_8()
        .pt(px(44.))
        .pb_5()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_2xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(view.page.label()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child(subtitle),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .children(checked.map(|label| {
                    div()
                        .text_xs()
                        .text_color(rgb(palette.amber))
                        .mr_2()
                        .child(label)
                }))
                .child(
                    Button::new("refresh")
                        .ghost()
                        .small()
                        .icon(IconName::RotateCw)
                        .loading(view.health.loading || view.agent.loading || view.calendar.loading)
                        .tooltip("Refresh (⌘R)")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Refresh.boxed_clone(), cx)
                        }),
                )
                .child(
                    Button::new("open-in-dashboard")
                        .ghost()
                        .small()
                        .icon(IconName::ExternalLink)
                        .tooltip("Open in the dashboard (⌘D)")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(OpenInDashboard.boxed_clone(), cx)
                        }),
                ),
        )
}

pub fn uptime(seconds: u64) -> String {
    match seconds {
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d {}h", s / 86_400, s % 86_400 / 3600),
    }
}

pub fn ago(elapsed: Duration) -> String {
    match elapsed.as_secs() {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h", s / 3600),
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
}
