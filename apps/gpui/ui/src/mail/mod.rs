//! Mail triage, the Feed page's Mail tab: the proposals a Gmail sweep made, grouped by category.
//!
//! Every write goes through `POST /comms/triage/bulk`, for the selection or the row under the
//! cursor. Archive, trash and a class-raising category ask first (`MailAction::confirmation`).
//! Sweeps, reconciling and classification passes stay in the dashboard for now.

pub mod actions;
pub mod bar;
pub mod row;

pub use actions::*;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, rgb, Context, FocusHandle, FontWeight, SharedString, Window};
use sjel_gpui_core::mail::{
    self, category_label, MailAction, MailProposal, StatusFilter, CATEGORIES,
};
use sjel_gpui_core::shell::{self, ShellError};

use crate::data::{Remote, Repo};
use crate::theme::Palette;

/// How many rows render before "Show more".
// ponytail: a capped plain list; 821 pending threads made a full render the slow path. Move to
// `uniform_list` (fixed-height rows) when scrolling past the cap becomes routine.
const PAGE: usize = 200;

pub struct Mail {
    pub repo: Repo,
    pub palette: Option<Palette>,
    pub density: crate::theme::Density,
    pub opened_thread: Option<String>,
    pub rows: Remote<Vec<MailProposal>>,
    pub status: StatusFilter,
    pub category: Option<&'static str>,
    pub cursor: Option<String>,
    pub selected: Vec<String>,
    /// An action waiting for Enter, with the ids it applies to.
    pub(crate) pending: Option<(MailAction, Vec<String>)>,
    pub(crate) busy: bool,
    /// The outcome of the last write: an error, partial failures, or redactions.
    pub notice: Option<(String, bool)>,
    pub(crate) shown: usize,
    pub(crate) focus: FocusHandle,
    pub(crate) focus_next: bool,
}

impl Mail {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            palette: None,
            density: crate::theme::Density::Desktop,
            opened_thread: None,
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

    pub fn set_density(&mut self, density: crate::theme::Density, cx: &mut Context<Self>) {
        if self.density != density {
            self.density = density;
            cx.notify();
        }
    }

    pub fn set_repo(&mut self, repo: Repo, cx: &mut Context<Self>) {
        self.repo = repo;
        self.load(cx);
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

    pub(crate) fn find(&self, id: &str) -> Option<&MailProposal> {
        self.rows.value.iter().flatten().find(|m| m.id == id)
    }

    /// The threads an action applies to: the selection, else the row under the cursor. Threads
    /// with a Gmail write in flight are left out.
    pub(crate) fn targets(&self) -> Vec<String> {
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
    pub(crate) fn request(&mut self, action: MailAction, cx: &mut Context<Self>) {
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

    /// Runs `action` on a single specific item (used by mobile triage drawer).
    pub(crate) fn direct_action(&mut self, id: String, action: MailAction, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if action.confirmation(1).is_some() {
            self.pending = Some((action, vec![id]));
            cx.notify();
        } else {
            self.run(action, vec![id], cx);
        }
    }

    pub(crate) fn run(&mut self, action: MailAction, ids: Vec<String>, cx: &mut Context<Self>) {
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

    /// Retries or cancels a Gmail action that stopped after its automatic retries.
    pub(crate) fn gmail_job(&mut self, id: String, decision: &'static str, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        if self.busy {
            return;
        }
        self.busy = true;
        let work = shell::run(async move { shell.mail_gmail_job(&id, decision).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                view.notice = Some(match result {
                    Ok(()) if decision == "retry" => ("Gmail action retried.".into(), false),
                    Ok(()) => ("Gmail action canceled.".into(), false),
                    Err(error) => (format!("Not applied: {error}"), true),
                });
                view.load(cx);
            });
        })
        .detach();
        cx.notify();
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

    pub(crate) fn toggle(&mut self, id: String, cx: &mut Context<Self>) {
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

    pub(crate) fn on_confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((action, ids)) = self.pending.take() {
            self.run(action, ids, cx);
        }
    }

    pub(crate) fn on_cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.pending.take().is_none() {
            self.selected.clear();
        }
        cx.notify();
    }
}

pub(crate) fn notice(result: &mail::BulkResult) -> Option<(String, bool)> {
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

fn muted(text: &str, palette: Palette) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(text.to_owned())
}

impl gpui_kit::Render for Mail {
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

        let density_selector = div().flex().items_center().gap_1().children(
            [
                crate::theme::Density::Compact,
                crate::theme::Density::Desktop,
                crate::theme::Density::Comfortable,
            ]
            .into_iter()
            .map(|d| {
                let active = self.density == d;
                Button::new(SharedString::from(format!("mail-density-{}", d.label())))
                    .xsmall()
                    .ghost()
                    .when(active, |b| b.primary())
                    .label(d.label())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.set_density(d, cx);
                    }))
            }),
        );

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(muted(
                        "j/k move · x select · d dismiss · e archive · # trash · w waiting · o Gmail",
                        palette,
                    ))
                    .child(density_selector),
            )
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
            authenticity: None,
            model: None,
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
