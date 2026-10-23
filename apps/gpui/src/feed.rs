//! Native Feed: triage the comms feed and read one entry.
//!
//! Comms owns the data (`capabilities/comms/src/server/content.rs`); this page reads
//! `GET /comms/feed` and writes decisions only through `POST /comms/feed/:id/status`, the route
//! that records the ledger row with them. The keys are the dashboard's
//! (`dashboard/src/routes/feed/+page.svelte`, "Keyboard triage"), so the two surfaces agree.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, FocusHandle,
    FontWeight, KeyBinding, SharedString, Window,
};
use sjel_gpui_core::shell::{self, FeedDetail, FeedEntry, FeedSurface, ShellError};

use crate::data::{Remote, Repo};
use crate::mail::Mail;
use crate::theme::Palette;
use sjel_gpui_ui::filter_pill;

actions!(
    feed,
    [
        /// Move to the next entry.
        Next,
        /// Move to the previous entry.
        Previous,
        /// Keep the entry, or take the keep back.
        Keep,
        /// Dismiss the entry.
        Dismiss,
        /// Undo a keep or dismiss.
        Undo,
        /// Read the entry.
        Read,
        /// Open the original in the browser.
        OpenOriginal,
        /// Back to the list.
        Back,
    ]
);

/// How far back the list reads. The dashboard's inbox reads 7 days by default; a native window
/// that stays open longer gets a month.
const DAYS: u32 = 30;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("j", Next, Some("Feed")),
        KeyBinding::new("down", Next, Some("Feed")),
        KeyBinding::new("k", Previous, Some("Feed")),
        KeyBinding::new("up", Previous, Some("Feed")),
        KeyBinding::new("s", Keep, Some("Feed")),
        KeyBinding::new("d", Dismiss, Some("Feed")),
        KeyBinding::new("u", Undo, Some("Feed")),
        KeyBinding::new("enter", Read, Some("Feed")),
        KeyBinding::new("o", Read, Some("Feed")),
        KeyBinding::new("shift-o", OpenOriginal, Some("Feed")),
        KeyBinding::new("escape", Back, Some("Feed")),
        KeyBinding::new("cmd-[", Back, Some("Feed")),
    ]);
}

/// Reading is the comms feed; Mail is Gmail triage (`crate::mail`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Reading,
    Mail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    New,
    Kept,
    Dismissed,
}

impl Filter {
    fn status(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Kept => "keeper",
            Self::Dismissed => "dismissed",
        }
    }
}

pub struct Feed {
    repo: Repo,
    pub tab: Tab,
    pub mail: Entity<Mail>,
    palette: Option<Palette>,
    pub rows: Remote<Vec<FeedEntry>>,
    pub filter: Filter,
    /// `None` shows both streams.
    pub stream: Option<&'static str>,
    pub cursor: Option<String>,
    /// The entry the reader shows; `None` shows the list.
    pub reading: Option<String>,
    pub detail: Remote<FeedDetail>,
    /// The last write that failed, shown above the list until the next one.
    pub write_error: Option<ShellError>,
    focus: FocusHandle,
    focus_next: bool,
}

impl Feed {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            mail: cx.new(|cx| Mail::new(repo.clone(), cx)),
            repo,
            tab: Tab::Reading,
            palette: None,
            rows: Remote::default(),
            filter: Filter::New,
            stream: None,
            cursor: None,
            reading: None,
            detail: Remote::default(),
            write_error: None,
            focus: cx.focus_handle(),
            focus_next: true,
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
        self.mail
            .update(cx, |mail, cx| mail.set_palette(palette, cx));
    }

    /// Called when the page opens: read the open tab again and give it the keys.
    pub fn activate(&mut self, cx: &mut Context<Self>) {
        match self.tab {
            Tab::Reading => {
                self.focus_next = true;
                self.load(cx);
            }
            Tab::Mail => self.mail.update(cx, |mail, cx| mail.activate(cx)),
        }
    }

    /// Reads the open tab again (⌘R).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        match self.tab {
            Tab::Reading => self.load(cx),
            Tab::Mail => self.mail.update(cx, |mail, cx| mail.load(cx)),
        }
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            self.activate(cx);
            cx.notify();
        }
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
        let work = shell::run(async move { shell.feed(DAYS).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.rows.settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn visible(&self) -> Vec<&FeedEntry> {
        in_view(self.rows.value.iter().flatten(), self.filter, self.stream)
    }

    fn count(&self, filter: Filter) -> usize {
        in_view(self.rows.value.iter().flatten(), filter, self.stream).len()
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.cursor = None;
        cx.notify();
    }

    fn set_stream(&mut self, stream: Option<&'static str>, cx: &mut Context<Self>) {
        self.stream = stream;
        self.cursor = None;
        cx.notify();
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.visible().iter().map(|e| e.id.clone()).collect();
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

    /// The entry the keys act on: the open one in the reader, else the cursor's.
    fn target(&self) -> Option<String> {
        self.reading.clone().or_else(|| self.cursor.clone())
    }

    /// Writes `status` for `id`, showing it at once and putting the old one back if comms refuses.
    fn decide(&mut self, id: String, status: &'static str, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        let surface = if self.reading.is_some() {
            FeedSurface::Reader
        } else {
            FeedSurface::Inbox
        };
        // The cursor moves on to the row below, so a run of `d` presses works down the list.
        if self.reading.is_none() && self.cursor.as_deref() == Some(id.as_str()) {
            let ids: Vec<String> = self.visible().iter().map(|e| e.id.clone()).collect();
            let at = ids.iter().position(|i| *i == id).unwrap_or(0);
            self.cursor = ids
                .get(at + 1)
                .or_else(|| at.checked_sub(1).and_then(|i| ids.get(i)))
                .cloned();
        }
        let before = self.set_local_status(&id, status);
        self.write_error = None;
        cx.notify();
        let work = shell::run({
            let id = id.clone();
            async move { shell.set_feed_status(&id, status, surface).await }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if let Err(error) = result {
                    if let Some(before) = before {
                        view.set_local_status(&id, before);
                    }
                    view.write_error = Some(error);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Sets the status in the list and the reader, and answers what it was.
    fn set_local_status(&mut self, id: &str, status: &str) -> Option<&'static str> {
        if let Some(detail) = self.detail.value.as_mut().filter(|d| d.id == id) {
            detail.status = status.to_owned();
        }
        let row = self.rows.value.iter_mut().flatten().find(|e| e.id == id)?;
        let before = [Filter::New, Filter::Kept, Filter::Dismissed]
            .into_iter()
            .map(Filter::status)
            .find(|s| *s == row.status);
        row.status = status.to_owned();
        before
    }

    fn status_of(&self, id: &str) -> Option<&str> {
        self.rows
            .value
            .iter()
            .flatten()
            .find(|e| e.id == id)
            .map(|e| e.status.as_str())
    }

    fn open(&mut self, id: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        let first = self.status_of(&id) == Some("new");
        self.cursor = Some(id.clone());
        self.reading = Some(id.clone());
        self.detail = Remote::new_loading();
        cx.notify();
        let expected = id.clone();
        let work = shell::run(async move {
            // Telemetry for the ranking model; a failed write never stops the reader.
            let _ = shell.record_feed_open(&id, first).await;
            shell.feed_detail(&id).await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.reading.as_deref() == Some(expected.as_str()) {
                    view.detail.settle(result);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn url_of(&self, id: &str) -> Option<String> {
        self.rows
            .value
            .iter()
            .flatten()
            .find(|e| e.id == id)
            .map(|e| e.url.clone())
            .or_else(|| self.detail.value.as_ref().map(|d| d.url.clone()))
    }

    fn on_next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        if self.reading.is_none() {
            self.step(true, cx);
        }
    }

    fn on_previous(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
        if self.reading.is_none() {
            self.step(false, cx);
        }
    }

    fn on_keep(&mut self, _: &Keep, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.target() {
            let next = if self.status_of(&id) == Some("keeper") {
                "new"
            } else {
                "keeper"
            };
            self.decide(id, next, cx);
        }
    }

    fn on_dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.target() {
            self.decide(id, "dismissed", cx);
        }
    }

    fn on_undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.target() {
            self.decide(id, "new", cx);
        }
    }

    fn on_read(&mut self, _: &Read, _: &mut Window, cx: &mut Context<Self>) {
        if self.reading.is_none() {
            if let Some(id) = self.cursor.clone() {
                self.open(id, cx);
            }
        }
    }

    fn on_open_original(&mut self, _: &OpenOriginal, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(url) = self.target().and_then(|id| self.url_of(&id)) {
            cx.open_url(&url);
        }
    }

    fn on_back(&mut self, _: &Back, _: &mut Window, cx: &mut Context<Self>) {
        // Escape closes the reader first, then clears the cursor.
        if self.reading.take().is_some() || self.cursor.take().is_some() {
            cx.notify();
        }
    }
}

impl Render for Feed {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_next) {
            window.focus(&self.focus, cx);
        }
        let palette = self.palette.unwrap_or_else(Palette::dark);
        let reading = self.tab == Tab::Reading;
        let body = if !reading {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.title_row(cx))
                .child(self.mail.clone())
                .into_any_element()
        } else if self.reading.is_some() {
            self.reader(palette, cx)
        } else {
            self.list(palette, cx)
        };
        // Only the Reading tab carries the Feed keys, so s, u and ⇧O never act on an entry the
        // Mail tab hides.
        div()
            .when(reading, |el| el.key_context("Feed"))
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_previous))
            .on_action(cx.listener(Self::on_keep))
            .on_action(cx.listener(Self::on_dismiss))
            .on_action(cx.listener(Self::on_undo))
            .on_action(cx.listener(Self::on_read))
            .on_action(cx.listener(Self::on_open_original))
            .on_action(cx.listener(Self::on_back))
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .when_some(self.write_error.as_ref(), |el, error| {
                el.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.red))
                        .child(format!("Not saved: {error}")),
                )
            })
            .child(body)
    }
}

impl Feed {
    fn list(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let pill = |label: &'static str, filter: Filter, cx: &mut Context<Self>| {
            filter_pill(
                label,
                self.count(filter),
                self.filter == filter,
                palette,
                cx.listener(move |view, _, _, cx| view.set_filter(filter, cx)),
            )
        };
        let stream_button =
            |label: &'static str, stream: Option<&'static str>, cx: &mut Context<Self>| {
                Button::new(SharedString::from(format!("feed-stream-{label}")))
                    .small()
                    .label(label)
                    .when(self.stream == stream, |b| b.primary())
                    .when(self.stream != stream, |b| b.ghost())
                    .on_click(cx.listener(move |view, _, _, cx| view.set_stream(stream, cx)))
            };
        let toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .child(pill("New", Filter::New, cx))
            .child(pill("Kept", Filter::Kept, cx))
            .child(pill("Dismissed", Filter::Dismissed, cx))
            .child(div().flex_1())
            .child(stream_button("All", None, cx))
            .child(stream_button("News", Some("news"), cx))
            .child(stream_button("Media", Some("media"), cx));

        let mut list = div().flex().flex_col().gap_1();
        if self.rows.value.is_none() && self.rows.loading {
            list = list.child(muted("Loading the feed…", palette));
        }
        if let Some(error) = &self.rows.error {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
            );
        }
        let visible = self.visible();
        if visible.is_empty() && !self.rows.loading && self.rows.error.is_none() {
            list = list.child(muted(
                match self.filter {
                    Filter::New => "Nothing new. The inbox is clear.",
                    Filter::Kept => "Nothing kept in the last 30 days.",
                    Filter::Dismissed => "Nothing dismissed in the last 30 days.",
                },
                palette,
            ));
        }
        let mut day = None;
        for (i, entry) in visible.into_iter().enumerate() {
            if day != Some(entry.day.as_str()) {
                day = Some(entry.day.as_str());
                list = list.child(
                    div()
                        .pt_3()
                        .pb_1()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(palette.muted))
                        .child(entry.day.clone()),
                );
            }
            let id = entry.id.clone();
            let chosen = self.cursor.as_deref() == Some(entry.id.as_str());
            let preview = entry
                .summary
                .as_deref()
                .or(entry.digest_preview.as_deref())
                .unwrap_or_default()
                .trim()
                .to_owned();
            list = list.child(
                div()
                    .id(("feed-row", i))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .when(chosen, |el| el.bg(rgba(palette.raised)))
                    .hover(move |style| style.bg(rgba(palette.raised)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| view.open(id.clone(), cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title(entry.title.as_deref(), &entry.url)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(meta(entry)),
                            ),
                    )
                    .when(!preview.is_empty(), |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .line_clamp(2)
                                .child(preview),
                        )
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.title_row(cx))
            .child(muted(
                "j/k move · s keep · d dismiss · u undo · o read",
                palette,
            ))
            .child(toolbar)
            .child(list)
            .into_any_element()
    }

    /// The page title with the Reading | Mail switch.
    fn title_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let tab = |label: &'static str, tab: Tab, cx: &mut Context<Self>| {
            Button::new(SharedString::from(format!("feed-tab-{label}")))
                .small()
                .label(label)
                .when(self.tab == tab, |b| b.primary())
                .when(self.tab != tab, |b| b.ghost())
                .on_click(cx.listener(move |view, _, _, cx| view.set_tab(tab, cx)))
        };
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Feed"),
            )
            .child(tab("Reading", Tab::Reading, cx))
            .child(tab("Mail", Tab::Mail, cx))
            .into_any_element()
    }

    fn reader(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let back = Button::new("feed-back")
            .small()
            .ghost()
            .label("← Feed")
            .on_click(cx.listener(|view, _, window, cx| view.on_back(&Back, window, cx)));
        let Some(detail) = &self.detail.value else {
            let state = match &self.detail.error {
                Some(error) => div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
                None => muted("Opening…", palette),
            };
            return div()
                .flex()
                .flex_col()
                .gap_3()
                .child(div().child(back))
                .child(state)
                .into_any_element();
        };
        let kept = detail.status == "keeper";
        let actions = div()
            .flex()
            .gap_2()
            .child(back)
            .child(div().flex_1())
            .child(
                Button::new("feed-keep")
                    .small()
                    .label(if kept { "Kept · S" } else { "Keep · S" })
                    .when(kept, |b| b.primary())
                    .on_click(cx.listener(|view, _, window, cx| view.on_keep(&Keep, window, cx))),
            )
            .child(
                Button::new("feed-dismiss")
                    .small()
                    .label("Dismiss · D")
                    .on_click(
                        cx.listener(|view, _, window, cx| view.on_dismiss(&Dismiss, window, cx)),
                    ),
            )
            .child(
                Button::new("feed-original")
                    .small()
                    .label("Open original · ⇧O")
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.on_open_original(&OpenOriginal, window, cx)
                    })),
            );
        let why = detail
            .evaluation
            .as_ref()
            .map(|e| e.explanation.clone())
            .filter(|t| !t.trim().is_empty())
            .or_else(|| detail.relevance.first().map(|r| r.rationale.clone()));
        let text = |label: &'static str, body: &Option<String>| {
            body.as_ref()
                .filter(|t| !t.trim().is_empty())
                .map(|body| section(label, body.clone(), palette))
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .max_w(px(760.))
            .child(actions)
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title(detail.title.as_deref(), &detail.url)),
            )
            .child(muted(
                &[
                    Some(detail.kind.as_str()),
                    detail.author.as_deref(),
                    Some(detail.day.as_str()),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · "),
                palette,
            ))
            .when_some(why, |el, why| el.child(section("WHY THIS", why, palette)))
            .children(text("SUMMARY", &detail.summary))
            .children(text("TRANSCRIPT", &detail.transcript))
            .when(
                detail.summary.is_none() && detail.transcript.is_none(),
                |el| {
                    el.child(muted(
                        match detail.content_status.as_str() {
                            "none" => "Comms holds no text for this entry. Open the original.",
                            _ => "No summary yet. Open the original.",
                        },
                        palette,
                    ))
                },
            )
            .into_any_element()
    }
}

/// The entries `filter` and `stream` show, in the order the list draws them: newest day first,
/// best score first within a day.
fn in_view<'a>(
    rows: impl Iterator<Item = &'a FeedEntry>,
    filter: Filter,
    stream: Option<&str>,
) -> Vec<&'a FeedEntry> {
    let mut rows: Vec<&FeedEntry> = rows
        .filter(|e| e.status == filter.status())
        .filter(|e| stream.is_none_or(|s| e.stream == s))
        .collect();
    rows.sort_by(|a, b| b.day.cmp(&a.day).then(b.score().total_cmp(&a.score())));
    rows
}

fn title(title: Option<&str>, url: &str) -> String {
    title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or(url)
        .to_owned()
}

fn meta(entry: &FeedEntry) -> String {
    let score = entry.score();
    let mut parts = vec![entry.kind.clone()];
    if let Some(author) = entry.author.as_deref().filter(|a| !a.is_empty()) {
        parts.push(author.to_owned());
    }
    if score > 0.0 {
        parts.push(format!("{:.0}%", score * 100.0));
    }
    parts.join(" · ")
}

fn muted(text: &str, palette: Palette) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(text.to_owned())
}

fn section(label: &'static str, body: String, palette: Palette) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.muted))
                .child(label),
        )
        .child(div().text_sm().line_height(px(22.)).child(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: char, day: &str, status: &str, score: f64) -> FeedEntry {
        FeedEntry {
            id: id.to_string().repeat(64),
            stream: "news".into(),
            kind: "article".into(),
            title: Some(id.to_string()),
            url: format!("https://example.org/{id}"),
            author: None,
            summary: None,
            digest_preview: None,
            day: day.into(),
            status: status.into(),
            relevance: None,
            evaluation: Some(shell::FeedEvaluation {
                overall_score: score,
                explanation: String::new(),
            }),
        }
    }

    #[test]
    fn newest_day_first_then_best_score() {
        let rows = [
            entry('a', "2026-10-20", "new", 0.9),
            entry('b', "2026-10-21", "new", 0.1),
            entry('c', "2026-10-21", "new", 0.8),
            entry('d', "2026-10-21", "keeper", 1.0),
        ];
        let order = |stream| -> String {
            in_view(rows.iter(), Filter::New, stream)
                .iter()
                .map(|e| e.title.clone().unwrap())
                .collect()
        };
        assert_eq!(order(None), "cba");
        assert_eq!(order(Some("media")), "");
    }
}
