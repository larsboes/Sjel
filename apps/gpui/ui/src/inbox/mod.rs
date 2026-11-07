//! Inbox: one field for the thing in your hand, and the list of what you already kept.
//!
//! The phone is a capture device first (research/offline-on-the-phone.md). This page is the field
//! that catches a thought, a link or a price while somebody is walking, with one chip for the four
//! tags the comms capability knows. It writes into the phone's own replica and queues the capture
//! for the Mac, so it works with no Mac in reach — and the row says which of the two it is looking
//! at, because a capture that looks sent and is not is worse than one that says it is waiting.
//!
//! Triage is not here. What the Mac does with a capture — a wishlist item, a feed source, a place —
//! is `crates/comms`'s decision, and the preview under the field only says where it will land.

use std::sync::Arc;

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, EventEmitter, FontWeight, MouseButton,
    Render, SharedString, Window,
};
use gpui_kit::component::input::Input;
use sjel_gpui_core::shell;
use sjel_gpui_core::store::{LocalCapture, TursoStore};

use crate::theme::ThemeContext;
use crate::{Density, Palette};

pub mod capture;
pub mod field;
pub mod triage;
pub mod types;

pub use capture::capture_tag_pill;
pub use triage::{
    bind_keys as bind_triage_keys, Filter as TriageFilter, Inbox as TriageInbox,
    InboxEvent as TriageInboxEvent,
};
pub use types::InboxEvent;

use self::capture::render_capture_row;

/// The capture field and what it has caught.
pub struct Inbox {
    /// The phone's replica, which is where a capture goes before the Mac hears anything.
    pub(crate) store: Option<Arc<TursoStore>>,
    /// The field, built once so its text survives a reload.
    pub(crate) input: Option<(
        gpui::Entity<gpui_kit::component::input::InputState>,
        gpui::Subscription,
    )>,
    /// The chip the person pressed, or none for "let the rules decide".
    pub(crate) tag: Option<&'static str>,
    /// Why the field would not keep what was typed.
    pub(crate) error: Option<SharedString>,
    /// This phone's captures, newest first.
    pub(crate) captures: Vec<LocalCapture>,
    /// The captures the Mac refused, by the id the queue holds them under. A refusal is shown on
    /// the row it belongs to: the projects band names records, and a capture is not a record.
    pub(crate) refusals: Vec<(String, String)>,
    /// True while a capture is being written, so one press cannot make two.
    pub(crate) saving: bool,
    pub(crate) loaded: bool,
    pub(crate) palette: Palette,
    pub(crate) palette_override: Option<Palette>,
    pub(crate) density: Density,
    /// Bumped per read, so a slow answer cannot replace a newer list.
    pub(crate) generation: u64,
}

impl EventEmitter<InboxEvent> for Inbox {}

impl Inbox {
    pub fn new(density: Density) -> Self {
        Self {
            store: None,
            input: None,
            tag: None,
            error: None,
            captures: Vec::new(),
            refusals: Vec::new(),
            saving: false,
            loaded: false,
            palette: Palette::dark(),
            palette_override: None,
            density,
            generation: 0,
        }
    }

    /// Hands the page the phone's replica, or takes it away. A phone whose store did not open keeps
    /// the field but says the capture cannot be saved, which is what the store error above it says.
    pub fn set_store(&mut self, store: Option<Arc<TursoStore>>, cx: &mut Context<Self>) {
        self.store = store;
        self.load(cx);
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        self.palette_override = Some(palette);
        if self.palette != palette {
            self.palette = palette;
            cx.notify();
        }
    }

    /// True once the replica has answered a list, so a host can tell empty from loading.
    pub fn loaded(&self) -> bool {
        self.loaded
    }

    /// What the page is showing, for a test that reads the rows.
    pub fn captures(&self) -> &[LocalCapture] {
        &self.captures
    }

    /// The Mac's reason for refusing this capture, if it refused one.
    pub(crate) fn refusal_of(&self, capture: &LocalCapture) -> Option<String> {
        self.refusals
            .iter()
            .find(|(path, _)| path == &capture.id)
            .map(|(_, reason)| reason.clone())
    }

    /// How many captures have not reached the Mac.
    pub fn waiting(&self) -> usize {
        self.captures
            .iter()
            .filter(|capture| !capture.on_the_mac())
            .count()
    }

    /// Reads this phone's captures. Nothing happens without a store: the field's own error says so.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            self.loaded = true;
            cx.notify();
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        let work = shell::run(async move {
            let captures = store.captures().await.map_err(|e| e.to_string())?;
            let refusals = store.refusals().await.map_err(|e| e.to_string())?;
            Ok((captures, refusals))
        });
        cx.spawn(async move |this, cx| {
            let answer = work
                .await
                .unwrap_or_else(|| Err("The captures were not read.".to_owned()));
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.loaded = true;
                match answer {
                    Ok((list, refusals)) => {
                        view.captures = list;
                        view.refusals = refusals;
                        view.error = None;
                    }
                    Err(error) => view.error = Some(SharedString::from(error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for Inbox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.palette_override.is_none() {
            self.palette = ThemeContext::of(window, cx).palette;
        }
        let palette = self.palette;
        let input = self.field(window, cx);
        let waiting = self.waiting();
        let total = self.captures.len();

        let rows: Vec<AnyElement> = self
            .captures
            .iter()
            .enumerate()
            .map(|(index, capture)| {
                let refusal = self.refusal_of(capture);
                render_capture_row(capture, refusal, index, self.density, palette)
            })
            .collect();
        let list = if rows.is_empty() {
            div()
                .rounded(px(12.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border))
                .px_4()
                .py_3()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Nothing captured yet. The field above is the whole page.")
                .into_any_element()
        } else {
            crate::list(rows, palette)
        };

        div()
            .id("inbox")
            .flex()
            .flex_col()
            .gap_4()
            .text_color(rgb(palette.text))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("INBOX"),
                    )
                    .child(div().text_xs().text_color(rgb(palette.muted)).child(
                        match (total, waiting) {
                            (0, _) => "Type it here; it is kept on this phone first.".to_owned(),
                            (_, 0) => format!("{total} captured, all on the Mac"),
                            (_, 1) => "1 waiting for the Mac".to_owned(),
                            (_, n) => format!("{n} waiting for the Mac"),
                        },
                    )),
            )
            .child(
                div()
                    .rounded(px(12.))
                    .bg(rgba(palette.surface))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Input::new(&input))
                    .child(self.chips(cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(self.preview(cx))
                            .child(
                                div()
                                    .id("inbox-keep")
                                    .flex_none()
                                    .min_h(px(38.))
                                    .px_4()
                                    .rounded(px(8.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(rgba(if self.saving {
                                        palette.muted
                                    } else {
                                        palette.accent
                                    }))
                                    .text_color(rgb(0xffffff))
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(if self.saving { "Keeping…" } else { "Keep" })
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|view, _, window, cx| view.keep(window, cx)),
                                    ),
                            ),
                    )
                    .when_some(self.error.clone(), |card, error| {
                        card.child(div().text_xs().text_color(rgb(palette.red)).child(error))
                    }),
            )
            .child(crate::section("WHAT YOU KEPT", list, palette))
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::capture::{preview_label, state_of};
    use super::*;

    fn captured(id: &str, text: &str, remote: Option<&str>) -> LocalCapture {
        LocalCapture {
            id: id.to_owned(),
            text: Some(text.to_owned()),
            url: None,
            tag: None,
            remote_id: remote.map(str::to_owned),
            created_at: "2026-10-24T07:00:00Z".to_owned(),
            status: remote.map(|_| "new".to_owned()),
            routed_to: None,
            client: None,
        }
    }

    /// A page with no replica is empty rather than loading forever, and says so: the field still
    /// accepts text, and the error under it is what stops a capture being lost silently.
    #[gpui::test]
    fn a_page_with_no_store_is_empty_and_loaded(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let view = cx.new(|cx| {
            let mut inbox = Inbox::new(Density::Mobile);
            inbox.load(cx);
            inbox
        });
        view.read_with(cx, |inbox, _| {
            assert!(
                inbox.loaded(),
                "a page with no store has nothing to wait for"
            );
            assert!(inbox.captures().is_empty());
            assert_eq!(inbox.waiting(), 0);
        });
    }

    /// The preview answers the chip as well as the text: a press that changed nothing visible reads
    /// as a dropped press on a phone.
    #[gpui::test]
    fn the_preview_names_where_a_capture_will_land(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        assert_eq!(preview_label("", Some("buy")), "To wishlist");
        assert_eq!(preview_label("Lisbon", Some("go")), "To places");
        assert_eq!(preview_label("a thought", None), "Stays in the inbox");
        assert_eq!(
            preview_label("https://www.youtube.com/watch?v=x", None),
            "To feed",
            "the host decides, the way the capability does"
        );
    }

    /// The state line says what the Mac said, not just "on the Mac": a capture the operator has
    /// already dealt with must not read as still waiting for triage.
    #[gpui::test]
    fn the_state_line_follows_the_macs_triage(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let palette = Palette::light();
        let waiting = captured("capture:local:1", "waiting", None);
        assert_eq!(state_of(&waiting, palette).0, "Waiting for the Mac");
        for (status, expected) in [
            ("new", "On the Mac"),
            ("done", "Done on the Mac"),
            ("dismissed", "Dismissed on the Mac"),
        ] {
            let mut row = captured("abc", "kept", Some("abc"));
            row.status = Some(status.to_owned());
            assert_eq!(state_of(&row, palette).0, expected);
        }
    }

    /// The count the header shows comes from the rows: a capture with the Mac's id has landed, one
    /// without it is still here. Nothing else in the page decides that.
    #[gpui::test]
    fn the_page_counts_what_has_not_reached_the_mac(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let view = cx.new(|_| {
            let mut inbox = Inbox::new(Density::Mobile);
            inbox.captures = vec![
                captured("capture:local:2", "call the plumber", None),
                captured("capture:local:1", "read this", Some("9f2c1a7b")),
            ];
            inbox
        });
        view.read_with(cx, |inbox, _| {
            assert_eq!(inbox.captures().len(), 2);
            assert_eq!(inbox.waiting(), 1, "one is still waiting for the Mac");
        });
    }
}
