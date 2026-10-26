//! The terminal seat: every terminal the Mac holds as a tab, and the menu that starts a new one.
//!
//! The Mac and the phone draw the same deck. Which terminals exist is the capability's answer, so
//! the deck asks for the list rather than keeping one of its own: a terminal opened on the phone is
//! a tab on the Mac the next time the Mac's seat loads.

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, MouseButton, SharedString, Subscription, Window,
};
use sjel_gpui_core::shell;
use sjel_gpui_core::terminal::{Host, Options, Terminal as Peer};

use super::{NewTerminal, NextTab, PrevTab, TerminalEvent, TerminalView, UNANSWERED};
use crate::theme::{Palette, TerminalFont};

/// The preset the menu starts with when the capability offers it.
const DEFAULT_PRESET: &str = "shell";

pub struct TerminalDeck {
    host: Host,
    touch: bool,
    cols: usize,
    rows: usize,
    tabs: Vec<(Entity<TerminalView>, Subscription)>,
    active: usize,
    /// The new-terminal menu while it is open, and the preset it will start.
    menu: Option<Options>,
    preset: String,
    /// Why there is no tab, or what is being waited for, in the words the empty seat shows.
    note: Option<SharedString>,
    style: Option<(Palette, TerminalFont, u8)>,
    focus: FocusHandle,
    /// A tab was added or chosen: the next frame gives its grid the keys.
    focus_pending: bool,
}

impl EventEmitter<TerminalEvent> for TerminalDeck {}

impl Focusable for TerminalDeck {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        match self.tabs.get(self.active) {
            Some((view, _)) => view.read(cx).focus_handle(cx),
            None => self.focus.clone(),
        }
    }
}

impl TerminalDeck {
    /// A deck over `host`, sized `cols` by `rows` until its grid measures itself. `touch` draws
    /// the phone's key row in each tab.
    pub fn new(host: Host, touch: bool, cols: usize, rows: usize, cx: &mut Context<Self>) -> Self {
        Self {
            host,
            touch,
            cols,
            rows,
            tabs: Vec::new(),
            active: 0,
            menu: None,
            preset: DEFAULT_PRESET.to_owned(),
            note: None,
            style: None,
            focus: cx.focus_handle(),
            focus_pending: false,
        }
    }

    /// The palette and font every tab draws with. The host hands them in on every frame.
    pub fn set_style(&mut self, palette: Palette, font: TerminalFont, size: u8) {
        self.style = Some((palette, font, size));
    }

    /// Every live terminal as a tab, oldest first with the newest chosen, or a new login shell
    /// when none is running.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.note = Some("Opening a terminal on the Mac…".into());
        let host = self.host.clone();
        let (cols, rows) = (self.cols as u16, self.rows as u16);
        let work = shell::run(async move {
            let live: Vec<Peer> = host
                .terminals()
                .await?
                .into_iter()
                .filter(|peer| peer.alive)
                .collect();
            if !live.is_empty() {
                return Ok(live);
            }
            host.attach_or_open(cols, rows).await.map(|peer| vec![peer])
        });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or_else(|| Err(UNANSWERED.into()));
            let _ = weak.update(cx, |deck, cx| {
                match result {
                    // The list is newest first; tabs read left to right in the order they began.
                    Ok(peers) => {
                        for peer in peers.into_iter().rev() {
                            deck.adopt(peer, cx);
                        }
                        deck.note = None;
                    }
                    Err(why) => {
                        deck.note = Some(format!("The terminal did not open: {why}").into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A tab for a terminal the capability already holds, chosen and attached.
    pub fn adopt(&mut self, peer: Peer, cx: &mut Context<Self>) {
        let palette = self
            .style
            .map_or_else(Palette::dark, |(palette, _, _)| palette);
        let (cols, rows, touch) = (self.cols, self.rows, self.touch);
        let host = self.host.clone();
        let view = cx.new(|cx| {
            let view = TerminalView::new(peer, host, 0, cols, rows, palette, cx);
            if touch {
                view.touch()
            } else {
                view
            }
        });
        view.update(cx, |view, cx| view.attach(cx));
        // The tab's own events are the host's: the phone raises its keyboard, and a pinch
        // changes a setting only the host can save.
        let subscription = cx.subscribe(&view, |_, _, event: &TerminalEvent, cx| match event {
            TerminalEvent::Editing(open) => cx.emit(TerminalEvent::Editing(*open)),
            TerminalEvent::Zoom(steps) => cx.emit(TerminalEvent::Zoom(*steps)),
        });
        self.tabs.push((view, subscription));
        self.active = self.tabs.len() - 1;
        self.focus_pending = true;
        cx.notify();
    }

    /// Starts `preset` in `cwd` and opens it as a new tab.
    fn open(&mut self, cwd: String, cx: &mut Context<Self>) {
        self.menu = None;
        let host = self.host.clone();
        let preset = self.preset.clone();
        let (cols, rows) = (self.cols as u16, self.rows as u16);
        let work = shell::run(async move { host.open_row(&preset, Some(&cwd), cols, rows).await });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or_else(|| Err(UNANSWERED.into()));
            let _ = weak.update(cx, |deck, cx| match result {
                Ok(peer) => {
                    deck.note = None;
                    deck.adopt(peer, cx);
                }
                Err(why) => {
                    deck.note = Some(format!("The terminal did not open: {why}").into());
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
            return;
        }
        let host = self.host.clone();
        let work = shell::run(async move { host.options().await });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or_else(|| Err(UNANSWERED.into()));
            let _ = weak.update(cx, |deck, cx| {
                match result {
                    Ok(options) => {
                        if !options.presets.contains(&deck.preset) {
                            deck.preset = options
                                .presets
                                .first()
                                .cloned()
                                .unwrap_or_else(|| DEFAULT_PRESET.to_owned());
                        }
                        deck.menu = Some(options);
                    }
                    Err(why) => {
                        deck.note = Some(format!("The Mac did not list presets: {why}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The tab `by` places along, wrapping at both ends.
    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let count = self.tabs.len() as isize;
        if count > 1 {
            let next = (self.active as isize + by).rem_euclid(count) as usize;
            self.choose(next, cx);
        }
    }

    fn choose(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.tabs.len() {
            self.active = ix;
            self.focus_pending = true;
            cx.notify();
        }
    }

    /// Removes a tab. A program still running is ended first: a tab is the only way back to it
    /// from this client.
    fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        let (view, _) = self.tabs.remove(ix);
        if view.read(cx).peer().alive {
            view.update(cx, |view, cx| view.close(cx));
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
        self.focus_pending = true;
        cx.notify();
    }

    fn tab_strip(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let tabs = self.tabs.iter().enumerate().map(|(ix, (view, _))| {
            let peer = view.read(cx).peer();
            let label = format!("{} · {}", peer.label(), folder_label(&peer.cwd));
            let active = ix == self.active;
            let alive = peer.alive;
            div()
                .id(("terminal-tab", ix))
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .pl_2()
                .pr_1()
                .py(px(3.))
                .rounded(px(6.))
                .text_xs()
                .cursor_pointer()
                .when(active, |tab| {
                    tab.bg(rgba(palette.raised))
                        .text_color(rgb(palette.text))
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!active, |tab| tab.text_color(rgb(palette.muted)))
                .when(!alive, |tab| tab.opacity(0.5))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |deck, _, _, cx| deck.choose(ix, cx)),
                )
                .child(SharedString::from(label))
                .child(
                    div()
                        .id(("terminal-tab-close", ix))
                        .px_1()
                        .rounded(px(4.))
                        .text_color(rgb(palette.muted))
                        .child("×")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |deck, _, _, cx| {
                                cx.stop_propagation();
                                deck.close_tab(ix, cx);
                            }),
                        ),
                )
        });
        div()
            .id("terminal-tabs")
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .pt_2()
            .overflow_x_scroll()
            .children(tabs)
            .child(
                div()
                    .id("terminal-new")
                    .flex_none()
                    .px_2()
                    .py(px(3.))
                    .rounded(px(6.))
                    .text_xs()
                    .cursor_pointer()
                    .text_color(rgb(palette.muted))
                    .when(self.menu.is_some(), |button| {
                        button.bg(rgba(palette.raised))
                    })
                    .child("+ New")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|deck, _, _, cx| deck.toggle_menu(cx)),
                    ),
            )
            .into_any_element()
    }

    /// The new-terminal menu: the program to start, then the folder to start it in.
    fn menu(&self, options: &Options, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let presets = options.presets.iter().map(|preset| {
            let chosen = *preset == self.preset;
            let name = preset.clone();
            div()
                .id(SharedString::from(format!("terminal-preset-{preset}")))
                .px_2()
                .py(px(3.))
                .rounded(px(6.))
                .text_xs()
                .cursor_pointer()
                .when(chosen, |chip| {
                    chip.bg(rgb(palette.accent)).text_color(rgb(0xffffff))
                })
                .when(!chosen, |chip| {
                    chip.bg(rgba(palette.raised)).text_color(rgb(palette.muted))
                })
                .child(SharedString::from(preset.clone()))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |deck, _, _, cx| {
                        deck.preset = name.clone();
                        cx.notify();
                    }),
                )
        });
        let folders = options.folders.iter().enumerate().map(|(ix, folder)| {
            let path = folder.clone();
            div()
                .id(("terminal-folder", ix))
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .px_2()
                .py(px(5.))
                .rounded(px(6.))
                .text_sm()
                .cursor_pointer()
                .hover(|row| row.bg(rgba(palette.raised)))
                .child(SharedString::from(folder_label(folder)))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(SharedString::from(folder.clone())),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |deck, _, _, cx| deck.open(path.clone(), cx)),
                )
        });
        div()
            .mx_3()
            .mt_2()
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .child(div().flex().flex_wrap().gap_1().children(presets))
            .child(
                div()
                    .id("terminal-folders")
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(folders),
            )
            .into_any_element()
    }
}

/// A folder as a tab or a menu row names it: `~` for home, its last component otherwise.
fn folder_label(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if path.is_empty() || (!home.is_empty() && path.trim_end_matches('/') == home) {
        return "~".to_owned();
    }
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_owned()
}

impl Render for TerminalDeck {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .style
            .map_or_else(Palette::dark, |(palette, _, _)| palette);
        let active = self.tabs.get(self.active).map(|(view, _)| view.clone());
        if let (Some(view), Some((palette, font, size))) = (&active, self.style) {
            view.update(cx, |view, cx| view.set_style(palette, font, size, cx));
        }
        if self.focus_pending {
            self.focus_pending = false;
            if let Some(view) = &active {
                window.focus(&view.read(cx).focus_handle(cx), cx);
            }
        }
        let strip = self.tab_strip(palette, cx);
        let menu = self
            .menu
            .as_ref()
            .map(|options| self.menu(options, palette, cx));
        let body = match active {
            Some(view) => view.into_any_element(),
            None => div()
                .p_4()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child(self.note.clone().unwrap_or_else(|| {
                    SharedString::from("No terminal is running. + New starts one.")
                }))
                .into_any_element(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .on_action(cx.listener(|deck, _: &NewTerminal, _, cx| {
                if deck.menu.is_none() {
                    deck.toggle_menu(cx);
                }
            }))
            .on_action(cx.listener(|deck, _: &NextTab, _, cx| deck.step(1, cx)))
            .on_action(cx.listener(|deck, _: &PrevTab, _, cx| deck.step(-1, cx)))
            .child(strip)
            .children(menu)
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_named_by_its_last_component_and_home_is_a_tilde() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/someone".into());
        assert_eq!(folder_label(&home), "~");
        assert_eq!(folder_label(""), "~");
        assert_eq!(folder_label(&format!("{home}/Developer/Sjel")), "Sjel");
        assert_eq!(folder_label("/tmp/work/"), "work");
    }
}
