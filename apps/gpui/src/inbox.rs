//! Native capture inbox: everything captured and not yet sorted, from `sjel capture`, the iOS share
//! sheet or the field on this page. Rules suggest where an item goes; nothing moves until it is
//! pressed. The web page is `dashboard/src/routes/inbox/+page.svelte`.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, EventEmitter,
    FocusHandle, FontWeight, KeyBinding, SharedString, Subscription, Window,
};
use sjel_gpui_core::capture::{self, Capture, TAGS};
use sjel_gpui_core::shell::{self, ShellError};

use crate::data::{Remote, Repo};
use crate::theme::Palette;
use sjel_gpui_ui::filter_pill;

actions!(
    inbox,
    [
        /// Move to the next capture.
        Next,
        /// Move to the previous capture.
        Previous,
        /// Send the capture where the rules suggest.
        Apply,
        /// Mark the capture done.
        Done,
        /// Dismiss the capture.
        Dismiss,
        /// Cycle the tag: buy, look, go, learn, none.
        CycleTag,
        /// Open the capture's link in the browser.
        OpenLink,
        /// Prepare a YouTube capture in ytalbum.
        OpenYtAlbum,
        /// Type a new capture.
        NewCapture,
        /// Edit the capture's text.
        EditText,
        /// Put a done or dismissed capture back to new.
        Restore,
    ]
);

pub enum InboxEvent {
    OpenYtAlbum(String),
}

impl EventEmitter<InboxEvent> for Inbox {}

pub fn bind_keys(cx: &mut App) {
    // The "CaptureList" context sits on the list only, so these letters never fire while the
    // capture field has focus.
    cx.bind_keys([
        KeyBinding::new("j", Next, Some("CaptureList")),
        KeyBinding::new("down", Next, Some("CaptureList")),
        KeyBinding::new("k", Previous, Some("CaptureList")),
        KeyBinding::new("up", Previous, Some("CaptureList")),
        KeyBinding::new("r", Apply, Some("CaptureList")),
        KeyBinding::new("e", Done, Some("CaptureList")),
        KeyBinding::new("d", Dismiss, Some("CaptureList")),
        KeyBinding::new("t", CycleTag, Some("CaptureList")),
        KeyBinding::new("o", OpenLink, Some("CaptureList")),
        KeyBinding::new("y", OpenYtAlbum, Some("CaptureList")),
        KeyBinding::new("n", NewCapture, Some("CaptureList")),
        KeyBinding::new("enter", EditText, Some("CaptureList")),
        KeyBinding::new("u", Restore, Some("CaptureList")),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    New,
    Done,
    Dismissed,
}

impl Filter {
    fn status(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Done => "done",
            Self::Dismissed => "dismissed",
        }
    }
}

pub struct Inbox {
    repo: Repo,
    palette: Option<Palette>,
    pub rows: Remote<Vec<Capture>>,
    pub filter: Filter,
    pub cursor: Option<String>,
    /// Capture ids with a write in flight; their buttons wait.
    busy: Vec<String>,
    /// The last write that failed.
    pub write_error: Option<ShellError>,
    input: Option<(Entity<InputState>, Subscription)>,
    /// The capture whose text is open for editing, with its field.
    pub editing: Option<(String, Entity<InputState>, Subscription)>,
    list_focus: FocusHandle,
    focus_list_next: bool,
}

impl Inbox {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            palette: None,
            rows: Remote::default(),
            filter: Filter::New,
            cursor: None,
            busy: Vec::new(),
            write_error: None,
            input: None,
            editing: None,
            list_focus: cx.focus_handle(),
            focus_list_next: true,
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        self.focus_list_next = true;
        self.load(cx);
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.rows.settle(Err(error));
                cx.notify();
                return;
            }
        };
        self.rows.loading = true;
        cx.notify();
        let work = shell::run(async move { shell.captures().await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.rows.settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn visible(&self) -> Vec<&Capture> {
        in_view(self.rows.value.iter().flatten(), self.filter)
    }

    /// New captures the rules can send on.
    fn routable(&self) -> Vec<Capture> {
        in_view(self.rows.value.iter().flatten(), Filter::New)
            .into_iter()
            .filter(|c| c.route().is_some())
            .cloned()
            .collect()
    }

    fn find(&self, id: &str) -> Option<Capture> {
        self.rows
            .value
            .iter()
            .flatten()
            .find(|c| c.id == id)
            .cloned()
    }

    fn replace(&mut self, next: Capture) {
        if let Some(row) = self
            .rows
            .value
            .iter_mut()
            .flatten()
            .find(|c| c.id == next.id)
        {
            *row = next;
        }
    }

    /// Runs one write for `id` and puts the capture comms answers in place of the old one.
    fn act<F>(&mut self, id: String, cx: &mut Context<Self>, work: F)
    where
        F: FnOnce(
            std::sync::Arc<shell::Shell>,
        ) -> futures::future::BoxFuture<'static, shell::ShellResult<Capture>>,
    {
        let Ok(shell) = self.repo.shell() else { return };
        if self.busy.contains(&id) {
            return;
        }
        self.busy.push(id.clone());
        self.write_error = None;
        cx.notify();
        let work = shell::run(work(shell));
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.busy.retain(|b| *b != id);
                match result {
                    Ok(next) => view.replace(next),
                    Err(error) => view.write_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn patch(&mut self, id: String, patch: serde_json::Value, cx: &mut Context<Self>) {
        let key = id.clone();
        self.act(key, cx, move |shell| {
            Box::pin(async move { shell.patch_capture(&id, patch).await })
        });
    }

    fn apply(&mut self, capture: Capture, cx: &mut Context<Self>) {
        let Some(to) = capture.route() else { return };
        self.act(capture.id.clone(), cx, move |shell| {
            Box::pin(async move { shell.route_capture(&capture, to).await })
        });
    }

    fn apply_all(&mut self, cx: &mut Context<Self>) {
        for capture in self.routable() {
            self.apply(capture, cx);
        }
    }

    fn add(&mut self, raw: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        let (text, url) = split_draft(&raw);
        if text.is_none() && url.is_none() {
            return;
        }
        self.write_error = None;
        let work =
            shell::run(async move { shell.add_capture(text.as_deref(), url.as_deref()).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(capture) => {
                        if let Some(rows) = view.rows.value.as_mut() {
                            rows.insert(0, capture);
                        }
                    }
                    Err(error) => view.write_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.cursor = None;
        cx.notify();
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.visible().iter().map(|c| c.id.clone()).collect();
        let at = self
            .cursor
            .as_ref()
            .and_then(|id| ids.iter().position(|i| i == id));
        let next = match (at, forward) {
            (None, _) => 0,
            (Some(i), true) => (i + 1).min(ids.len().saturating_sub(1)),
            (Some(i), false) => i.saturating_sub(1),
        };
        self.cursor = ids.get(next).cloned();
        cx.notify();
    }

    fn selected(&self) -> Option<Capture> {
        self.cursor.as_deref().and_then(|id| self.find(id))
    }

    /// Moves the cursor off `id` before it leaves the New view, so a run of presses works down
    /// the list.
    fn advance_past(&mut self, id: &str) {
        let ids: Vec<String> = self.visible().iter().map(|c| c.id.clone()).collect();
        if let Some(at) = ids.iter().position(|i| i == id) {
            self.cursor = ids
                .get(at + 1)
                .or_else(|| at.checked_sub(1).and_then(|i| ids.get(i)))
                .cloned();
        }
    }

    fn set_status(&mut self, id: String, status: &str, cx: &mut Context<Self>) {
        self.advance_past(&id);
        self.patch(id, serde_json::json!({ "status": status }), cx);
    }

    fn set_tag(&mut self, id: String, tag: Option<&str>, cx: &mut Context<Self>) {
        // Comms reads an absent field as "unchanged", so an empty string clears the tag, as the
        // dashboard's select does.
        self.patch(id, serde_json::json!({ "tag": tag.unwrap_or("") }), cx);
    }

    /// Opens the capture's text in a field. Enter saves it, leaving the field cancels. A price
    /// typed at the end ("Linen shirt 49,90 €") is what the wishlist route reads.
    fn edit_text(&mut self, capture: &Capture, window: &mut Window, cx: &mut Context<Self>) {
        let text = capture.text.clone().unwrap_or_default();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_value(text, window, cx);
            state
        });
        let id = capture.id.clone();
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |view, input, event, window, cx| match event {
                InputEvent::PressEnter { .. } => {
                    let text = input.read(cx).value().trim().to_owned();
                    view.editing = None;
                    window.focus(&view.list_focus, cx);
                    view.patch(id.clone(), serde_json::json!({ "text": text }), cx);
                }
                InputEvent::Blur => {
                    view.editing = None;
                    cx.notify();
                }
                _ => {}
            },
        );
        input.update(cx, |input, cx| input.focus(window, cx));
        self.editing = Some((capture.id.clone(), input, subscription));
        cx.notify();
    }

    fn capture_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.input {
            return input.clone();
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("A link or a thought. #buy #look #go #learn sort it.")
        });
        let subscription = cx.subscribe_in(&input, window, |view, input, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let text = input.read(cx).value().trim().to_owned();
                if text.is_empty() {
                    return;
                }
                input.update(cx, |input, cx| input.set_value("", window, cx));
                view.add(text, cx);
            }
        });
        self.input = Some((input.clone(), subscription));
        input
    }

    fn on_next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        self.step(true, cx);
    }

    fn on_previous(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
        self.step(false, cx);
    }

    fn on_apply(&mut self, _: &Apply, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(capture) = self.selected().filter(|c| c.status == "new") {
            self.advance_past(&capture.id);
            self.apply(capture, cx);
        }
    }

    fn on_done(&mut self, _: &Done, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.cursor.clone() {
            self.set_status(id, "done", cx);
        }
    }

    fn on_dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.cursor.clone() {
            self.set_status(id, "dismissed", cx);
        }
    }

    fn on_cycle_tag(&mut self, _: &CycleTag, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(capture) = self.selected() {
            self.set_tag(capture.id.clone(), next_tag(capture.tag.as_deref()), cx);
        }
    }

    fn on_open_link(&mut self, _: &OpenLink, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(url) = self.selected().and_then(|c| c.url) {
            cx.open_url(&url);
        }
    }

    fn on_open_ytalbum(&mut self, _: &OpenYtAlbum, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(url) = self
            .selected()
            .and_then(|c| c.url)
            .filter(|url| is_youtube_url(url))
        {
            cx.emit(InboxEvent::OpenYtAlbum(url));
        }
    }

    fn on_edit_text(&mut self, _: &EditText, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(capture) = self.selected() {
            self.edit_text(&capture, window, cx);
        }
    }

    fn on_restore(&mut self, _: &Restore, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(capture) = self.selected().filter(|c| c.status != "new") {
            self.set_status(capture.id, "new", cx);
        }
    }

    fn on_new_capture(&mut self, _: &NewCapture, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.capture_field(window, cx);
        input.update(cx, |input, cx| input.focus(window, cx));
    }
}

impl Render for Inbox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_list_next) {
            window.focus(&self.list_focus, cx);
        }
        let palette = self.palette.unwrap_or_else(Palette::dark);
        let field = self.capture_field(window, cx);
        let routable = self.routable().len();
        let count = |filter| in_view(self.rows.value.iter().flatten(), filter).len();

        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Inbox"),
            )
            .child(muted(
                "j/k move · r send on · e done · d dismiss · t tag · ↩ edit · u restore · o link · y ytalbum · n new",
                palette,
            ))
            .child(div().flex_1())
            .when(routable > 0, |el| {
                el.child(
                    Button::new("inbox-apply-all")
                        .small()
                        .label(format!("Apply {routable} suggestions"))
                        .on_click(cx.listener(|view, _, _, cx| view.apply_all(cx))),
                )
            });

        let pills = div()
            .flex()
            .gap_2()
            .child(filter_pill(
                "New",
                count(Filter::New),
                self.filter == Filter::New,
                palette,
                cx.listener(|view, _, _, cx| view.set_filter(Filter::New, cx)),
            ))
            .child(filter_pill(
                "Done",
                count(Filter::Done),
                self.filter == Filter::Done,
                palette,
                cx.listener(|view, _, _, cx| view.set_filter(Filter::Done, cx)),
            ))
            .child(filter_pill(
                "Dismissed",
                count(Filter::Dismissed),
                self.filter == Filter::Dismissed,
                palette,
                cx.listener(|view, _, _, cx| view.set_filter(Filter::Dismissed, cx)),
            ));

        // While a capture's text is open, the list drops its context, so the letters it binds
        // type into the field.
        let mut list = div()
            .when(self.editing.is_none(), |el| el.key_context("CaptureList"))
            .track_focus(&self.list_focus)
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_previous))
            .on_action(cx.listener(Self::on_apply))
            .on_action(cx.listener(Self::on_done))
            .on_action(cx.listener(Self::on_dismiss))
            .on_action(cx.listener(Self::on_cycle_tag))
            .on_action(cx.listener(Self::on_open_link))
            .on_action(cx.listener(Self::on_open_ytalbum))
            .on_action(cx.listener(Self::on_new_capture))
            .on_action(cx.listener(Self::on_edit_text))
            .on_action(cx.listener(Self::on_restore))
            .flex()
            .flex_col()
            .gap_1();
        if self.rows.value.is_none() && self.rows.loading {
            list = list.child(muted("Loading the inbox…", palette));
        }
        if let Some(error) = &self.rows.error {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
            );
        }
        let rows: Vec<Capture> = self.visible().into_iter().cloned().collect();
        if rows.is_empty() && !self.rows.loading && self.rows.error.is_none() {
            list = list.child(muted("Nothing waiting.", palette));
        }
        for (i, capture) in rows.into_iter().enumerate() {
            list = list.child(self.row(i, capture, palette, cx));
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(header)
            .child(Input::new(&field))
            .when_some(self.write_error.as_ref(), |el, error| {
                el.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.red))
                        .child(format!("Not saved: {error}")),
                )
            })
            .child(pills)
            .child(list)
    }
}

impl Inbox {
    fn row(
        &self,
        i: usize,
        capture: Capture,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chosen = self.cursor.as_deref() == Some(capture.id.as_str());
        let busy = self.busy.contains(&capture.id);
        let is_new = capture.status == "new";
        let route = capture.route().filter(|_| is_new);
        let id = capture.id.clone();
        let tag = capture
            .tag
            .clone()
            .map(|t| format!("#{t}"))
            .or_else(|| capture.suggested_tag().map(|t| format!("#{t}?")));
        let detail = [
            capture::host(capture.url.as_deref()).filter(|_| capture.text.is_some()),
            Some(capture.created_at.get(..10).unwrap_or_default().to_owned()),
            capture.client.clone(),
            capture.routed_to.clone().map(|r| format!("→ {r}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");

        let mut actions = div().flex().items_center().gap_1();
        if is_new {
            if capture.url.as_deref().is_some_and(is_youtube_url) {
                let url = capture.url.clone().unwrap_or_default();
                actions = actions.child(
                    Button::new(SharedString::from(format!("inbox-ytalbum-{i}")))
                        .small()
                        .label("Open ytalbum")
                        .disabled(busy)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(InboxEvent::OpenYtAlbum(url.clone()))
                        })),
                );
            }
            if let Some(to) = route {
                let capture = capture.clone();
                actions = actions.child(
                    Button::new(SharedString::from(format!("inbox-route-{i}")))
                        .small()
                        .label(to.label())
                        .disabled(busy)
                        .on_click(
                            cx.listener(move |view, _, _, cx| view.apply(capture.clone(), cx)),
                        ),
                );
            }
            let done = id.clone();
            let dismiss = id.clone();
            actions = actions
                .child(
                    Button::new(SharedString::from(format!("inbox-done-{i}")))
                        .small()
                        .ghost()
                        .label("Done")
                        .disabled(busy)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.set_status(done.clone(), "done", cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("inbox-dismiss-{i}")))
                        .small()
                        .ghost()
                        .label("Dismiss")
                        .disabled(busy)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.set_status(dismiss.clone(), "dismissed", cx)
                        })),
                );
        } else {
            let restore = id.clone();
            actions = actions.child(
                Button::new(SharedString::from(format!("inbox-restore-{i}")))
                    .small()
                    .ghost()
                    .label("Restore")
                    .disabled(busy)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.set_status(restore.clone(), "new", cx)
                    })),
            );
        }
        if chosen {
            let capture = capture.clone();
            actions =
                actions.child(
                    Button::new(SharedString::from(format!("inbox-edit-{i}")))
                        .small()
                        .ghost()
                        .label("Edit")
                        .disabled(busy)
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.edit_text(&capture, window, cx)
                        })),
                );
        }
        let editor = self
            .editing
            .as_ref()
            .filter(|(editing, _, _)| *editing == capture.id)
            .map(|(_, input, _)| input.clone());

        let tags = chosen.then(|| {
            div().flex().gap_1().children(TAGS.iter().map(|tag| {
                let id = id.clone();
                let on = capture.tag.as_deref() == Some(*tag);
                Button::new(SharedString::from(format!("inbox-tag-{i}-{tag}")))
                    .xsmall()
                    .label(format!("#{tag}"))
                    .when(on, |b| b.primary())
                    .when(!on, |b| b.ghost())
                    .disabled(busy)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.set_tag(id.clone(), (!on).then_some(*tag), cx)
                    }))
            }))
        });

        let select = id.clone();
        div()
            .id(("capture-row", i))
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .rounded(px(6.))
            .when(chosen, |el| el.bg(rgba(palette.raised)))
            .hover(move |style| style.bg(rgba(palette.raised)))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.cursor = Some(select.clone());
                window.focus(&view.list_focus, cx);
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(match editor {
                        Some(input) => div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&input).small())
                            .into_any_element(),
                        None => div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .when(!is_new, |el| el.text_color(rgb(palette.muted)))
                            .child(capture.label())
                            .into_any_element(),
                    })
                    .when_some(tag, |el, tag| {
                        el.child(div().text_xs().text_color(rgb(palette.muted)).child(tag))
                    })
                    .child(actions),
            )
            .child(div().text_xs().text_color(rgb(palette.muted)).child(detail))
            .children(tags)
            .into_any_element()
    }
}

fn is_youtube_url(url: &str) -> bool {
    capture::host(Some(url)).is_some_and(|host| {
        host == "youtu.be" || host == "youtube.com" || host.ends_with(".youtube.com")
    })
}

/// The captures `filter` shows, newest first.
fn in_view<'a>(rows: impl Iterator<Item = &'a Capture>, filter: Filter) -> Vec<&'a Capture> {
    let mut rows: Vec<&Capture> = rows.filter(|c| c.status == filter.status()).collect();
    rows.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    rows
}

/// The field's text as the dashboard splits it: the first `http(s)://` word is the link, the rest
/// is the text. Comms takes a `#tag` out of the text itself.
fn split_draft(raw: &str) -> (Option<String>, Option<String>) {
    let words: Vec<&str> = raw.split_whitespace().collect();
    let url = words
        .iter()
        .find(|w| w.starts_with("http://") || w.starts_with("https://"))
        .map(|w| (*w).to_owned());
    let text = words
        .iter()
        .filter(|w| Some(**w) != url.as_deref())
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    ((!text.is_empty()).then_some(text), url)
}

/// `t` walks the tags in order and then back to none.
fn next_tag(tag: Option<&str>) -> Option<&'static str> {
    match tag.and_then(|t| TAGS.iter().position(|x| *x == t)) {
        None => Some(TAGS[0]),
        Some(i) => TAGS.get(i + 1).copied(),
    }
}

fn muted(text: &str, palette: Palette) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_splits_into_text_and_link() {
        assert_eq!(
            split_draft("#buy shoes https://shop.example/x 49 €"),
            (
                Some("#buy shoes 49 €".into()),
                Some("https://shop.example/x".into())
            )
        );
        assert_eq!(
            split_draft("  https://a.example  "),
            (None, Some("https://a.example".into()))
        );
        assert_eq!(
            split_draft("just a thought"),
            (Some("just a thought".into()), None)
        );
    }

    #[test]
    fn t_cycles_through_the_tags_and_back_to_none() {
        assert_eq!(next_tag(None), Some("buy"));
        assert_eq!(next_tag(Some("buy")), Some("look"));
        assert_eq!(next_tag(Some("learn")), None);
    }
}
