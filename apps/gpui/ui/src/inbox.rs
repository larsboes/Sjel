//! Inbox: one field for the thing in your hand, and the list of what you already kept.
//!
//! The phone is a capture device first (research/offline-on-the-phone.md). This page is the field
//! that catches a thought, a link or a price while somebody is walking, with one chip for the four
//! tags the comms capability knows. It writes into the phone's own replica and queues the capture
//! for the Mac, so it works with no Mac in reach — and the row says which of the two it is looking
//! at, because a capture that looks sent and is not is worse than one that says it is waiting.
//!
//! Triage is not here. What the Mac does with a capture — a wishlist item, a feed source, a place —
//! is `capabilities/comms`'s decision, and the preview under the field only says where it will land.

use std::sync::Arc;

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, EventEmitter, FontWeight, MouseButton,
    Render, SharedString, Window,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use sjel_gpui_core::capture::{self, Draft, TAGS};
use sjel_gpui_core::shell;
use sjel_gpui_core::store::{LocalCapture, TursoStore};

use crate::theme::tint;
use crate::{Density, Palette};

/// What the page asks its host to do. The host owns the sync and the keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InboxEvent {
    /// A capture was written here and is waiting for the Mac, so the host may sync now.
    Wrote,
    /// The field took or lost the keyboard, so the host can show or hide it.
    Editing(bool),
}

/// The capture field and what it has caught.
pub struct Inbox {
    /// The phone's replica, which is where a capture goes before the Mac hears anything.
    store: Option<Arc<TursoStore>>,
    /// The field, built once so its text survives a reload.
    input: Option<(gpui::Entity<InputState>, gpui::Subscription)>,
    /// The chip the person pressed, or none for "let the rules decide".
    tag: Option<&'static str>,
    /// Why the field would not keep what was typed.
    error: Option<SharedString>,
    /// This phone's captures, newest first.
    captures: Vec<LocalCapture>,
    /// The captures the Mac refused, by the id the queue holds them under. A refusal is shown on
    /// the row it belongs to: the projects band names records, and a capture is not a record.
    refusals: Vec<(String, String)>,
    /// True while a capture is being written, so one press cannot make two.
    saving: bool,
    loaded: bool,
    palette: Palette,
    density: Density,
    /// Bumped per read, so a slow answer cannot replace a newer list.
    generation: u64,
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
    fn refusal_of(&self, capture: &LocalCapture) -> Option<String> {
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

    /// Keeps what is in the field: written into the replica first, queued for the Mac second.
    fn keep(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let Some(input) = self.input.as_ref().map(|(input, _)| input.clone()) else {
            return;
        };
        let typed = input.read(cx).value().to_string();
        let draft = match Draft::parse(&typed, self.tag) {
            Ok(draft) => draft,
            Err(why) => {
                self.error = Some(SharedString::from(why));
                cx.notify();
                return;
            }
        };
        let Some(store) = self.store.clone() else {
            self.error = Some(SharedString::from(
                "Captures cannot be saved on this phone: its store did not open.",
            ));
            cx.notify();
            return;
        };
        self.saving = true;
        self.error = None;
        let work = shell::run(async move {
            store
                .save_capture(
                    draft.text.as_deref(),
                    draft.url.as_deref(),
                    draft.tag.as_deref(),
                )
                .await
                .map_err(|e| e.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let answer = work
                .await
                .unwrap_or_else(|| Err("The capture was not written.".to_owned()));
            let _ = this.update_in(cx, |view, window, cx| {
                view.saving = false;
                match answer {
                    Ok(_) => {
                        // The field empties and lets go of the chip: the next capture is a new
                        // thought, not a correction of the last one.
                        view.tag = None;
                        if let Some((input, _)) = view.input.as_ref() {
                            let input = input.clone();
                            input.update(cx, |input, cx| input.set_value("", window, cx));
                        }
                        view.load(cx);
                        cx.emit(InboxEvent::Wrote);
                    }
                    Err(error) => view.error = Some(SharedString::from(error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The field, created once. Enter keeps the capture, and the keyboard follows the focus.
    fn field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Entity<InputState> {
        if let Some((input, _)) = &self.input {
            return input.clone();
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("A thought, a link, or #buy …"));
        let subscription =
            cx.subscribe_in(&input, window, |view, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => view.keep(window, cx),
                InputEvent::Focus => cx.emit(InboxEvent::Editing(true)),
                InputEvent::Blur => cx.emit(InboxEvent::Editing(false)),
                _ => {}
            });
        self.input = Some((input.clone(), subscription));
        input
    }

    /// The preview under the field: where this will land, or that it waits.
    fn preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let typed = self
            .input
            .as_ref()
            .map(|(input, _)| input.read(cx).value().to_string())
            .unwrap_or_default();
        div()
            .text_xs()
            .text_color(rgb(palette.muted))
            .child(preview_label(&typed, self.tag))
            .into_any_element()
    }

    /// The four tags, one at a time, and none for the rules to decide.
    fn chips(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let mut row = div().flex().items_center().gap_2();
        for tag in TAGS {
            let active = self.tag == Some(tag);
            row = row.child(
                div()
                    .id(SharedString::from(format!("inbox-tag-{tag}")))
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_h(px(34.))
                    .px_3()
                    .rounded(px(8.))
                    .text_xs()
                    .cursor_pointer()
                    .when(active, |chip| {
                        chip.bg(rgba(palette.accent))
                            .text_color(rgb(0xffffff))
                            .font_weight(FontWeight::SEMIBOLD)
                    })
                    .when(!active, |chip| {
                        chip.bg(rgba(palette.raised))
                            .text_color(rgb(palette.muted))
                            .border_1()
                            .border_color(rgba(palette.border))
                    })
                    .child(format!("#{tag}"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            view.tag = if view.tag == Some(tag) {
                                None
                            } else {
                                Some(tag)
                            };
                            cx.notify();
                        }),
                    ),
            );
        }
        row.into_any_element()
    }

    fn capture_row(&self, capture: &LocalCapture, index: usize) -> AnyElement {
        let palette = self.palette;
        let refusal = self.refusal_of(capture);
        let tag_pill = match capture.tag.as_deref() {
            Some(tag) => {
                let tag_color = match tag {
                    "todo" | "task" => palette.accent,
                    "idea" => palette.amber,
                    "place" | "go" => palette.green,
                    "buy" | "price" => palette.accent,
                    _ => palette.muted,
                };
                div()
                    .px_1p5()
                    .py(px(0.5))
                    .rounded(px(3.))
                    .bg(rgba(tint(tag_color, 0x18)))
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(tag_color))
                    .child(format!("#{tag}"))
            }
            None => div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("no tag"),
        };
        let url_line = capture.url.as_deref().and_then(|url| {
            if capture.text.as_deref() != Some(url) {
                Some(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.accent))
                        .truncate()
                        .child(url.to_string()),
                )
            } else {
                None
            }
        });
        // The state is a sentence of the Mac's and can be long, so it goes in the column under the
        // title rather than beside the tag: a row that pushes its own text off the card is a row
        // nobody can read.
        let mut column = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(1.))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.text))
                    .truncate()
                    .child(capture.label()),
            )
            .children(url_line)
            .child(div().flex().items_center().gap_2().child(tag_pill).when(
                refusal.is_none(),
                |line| {
                    let (state, colour) = state_of(capture, palette);
                    line.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_1p5()
                            .py(px(0.5))
                            .rounded_full()
                            .bg(rgba(tint(colour, 0x18)))
                            .child(div().size(px(5.)).rounded_full().bg(rgb(colour)))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(colour))
                                    .child(state),
                            ),
                    )
                },
            ));
        if let Some(reason) = refusal {
            // The Mac said no. It is said here, on the row, because that is where the capture is
            // and where the person can do something about it.
            column = column.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(format!("The Mac refused it: {reason}")),
            );
        }
        div()
            .id(("inbox-capture", index))
            .flex()
            .items_center()
            .gap_3()
            .px(self.density.pad_x())
            .py(self.density.pad_y())
            .min_h(self.density.row_height())
            .child(column)
            .when_some(capture.tag.clone(), |row, tag| {
                row.child(
                    div()
                        .flex_none()
                        .px_2()
                        .py(px(1.5))
                        .rounded(px(4.))
                        .bg(rgba(tint(palette.raised, 0x66)))
                        .border_1()
                        .border_color(rgba(palette.border))
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(
                            capture::route_for(Some(&tag), capture.url.as_deref())
                                .map(|route| route.label().to_owned())
                                .unwrap_or_else(|| "Inbox".to_owned()),
                        ),
                )
            })
            .into_any_element()
    }
}

/// The line that says where a capture is, in the Mac's own terms once it has one.
///
/// A capture this phone has not sent is waiting; one the Mac has is `new` until the operator
/// triages it, and then it is `done` or `dismissed`. Saying "on the Mac" for all three would show a
/// finished thought as if it were still in the inbox.
fn state_of(capture: &LocalCapture, palette: Palette) -> (String, u32) {
    if !capture.on_the_mac() {
        return ("Waiting for the Mac".to_owned(), palette.amber);
    }
    match capture.status.as_deref() {
        Some("done") => ("Done on the Mac".to_owned(), palette.green),
        Some("dismissed") => ("Dismissed on the Mac".to_owned(), palette.muted),
        _ => ("On the Mac".to_owned(), palette.green),
    }
}

/// What the line under the field says.
///
/// A chip is a decision even before there is text, so it is answered: pressing `#buy` and reading
/// "Stays in the inbox" would say the press did nothing.
fn preview_label(typed: &str, tag: Option<&str>) -> String {
    match Draft::parse(typed, tag) {
        Ok(draft) => draft.preview(),
        Err(_) => capture::route_for(tag, None)
            .map(|route| route.label().to_owned())
            .unwrap_or_else(|| "Stays in the inbox".to_owned()),
    }
}

impl Render for Inbox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        let input = self.field(window, cx);
        let waiting = self.waiting();
        let total = self.captures.len();

        let rows: Vec<AnyElement> = self
            .captures
            .iter()
            .enumerate()
            .map(|(index, capture)| self.capture_row(capture, index))
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
