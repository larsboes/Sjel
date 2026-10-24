//! Mail triage, the Feed page's Mail tab: the proposals a Gmail sweep made, grouped by category.
//!
//! Every write goes through `POST /comms/triage/bulk`, for the selection or the row under the
//! cursor. Archive, trash and a class-raising category ask first (`MailAction::confirmation`).
//! Sweeps, reconciling and classification passes stay in the dashboard for now.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, AnyElement, App, Context, FocusHandle, FontWeight,
    KeyBinding, SharedString, Window,
};
use sjel_gpui_core::mail::{
    self, category_label, MailAction, MailProposal, StatusFilter, CATEGORIES,
};
use sjel_gpui_core::shell::{self, ShellError};

use crate::data::{Remote, Repo};
use crate::theme::Palette;

actions!(
    mail,
    [
        /// Move to the next thread.
        Next,
        /// Move to the previous thread.
        Previous,
        /// Select or unselect the thread.
        ToggleSelect,
        /// Dismiss from Sjel. Gmail is not touched.
        Dismiss,
        /// Archive in Sjel and Gmail, after confirming.
        Archive,
        /// Move to Gmail Trash, after confirming.
        Trash,
        /// Add or remove Gmail's Waiting label.
        ToggleWaiting,
        /// Open the thread in Gmail.
        OpenInGmail,
        /// Confirm the action that asked.
        Confirm,
        /// Cancel the question, else clear the selection.
        Cancel,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("j", Next, Some("MailList")),
        KeyBinding::new("down", Next, Some("MailList")),
        KeyBinding::new("k", Previous, Some("MailList")),
        KeyBinding::new("up", Previous, Some("MailList")),
        KeyBinding::new("x", ToggleSelect, Some("MailList")),
        KeyBinding::new("d", Dismiss, Some("MailList")),
        KeyBinding::new("e", Archive, Some("MailList")),
        KeyBinding::new("#", Trash, Some("MailList")),
        KeyBinding::new("shift-3", Trash, Some("MailList")),
        KeyBinding::new("w", ToggleWaiting, Some("MailList")),
        KeyBinding::new("o", OpenInGmail, Some("MailList")),
        KeyBinding::new("enter", Confirm, Some("MailList")),
        KeyBinding::new("escape", Cancel, Some("MailList")),
    ]);
}

/// How many rows render before "Show more".
// ponytail: a capped plain list; 821 pending threads made a full render the slow path. Move to
// `uniform_list` (fixed-height rows) when scrolling past the cap becomes routine.
const PAGE: usize = 200;

pub struct Mail {
    repo: Repo,
    palette: Option<Palette>,
    pub rows: Remote<Vec<MailProposal>>,
    pub status: StatusFilter,
    pub category: Option<&'static str>,
    pub cursor: Option<String>,
    pub selected: Vec<String>,
    /// An action waiting for Enter, with the ids it applies to.
    pending: Option<(MailAction, Vec<String>)>,
    busy: bool,
    /// The outcome of the last write: an error, partial failures, or redactions.
    pub notice: Option<(String, bool)>,
    shown: usize,
    focus: FocusHandle,
    focus_next: bool,
}

impl Mail {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            palette: None,
            rows: Remote::default(),
            status: StatusFilter::Pending,
            category: None,
            cursor: None,
            selected: Vec::new(),
            pending: None,
            busy: false,
            notice: None,
            shown: PAGE,
            focus: cx.focus_handle(),
            focus_next: true,
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        self.focus_next = true;
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
        let work = shell::run(async move { shell.mail_proposals().await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.rows.settle(result);
                // A thread that left the view cannot stay selected.
                let ids: Vec<String> = view.visible().iter().map(|m| m.id.clone()).collect();
                view.selected.retain(|id| ids.contains(id));
                cx.notify();
            });
        })
        .detach();
    }

    pub fn visible(&self) -> Vec<&MailProposal> {
        mail::in_view(self.rows.value.iter().flatten(), self.status, self.category)
    }

    fn find(&self, id: &str) -> Option<&MailProposal> {
        self.rows.value.iter().flatten().find(|m| m.id == id)
    }

    /// The threads an action applies to: the selection, else the row under the cursor. Threads
    /// with a Gmail write in flight are left out.
    fn targets(&self) -> Vec<String> {
        let ids = if self.selected.is_empty() {
            self.cursor.iter().cloned().collect()
        } else {
            self.selected.clone()
        };
        ids.into_iter()
            .filter(|id| self.find(id).is_some_and(MailProposal::actionable))
            .collect()
    }

    /// Runs `action` now, or asks first when it moves threads or destroys text.
    fn request(&mut self, action: MailAction, cx: &mut Context<Self>) {
        let ids = self.targets();
        if ids.is_empty() || self.busy {
            return;
        }
        if action.confirmation(ids.len()).is_some() {
            self.pending = Some((action, ids));
            cx.notify();
        } else {
            self.run(action, ids, cx);
        }
    }

    fn run(&mut self, action: MailAction, ids: Vec<String>, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        self.pending = None;
        self.busy = true;
        self.notice = None;
        if ids.len() == 1 && self.cursor.as_ref() == ids.first() {
            self.advance_past(&ids[0]);
        }
        cx.notify();
        let work = shell::run(async move { shell.mail_action(&ids, &action).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(result) => {
                        view.selected.retain(|id| !result.succeeded.contains(id));
                        view.notice = notice(&result);
                    }
                    Err(error) => view.notice = Some((format!("Not applied: {error}"), true)),
                }
                // Comms' answer is the truth: Gmail state, redactions and sync status all change.
                view.load(cx);
            });
        })
        .detach();
    }

    fn advance_past(&mut self, id: &str) {
        let ids: Vec<String> = self.visible().iter().map(|m| m.id.clone()).collect();
        if let Some(at) = ids.iter().position(|i| i == id) {
            self.cursor = ids
                .get(at + 1)
                .or_else(|| at.checked_sub(1).and_then(|i| ids.get(i)))
                .cloned();
        }
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.visible().iter().map(|m| m.id.clone()).collect();
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
        // Moving past the rendered rows renders the next page.
        if next >= self.shown {
            self.shown = next + PAGE;
        }
        cx.notify();
    }

    fn set_status(&mut self, status: StatusFilter, cx: &mut Context<Self>) {
        self.status = status;
        self.reset_view(cx);
    }

    fn set_category(&mut self, category: Option<&'static str>, cx: &mut Context<Self>) {
        self.category = category;
        self.reset_view(cx);
    }

    fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.cursor = None;
        self.selected.clear();
        self.pending = None;
        self.shown = PAGE;
        cx.notify();
    }

    fn toggle(&mut self, id: String, cx: &mut Context<Self>) {
        if let Some(at) = self.selected.iter().position(|s| *s == id) {
            self.selected.remove(at);
        } else if self.find(&id).is_some_and(MailProposal::actionable) {
            self.selected.push(id);
        }
        cx.notify();
    }

    fn on_next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        self.step(true, cx);
    }

    fn on_previous(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
        self.step(false, cx);
    }

    fn on_toggle_select(&mut self, _: &ToggleSelect, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.cursor.clone() {
            self.toggle(id, cx);
        }
    }

    fn on_dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        self.request(MailAction::Dismiss, cx);
    }

    fn on_archive(&mut self, _: &Archive, _: &mut Window, cx: &mut Context<Self>) {
        self.request(MailAction::Archive, cx);
    }

    fn on_trash(&mut self, _: &Trash, _: &mut Window, cx: &mut Context<Self>) {
        self.request(MailAction::Trash, cx);
    }

    fn on_toggle_waiting(&mut self, _: &ToggleWaiting, _: &mut Window, cx: &mut Context<Self>) {
        // One direction for the batch, read from the first thread, so a mixed selection ends
        // up uniform rather than flipped one by one.
        let waiting = self
            .targets()
            .first()
            .and_then(|id| self.find(id))
            .is_some_and(|m| m.waiting);
        self.request(
            if waiting {
                MailAction::ClearWaiting
            } else {
                MailAction::Waiting
            },
            cx,
        );
    }

    fn on_open_in_gmail(&mut self, _: &OpenInGmail, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(url) = self
            .cursor
            .as_deref()
            .and_then(|id| self.find(id))
            .map(MailProposal::gmail_url)
        {
            cx.open_url(&url);
        }
    }

    fn on_confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((action, ids)) = self.pending.take() {
            self.run(action, ids, cx);
        }
    }

    fn on_cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.pending.take().is_none() {
            self.selected.clear();
        }
        cx.notify();
    }
}

fn notice(result: &mail::BulkResult) -> Option<(String, bool)> {
    let mut parts = Vec::new();
    if !result.failures.is_empty() {
        parts.push(format!(
            "{} updated; {} failed: {}",
            result.succeeded.len(),
            result.failures.len(),
            result.failures[0].error
        ));
    }
    if result.narrowed > 0 {
        parts.push(format!(
            "{} stored subject(s) and preview(s) were redacted for good.",
            result.narrowed
        ));
    }
    (!parts.is_empty()).then(|| (parts.join(" "), !result.failures.is_empty()))
}

impl Render for Mail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(Palette::dark);
        if std::mem::take(&mut self.focus_next) {
            window.focus(&self.focus, cx);
        }
        let all: Vec<&MailProposal> = self.rows.value.iter().flatten().collect();
        let status_count = |s: StatusFilter| all.iter().filter(|m| s.matches(&m.status)).count();
        let category_count = |c: &str| {
            all.iter()
                .filter(|m| self.status.matches(&m.status) && m.stream == c)
                .count()
        };

        let statuses = div().flex().gap_1().children(StatusFilter::ALL.map(|s| {
            Button::new(SharedString::from(format!("mail-status-{}", s.label())))
                .small()
                .label(format!("{} {}", s.label(), status_count(s)))
                .when(self.status == s, |b| b.primary())
                .when(self.status != s, |b| b.ghost())
                .on_click(cx.listener(move |view, _, _, cx| view.set_status(s, cx)))
        }));
        let categories = div()
            .flex()
            .flex_wrap()
            .gap_1()
            .child(
                Button::new("mail-category-all")
                    .xsmall()
                    .label("All categories")
                    .when(self.category.is_none(), |b| b.primary())
                    .when(self.category.is_some(), |b| b.ghost())
                    .on_click(cx.listener(|view, _, _, cx| view.set_category(None, cx))),
            )
            .children(CATEGORIES.iter().map(|(key, label)| {
                let on = self.category == Some(*key);
                Button::new(SharedString::from(format!("mail-category-{key}")))
                    .xsmall()
                    .label(format!("{label} {}", category_count(key)))
                    .when(on, |b| b.primary())
                    .when(!on, |b| b.ghost())
                    .on_click(cx.listener(move |view, _, _, cx| view.set_category(Some(key), cx)))
            }));

        let targets = self.targets();
        let bar = (!self.selected.is_empty() || self.pending.is_some())
            .then(|| self.action_bar(&targets, palette, cx));

        let mut list = div()
            .key_context("MailList")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_previous))
            .on_action(cx.listener(Self::on_toggle_select))
            .on_action(cx.listener(Self::on_dismiss))
            .on_action(cx.listener(Self::on_archive))
            .on_action(cx.listener(Self::on_trash))
            .on_action(cx.listener(Self::on_toggle_waiting))
            .on_action(cx.listener(Self::on_open_in_gmail))
            .on_action(cx.listener(Self::on_confirm))
            .on_action(cx.listener(Self::on_cancel))
            .flex()
            .flex_col()
            .gap_1();
        if self.rows.value.is_none() && self.rows.loading {
            list = list.child(muted("Loading mail proposals…", palette));
        }
        if let Some(error) = &self.rows.error {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
            );
        }
        let visible: Vec<MailProposal> = self.visible().into_iter().cloned().collect();
        if visible.is_empty() && !self.rows.loading && self.rows.error.is_none() {
            list = list.child(muted("No mail in this view.", palette));
        }
        let total = visible.len();
        let mut category = None;
        for (i, proposal) in visible.into_iter().take(self.shown).enumerate() {
            if category.as_deref() != Some(proposal.stream.as_str()) {
                category = Some(proposal.stream.clone());
                list = list.child(
                    div()
                        .pt_3()
                        .pb_1()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(palette.muted))
                        .child(category_label(&proposal.stream).to_uppercase()),
                );
            }
            list = list.child(self.row(i, proposal, palette, cx));
        }
        if total > self.shown {
            list = list.child(
                Button::new("mail-show-more")
                    .small()
                    .ghost()
                    .label(format!("Show more ({} of {total} shown)", self.shown))
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.shown += PAGE;
                        cx.notify();
                    })),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(muted(
                "j/k move · x select · d dismiss · e archive · # trash · w waiting · o Gmail",
                palette,
            ))
            .child(statuses)
            .child(categories)
            .when_some(self.notice.clone(), |el, (text, failed)| {
                el.child(
                    div()
                        .text_sm()
                        .text_color(rgb(if failed { palette.red } else { palette.amber }))
                        .child(text),
                )
            })
            .children(bar)
            .child(list)
    }
}

impl Mail {
    fn action_bar(
        &self,
        targets: &[String],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some((action, ids)) = &self.pending {
            let question = action.confirmation(ids.len()).unwrap_or_default();
            return div()
                .flex()
                .items_center()
                .gap_2()
                .p_2()
                .rounded(px(6.))
                .bg(rgba(palette.raised))
                .child(div().flex_1().text_sm().child(question))
                .child(
                    Button::new("mail-cancel")
                        .small()
                        .ghost()
                        .label("Cancel · Esc")
                        .on_click(
                            cx.listener(|view, _, window, cx| view.on_cancel(&Cancel, window, cx)),
                        ),
                )
                .child(
                    Button::new("mail-confirm")
                        .small()
                        .primary()
                        .label("Confirm · ↩")
                        .on_click(
                            cx.listener(|view, _, window, cx| {
                                view.on_confirm(&Confirm, window, cx)
                            }),
                        ),
                )
                .into_any_element();
        }
        let none = targets.is_empty() || self.busy;
        let action =
            |id: &'static str, label: &'static str, action: MailAction, cx: &mut Context<Self>| {
                Button::new(id)
                    .small()
                    .ghost()
                    .label(label)
                    .disabled(none)
                    .on_click(cx.listener(move |view, _, _, cx| view.request(action.clone(), cx)))
            };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .p_2()
            .rounded(px(6.))
            .bg(rgba(palette.raised))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .mr_2()
                    .child(format!("{} selected", self.selected.len())),
            )
            .children(CATEGORIES.iter().map(|(key, label)| {
                Button::new(SharedString::from(format!("mail-set-{key}")))
                    .xsmall()
                    .ghost()
                    .label(format!("→ {label}"))
                    .disabled(none)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.request(MailAction::Categorize((*key).to_owned()), cx)
                    }))
            }))
            .child(action(
                "mail-bulk-dismiss",
                "Dismiss",
                MailAction::Dismiss,
                cx,
            ))
            .child(action(
                "mail-bulk-waiting",
                "Waiting",
                MailAction::Waiting,
                cx,
            ))
            .child(action(
                "mail-bulk-archive",
                "Archive",
                MailAction::Archive,
                cx,
            ))
            .child(action("mail-bulk-trash", "Trash", MailAction::Trash, cx))
            .into_any_element()
    }

    fn row(
        &self,
        i: usize,
        m: MailProposal,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chosen = self.cursor.as_deref() == Some(m.id.as_str());
        let picked = self.selected.contains(&m.id);
        let mut meta = vec![
            m.from_addr
                .clone()
                .unwrap_or_else(|| "Unknown sender".into()),
            m.internal_date
                .as_deref()
                .and_then(|d| d.get(..10))
                .unwrap_or_default()
                .to_owned(),
        ];
        if let Some(top) = m.relevance.first() {
            meta.push(format!("{} {:.2}", top.profile_label, top.score));
        }
        if m.waiting {
            meta.push("Waiting".into());
        }
        if let Some(sync) = m.gmail_sync_status.as_deref().filter(|s| *s != "synced") {
            meta.push(format!("Gmail sync: {sync}"));
        }
        if m.status != "proposed" {
            meta.push(m.status.clone());
        }
        let select = m.id.clone();
        let toggle = m.id.clone();
        div()
            .id(("mail-row", i))
            .flex()
            .gap_2()
            .px_3()
            .py_2()
            .rounded(px(6.))
            .when(chosen, |el| el.bg(rgba(palette.raised)))
            .hover(move |style| style.bg(rgba(palette.raised)))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.cursor = Some(select.clone());
                window.focus(&view.focus, cx);
                cx.notify();
            }))
            .child(
                div()
                    .id(("mail-pick", i))
                    .w(px(18.))
                    .text_sm()
                    .text_color(rgb(if picked { palette.text } else { palette.muted }))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        view.toggle(toggle.clone(), cx);
                    }))
                    .child(if picked { "☑" } else { "☐" }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(m.subject.clone().unwrap_or_else(|| "(No subject)".into())),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(meta.join(" · ")),
                    )
                    .when_some(m.snippet.clone(), |el, snippet| {
                        el.child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(snippet),
                        )
                    }),
            )
            .into_any_element()
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
    use gpui::TestAppContext;
    use sjel_gpui_core::mail::{BulkFailure, BulkResult};

    fn proposal(id: &str) -> MailProposal {
        MailProposal {
            id: id.into(),
            from_addr: None,
            subject: None,
            snippet: None,
            internal_date: None,
            stream: "aktiv".into(),
            status: "proposed".into(),
            gmail_sync_status: Some("synced".into()),
            gmail_sync_action: None,
            waiting: false,
            relevance: Vec::new(),
        }
    }

    /// Trash waits for a confirmation, holding the ids it will apply to, and sets nothing busy:
    /// no request has started. Dismiss asks nothing.
    #[gpui::test]
    fn archive_and_trash_ask_before_anything_is_sent(cx: &mut TestAppContext) {
        let mail = cx.new(|cx| {
            let mut mail = Mail::new(Repo::new(Err(ShellError::NotSetUp)), cx);
            mail.rows.value = Some(vec![proposal("a"), proposal("b")]);
            mail.cursor = Some("a".into());
            mail.selected = vec!["a".into(), "b".into()];
            mail
        });
        mail.update(cx, |mail, cx| {
            mail.request(MailAction::Trash, cx);
            let (action, ids) = mail.pending.clone().expect("trash asks first");
            assert_eq!(action, MailAction::Trash);
            assert_eq!(ids, ["a", "b"]);
            assert!(mail.notice.is_none() && !mail.busy);
        });
        mail.update(cx, |mail, cx| {
            mail.pending = None;
            // Dismiss does not ask: it goes straight to `run`, which needs a shell.
            mail.request(MailAction::Dismiss, cx);
            assert!(mail.pending.is_none());
        });
    }

    #[test]
    fn the_notice_says_what_failed_and_what_was_redacted() {
        assert_eq!(notice(&BulkResult::default()), None);
        let result = BulkResult {
            succeeded: vec!["a".into()],
            failures: vec![BulkFailure {
                id: "b".into(),
                error: "gmail refused".into(),
            }],
            narrowed: 2,
        };
        let (text, failed) = notice(&result).unwrap();
        assert!(failed);
        assert!(text.starts_with("1 updated; 1 failed: gmail refused"));
        assert!(text.contains("2 stored subject(s)"));
    }
}
