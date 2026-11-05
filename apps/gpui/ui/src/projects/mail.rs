//! Mail proposal review and target record selection for Projects.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{div, prelude::*, rgb, rgba, AnyElement, Context, FontWeight};

use crate::data::Remote;
use crate::theme::Palette;
use sjel_gpui_core::shell::{self, ProjectRecord, ShellError};

use super::Projects;

impl Projects {
    pub(super) fn load_project_mail(&mut self, more: bool, cx: &mut Context<Self>) {
        if self.mail.loading || self.mail_busy {
            return;
        }
        let Some((project, _)) = &self.project else {
            return;
        };
        let project = project.clone();
        let before = if more { self.mail_cursor } else { None };
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                let error = if self.repo.is_replica() {
                    ShellError::Store("Mail review needs an owner-authenticated local shell, which is unavailable on this device.".into())
                } else {
                    error
                };
                self.mail.settle(Err(error));
                cx.notify();
                return;
            }
        };
        self.mail.loading = true;
        self.mail.error = None;
        if !more {
            self.mail.value = None;
            self.mail_cursor = None;
            self.mail_more = false;
        }
        let work = shell::run(async move { shell.project_mail_proposals(before).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.project.as_ref().map(|p| p.0.as_str()) != Some(project.as_str()) {
                    return;
                }
                match result {
                    Ok(page) => {
                        view.mail_more = page.limit > 0 && page.proposals.len() == page.limit;
                        view.mail_cursor = page.proposals.last().map(|p| p.id);
                        let mut proposals = if more {
                            view.mail.value.take().unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        proposals.extend(
                            page.proposals
                                .into_iter()
                                .filter(|p| p.project == project && p.status == "pending"),
                        );
                        view.mail.settle(Ok(proposals));
                    }
                    Err(error) => view.mail.settle(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn select_mail_target(&mut self, path: String, cx: &mut Context<Self>) {
        if self.mail_busy || self.mail_target_loading {
            return;
        }
        let Some(proposal) = &self.mail_selected else {
            return;
        };
        let proposal_id = proposal.id;
        let project = proposal.project.clone();
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.mail_message = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        self.mail_target = None;
        self.mail_target_loading = true;
        self.mail_message = None;
        let work = shell::run(async move { shell.project_mail_target(&path).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.mail_target_loading = false;
                if view.project.as_ref().map(|p| p.0.as_str()) != Some(project.as_str())
                    || view.mail_selected.as_ref().map(|p| p.id) != Some(proposal_id)
                {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(target) => view.mail_target = Some(target),
                    Err(error) => {
                        view.mail_message = Some(format!("Could not read target: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn decide_project_mail(&mut self, approve: bool, cx: &mut Context<Self>) {
        if self.mail_busy
            || self.mail.loading
            || self.mail_target_loading
            || self.mail.error.is_some()
        {
            return;
        }
        let Some(proposal) = &self.mail_selected else {
            return;
        };
        let body = if approve {
            let Some(target) = &self.mail_target else {
                self.mail_message = Some("Select and review an existing target first.".into());
                cx.notify();
                return;
            };
            serde_json::json!({"version": proposal.version, "decision": "approve", "target_path": target.path, "base": target.content})
        } else {
            serde_json::json!({"version": proposal.version, "decision": "reject"})
        };
        let project = proposal.project.clone();
        let id = proposal.id;
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.mail_message = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        self.mail_busy = true;
        self.mail_message = None;
        let work = shell::run(async move { shell.project_mail_decision(id, body).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.mail_busy = false;
                if let Ok(outcome) = &result {
                    if let Some(path) = &outcome.target_path { view.repo.notes().forget(path); }
                }
                if view.project.as_ref().map(|p| p.0.as_str()) != Some(project.as_str()) { cx.notify(); return; }
                match result {
                    Ok(outcome) => {
                        view.mail_selected = None;
                        view.mail_target = None;
                        view.mail_message = Some(if outcome.applied && !outcome.rendered {
                            format!("Evidence was saved, but rendering failed: {}", outcome.render_error.as_deref().unwrap_or("unknown rendering error"))
                        } else if outcome.applied { "Evidence approved and saved. The suggested task was not created.".into() }
                        else if !approve { "Proposal rejected.".into() }
                        else { "The server did not report an applied approval. Refresh before reviewing again.".into() });
                        view.refresh_records(cx);
                        view.load_project_mail(false, cx);
                    }
                    Err(error @ ShellError::Refused(409, _)) => {
                        view.mail_selected = None;
                        view.mail_target = None;
                        view.mail_message = Some(format!("Decision refused: {error}. Refresh and review the proposal and target again."));
                        view.load_project_mail(false, cx);
                    }
                    Err(error) => {
                        view.mail_selected = None;
                        view.mail_target = None;
                        view.mail_message = Some(format!("Decision result unavailable: {error}. The server may have applied it; refresh before retrying."));
                        view.load_project_mail(false, cx);
                    },
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(super) fn render_project_mail(
        &self,
        records: &Remote<Vec<ProjectRecord>>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut body = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .child(div().text_lg().child("Mail review"));
        if self.mail.loading {
            body = body.child("Loading mail proposals…");
        }
        if let Some(error) = &self.mail.error {
            body = body
                .child(
                    div()
                        .text_color(rgb(palette.red))
                        .child(format!("Could not load mail proposals: {error}")),
                )
                .child(
                    Button::new("project-mail-retry")
                        .small()
                        .label("Retry")
                        .on_click(cx.listener(|view, _, _, cx| view.load_project_mail(false, cx))),
                );
        }
        if let Some(message) = &self.mail_message {
            body = body.child(div().text_sm().child(message.clone()));
        }
        if let Some(proposal) = &self.mail_selected {
            let mut detail = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .border_1()
                .border_color(rgba(palette.border))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(proposal.title.clone()),
                )
                .child(format!(
                    "Classification: {} · Version: {}",
                    proposal.data_class, proposal.version
                ))
                .child(format!("Source: {}", proposal.source_id))
                .child("Evidence (exact)")
                .child(proposal.evidence.clone())
                .child(format!(
                    "Task suggestion (not applied): {}",
                    proposal.task_suggestion.as_deref().unwrap_or("none")
                ));
            if let Some(url) = proposal.gmail_url() {
                detail = detail.child(
                    Button::new("project-mail-source")
                        .small()
                        .label("Open source in Gmail")
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            }
            detail = detail.child("Select an existing project record:");
            if records.loading {
                detail = detail.child("Loading project records…");
            }
            if let Some(error) = &records.error {
                detail = detail.child(format!("Could not load records: {error}"));
            }
            if let Some(rows) = &records.value {
                for record in rows {
                    let path = record.path.clone();
                    detail = detail.child(
                        Button::new(format!("mail-target-{path}"))
                            .small()
                            .label(format!("{} · {}", record.title, record.path))
                            .disabled(self.mail_busy || self.mail_target_loading)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.select_mail_target(path.clone(), cx)
                            })),
                    );
                }
                if rows.is_empty() {
                    detail = detail.child("No existing records are available.");
                }
            }
            if self.mail_target_loading {
                detail = detail.child("Loading the target's exact content…");
            }
            if let Some(target) = &self.mail_target {
                detail = detail
                    .child(format!(
                        "Selected target: {} · {}",
                        target.title, target.path
                    ))
                    .child("Exact current content (approval requires this unchanged):")
                    .child(target.content.clone())
                    .child(
                        Button::new("project-mail-approve")
                            .small()
                            .primary()
                            .label("Confirm approval and append evidence")
                            .disabled(
                                self.mail_busy || self.mail.loading || self.mail.error.is_some(),
                            )
                            .on_click(
                                cx.listener(|view, _, _, cx| view.decide_project_mail(true, cx)),
                            ),
                    );
            }
            detail = detail
                .child(
                    Button::new("project-mail-reject")
                        .small()
                        .label("Reject proposal")
                        .disabled(self.mail_busy || self.mail.loading || self.mail.error.is_some())
                        .on_click(
                            cx.listener(|view, _, _, cx| view.decide_project_mail(false, cx)),
                        ),
                )
                .child(
                    Button::new("project-mail-close")
                        .small()
                        .ghost()
                        .label("Close review")
                        .disabled(self.mail_busy || self.mail_target_loading)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.mail_selected = None;
                            view.mail_target = None;
                            cx.notify();
                        })),
                );
            body = body.child(detail);
        }
        if let Some(proposals) = &self.mail.value {
            if proposals.is_empty() {
                body = body.child(if self.mail_more {
                    "No proposals for this project on the pages loaded so far."
                } else {
                    "No mail proposals need review."
                });
            }
            for proposal in proposals {
                let selected = proposal.clone();
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(format!("{} · {}", proposal.title, proposal.data_class))
                        .child(
                            Button::new(format!("project-mail-review-{}", proposal.id))
                                .small()
                                .label("Review")
                                .disabled(self.mail_busy || self.mail_target_loading)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.mail_selected = Some(selected.clone());
                                    view.mail_target = None;
                                    view.mail_message = None;
                                    cx.notify();
                                })),
                        ),
                );
            }
            if self.mail_more {
                body = body.child(
                    Button::new("project-mail-more")
                        .small()
                        .label("Load older proposals")
                        .disabled(self.mail.loading || self.mail_busy)
                        .on_click(cx.listener(|view, _, _, cx| view.load_project_mail(true, cx))),
                );
            }
        }
        div()
            .id("project-mail-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(body)
            .into_any_element()
    }
}
