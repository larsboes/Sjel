//! Right-side panel dock (Terminal, Agent, Activity, Soundscape) and soundscape slide-over panel.

use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, Focusable, MouseButton,
    SharedString, Window,
};
use sjel_gpui_core::terminal::Host;
use sjel_gpui_ui::sidebar::{sidebar_frame, Seat};
use sjel_gpui_ui::terminal::{TerminalDeck, TerminalEvent};
use sjel_gpui_ui::theme::{step_terminal_font_size, Palette};

use super::{AppearanceSettings, Workspace, TERMINAL_COLS, TERMINAL_ROWS};

impl Workspace {
    /// The panel, created on first use. Separate from [`Self::open_soundscape`] because the sidebar
    /// and the palette both want it without asking for it to appear.
    pub(super) fn soundscape_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<crate::soundscape::Soundscape> {
        if let Some(panel) = &self.soundscape_panel {
            return panel.clone();
        }
        let repo = self.repo.clone();
        let panel = cx.new(|cx| crate::soundscape::new(repo, cx));
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

    /// Open or focus the terminal drawer.
    pub(super) fn open_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.right_panel.open_on(Seat::Terminal);
        cx.notify();
        if let Some(deck) = &self.terminal {
            window.focus(&Focusable::focus_handle(deck, cx), cx);
            return;
        }
        let Ok(shell) = self.repo.shell() else {
            self.terminal_note = Some("Sjel is not running on this Mac.".into());
            return;
        };
        let deck = cx.new(|cx| {
            TerminalDeck::new(Host::Shell(shell), false, TERMINAL_COLS, TERMINAL_ROWS, cx)
        });
        cx.subscribe(&deck, |workspace, _, event: &TerminalEvent, cx| {
            if let TerminalEvent::Zoom(steps) = event {
                let settings = workspace.appearance_settings;
                let terminal_font_size =
                    step_terminal_font_size(settings.terminal_font_size, *steps);
                workspace.set_appearance(
                    AppearanceSettings {
                        terminal_font_size,
                        ..settings
                    },
                    cx,
                );
            }
        })
        .detach();
        deck.update(cx, |deck, cx| deck.load(cx));
        self.terminal = Some(deck);
    }

    /// Renders the docked right edge panel if open.
    pub(super) fn render_right_panel(
        &mut self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.right_panel.is_open() {
            return None;
        }
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
                Some(deck) => {
                    let settings = self.appearance_settings;
                    deck.update(cx, |deck, _| {
                        deck.set_style(
                            palette,
                            settings.terminal_font,
                            settings.terminal_font_size,
                        )
                    });
                    deck.clone().into_any_element()
                }
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
                .relative()
                .w(px(self.right_panel.width()))
                .flex_none()
                .h_full()
                .border_l_1()
                .border_color(rgba(palette.border))
                .child(sidebar_frame(palette, seat, body, on_seat, on_close))
                .child(
                    div()
                        .id("edge-resize")
                        .absolute()
                        .top_0()
                        .left(px(-3.))
                        .w(px(6.))
                        .h_full()
                        .cursor_col_resize()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|view, _, _, cx| {
                                view.resizing_edge = true;
                                cx.stop_propagation();
                            }),
                        ),
                )
                .into_any_element(),
        )
    }
}
